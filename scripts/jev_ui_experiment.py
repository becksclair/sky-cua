"""Opt-in, local Jev UI feasibility experiment; never used by runtime or CI.

Freeze first, inspect frozen inputs, then explicitly run paid API comparisons.
Jev uses the external existing research helper; Luna uses the current judge route.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import statistics
import time
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from _agent_perf_judge import (
    DEFAULT_JUDGE_MODEL,
    DEFAULT_JUDGE_REASONING_EFFORT,
    prepare_judge_codex_home,
)
from _codex_exec import read_last_message, run_codex_exec

ROOT = Path(__file__).resolve().parents[1]
HELPER = Path.home() / ".agents/research/jev-client/jev.py"
THRESHOLD = 0.9


def dump(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def question(case: dict[str, Any]) -> dict[str, Any]:
    task = case["task"]
    if task == "outcome":
        instruction = (
            "Classify evidence for the intended operation, not whether a tool call returned OK. "
            "A stale banner is not current success. Missing, truncated or contradictory evidence "
            "is insufficient. Distinguish explicit in-progress work from unknown outcomes."
        )
        criteria = {
            "saved": "Current operation completed and the intended change is persisted.",
            "rejected": "Current operation explicitly rejected or failed validation.",
            "pending": "Current operation explicitly still in progress.",
            "insufficient": "Evidence does not establish any of the other outcomes.",
        }
    elif task == "ranking":
        instruction = (
            "Select the unique observed actionable candidate matching the intent. "
            "Respect parent context and disabled state. Return abstain for absent targets, "
            "ambiguous duplicates, or requests that require more than one target. "
            "Never invent IDs or coordinates."
        )
        criteria = {
            c["id"]: f"Select observed candidate {c['id']}" for c in case["state"]["candidates"]
        }
        criteria["abstain"] = "No unique actionable candidate satisfies the intent."
    else:
        instruction = (
            "Judge whether the visible episode supports its completion/recovery claim. "
            "An acknowledged action or model claim alone does not prove outcome. "
            "Use successful observation payloads as well as errors."
        )
        criteria = {
            "supported": "Observed evidence establishes the claim.",
            "contradicted": "Observed evidence contradicts the claim.",
            "insufficient": "Insufficient evidence to establish or contradict the claim.",
        }
    return {"type": "choice", "instructions": instruction, "criteria": criteria}


def freeze(out: Path) -> None:
    from _jev_ui_fixtures import build_cases

    cases = build_cases()
    groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for c in cases:
        groups[f"{c['task']}-{c['family']}"].append(c)
    batches = []
    for name, members in groups.items():
        questions = {
            f"q{i}": {
                **question(c),
                "instructions": f"For state case c{i} only: " + question(c)["instructions"],
            }
            for i, c in enumerate(members)
        }
        batches.append(
            {
                "name": name,
                "ids": [c["id"] for c in members],
                "payload": {
                    "state": {f"c{i}": c["state"] for i, c in enumerate(members)},
                    "questions": questions,
                },
            }
        )
    repeat_ids = {
        "outcome-04-07",
        "outcome-04-08",
        "ranking-04-02",
        "ranking-04-03",
        "annotation-02-03",
        "annotation-04-03",
    }
    members = [c for c in cases if c["id"] in repeat_ids]
    for repetition in range(2):
        batches.append(
            {
                "name": f"boundary-repeat-{repetition + 1}",
                "ids": [c["id"] for c in members],
                "repeat": True,
                "payload": {
                    "state": {f"c{i}": c["state"] for i, c in enumerate(members)},
                    "questions": {
                        f"q{i}": {
                            **question(c),
                            "instructions": f"For state case c{i} only: "
                            + question(c)["instructions"],
                        }
                        for i, c in enumerate(members)
                    },
                },
            }
        )
    manifest = {
        "version": 1,
        "cases": cases,
        "batches": batches,
        "jev_model": "jev-1.13",
        "luna_model": DEFAULT_JUDGE_MODEL,
        "luna_effort": DEFAULT_JUDGE_REASONING_EFFORT,
        "jev_probability_threshold": THRESHOLD,
        "protocol": {
            "no_tuning": True,
            "order": "alternate Jev/Luna by batch index",
            "gate": "zero false-success/wrong-target; >=80% decidable coverage; >=20% replay time reduction",
            "cost": "Jev provider-billed; Luna usage only, subscription billing unavailable",
            "workflow": "offline batch decision replay; no claim of live agent speedup",
        },
    }
    if (out / "frozen.json").exists():
        raise FileExistsError(
            "Refusing to overwrite frozen inputs; choose a fresh output directory"
        )
    dump(out / "frozen.json", manifest)
    (out / "frozen.sha256").write_text(
        hashlib.sha256((out / "frozen.json").read_bytes()).hexdigest() + "\n"
    )
    print(f"Frozen {len(cases)} cases in {len(batches)} batches", flush=True)


def freeze_single(out: Path) -> None:
    """Exploratory batching ablation; reuses cases, never adds accuracy samples."""
    source = json.loads((out.parent / "frozen.json").read_text())
    selected = [
        c
        for c in source["cases"]
        if c["task"] == "ranking" and c["split"] == "heldout" and c["id"].endswith(("-01", "-02"))
    ]
    source["cases"] = selected
    source["batches"] = [
        {
            "name": c["id"],
            "ids": [c["id"]],
            "payload": {"state": c["state"], "questions": {"q0": question(c)}},
        }
        for c in selected
    ]
    source["protocol"]["exploratory_ablation"] = (
        "Same exact and semantic heldout ranking cases, one case per call; no new independent examples; frozen threshold unchanged."
    )
    if (out / "frozen.json").exists():
        raise FileExistsError("Refusing to overwrite frozen inputs")
    dump(out / "frozen.json", source)
    (out / "frozen.sha256").write_text(
        hashlib.sha256((out / "frozen.json").read_bytes()).hexdigest() + "\n"
    )
    print(f"Frozen {len(selected)} repeated single-case probes", flush=True)


def jev_client(*, experimental_images: bool = False) -> Any:
    spec = importlib.util.spec_from_file_location("jev_external", HELPER)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.JevClient(experimental_images=experimental_images)


def run(out: Path) -> None:
    if (out / "records.json").exists():
        raise FileExistsError(
            "Refusing to overwrite measured results; choose a fresh output directory"
        )
    raw = (out / "frozen.json").read_bytes()
    assert hashlib.sha256(raw).hexdigest() == (out / "frozen.sha256").read_text().strip()
    manifest = json.loads(raw)
    records = []
    client = jev_client()
    try:
        home = prepare_judge_codex_home(out)
        for bi, batch in enumerate(manifest["batches"]):
            for route in ["jev", "luna"] if bi % 2 == 0 else ["luna", "jev"]:
                start = time.perf_counter()
                record: dict[str, Any] = {
                    "batch": batch["name"],
                    "route": route,
                    "ids": batch["ids"],
                }
                try:
                    if route == "jev":
                        response = client.call({"model": manifest["jev_model"], **batch["payload"]})
                        record.update(response=response, api_ms=client.last_elapsed_ms)
                        record["predictions"] = {
                            k: v.get("choice") for k, v in response["answers"].items()
                        }
                        record["gated"] = {
                            k: v.get("choice")
                            if v.get("probabilities", {}).get(v.get("choice"), 0) >= THRESHOLD
                            else "abstain"
                            for k, v in response["answers"].items()
                        }
                    else:
                        folder = out / "luna" / batch["name"]
                        folder.mkdir(parents=True, exist_ok=True)
                        schema = {
                            "type": "object",
                            "properties": {
                                k: {"type": "string", "enum": list(v["criteria"])}
                                for k, v in batch["payload"]["questions"].items()
                            },
                            "required": list(batch["payload"]["questions"]),
                            "additionalProperties": False,
                        }
                        schema_path = folder / "schema.json"
                        dump(schema_path, schema)
                        result = run_codex_exec(
                            prompt="Do not use tools. Answer each question using only its designated case. Return JSON.\n"
                            + json.dumps(batch["payload"]),
                            artifact_dir=folder,
                            output_schema=schema_path,
                            model=manifest["luna_model"],
                            reasoning_effort=manifest["luna_effort"],
                            extra_env={"CODEX_HOME": str(home)},
                            extra_args=["--sandbox", "read-only"],
                        )
                        if result.exit_code != 0 or not result.last_message_path.exists():
                            raise RuntimeError(f"Luna exit {result.exit_code}; see local trace")
                        record["predictions"] = read_last_message(result.last_message_path)
                        events = [
                            json.loads(line)
                            for line in result.transcript_path.read_text().splitlines()
                            if line.strip()
                        ]
                        record["usage"] = [
                            e.get("usage") for e in events if e.get("type") == "turn.completed"
                        ]
                    record["status"] = "ok"
                except Exception as exc:
                    record["status"] = "error"
                    record["error_type"] = type(exc).__name__
                    # No exception body: external libraries can include request credentials.
                record["elapsed_ms"] = (time.perf_counter() - start) * 1000
                records.append(record)
                dump(out / "records.json", records)
                print(
                    batch["name"], route, record["status"], round(record["elapsed_ms"]), flush=True
                )
                if record["status"] == "error":
                    print("Prerequisite failure; stopping paired run.", flush=True)
                    return
    finally:
        client.close()
        (out / "judge-codex-home" / "auth.json").unlink(missing_ok=True)


def summarize(out: Path) -> None:
    manifest = json.loads((out / "frozen.json").read_text())
    records = json.loads((out / "records.json").read_text())
    cases = {c["id"]: c for c in manifest["cases"]}
    summary: dict[str, Any] = {}
    for route in ["rules", "jev", "jev-gated", "luna"]:
        for task in ["outcome", "ranking", "annotation"]:
            for split in ["development", "heldout"]:
                selected = [c for c in cases.values() if c["task"] == task and c["split"] == split]
                matrix: Counter[str] = Counter()
                errors = 0
                for case in selected:
                    pred = case["rules_prediction"]
                    if route != "rules":
                        rec = next(
                            (
                                r
                                for r in records
                                if case["id"] in r["ids"] and r["route"] == route.split("-")[0]
                            ),
                            None,
                        )
                        if not rec or rec["status"] != "ok":
                            errors += 1
                            continue
                        key = "q" + str(rec["ids"].index(case["id"]))
                        pred = rec["gated" if route == "jev-gated" else "predictions"].get(
                            key, "invalid"
                        )
                    matrix[case["expected"] + " -> " + str(pred)] += 1
                summary[f"{route}/{task}/{split}"] = {
                    "n": len(selected),
                    "api_missing_or_errors": errors,
                    "matrix": dict(matrix),
                }
    timing = {}
    for route in ["jev", "luna"]:
        rr = [r for r in records if r["route"] == route]
        times = sorted(r["elapsed_ms"] for r in rr)
        timing[route] = {
            "batches": len(rr),
            "total_ms": sum(times),
            "p50_ms": statistics.median(times) if times else None,
            "p95_ms": times[math.ceil(0.95 * len(times)) - 1] if times else None,
            "provider_billed_usd": sum(
                r.get("response", {}).get("usage", {}).get("cost", 0) for r in rr
            )
            if route == "jev"
            else None,
        }
    dump(out / "summary.json", {"classification": summary, "timing": timing})
    print(json.dumps(timing, indent=2))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["freeze", "freeze-single", "run", "summarize"])
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/jev-ui")
    args = parser.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    {"freeze": freeze, "freeze-single": freeze_single, "run": run, "summarize": summarize}[
        args.mode
    ](args.out)


if __name__ == "__main__":
    main()
