#!/usr/bin/env python3
"""Host-side judge CLI for sky-cua agent tool-use performance.

Given a codex CUA smoke's transcript, final message, and deterministic coverage
summary, run the host Codex ``performance_judge`` profile and emit a verdict. Always
writes ``judge-verdict.json`` and ``judge-triage.json`` into the artifact dir,
even on failure, so follow-up tooling has a stable path to the triage list.
Exits non-zero when the score is below the threshold.

This runs on the HOST after a VM smoke; the transcript/summary are pulled back
from the VM by run_gui_testing_vm_smoke.py.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from _agent_perf_judge import (
    DEFAULT_JUDGE_MODEL,
    DEFAULT_JUDGE_REASONING_EFFORT,
    DEFAULT_THRESHOLD,
    judge_transcript,
)


def _default_threshold() -> int:
    raw = os.environ.get("SKY_CUA_JUDGE_THRESHOLD", "").strip()
    if raw.isdecimal():
        return int(raw)
    return DEFAULT_THRESHOLD


def main() -> int:
    parser = argparse.ArgumentParser(description="Score sky-cua agent tool-use from a codex run.")
    parser.add_argument("--transcript", type=Path, required=True, help="codex-output.jsonl path.")
    parser.add_argument("--last-message", type=Path, required=True, help="last-message.json path.")
    parser.add_argument(
        "--coverage-summary", type=Path, required=True, help="coverage-summary.json path."
    )
    parser.add_argument(
        "--artifact-dir",
        type=Path,
        required=True,
        help="Directory to write judge-verdict.json, judge-triage.json, and the judge run.",
    )
    parser.add_argument(
        "--exit-code", type=int, default=0, help="The smoke's codex exec exit code (context)."
    )
    parser.add_argument("--threshold", type=int, default=_default_threshold())
    parser.add_argument("--model", default=DEFAULT_JUDGE_MODEL)
    parser.add_argument("--reasoning-effort", default=DEFAULT_JUDGE_REASONING_EFFORT)
    args = parser.parse_args()

    coverage_summary = json.loads(args.coverage_summary.read_text())
    last_message = json.loads(args.last_message.read_text())

    artifact_dir = args.artifact_dir.resolve()
    artifact_dir.mkdir(parents=True, exist_ok=True)

    verdict = judge_transcript(
        transcript_path=args.transcript,
        coverage_summary=coverage_summary,
        last_message=last_message,
        artifact_dir=artifact_dir,
        exit_code=args.exit_code,
        threshold=args.threshold,
        model=args.model,
        reasoning_effort=args.reasoning_effort,
    )

    # Always persist the verdict and the triage list, even on a failing score.
    (artifact_dir / "judge-verdict.json").write_text(
        json.dumps(verdict, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    (artifact_dir / "judge-triage.json").write_text(
        json.dumps(verdict.get("triage", []), indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(json.dumps(verdict, indent=2, sort_keys=True))

    if not verdict.get("pass"):
        print(
            f"agent performance judge FAILED: score {verdict.get('score')} < {args.threshold}; "
            f"inspect {artifact_dir}",
            flush=True,
        )
        return 1
    print(f"agent performance judge passed: score {verdict.get('score')} >= {args.threshold}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
