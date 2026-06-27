"""Host-side LLM judge for sky-cua agent tool-use performance.

After the codex CUA smoke runs in the VM and produces a transcript plus a
deterministic coverage summary, this module condenses the transcript, asks a
HOST codex exec run (gpt-5.5, high reasoning) to score the agent's tool use
against a fixed rubric, and returns a structured verdict. The verdict carries a
0-100 score, four 0-25 subscores, and an always-present triage list of
tool/workflow issues for follow-up.

The judge runs on the host (which has gpt-5.5 auth); the VM only produces the
transcript. The judge itself calls no tools: it runs in an isolated codex home
that contains only auth, and the rubric demands JSON-only output.
"""

from __future__ import annotations

import json
import shutil
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from _codex_exec import read_last_message, run_codex_exec, transcript_mcp_tool_calls
from _cua_coverage import bare_tool_name, call_failed
from _plugin_bundle import DEFAULT_CODEX_HOME, REPO_ROOT

AGENT_PERF_JUDGE_VERDICT_SCHEMA = (
    REPO_ROOT / "scripts" / "schemas" / "agent_perf_judge_verdict.json"
)
DEFAULT_THRESHOLD = 70
DEFAULT_JUDGE_MODEL = "gpt-5.5"
DEFAULT_JUDGE_REASONING_EFFORT = "high"

_ARGS_EXCERPT_LIMIT = 400
_ERROR_EXCERPT_LIMIT = 300


def prepare_judge_codex_home(artifact_dir: Path) -> Path:
    """Isolated codex home for the judge: host auth only, no plugin, no tools."""
    judge_home = (artifact_dir / "judge-codex-home").resolve()
    judge_home.mkdir(parents=True, exist_ok=True)
    auth_src = DEFAULT_CODEX_HOME / "auth.json"
    if not auth_src.exists():
        raise FileNotFoundError(
            f"host codex auth not found at {auth_src}; the judge needs host gpt-5.5 auth"
        )
    shutil.copy2(auth_src, judge_home / "auth.json")
    # Disable the apps/plugin surface so the judge run never loads sky-cua tools.
    (judge_home / "config.toml").write_text(
        'model_reasoning_effort = "high"\n\n[features]\napps = false\n',
        encoding="utf-8",
    )
    return judge_home


def _call_id(item: Mapping[str, Any]) -> str | None:
    for key in ("id", "call_id", "callId", "tool_call_id", "toolCallId"):
        value = item.get(key)
        if isinstance(value, str) and value:
            return value
    return None


def _arguments_excerpt(item: Mapping[str, Any]) -> str:
    raw = item.get("arguments")
    if raw is None:
        raw = item.get("args")
    if isinstance(raw, (dict, list)):
        raw = json.dumps(raw, default=str, sort_keys=True)
    if not isinstance(raw, str):
        return ""
    return raw[:_ARGS_EXCERPT_LIMIT]


def _error_excerpt(item: Mapping[str, Any]) -> str:
    error = item.get("error")
    if isinstance(error, str) and error.strip():
        return error.strip()[:_ERROR_EXCERPT_LIMIT]
    return json.dumps(item, default=str)[:_ERROR_EXCERPT_LIMIT]


def condense_transcript(
    transcript_path: Path,
    *,
    max_head: int = 40,
    max_tail: int = 60,
    char_budget: int = 40_000,
) -> list[dict[str, Any]]:
    """Compact, image-free, ordered tool-call list for the judge prompt.

    Pairs started/completed items by id (completed wins), drops all result/image
    payloads (the dominant token sink) and keeps only tool, server, a truncated
    argument excerpt, an ok/error status, and a short error excerpt on failure.
    Elides the middle of very long runs (first ``max_head`` + last ``max_tail``)
    and enforces a global character budget.
    """
    order: list[str] = []
    merged: dict[str, dict[str, Any]] = {}
    anonymous: list[dict[str, Any]] = []
    for item in transcript_mcp_tool_calls(transcript_path):
        tool_raw = item.get("tool") or item.get("tool_name") or item.get("name")
        tool = bare_tool_name(tool_raw) if isinstance(tool_raw, str) else "?"
        entry = {
            "tool": tool,
            "server": item.get("server"),
            "arguments": _arguments_excerpt(item),
            "status": "error" if call_failed(item) else "ok",
        }
        if entry["status"] == "error":
            entry["error"] = _error_excerpt(item)
        call_id = _call_id(item)
        if call_id is None:
            anonymous.append(entry)
            continue
        if call_id not in merged:
            merged[call_id] = entry
            order.append(call_id)
        else:
            existing = merged[call_id]
            # completed item refines status/error/args of the started item
            if not existing.get("arguments") and entry.get("arguments"):
                existing["arguments"] = entry["arguments"]
            if entry["status"] == "error":
                existing["status"] = "error"
                existing["error"] = entry.get("error", "")

    calls = [merged[cid] for cid in order] + anonymous
    elided = False
    if len(calls) > max_head + max_tail:
        calls = [
            *calls[:max_head],
            {"elided": len(calls) - max_head - max_tail},
            *calls[-max_tail:],
        ]
        elided = True
    for index, entry in enumerate(calls):
        entry["seq"] = index

    # Enforce the global character budget by trimming from the middle.
    while calls and len(json.dumps(calls, default=str)) > char_budget and len(calls) > 4:
        mid = len(calls) // 2
        calls.pop(mid)
        elided = True
    if elided and calls:
        calls[-1] = {**calls[-1], "note": "transcript condensed for the judge"}
    return calls


def build_judge_prompt(
    *,
    condensed_calls: list[dict[str, Any]],
    coverage_summary: Mapping[str, Any],
    last_message: Mapping[str, Any],
    exit_code: int,
) -> str:
    """Rubric prompt: score tool-use 0-100 across four 0-25 dimensions."""
    return (
        "You are grading a computer-use + browser-use agent (sky-cua) on how well it drove its "
        "MCP tools to complete a scripted desktop+browser workflow. Do NOT call any tools. Return "
        "ONLY JSON matching the provided output schema.\n\n"
        "Score four dimensions, each 0-25; the overall `score` MUST equal the sum of the four "
        "subscores (so 0-100):\n"
        "- tool_selection (0-25): did it pick the right tool and surface for each step (desktop vs "
        "browser, semantic vs physical input, observe before acting)?\n"
        "- error_recovery (0-25): when a tool returned an error, did it diagnose and recover "
        "without giving up or fabricating success? Penalize ignoring errors or inventing results.\n"
        "- efficiency (0-25): did it avoid redundant loops, repeated identical calls, and "
        "no-progress screenshot churn? Reward a direct path.\n"
        "- task_completion (0-25): were all required tools/operations exercised and the workflow "
        "finished? Use the deterministic coverage matrix below as ground truth for this dimension.\n\n"
        "Rules:\n"
        "- The coverage matrix is authoritative fact: if it reports missing tools/operations or "
        "tool errors, task_completion and error_recovery must reflect that.\n"
        "- ALWAYS return a non-empty `triage` array. On a clean pass, list the weakest observed "
        "behaviors (severity low) so they can still be reviewed.\n"
        "- Set `pass` to your judgement of whether the run is acceptable; the harness recomputes "
        "the authoritative pass from the score and threshold.\n\n"
        f"Deterministic coverage matrix (ground truth):\n{json.dumps(dict(coverage_summary), indent=2)}\n\n"
        f"codex exit code: {exit_code}\n\n"
        f"Agent final structured message:\n{json.dumps(dict(last_message), indent=2)[:4000]}\n\n"
        f"Ordered tool-call trace (results/images stripped):\n{json.dumps(condensed_calls, indent=2)}\n"
    )


def judge_transcript(
    *,
    transcript_path: Path,
    coverage_summary: Mapping[str, Any],
    last_message: Mapping[str, Any],
    artifact_dir: Path,
    exit_code: int = 0,
    threshold: int = DEFAULT_THRESHOLD,
    model: str = DEFAULT_JUDGE_MODEL,
    reasoning_effort: str = DEFAULT_JUDGE_REASONING_EFFORT,
) -> dict[str, Any]:
    """Run the host judge and return the verdict with an authoritative `pass`."""
    artifact_dir.mkdir(parents=True, exist_ok=True)
    judge_home = prepare_judge_codex_home(artifact_dir)
    condensed = condense_transcript(transcript_path)
    prompt = build_judge_prompt(
        condensed_calls=condensed,
        coverage_summary=coverage_summary,
        last_message=last_message,
        exit_code=exit_code,
    )
    result = run_codex_exec(
        prompt=prompt,
        artifact_dir=artifact_dir / "judge-run",
        output_schema=AGENT_PERF_JUDGE_VERDICT_SCHEMA,
        model=model,
        reasoning_effort=reasoning_effort,
        extra_env={"CODEX_HOME": str(judge_home)},
    )
    if result.exit_code != 0 or not result.last_message_path.exists():
        raise RuntimeError(
            f"judge codex exec failed (exit {result.exit_code}); inspect {result.artifact_dir}"
        )
    verdict = read_last_message(result.last_message_path)
    score = verdict.get("score")
    score_int = score if isinstance(score, int) else 0
    # The harness owns the authoritative pass/fail decision, not the model's arithmetic.
    verdict["score"] = score_int
    verdict["pass"] = score_int >= threshold
    verdict["threshold"] = threshold
    return verdict
