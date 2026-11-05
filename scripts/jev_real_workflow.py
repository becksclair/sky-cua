"""Opt-in KWrite edit/save comparison with independent target and exact-byte proof.

The caller fixes each intent before either model sees the native observation.
Run ``dry`` for an oracle-driven desktop preflight without provider calls.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import subprocess
import time
from pathlib import Path
from typing import Any

from _jev_desktop_models import DesktopJudges
from jev_desktop_pilot import Desktop, public_candidates
from jev_ui_experiment import dump
from live_desktop_smoke import appshot_action_fences, grouped_structured_result

ROOT = Path(__file__).resolve().parents[1]
THRESHOLD = 0.8
PAIR_COUNT = 3
MIN_SAVINGS = 0.2
STEPS = [
    {
        "id": "editor",
        "intent": "Replace the text of the current document in the main editing area.",
        "operation": "set_value",
    },
    {
        "id": "save",
        "intent": "Save the edited current document.",
        "operation": "press_key",
        "key": "Ctrl+S",
        "deterministic": True,
    },
]
# Evaluation only: never included in candidates or judge requests.
ORACLE = {"editor": {"labels": {"source.txt"}, "roles": {"text"}, "state_flags": {"editable"}}}


def candidates_for(snapshot: dict[str, Any]) -> list[dict[str, Any]]:
    """Project native accessibility data without inventing names or widget roles."""
    allowed = {
        "menu",
        "menu item",
        "push button",
        "button",
        "text",
        "entry",
        "combo box",
        "editable text",
    }
    result = []
    elements = snapshot["elements"]
    by_index = {e["element_index"]: e for e in elements}
    for e in elements:
        flags = e.get("state_flags", [])
        bounds = e.get("bounds") or {}
        if (
            e["role"] not in allowed
            or "showing" not in flags
            or "hidden" in flags
            or bounds.get("width", 0) <= 1
            or bounds.get("height", 0) <= 1
        ):
            continue
        parent = by_index.get(e.get("parent_index"), {})
        result.append(
            {
                "id": f"e{e['element_index']}",
                "element_index": e["element_index"],
                "backend_ref": e["backend_ref"],
                "label": e.get("name") or "",
                "role": e["role"],
                "context": parent.get("name") or e.get("description") or "",
                "description": e.get("description") or "",
                "state_flags": flags,
                "bounds": bounds,
                "disabled": "enabled" not in flags,
                "visible": True,
            }
        )
    return result


def target_correct(step_id: str, candidate: dict[str, Any] | None) -> bool:
    if candidate is None or candidate["disabled"] or not candidate["visible"]:
        return False
    label = candidate["label"]
    expectation = ORACLE[step_id]
    return (
        label in expectation["labels"]
        and candidate["role"] in expectation["roles"]
        and expectation["state_flags"].issubset(candidate.get("state_flags", []))
    )


def select_target(
    judges: Any,
    step: dict[str, Any],
    candidates: list[dict[str, Any]],
    route: str,
    folder: Path,
    record: dict[str, Any],
    st: dict[str, Any],
    persist: Any,
    *,
    dry: bool = False,
) -> str:
    choice = None
    st["source"] = "pending"
    if dry and choice is None:
        matches = [c for c in candidates if target_correct(step["id"], c)]
        if len(matches) != 1:
            raise RuntimeError("Preflight oracle needs one unique native target")
        choice = matches[0]["id"]
        st["source"] = "preflight_oracle"
    if choice is None and route == "hybrid":
        st.update(status="selecting", pending_model="jev")
        record["jev_calls"] += 1
        persist()
        st["jev"] = judges.select(
            "jev", step["intent"], public_candidates(candidates), folder / "jev"
        )
        probability = st["jev"].get("probability", 0)
        if (
            st["jev"]["choice"] != "abstain"
            and isinstance(probability, (int, float))
            and not isinstance(probability, bool)
            and math.isfinite(probability)
            and THRESHOLD <= probability <= 1
        ):
            choice = st["jev"]["choice"]
            st["source"] = "jev"
        persist()
    if choice is None:
        st.update(status="selecting", pending_model="terra")
        record["terra_calls"] += 1
        persist()
        st["terra"] = judges.select(
            "terra", step["intent"], public_candidates(candidates), folder / "terra"
        )
        choice = st["terra"]["choice"]
        st["source"] = "terra"
    st.pop("pending_model", None)
    st["choice"] = choice
    persist()
    return choice


def file_proof(source: Path, expected: bytes, initial: bytes) -> dict[str, Any]:
    actual = source.read_bytes() if source.is_file() else None
    return {
        "expected_sha256": hashlib.sha256(expected).hexdigest(),
        "expected_bytes": len(expected),
        "initial_sha256": hashlib.sha256(initial).hexdigest(),
        "source_exists": actual is not None,
        "source_matches": actual == expected,
        "source_changed": actual is not None and actual != initial,
        "source_sha256": hashlib.sha256(actual).hexdigest() if actual is not None else None,
        "source_bytes": len(actual) if actual is not None else None,
    }


def run_flow(
    d: Desktop, judges: Any, folder: Path, route: str, dry: bool = False
) -> dict[str, Any]:
    if route not in {"baseline", "hybrid"}:
        raise ValueError("Unknown workflow route")
    folder.mkdir(parents=True, exist_ok=False)
    source = folder / "source.txt"
    initial = f"Disposable KWrite input: {folder.name}\nReplace this text.\n".encode()
    expected = (
        f"Replaced KWrite workflow proof: {folder.name}\nSaved through native input.\n".encode()
    )
    record: dict[str, Any] = {
        "route": route,
        "dry": dry,
        "steps": [],
        "terra_calls": 0,
        "jev_calls": 0,
        "wrong_targets": 0,
        "status": "setup",
        "success": False,
    }
    proc = None
    started = None
    active_step = None

    def persist() -> None:
        dump(folder / "progress.json", record)

    persist()
    try:
        source.write_bytes(initial)
        env = dict(os.environ, QT_LINUX_ACCESSIBILITY_ALWAYS_ON="1", QT_ACCESSIBILITY="1")
        for key, name in (
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
        ):
            profile = folder / name
            profile.mkdir(mode=0o700)
            env[key] = str(profile)
        proc = subprocess.Popen(
            ["kwrite", str(source)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
        matches: list[dict[str, Any]] = []
        for _ in range(60):
            windows = grouped_structured_result(
                d.call("list_resources", {"surface": "desktop", "resource": "windows"})
            )["windows"]
            matches = [w for w in windows if w.get("pid") == proc.pid]
            if len(matches) == 1:
                break
            time.sleep(0.2)
        if len(matches) != 1:
            raise RuntimeError("No unique owned KWrite window")
        wid = matches[0]["window_id"]
        d.call("activate_window", {"window_id": wid})
        record["status"] = "running"
        for step in STEPS:
            st: dict[str, Any] = {
                "step": step["id"],
                "intent": step["intent"],
                "operation": step["operation"],
                "status": "observing",
            }
            active_step = st
            record["steps"].append(st)
            sd = folder / step["id"]
            sd.mkdir()
            persist()
            step_started = time.perf_counter()
            if started is None:
                started = time.perf_counter()
            snapshot, raw = d.observe(wid)
            dump(sd / "observation.json", raw)
            candidates = candidates_for(snapshot)
            dump(sd / "candidates.json", candidates)
            selected = None
            if not step.get("deterministic"):
                choice = select_target(
                    judges, step, candidates, route, sd, record, st, persist, dry=dry
                )
                selected = next((c for c in candidates if c["id"] == choice), None)
                st["target_correct"] = target_correct(step["id"], selected)
                st["selected"] = selected
                if not st["target_correct"]:
                    record["wrong_targets"] += 1
                    persist()
                    raise RuntimeError(f"Wrong target for {step['id']}: {choice!r}")
            else:
                st["source"] = "deterministic_keyboard"
            st["status"] = "revalidating"
            persist()
            d.call("activate_window", {"window_id": wid})
            fresh, fresh_raw = d.observe(wid)
            dump(sd / "pre-action.json", fresh_raw)
            current = []
            if selected is not None:
                current = [
                    c
                    for c in candidates_for(fresh)
                    if c["backend_ref"] == selected["backend_ref"]
                    and c["label"] == selected["label"]
                    and c["role"] == selected["role"]
                    and target_correct(step["id"], c)
                ]
                if len(current) != 1:
                    raise RuntimeError("Target identity changed")
            dump(
                sd / "focused-before-action.json",
                d.call("list_resources", {"surface": "desktop", "resource": "focused_window"}),
            )
            action_args = dict(appshot_action_fences(fresh))
            if step["operation"] == "set_value":
                tool = "desktop_set_value"
                action_args.update(
                    element_index=current[0]["element_index"], value=expected.decode()
                )
            else:
                tool = "desktop_keyboard"
                action_args.update(operation="press_key", key=step["key"])
            st.update(status="acting", action={"tool": tool, "arguments": action_args})
            persist()
            result = d.call(tool, action_args)
            dump(sd / "action.json", result)
            st["status"] = "observing_result"
            persist()
            _post, post_raw = d.observe(wid)
            dump(sd / "post-action.json", post_raw)
            st.update(status="ok", total_ms=(time.perf_counter() - step_started) * 1000)
            persist()
        deadline = time.perf_counter() + 5
        proof = file_proof(source, expected, initial)
        while not proof["source_matches"] and time.perf_counter() < deadline:
            time.sleep(0.05)
            proof = file_proof(source, expected, initial)
        dump(folder / "file-proof.json", proof)
        assert started is not None
        record["total_ms"] = (time.perf_counter() - started) * 1000
        if not proof["source_matches"] or not proof["source_changed"]:
            raise RuntimeError("Exact-byte edit/save file proof failed")
        record.update(status="ok", success=True)
    except Exception as exc:
        (folder / "error.txt").write_text(str(exc))
        record.update(status="error", success=False, error=str(exc))
        if active_step is not None and active_step["status"] != "ok":
            active_step.update(status="error", error=str(exc))
        if started is not None and "total_ms" not in record:
            record["total_ms"] = (time.perf_counter() - started) * 1000
        persist()
    finally:
        if proc is not None:
            try:
                if proc.poll() is None:
                    proc.terminate()
                    try:
                        proc.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        proc.kill()
                        proc.wait(timeout=5)
            except Exception as exc:
                record.update(status="error", success=False, cleanup_error=str(exc))
        persist()
        dump(folder / "result.json", record)
    print(json.dumps({k: v for k, v in record.items() if k != "steps"}), flush=True)
    return record


def paired_summary(records: list[dict[str, Any]]) -> dict[str, Any]:
    pairs = []
    for pair in range(PAIR_COUNT):
        indexed = {r["route"]: r for r in records if r.get("pair") == pair}
        if set(indexed) != {"baseline", "hybrid"}:
            raise ValueError(f"Incomplete or invalid pair {pair}")
        if sum(r.get("pair") == pair for r in records) != 2:
            raise ValueError(f"Duplicate route in pair {pair}")
        baseline = indexed["baseline"]["total_ms"]
        hybrid = indexed["hybrid"]["total_ms"]
        if baseline <= 0 or hybrid <= 0:
            raise ValueError("Workflow timing must be positive")
        pairs.append(
            {
                "pair": pair,
                "baseline_ms": baseline,
                "hybrid_ms": hybrid,
                "savings_fraction": 1 - hybrid / baseline,
            }
        )
    if len(records) != 2 * PAIR_COUNT:
        raise ValueError("Unexpected measured workflow count")
    baseline_total = sum(p["baseline_ms"] for p in pairs)
    hybrid_total = sum(p["hybrid_ms"] for p in pairs)
    savings = 1 - hybrid_total / baseline_total
    wrong = sum(r["wrong_targets"] for r in records)
    all_correct = all(r["success"] and r["status"] == "ok" for r in records)
    return {
        "pairs": pairs,
        "baseline_total_ms": baseline_total,
        "hybrid_total_ms": hybrid_total,
        "savings_fraction": savings,
        "wrong_targets": wrong,
        "all_file_proofs_correct": all_correct,
        "passed": all_correct and wrong == 0 and hybrid_total <= baseline_total * (1 - MIN_SAVINGS),
    }


def frozen_protocol() -> dict[str, Any]:
    return {
        "steps": STEPS,
        "threshold": THRESHOLD,
        "model": "gpt-6.1-terra",
        "effort": "medium",
        "workflow": "KWrite replace main editor text; deterministic Ctrl+S; verify changed exact output bytes",
        "warmups_per_route": 1,
        "pairs": PAIR_COUNT,
        "order": [
            ["baseline", "hybrid"] if i % 2 == 0 else ["hybrid", "baseline"]
            for i in range(PAIR_COUNT)
        ],
        "minimum_savings_fraction": MIN_SAVINGS,
        "savings_metric": "1 - sum(paired hybrid wall ms) / sum(paired baseline wall ms)",
        "gate": "zero wrong target selections (rejected before action); all changed exact-byte file proofs correct; savings >= minimum",
        "timing": "immediately before first observe through persisted file proof; setup/cleanup excluded",
        "scope": "caller-known editor intent; native editing and common save shortcut; no Terra planner turn",
        "oracle": {
            key: {field: sorted(values) for field, values in value.items()}
            for key, value in ORACLE.items()
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["dry", "run"])
    parser.add_argument("--out", type=Path, default=ROOT / "artifacts/jev-real-workflow/pilot")
    parser.add_argument("--client-path", type=Path)
    parser.add_argument("--service-path", type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if args.mode == "run":
        if (out / "frozen.json").exists():
            raise FileExistsError("Already frozen; choose a fresh output directory")
        dump(out / "frozen.json", frozen_protocol())
        (out / "frozen.sha256").write_text(
            hashlib.sha256((out / "frozen.json").read_bytes()).hexdigest()
        )
    desktop_options = {}
    if args.client_path is not None:
        desktop_options["client_path"] = args.client_path
    if args.service_path is not None:
        desktop_options["service_path"] = args.service_path
    d = Desktop(out, **desktop_options)
    try:
        if args.mode == "dry":
            return 0 if run_flow(d, None, out / "dry", "baseline", True)["success"] else 1
        with DesktopJudges(out) as judges:
            for route in ["baseline", "hybrid"]:
                if not run_flow(d, judges, out / f"warmup-{route}", route)["success"]:
                    return 1
            records = []
            for pair, routes in enumerate(frozen_protocol()["order"]):
                for route in routes:
                    result = run_flow(d, judges, out / f"pair-{pair}-{route}", route)
                    result["pair"] = pair
                    records.append(result)
                    dump(out / "results.json", records)
                    if not result["success"]:
                        return 1
            summary = paired_summary(records)
            dump(out / "summary.json", summary)
            print(json.dumps(summary), flush=True)
            return 0 if summary["passed"] else 1
    finally:
        d.close()


if __name__ == "__main__":
    raise SystemExit(main())
