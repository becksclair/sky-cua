#!/usr/bin/env python3
"""mpv fallback-anchor fixture for the agentic-loop live smoke.

mpv, launched with `--idle --force-window`, opens a native Wayland window
that registers NO AT-SPI tree at all (verified live on this project's KDE
Wayland proving host: `observe(surface="desktop", app_id="mpv.desktop")`
returns exactly one `window`-role element with `backend_ref: null` and
`AccessibilityCoverageLimited` in diagnostics). That makes it a reliable,
AT-SPI-dark target for the honest single-node "vision anchor" fallback
described in ``crates/sky-cua-linux/src/backend.rs::linux_window_elements``:
one ``window``-role element carrying a ``vision_anchor`` state flag, a
description pointing the model at the screenshot + ``snapshot_id`` pixel
path, and no fabricated children. This fixture proves that fallback path end
to end through the installed runtime and a real agent CLI, per
``docs/features/wayland-fallback-vision-anchor.md``.

Fixture flow:

1. Launch mpv into idle mode with a distinctive window title, so the window
   exists but has no media loaded, no vault, no config, and no first-run
   affordance to navigate past.
2. Drive the installed sky-cua runtime through an agent CLI (`opencode` or
   `pi`) with a goal that observes the titled window and reports its
   accessibility structure. The act of observing is what surfaces the
   vision-anchor snapshot; the gate below confirms the `vision_anchor`
   element actually appeared in the agent's observe result.
3. Deterministically prove fallback-only mode from the *raw* (unredacted)
   agent transcript: at least one observe response must carry a
   `vision_anchor` state flag and must not carry any richer AT-SPI role,
   which is the harness-side ground truth independent of what the agent
   claims to have done.
4. Tear down: kill the mpv process this fixture launched (matched by the
   distinctive `--title` value, not just the `mpv` binary name, so an
   unrelated mpv instance is left alone) and let the window close.

This fixture requires the raw (unredacted) agent stdout log to inspect tool
result payloads for `vision_anchor` evidence, since
`_agent_mcp_smoke.redact_pi_json_stdout` strips tool result content by
default to avoid persisting screenshot/observe payloads to disk. It sets
`SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG=1` for the duration of its own agent run
(restoring the previous value afterward) rather than weakening redaction
for every fixture.
"""

from __future__ import annotations

import contextlib
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

from _agent_mcp_smoke import make_artifact_dir, run_agent, write_result
from _model_preflight import (
    DEFAULT_FALLBACK_ANCHOR_MODELS,
    format_probe_table,
    select_working_model,
)
from live_agent_mcp_smoke import PI_MCP_WRAPPER_GUIDANCE

MPV_BIN = "mpv"
FIXTURE_WINDOW_TITLE = "sky-cua-fallback-anchor-target"
WINDOW_WAIT_SECONDS = 2.0

# Native-window fallback roles sky-cua emits when AT-SPI has nothing to
# offer: `window` is the pure-Wayland KWin/vision_anchor anchor; the
# `x11_*` roles come from the separate XWayland region fallback and never
# carry `vision_anchor`. Any role outside this set means AT-SPI actually
# reported real accessible children, which would disprove fallback-only
# mode for this fixture.
NATIVE_FALLBACK_ROLES = frozenset(
    {"window", "x11_container", "x11_action_region", "x11_leaf_region"}
)

# Teardown matches on the distinctive `--title` value rather than just the
# `mpv` binary name, so an unrelated mpv instance on the host is not killed.
MPV_PROCESS_MARKERS = (FIXTURE_WINDOW_TITLE,)


def build_launch_argv() -> list[str]:
    """Return the argv used to launch mpv into idle mode.

    `--idle` keeps mpv running with no media loaded (no first-run
    affordance to navigate past); `--force-window` guarantees a window
    exists even with nothing playing; `--no-config` skips any user mpv
    config that could change window behavior; `--title` gives the fixture
    a distinctive window to find and, on teardown, kill.
    """
    return [
        MPV_BIN,
        "--idle",
        "--force-window",
        "--no-config",
        f"--title={FIXTURE_WINDOW_TITLE}",
    ]


def build_fallback_anchor_prompt(agent: str) -> str:
    prompt = (
        "Use the sky-cua MCP tools (server name sky_cua, sky-cua, or computer-use). "
        f"Find the desktop window whose title contains '{FIXTURE_WINDOW_TITLE}'. "
        "This app exposes no accessibility tree on this desktop, so observe will "
        "return a fallback anchor instead of real elements; use it anyway to inspect "
        "the window and report its accessibility structure. "
        "Do not use shell commands, process inspection, OCR, window-manager commands, "
        "global keyboard shortcuts, or non-sky-cua desktop shortcuts as substitutes for "
        "sky-cua MCP tools. "
    )
    if agent == "pi":
        prompt += PI_MCP_WRAPPER_GUIDANCE + " "
    prompt += (
        "After a successful sky-cua action, return immediately without extra verification "
        "loops. Return a JSON object with keys: window_found (boolean), "
        "role (string, the reported role of the window element)."
    )
    return prompt


def observe_payload_proves_fallback(payload: object) -> bool:
    """True when an observe/get_app_state-shaped payload proves fallback-only mode.

    Fallback-only mode requires both: at least one element carries the
    `vision_anchor` state flag, and no element exposes a role outside the
    native-fallback role set (which would mean AT-SPI actually reported
    real accessible children, contradicting the fallback claim).
    """
    elements = _elements_from_payload(payload)
    if not elements:
        return False
    has_vision_anchor = any(_element_has_flag(element, "vision_anchor") for element in elements)
    has_rich_atspi_role = any(_element_has_rich_atspi_role(element) for element in elements)
    return has_vision_anchor and not has_rich_atspi_role


def _elements_from_payload(payload: object) -> list[dict[str, Any]] | None:
    if not isinstance(payload, dict):
        return None
    elements = payload.get("elements")
    if isinstance(elements, list) and elements and all(isinstance(e, dict) for e in elements):
        return elements
    return None


def _element_has_flag(element: dict[str, Any], flag: str) -> bool:
    flags = element.get("state_flags")
    return isinstance(flags, list) and flag in flags


def _element_has_rich_atspi_role(element: dict[str, Any]) -> bool:
    role = element.get("role")
    return isinstance(role, str) and role not in NATIVE_FALLBACK_ROLES


_STATES_RE = re.compile(r"states=([a-z0-9_,]+)")

# The text-form proof requires both flags in the same `states=` list:
# `native_window_fallback` marks the honest KWin single-window fallback (no
# AT-SPI), and `vision_anchor` marks the anchor element itself. A real
# AT-SPI element would carry neither in a `states=` summary, so requiring
# both together is robust against false positives on rich-AT-SPI apps.
_TEXT_FALLBACK_FLAGS = frozenset({"vision_anchor", "native_window_fallback"})


def text_proves_fallback(stdout_text: str) -> bool:
    """True when raw stdout text contains sky-cua's text-summary proof of
    the fallback anchor.

    Some agent CLIs (observed live with opencode) log sky-cua's observe
    result as the tool's TEXT-summary content block rather than structured
    JSON, e.g.:

        ... states=native_window_fallback,physical_target,vision_anchor,
        container,content_like,focused,active bounds=(...) ...

    This scans every `states=<comma-list>` token in the text and returns
    True if any one token's flag set is a superset of
    `{"vision_anchor", "native_window_fallback"}`.
    """
    for match in _STATES_RE.finditer(stdout_text):
        flags = set(match.group(1).split(","))
        if flags >= _TEXT_FALLBACK_FLAGS:
            return True
    return False


def stdout_proves_fallback(stdout_path: Path) -> bool:
    """Scan a raw agent stdout JSONL log for at least one observe response
    that proves fallback-only mode.

    Tool-result payloads land in the agent CLI's own event stream as nested
    JSON, sometimes double-encoded as a string inside a text content block,
    so this walks every JSON-shaped value reachable from each line (parsing
    embedded JSON strings too) looking for an `elements` list that satisfies
    `observe_payload_proves_fallback`. Some agent CLIs instead log sky-cua's
    observe result as a TEXT-summary content block with no structured JSON
    at all, so this also falls back to `text_proves_fallback` against the
    raw file text.
    """
    if not stdout_path.exists():
        return False
    stdout_text = stdout_path.read_text(encoding="utf-8")
    for raw_line in stdout_text.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if any(observe_payload_proves_fallback(candidate) for candidate in _iter_json_like(event)):
            return True
    return text_proves_fallback(stdout_text)


def _iter_json_like(value: object) -> list[object]:
    found: list[object] = []
    _collect_json_like(value, found)
    return found


def _collect_json_like(value: object, found: list[object]) -> None:
    if isinstance(value, dict):
        found.append(value)
        for item in value.values():
            _collect_json_like(item, found)
    elif isinstance(value, list):
        for item in value:
            _collect_json_like(item, found)
    elif isinstance(value, str):
        stripped = value.strip()
        if stripped[:1] in "{[":
            try:
                parsed = json.loads(stripped)
            except json.JSONDecodeError:
                return
            _collect_json_like(parsed, found)


def kill_fallback_anchor_mpv() -> None:
    """Kill the mpv process this fixture launched.

    Matches on the distinctive `--title` value instead of just the `mpv`
    binary name, so an unrelated mpv instance on the host is not killed.
    """
    try:
        listing = subprocess.run(
            ["pgrep", "-af", MPV_BIN],
            capture_output=True,
            text=True,
            check=False,
        )
    except FileNotFoundError:
        return
    for line in listing.stdout.splitlines():
        parts = line.split(maxsplit=1)
        if len(parts) != 2:
            continue
        pid_str, cmdline = parts
        if not any(marker in cmdline for marker in MPV_PROCESS_MARKERS):
            continue
        try:
            pid = int(pid_str)
        except ValueError:
            continue
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.kill(pid, signal.SIGKILL)


def run_fallback_anchor_smoke(*, agent: str, model: str | None = None) -> int:
    if agent not in {"opencode", "pi"}:
        raise ValueError(
            f"fallback-anchor fixture requires an agent with tool-evidence enforcement "
            f"(opencode or pi), got {agent!r}"
        )

    # Model pre-flight: the DEFAULT_FALLBACK_ANCHOR_MODELS candidates are
    # opencode/opencode-go model ids, so pre-flight only applies to the
    # opencode agent path. Pi selects its own model (defaulting to
    # DEFAULT_PI_SMOKE_MODEL inside run_agent) and is left untouched here —
    # extending pre-flight to pi's provider-specific model ids is future
    # work, not needed for this fixture today.
    if agent == "opencode" and model is None:
        selected_model, probe_results = select_working_model(DEFAULT_FALLBACK_ANCHOR_MODELS)
        print("model pre-flight results:", file=sys.stderr)
        print(format_probe_table(probe_results), file=sys.stderr)
        if selected_model is None:
            print(
                "model pre-flight FAILED: no candidate model was reachable; "
                "refusing to fall through to a hardcoded model that would hang.",
                file=sys.stderr,
            )
            return 1
        print(f"model pre-flight selected: {selected_model}", file=sys.stderr)
        model = selected_model

    artifact_dir = make_artifact_dir(agent, "fallback-anchor")
    launch_argv = build_launch_argv()
    launch_proc = subprocess.Popen(launch_argv)

    previous_keep_raw_log = os.environ.get("SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG")
    os.environ["SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG"] = "1"
    try:
        time.sleep(WINDOW_WAIT_SECONDS)

        prompt = build_fallback_anchor_prompt(agent)
        proc = run_agent(agent, prompt, artifact_dir, model=model)

        stdout_path = artifact_dir / f"{agent}.stdout.log"
        fallback_proved = stdout_proves_fallback(stdout_path)
        ok = proc.returncode == 0 and fallback_proved

        result = write_result(
            artifact_dir,
            agent,
            proc,
            dialog_alive=False,
            extra={
                "fixture": "fallback-anchor",
                "fallback_proved": fallback_proved,
                "ok": ok,
            },
        )

        if not ok:
            print(f"{agent} fallback-anchor smoke FAILED: {artifact_dir}", file=sys.stderr)
            print(json.dumps(result, indent=2), file=sys.stderr)
            return 1

        print(f"{agent} fallback-anchor smoke passed: {artifact_dir}")
        print(json.dumps(result, indent=2))
        return 0
    finally:
        if previous_keep_raw_log is None:
            os.environ.pop("SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG", None)
        else:
            os.environ["SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG"] = previous_keep_raw_log
        if launch_proc.poll() is None:
            launch_proc.terminate()
        kill_fallback_anchor_mpv()


if __name__ == "__main__":
    raise SystemExit(run_fallback_anchor_smoke(agent="pi"))
