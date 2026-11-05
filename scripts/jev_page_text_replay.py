"""Opt-in A/B replay of frozen TodoMVC observations: per-candidate vs shared page text.

The original real-browser pilot embedded the full page text in every candidate's
context. This replay sends Jev the same oracle-verified observations in both that
legacy layout and the shared layout (page text once in ``state.page_text``),
interleaved per observation, so payload layout is the only varied input.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from pathlib import Path
from typing import Any

from _jev_browser_workflow import public_candidates
from _jev_desktop_models import DesktopJudges, _payload
from jev_ui_experiment import dump

ROOT = Path(__file__).resolve().parents[1]
THRESHOLD = 0.8
LEGACY_PREFIX = "page text: "
FORMATS = ("legacy", "shared")
STAGE_DIRS = {"add": "add-first", "complete": "complete-first", "filter_active": "filter-active"}


def split_legacy(candidates: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], str]:
    """Remove the repeated page text from legacy contexts and return it once."""
    texts = set()
    shared = []
    for candidate in candidates:
        head, separator, text = candidate["context"].partition(LEGACY_PREFIX)
        if not separator:
            raise ValueError("Legacy candidate context lacks page text")
        texts.add(text)
        shared.append({**candidate, "context": head.removesuffix("\n")})
    if len(texts) != 1:
        raise ValueError("Observation candidates disagree on page text")
    return shared, texts.pop()


def observations(pilot: Path) -> list[dict[str, Any]]:
    """Measured (non-warm-up) observations whose executed target passed the oracle."""
    found = []
    for run in sorted(pilot.glob("pair-*-*")):
        result = json.loads((run / "result.json").read_text())
        for step in result["steps"]:
            if step.get("target_correct") is not True:
                raise ValueError(f"{run.name} {step['stage']} has no verified target")
            folder = run / STAGE_DIRS[step["stage"]]
            legacy = json.loads((folder / "candidates.json").read_text())
            shared, page_text = split_legacy(legacy)
            found.append(
                {
                    "name": f"{run.name}/{folder.name}",
                    "stage": step["stage"],
                    "intent": step["intent"],
                    "correct": step["choice"],
                    "legacy": legacy,
                    "shared": shared,
                    "page_text": page_text,
                }
            )
    return found


def payload_for(observation: dict[str, Any], layout: str) -> dict[str, Any]:
    if layout == "legacy":
        return _payload(observation["intent"], public_candidates(observation["legacy"]))
    return _payload(
        observation["intent"],
        public_candidates(observation["shared"]),
        observation["page_text"],
    )


def summarize(records: list[dict[str, Any]]) -> dict[str, Any]:
    """Per-layout and per-stage accuracy, gated acceptance, tokens, and latency."""

    def stats(rows: list[dict[str, Any]]) -> dict[str, Any]:
        accepted = [row for row in rows if row["accepted"]]
        tokens = [row["input_tokens"] for row in rows if row["input_tokens"] is not None]
        return {
            "calls": len(rows),
            "raw_correct": sum(row["choice"] == row["correct"] for row in rows),
            "accepted": len(accepted),
            "accepted_correct": sum(row["choice"] == row["correct"] for row in accepted),
            "wrong_accepted": sum(row["choice"] != row["correct"] for row in accepted),
            "mean_request_bytes": (
                statistics.fmean(row["request_bytes"] for row in rows) if rows else None
            ),
            "mean_input_tokens": statistics.fmean(tokens) if tokens else None,
            "median_elapsed_ms": (
                statistics.median(row["elapsed_ms"] for row in rows) if rows else None
            ),
            "probabilities": [row["probability"] for row in rows],
        }

    summary: dict[str, Any] = {}
    for layout in FORMATS:
        rows = [row for row in records if row["layout"] == layout]
        summary[layout] = {
            "overall": stats(rows),
            "stages": {
                stage: stats([row for row in rows if row["stage"] == stage]) for stage in STAGE_DIRS
            },
        }
    legacy, shared = summary["legacy"]["overall"], summary["shared"]["overall"]
    summary["passed"] = (
        shared["wrong_accepted"] == 0
        and shared["accepted_correct"] >= legacy["accepted_correct"] - 1
    )
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pilot", type=Path, default=ROOT / "artifacts/jev-real-browser/pilot")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=3)
    args = parser.parse_args()
    out = args.out.resolve()
    if (out / "frozen.json").exists():
        raise FileExistsError("Fresh output directory required")
    out.mkdir(parents=True, exist_ok=True)
    observed = observations(args.pilot.resolve())
    dump(
        out / "frozen.json",
        {
            "pilot": str(args.pilot.resolve()),
            "observations": [o["name"] for o in observed],
            "repeats": args.repeats,
            "threshold": THRESHOLD,
            "order": "per observation, alternate which layout is called first",
            "gate": "shared: zero wrong accepted; accepted-correct >= legacy accepted-correct - 1",
            "source_hashes": {
                str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in [
                    Path(__file__),
                    ROOT / "scripts/_jev_browser_workflow.py",
                    ROOT / "scripts/_jev_desktop_models.py",
                ]
            },
        },
    )
    records = []
    with DesktopJudges(out) as judges:
        for repeat in range(args.repeats):
            for index, observation in enumerate(observed):
                order = FORMATS if (repeat + index) % 2 == 0 else FORMATS[::-1]
                for layout in order:
                    folder = out / f"r{repeat}" / observation["name"].replace("/", "--") / layout
                    payload = payload_for(observation, layout)
                    result = judges.select(
                        "jev",
                        observation["intent"],
                        public_candidates(observation[layout]),
                        folder,
                        page_text=observation["page_text"] if layout == "shared" else None,
                    )
                    probability = result.get("probability")
                    record = {
                        "repeat": repeat,
                        "observation": observation["name"],
                        "stage": observation["stage"],
                        "layout": layout,
                        "correct": observation["correct"],
                        "choice": result["choice"],
                        "probability": probability,
                        "accepted": result["choice"] != "abstain"
                        and probability is not None
                        and probability >= THRESHOLD,
                        "request_bytes": len(json.dumps(payload)),
                        "input_tokens": (result.get("usage") or {}).get("input_tokens"),
                        "elapsed_ms": result["elapsed_ms"],
                    }
                    records.append(record)
                    dump(out / "records.json", records)
    summary = summarize(records)
    dump(out / "summary.json", summary)
    print(json.dumps({layout: summary[layout]["overall"] for layout in FORMATS}, indent=2))
    return 0 if summary["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
