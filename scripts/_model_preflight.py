#!/usr/bin/env python3
"""Model pre-flight: pick a reachable opencode model before a live smoke run.

The agentic-loop live smokes drive `opencode run --format json --model <M>
<prompt>` (see `_agent_mcp_smoke.run_agent`). Hardcoding a single model means
a live run silently hangs or 401s when that model's credits or availability
change. This module probes a preference-ordered list of candidate models
with a cheap one-word prompt and returns the first one that actually answers,
so the caller can select_working_model() once instead of guessing.

Probing runs the candidates concurrently (staggered launch) rather than
sequentially, since a live run may need to try several models and each probe
can take several seconds. Concurrent `opencode` processes share a local
SQLite state DB and can collide with `database is locked`; that failure mode
is transient, so a locked probe is retried with backoff instead of treated
as a real unavailability signal.
"""

from __future__ import annotations

import json
import subprocess
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from typing import Protocol

from _model_profiles import model_candidates

PROBE_PROMPT = "Reply with exactly one word: READY"

# Preference order: most capable / most cost-effective for the caller first.
# select_working_model returns the first of these that is actually reachable.
DEFAULT_FALLBACK_ANCHOR_MODELS = model_candidates("fallback_anchor")

_SUCCESS_EVENT_TYPES = {"text", "step_finish"}


class ProbeRunner(Protocol):
    def __call__(self, model: str, timeout_s: float, /) -> subprocess.CompletedProcess[str]: ...


@dataclass(frozen=True)
class ModelProbeResult:
    model: str
    ok: bool
    # "ok" | "no_credits" | "db_locked" | "not_found" | "timeout" | "error: <detail>"
    reason: str
    elapsed_s: float


def _default_probe_runner(model: str, timeout_s: float) -> subprocess.CompletedProcess[str]:
    """Real `opencode run` invocation used outside of tests.

    Unlike `_agent_mcp_smoke.run_agent`, which wraps opencode in `script` for
    a pty (needed for its interactive-shaped output), the probe uses the
    plain non-interactive form with stdin closed — this was verified live to
    produce JSON-lines output on success without a pty.
    """
    argv = ["opencode", "run", "--format", "json", "--model", model, PROBE_PROMPT]
    try:
        return subprocess.run(
            argv,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=timeout_s,
        )
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout if isinstance(exc.stdout, str) else ""
        stderr = exc.stderr if isinstance(exc.stderr, str) else ""
        return subprocess.CompletedProcess(argv, returncode=-9, stdout=stdout, stderr=stderr)


def _stdout_has_success_event(stdout: str) -> bool:
    for line in stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict) and event.get("type") in _SUCCESS_EVENT_TYPES:
            return True
    return False


def _classify(returncode: int, stdout: str, stderr: str, *, timed_out: bool) -> tuple[bool, str]:
    combined = f"{stdout}\n{stderr}"
    if timed_out:
        return False, "timeout"
    if "database is locked" in combined:
        return False, "db_locked"
    if "CreditsError" in combined or "Insufficient balance" in combined:
        return False, "no_credits"
    if "not found" in combined.lower() or "No models match pattern" in combined:
        return False, "not_found"
    if returncode != 0:
        detail = stderr.strip() or stdout.strip() or f"exit {returncode}"
        return False, f"error: {detail[:200]}"
    if _stdout_has_success_event(stdout):
        return True, "ok"
    return False, "error: no success event in output"


def probe_model(
    model: str,
    *,
    timeout_s: float = 90.0,
    runner: ProbeRunner | None = None,
) -> ModelProbeResult:
    """Probe a single model with a cheap prompt and classify the outcome."""
    active_runner = runner or _default_probe_runner
    start = time.monotonic()
    try:
        proc = active_runner(model, timeout_s)
    except subprocess.TimeoutExpired:
        return ModelProbeResult(model, False, "timeout", time.monotonic() - start)
    elapsed = time.monotonic() - start
    timed_out = proc.returncode == -9
    stdout = proc.stdout or ""
    stderr = proc.stderr or ""
    ok, reason = _classify(proc.returncode, stdout, stderr, timed_out=timed_out)
    return ModelProbeResult(model, ok, reason, elapsed)


def _probe_with_lock_retry(
    model: str,
    *,
    timeout_s: float,
    max_lock_retries: int,
    runner: ProbeRunner | None,
    retry_backoff_s: float,
) -> ModelProbeResult:
    result = probe_model(model, timeout_s=timeout_s, runner=runner)
    attempt = 0
    while result.reason == "db_locked" and attempt < max_lock_retries:
        attempt += 1
        time.sleep(retry_backoff_s * attempt)
        result = probe_model(model, timeout_s=timeout_s, runner=runner)
    return result


def select_working_model(
    candidates: list[str] | tuple[str, ...],
    *,
    timeout_s: float = 90.0,
    max_lock_retries: int = 3,
    runner: ProbeRunner | None = None,
    stagger_s: float = 1.5,
    retry_backoff_s: float = 3.0,
) -> tuple[str | None, list[ModelProbeResult]]:
    """Probe candidates concurrently and return the first reachable one.

    Probes launch on a staggered schedule (each ~`stagger_s` after the
    previous) rather than all at once, and a `database is locked` result is
    retried with backoff — both mitigate opencode's shared local SQLite
    state DB contending across concurrent processes, without isolating auth
    state (which would lose credentials). The winner is the first OK
    candidate in `candidates` order, not necessarily the first to finish, so
    selection stays deterministic regardless of probe timing.
    """
    results: list[ModelProbeResult | None] = [None] * len(candidates)
    lock = threading.Lock()

    def worker(index: int, model: str) -> None:
        time.sleep(index * stagger_s)
        result = _probe_with_lock_retry(
            model,
            timeout_s=timeout_s,
            max_lock_retries=max_lock_retries,
            runner=runner,
            retry_backoff_s=retry_backoff_s,
        )
        with lock:
            results[index] = result

    if candidates:
        with ThreadPoolExecutor(max_workers=len(candidates)) as pool:
            futures = [pool.submit(worker, index, model) for index, model in enumerate(candidates)]
            for future in futures:
                future.result()

    final_results = [result for result in results if result is not None]
    for result in final_results:
        if result.ok:
            return result.model, final_results
    return None, final_results


def format_probe_table(results: list[ModelProbeResult]) -> str:
    """Render probe results as a small aligned table for stderr diagnostics."""
    if not results:
        return "(no models probed)"
    width = max(len(result.model) for result in results)
    lines = []
    for result in results:
        status = "OK" if result.ok else "FAIL"
        lines.append(
            f"  {result.model.ljust(width)}  {status:<4}  {result.reason}"
            f"  ({result.elapsed_s:.1f}s)"
        )
    return "\n".join(lines)
