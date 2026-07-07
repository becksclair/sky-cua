#!/usr/bin/env python3
"""Obsidian fallback-anchor fixture for the agentic-loop live smoke.

Obsidian is an Electron app that does not launch with
``--force-renderer-accessibility``, so on this project's KDE Wayland proving
host it exposes no AT-SPI tree. When forced onto a native Wayland surface
(no XWayland window for the compositor to hand sky-cua), `observe`/
`get_app_state` falls back to the honest single-node "vision anchor" region
described in ``crates/sky-cua-linux/src/backend.rs::linux_window_elements``:
one ``window``-role element carrying a ``vision_anchor`` state flag, a
description pointing the model at the screenshot + ``snapshot_id`` pixel
path, and no fabricated children. This fixture proves that fallback path end
to end through the installed runtime and a real agent CLI, per
``plans/wayland_fallback_vision_anchors.md``.

Fixture flow:

1. Create a throwaway scratch vault directory (nothing in it, so Obsidian
   treats it as a brand-new vault and shows its first-run trust dialog).
2. Launch Obsidian into that vault via the ``obsidian://open?path=`` URI,
   forcing the Wayland Ozone backend so the compositor does not hand sky-cua
   an XWayland window (which would exercise the *other*, non-vision_anchor
   X11 region fallback instead).
3. Drive the installed sky-cua runtime through an agent CLI (`opencode` or
   `pi`) with a screenshot-guided goal: observe the fallback anchor, click
   the vault's trust-author affordance, confirm via a fresh screenshot.
4. Deterministically prove fallback-only mode from the *raw* (unredacted)
   agent transcript: at least one observe response must carry a
   `vision_anchor` state flag and must not carry any richer AT-SPI role,
   which is the harness-side ground truth independent of what the agent
   claims to have done.
5. Tear down: kill Obsidian's Electron process tree (helper processes don't
   share the wrapper's pgid, so processes are matched by command line) and
   delete the scratch vault.

Two things are explicit judgment calls made without a live Obsidian install
to verify against, and are deliberately isolated behind small, named seams
so they are cheap to correct after the first live run:

- `build_launch_argv` is the single function that constructs the launch
  command/URI and the Wayland-forcing flags. Obsidian's actual CLI/URI
  behavior (whether the URI form is honored, or whether Obsidian instead
  remembers its last vault) could not be verified offline.
- `TRUST_AUTHOR_AFFORDANCE` names the UI target for the agent goal prompt:
  the "Trust author" button on Obsidian's first-run vault trust dialog. If
  the first live run shows a different affordance (e.g. a "Create" button
  on a vault-picker screen instead), update this constant and
  `build_obsidian_prompt`; the pass gate and teardown logic do not depend
  on which affordance is targeted.

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
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any
from urllib.parse import quote

from _agent_mcp_smoke import make_artifact_dir, run_agent, write_result
from live_agent_mcp_smoke import PI_MCP_WRAPPER_GUIDANCE

OBSIDIAN_BIN = "obsidian"
FIXTURE_WINDOW_TITLE_HINT = "Obsidian"
TRUST_AUTHOR_AFFORDANCE = "Trust author"
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

# Electron respawns helper (renderer/gpu/utility) processes that do not
# share the launcher's process group, so teardown matches on command-line
# markers instead of killing a single Popen handle.
OBSIDIAN_PROCESS_MARKERS = ("obsidian", "app.asar")


def create_scratch_vault() -> Path:
    """Create an empty, resettable scratch vault directory.

    Left empty (no `.obsidian/` config) so Obsidian treats it as a brand
    new vault and shows its first-run trust dialog, giving the agent a
    deterministic affordance to target.
    """
    return Path(tempfile.mkdtemp(prefix="sky-cua-obsidian-vault-"))


def delete_scratch_vault(vault_path: Path) -> None:
    shutil.rmtree(vault_path, ignore_errors=True)


def build_launch_argv(vault_path: Path) -> list[str]:
    """Return the argv used to launch Obsidian into `vault_path`.

    Single seam: adjust the URI form, flags, or binary name here after the
    first live run, once Obsidian's actual CLI/vault-open behavior on the
    proving host is confirmed. The Wayland Ozone flags are load-bearing for
    the fallback this fixture proves: without them Obsidian may run under
    XWayland, which hits the X11 region fallback (no `vision_anchor` flag)
    instead of the pure-Wayland `vision_anchor` anchor.
    """
    uri = f"obsidian://open?path={quote(str(vault_path))}"
    return [
        OBSIDIAN_BIN,
        "--enable-features=UseOzonePlatform",
        "--ozone-platform=wayland",
        uri,
    ]


def build_obsidian_prompt(agent: str) -> str:
    prompt = (
        "Use the sky-cua MCP tools (server name sky_cua, sky-cua, or computer-use). "
        f"Find the window whose title contains '{FIXTURE_WINDOW_TITLE_HINT}'. "
        "This app exposes no accessibility tree on this desktop, so observe will "
        "return a fallback anchor instead of real elements; use it anyway to get the "
        "fallback region and a screenshot. Read the target's pixel position off the "
        f"screenshot and click the '{TRUST_AUTHOR_AFFORDANCE}' button with "
        "desktop_pointer using the observe response's snapshot_id and those pixel "
        "coordinates. Take a fresh screenshot afterward to confirm the click landed. "
        "Do not use shell commands, process inspection, OCR, window-manager commands, "
        "global keyboard shortcuts, or non-sky-cua desktop shortcuts as substitutes for "
        "sky-cua MCP tools. "
    )
    if agent == "pi":
        prompt += PI_MCP_WRAPPER_GUIDANCE + " "
    prompt += (
        "After a successful sky-cua action, return immediately without extra verification "
        "loops. Return a JSON object with keys: trust_dialog_seen (boolean), "
        "clicked (boolean)."
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


def stdout_proves_fallback(stdout_path: Path) -> bool:
    """Scan a raw agent stdout JSONL log for at least one observe response
    that proves fallback-only mode.

    Tool-result payloads land in the agent CLI's own event stream as nested
    JSON, sometimes double-encoded as a string inside a text content block,
    so this walks every JSON-shaped value reachable from each line (parsing
    embedded JSON strings too) looking for an `elements` list that satisfies
    `observe_payload_proves_fallback`.
    """
    if not stdout_path.exists():
        return False
    for raw_line in stdout_path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if any(observe_payload_proves_fallback(candidate) for candidate in _iter_json_like(event)):
            return True
    return False


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


def kill_obsidian_process_tree() -> None:
    """Kill every Obsidian-related process, not just the launcher.

    Electron respawns renderer/gpu/utility helper processes that do not
    share the wrapper's process group, so this matches on command-line
    markers (the `obsidian` binary name and the `app.asar` bundle path)
    rather than terminating a single `Popen` handle.
    """
    try:
        listing = subprocess.run(
            ["pgrep", "-af", "obsidian"],
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
        if not any(marker in cmdline for marker in OBSIDIAN_PROCESS_MARKERS):
            continue
        try:
            pid = int(pid_str)
        except ValueError:
            continue
        with contextlib.suppress(ProcessLookupError, PermissionError):
            os.kill(pid, signal.SIGKILL)


def run_obsidian_fallback_smoke(*, agent: str, model: str | None = None) -> int:
    if agent not in {"opencode", "pi"}:
        raise ValueError(
            f"obsidian-fallback fixture requires an agent with tool-evidence enforcement "
            f"(opencode or pi), got {agent!r}"
        )

    artifact_dir = make_artifact_dir(agent, "obsidian-fallback")
    vault_path = create_scratch_vault()
    launch_argv = build_launch_argv(vault_path)
    launch_proc = subprocess.Popen(launch_argv)

    previous_keep_raw_log = os.environ.get("SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG")
    os.environ["SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG"] = "1"
    try:
        time.sleep(WINDOW_WAIT_SECONDS)

        prompt = build_obsidian_prompt(agent)
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
                "fixture": "obsidian-fallback",
                "vault_path": str(vault_path),
                "fallback_proved": fallback_proved,
                "ok": ok,
            },
        )

        if not ok:
            print(f"{agent} obsidian-fallback smoke FAILED: {artifact_dir}", file=sys.stderr)
            print(json.dumps(result, indent=2), file=sys.stderr)
            return 1

        print(f"{agent} obsidian-fallback smoke passed: {artifact_dir}")
        print(json.dumps(result, indent=2))
        return 0
    finally:
        if previous_keep_raw_log is None:
            os.environ.pop("SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG", None)
        else:
            os.environ["SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG"] = previous_keep_raw_log
        if launch_proc.poll() is None:
            launch_proc.terminate()
        kill_obsidian_process_tree()
        delete_scratch_vault(vault_path)


if __name__ == "__main__":
    raise SystemExit(run_obsidian_fallback_smoke(agent="pi"))
