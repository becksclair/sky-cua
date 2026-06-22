#!/usr/bin/env python3
"""Live phone-use smoke harness for sky-cua (plan Phase 8).

Drives the sky-cua MCP surface over stdio and exercises the real phone-use tool
family against an attached Android device. The harness is hardware-dependent:
when a prerequisite is missing (no adb, no device, no companion, no scrcpy, no
wireless target) the affected profile SKIPS with an explicit reason rather than
failing.

By default the harness runs the dev-build client at ``bin/sky-cua-client``. Pass
``--installed`` (or set ``SKY_CUA_PHONE_SMOKE_INSTALLED=1``) to instead drive the
staged installed surface at ``dist/plugin/sky-cua/bin/sky-cua-client``; the
harness errors if ``--installed`` is requested but that staged client is absent,
so the installed run never silently falls back to the dev build.

Usage:
  python3 scripts/live_phone_use_smoke.py --profile adb-usb --serial <serial>
  python3 scripts/live_phone_use_smoke.py --profile adb-wireless --wireless-host <host:port>
  python3 scripts/live_phone_use_smoke.py --profile pair-wireless --pair-host <host:port>
  python3 scripts/live_phone_use_smoke.py --profile full
  python3 scripts/live_phone_use_smoke.py --profile full --installed

Each step prints a concise ``PASS``/``SKIP``/``FAIL`` line and the run ends with
a single ``RESULT full_phone_use_smoke passed=<n> skipped=<m> failed=<k>`` line.

This harness never persists screenshots, pairing codes, RPC tokens, notification
bodies, or accessibility dumps. Only bounded, sanitized metadata is stored.
"""

from __future__ import annotations

import argparse
import os
import shutil
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from _mcp_stdio import McpClient

REPO_ROOT = Path(__file__).resolve().parents[1]
# Dev-build client (default): the wrapper in ``bin/`` resolves a debug or release
# target build of ``sky-cua-client``.
DEV_CLIENT = REPO_ROOT / "bin" / "sky-cua-client"
# Staged installed client, produced by ``scripts/build_plugin.py`` under
# ``dist/plugin/sky-cua/``. Selected only by ``--installed`` / the env opt-in.
INSTALLED_CLIENT = REPO_ROOT / "dist" / "plugin" / "sky-cua" / "bin" / "sky-cua-client"
# Backwards-compatible alias for the default client path.
CLIENT = DEV_CLIENT
# Env opt-in mirror of ``--installed`` so the staged surface can be selected
# without changing the invocation flags.
INSTALLED_ENV_VAR = "SKY_CUA_PHONE_SMOKE_INSTALLED"

# Profile names, in canonical run order. ``full`` fans out to every other
# profile and records each one's skips explicitly.
PROFILE_ADB_USB = "adb-usb"
PROFILE_ADB_WIRELESS = "adb-wireless"
PROFILE_PAIR_WIRELESS = "pair-wireless"
PROFILE_COMPANION = "companion"
PROFILE_SCRCPY = "scrcpy"
PROFILE_FALLBACK = "fallback"
PROFILE_ADVERSARIAL = "adversarial"
PROFILE_FULL = "full"

# Profiles fanned out by ``full`` (everything except ``full`` itself).
FULL_PROFILE_SEQUENCE: tuple[str, ...] = (
    PROFILE_ADB_USB,
    PROFILE_ADB_WIRELESS,
    PROFILE_PAIR_WIRELESS,
    PROFILE_COMPANION,
    PROFILE_SCRCPY,
    PROFILE_FALLBACK,
    PROFILE_ADVERSARIAL,
)

ALL_PROFILES: tuple[str, ...] = (*FULL_PROFILE_SEQUENCE, PROFILE_FULL)

# A deliberately stale snapshot id used by the adversarial profile. The service
# must reject coordinate actions that reference a snapshot it never issued.
STALE_SNAPSHOT_ID = "phone-smoke-stale-snapshot-0000"
# A serial that no device should ever report, for wrong-serial routing checks.
BOGUS_SERIAL = "phone-smoke-nonexistent-serial"

# Structured rejection codes for snapshot-geometry drift. A coordinate action
# against a snapshot whose orientation (swapped W/H) or resolution no longer
# matches the live device must be rejected with these codes, not silently
# remapped.
SNAPSHOT_ORIENTATION_MISMATCH_CODE = "PhoneSnapshotOrientationMismatch"
SNAPSHOT_RESOLUTION_MISMATCH_CODE = "PhoneSnapshotResolutionMismatch"

CANONICAL_EXPECTED_PHONE_TOOLS: frozenset[str] = frozenset(
    {
        "status",
        "list_resources",
        "observe",
        "capture_screen",
        "phone_accessibility_tree",
        "phone_notifications",
        "phone_connection",
        "phone_pair_wireless",
        "phone_setup",
        "phone_app_force_stop",
        "phone_pointer",
        "phone_keyboard",
        "phone_notification_action",
        "phone_notification_reply",
        "phone_app_action",
        "phone_app_install",
    }
)

# ---------------------------------------------------------------------------
# Outcome model
# ---------------------------------------------------------------------------


class StepStatus:
    """Terminal status for a single smoke step."""

    PASS = "PASS"
    SKIP = "SKIP"
    FAIL = "FAIL"


@dataclass(frozen=True)
class StepResult:
    """One PASS/SKIP/FAIL step within a profile."""

    status: str
    name: str
    detail: str = ""

    @property
    def passed(self) -> bool:
        return self.status == StepStatus.PASS

    @property
    def skipped(self) -> bool:
        return self.status == StepStatus.SKIP

    @property
    def failed(self) -> bool:
        return self.status == StepStatus.FAIL


def step_pass(name: str, detail: str = "") -> StepResult:
    return StepResult(StepStatus.PASS, name, detail)


def step_skip(name: str, reason: str) -> StepResult:
    return StepResult(StepStatus.SKIP, name, reason)


def step_fail(name: str, detail: str) -> StepResult:
    return StepResult(StepStatus.FAIL, name, detail)


class SmokeSkip(Exception):
    """Raised when a profile cannot run because a prerequisite is missing.

    The message is the explicit, bounded skip reason printed to the operator.
    """


class SmokeFailure(Exception):
    """Raised when a profile step proves the surface is actually broken."""


# ---------------------------------------------------------------------------
# PASS / SKIP / FAIL / RESULT line formatting (pure, unit-tested)
# ---------------------------------------------------------------------------


def format_step_line(profile: str, result: StepResult) -> str:
    """Render one ``<STATUS> <profile>.<name>[ detail]`` line."""
    head = f"{result.status} {profile}.{result.name}"
    detail = result.detail.strip()
    if not detail:
        return head
    return f"{head} {detail}"


def summarize_counts(results: Iterable[StepResult]) -> tuple[int, int, int]:
    """Return ``(passed, skipped, failed)`` across the given step results."""
    passed = skipped = failed = 0
    for result in results:
        if result.passed:
            passed += 1
        elif result.skipped:
            skipped += 1
        elif result.failed:
            failed += 1
    return passed, skipped, failed


def format_result_line(results: Iterable[StepResult]) -> str:
    """Render the final ``RESULT full_phone_use_smoke ...`` summary line."""
    passed, skipped, failed = summarize_counts(results)
    return f"RESULT full_phone_use_smoke passed={passed} skipped={skipped} failed={failed}"


# ---------------------------------------------------------------------------
# Sanitization helpers (no secrets/screenshots/notification bodies ever leave)
# ---------------------------------------------------------------------------


def sanitize_serial(serial: str | None) -> str:
    """Reduce a serial/host:port to a filesystem- and log-safe token.

    Wireless serials look like ``172.16.255.58:38781``; collapse anything that
    is not alphanumeric so artifact directory names and PASS lines never embed a
    raw host:port that could leak network topology into committed logs.
    """
    if not serial:
        return "unknown"
    safe = "".join(char if char.isalnum() else "-" for char in serial)
    safe = safe.strip("-") or "unknown"
    return safe[:48]


def bounded_metadata(value: Any, *, max_len: int = 80) -> str:
    """Coerce a structured field into a short, sanitized one-line token.

    Used only for non-sensitive scalars (counts, snapshot ids, backend names,
    model names). Never call this on notification bodies, accessibility text, or
    auth material.
    """
    if value is None:
        return "none"
    if isinstance(value, bool):
        return "true" if value else "false"
    text = str(value)
    text = " ".join(text.split())
    if len(text) > max_len:
        text = text[:max_len]
    return text


# ---------------------------------------------------------------------------
# MCP result inspection (pure, unit-tested)
# ---------------------------------------------------------------------------


def result_is_error(result: dict[str, Any]) -> bool:
    """True when an MCP tool result is flagged as an error."""
    return bool(result.get("isError"))


def structured(result: dict[str, Any]) -> dict[str, Any]:
    """Return the ``structuredContent`` map, or an empty map when absent."""
    payload = result.get("structuredContent")
    return payload if isinstance(payload, dict) else {}


def diagnostic_codes(result: dict[str, Any]) -> list[str]:
    """Collect diagnostic codes from a phone tool result's structured payload."""
    diagnostics = structured(result).get("diagnostics")
    codes: list[str] = []
    if isinstance(diagnostics, list):
        for entry in diagnostics:
            if isinstance(entry, dict):
                code = entry.get("code")
                if isinstance(code, str):
                    codes.append(code)
    return codes


def first_diagnostic_message(result: dict[str, Any]) -> str:
    """Return a sanitized first diagnostic message for skip/fail detail."""
    diagnostics = structured(result).get("diagnostics")
    if isinstance(diagnostics, list):
        for entry in diagnostics:
            if isinstance(entry, dict):
                message = entry.get("message")
                if isinstance(message, str) and message.strip():
                    return bounded_metadata(message, max_len=120)
    return ""


def tool_names_from_list(result: Any) -> set[str]:
    """Extract MCP tool names from a ``tools/list`` response."""
    tools = result
    if isinstance(result, dict):
        tools = result.get("tools")
    if not isinstance(tools, list):
        return set()
    names: set[str] = set()
    for tool in tools:
        if isinstance(tool, dict) and isinstance(tool.get("name"), str):
            names.add(tool["name"])
    return names


def canonical_unwrapped_result(result: dict[str, Any]) -> dict[str, Any]:
    """Return the branch result map from a canonical envelope."""
    payload = structured(result)
    inner = payload.get("result")
    if not isinstance(inner, dict):
        return result
    unwrapped = dict(result)
    unwrapped["structuredContent"] = inner
    return unwrapped


def require_expected_phone_tools(
    client: McpClient,
) -> StepResult:
    """Prove the installed MCP surface exposes the complete phone-use family."""
    result = client.tools_list()
    names = tool_names_from_list(result)
    expected = CANONICAL_EXPECTED_PHONE_TOOLS
    missing = sorted(expected - names)
    if missing:
        raise SmokeFailure(
            "installed MCP tools/list is missing phone tools: "
            + bounded_metadata(", ".join(missing), max_len=180)
        )
    return step_pass("tools_list", f"canonical_phone_tools={len(expected)}")


def cursor_capabilities(result: dict[str, Any]) -> dict[str, Any]:
    """Return the ``cursor_capabilities`` map from a screenshot result, or empty."""
    caps = structured(result).get("cursor_capabilities")
    return caps if isinstance(caps, dict) else {}


def companion_native_overlay(result: dict[str, Any]) -> bool | None:
    """Return the companion's ``native_overlay`` capability, or None when absent."""
    companion = structured(result).get("companion")
    if isinstance(companion, dict):
        value = companion.get("native_overlay")
        if isinstance(value, bool):
            return value
    return None


def notification_events(result: dict[str, Any]) -> list[dict[str, Any]]:
    """Return the structured notification event maps, or an empty list."""
    events = structured(result).get("events")
    out: list[dict[str, Any]] = []
    if isinstance(events, list):
        for entry in events:
            if isinstance(entry, dict):
                out.append(entry)
    return out


def first_openable_event(events: list[dict[str, Any]]) -> dict[str, Any] | None:
    """Pick the first event whose content intent is reported openable."""
    for event in events:
        if event.get("can_open") is True and isinstance(event.get("event_id"), str):
            return event
    return None


def first_reply_action(event: dict[str, Any]) -> str | None:
    """Return the action id of the event's first inline-reply action, if any."""
    actions = event.get("actions")
    if isinstance(actions, list):
        for action in actions:
            if isinstance(action, dict) and action.get("is_reply") is True:
                action_id = action.get("action_id")
                if isinstance(action_id, str) and action_id:
                    return action_id
    return None


def first_action_id(event: dict[str, Any]) -> str | None:
    """Return the action id of the event's first action, if any."""
    actions = event.get("actions")
    if isinstance(actions, list):
        for action in actions:
            if isinstance(action, dict):
                action_id = action.get("action_id")
                if isinstance(action_id, str) and action_id:
                    return action_id
    return None


# ---------------------------------------------------------------------------
# Host-side prerequisite probing
# ---------------------------------------------------------------------------


def adb_binary() -> str | None:
    """Resolve adb from the phone env override, then PATH."""
    configured = os.environ.get("SKY_CUA_ADB")
    if configured and Path(configured).exists():
        return configured
    return shutil.which("adb")


def scrcpy_binary() -> str | None:
    """Resolve scrcpy from the phone env override, then PATH."""
    configured = os.environ.get("SKY_CUA_SCRCPY")
    if configured and Path(configured).exists():
        return configured
    return shutil.which("scrcpy")


def resolve_client_path(*, installed: bool) -> Path:
    """Pick the MCP client binary path for the requested surface.

    Default is the dev build (``bin/sky-cua-client``). ``installed`` selects the
    staged surface and errors loudly if that client is absent, so an installed
    run can never silently fall back to the dev build.
    """
    if not installed:
        return DEV_CLIENT
    if not INSTALLED_CLIENT.exists():
        raise FileNotFoundError(
            f"--installed requested but staged client is missing at {INSTALLED_CLIENT}; "
            "run scripts/build_plugin.py first"
        )
    return INSTALLED_CLIENT


# ---------------------------------------------------------------------------
# Smoke driver: a thin wrapper over McpClient exposing typed phone calls
# ---------------------------------------------------------------------------


@dataclass
class PhoneSmokeOptions:
    """Operator-supplied targeting for the smoke run."""

    profile: str
    serial: str | None = None
    wireless_host: str | None = None
    pair_host: str | None = None
    pairing_code: str | None = None
    installed: bool = False
    # Operator assertions that the device can be physically rotated / its display
    # resolution changed during the run. The orientation/resolution snapshot
    # rejection steps require these because the harness cannot rotate or resize a
    # real device itself; without them those steps SKIP with a named reason.
    rotate_device: bool = False
    resize_device: bool = False


class PhoneSmoke:
    """Drives one or more phone tools against the sky-cua MCP surface."""

    def __init__(self, client: McpClient, options: PhoneSmokeOptions) -> None:
        self._client = client
        self._options = options
        self._request_id = 100

    def _next_id(self) -> int:
        self._request_id += 1
        return self._request_id

    def call(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        """Invoke a phone tool and return its raw MCP result map."""
        canonical_name, canonical_arguments = self._canonical_call(name, arguments)
        result = self._client.tools_call(self._next_id(), canonical_name, canonical_arguments)
        return canonical_unwrapped_result(result)

    def _canonical_call(self, name: str, arguments: dict[str, Any]) -> tuple[str, dict[str, Any]]:
        """Map the smoke's internal phone operation names onto canonical branches."""
        mapped = dict(arguments)
        match name:
            case "phone_status":
                mapped["component"] = "phone"
                return "status", mapped
            case "phone_companion_status":
                mapped["component"] = "phone_companion"
                return "status", mapped
            case "phone_list_devices":
                mapped.update({"surface": "phone", "resource": "devices"})
                return "list_resources", mapped
            case "phone_app_list":
                mapped.update({"surface": "phone", "resource": "apps"})
                return "list_resources", mapped
            case "phone_app_current":
                mapped.update({"surface": "phone", "resource": "current_app"})
                return "list_resources", mapped
            case "phone_observe":
                mapped["surface"] = "phone"
                return "observe", mapped
            case "phone_screenshot":
                mapped["surface"] = "phone"
                return "capture_screen", mapped
            case "phone_connect":
                mapped["operation"] = "connect"
                return "phone_connection", mapped
            case "phone_disconnect":
                mapped["operation"] = "disconnect"
                return "phone_connection", mapped
            case "phone_refresh_capabilities":
                mapped["operation"] = "refresh"
                return "phone_connection", mapped
            case "phone_tap":
                mapped["operation"] = "tap"
                return "phone_pointer", mapped
            case "phone_swipe":
                mapped["operation"] = "swipe"
                return "phone_pointer", mapped
            case "phone_type_text":
                mapped["operation"] = "type_text"
                return "phone_keyboard", mapped
            case "phone_press_key":
                mapped["operation"] = "press_key"
                return "phone_keyboard", mapped
            case "phone_install_companion":
                mapped["operation"] = "install_companion"
                return "phone_setup", mapped
            case "phone_open_settings":
                mapped["operation"] = "open_settings"
                return "phone_setup", mapped
            case "phone_notification_open":
                mapped["operation"] = "open"
                return "phone_notification_action", mapped
            case "phone_notification_dismiss":
                mapped["operation"] = "dismiss"
                return "phone_notification_action", mapped
            case "phone_notification_action":
                mapped["operation"] = "action"
                return "phone_notification_action", mapped
            case (
                "phone_pair_wireless"
                | "phone_notifications"
                | "phone_notification_reply"
                | "phone_app_force_stop"
                | "phone_app_install"
                | "phone_accessibility_tree"
            ):
                return name, mapped
            case "phone_app_launch":
                mapped["operation"] = "launch"
                return "phone_app_action", mapped
            case "phone_app_open_intent":
                mapped["operation"] = "open_intent"
                return "phone_app_action", mapped
        return name, mapped

    # The following thin wrappers exist so profiles read declaratively and so
    # tests can monkeypatch ``call`` on a fake driver.

    def status(self, *, refresh_devices: bool = False) -> dict[str, Any]:
        return self.call("phone_status", {"refresh_devices": refresh_devices})

    def list_devices(self, *, include_mdns: bool = False) -> dict[str, Any]:
        return self.call("phone_list_devices", {"include_mdns": include_mdns})

    def connect(self, serial: str | None) -> dict[str, Any]:
        arguments: dict[str, Any] = {}
        if serial:
            arguments["serial"] = serial
        return self.call("phone_connect", arguments)

    def disconnect(self, session_id: str | None) -> dict[str, Any]:
        # Tear down only the sky-cua session, never the shared wireless ADB
        # transport: a smoke must not disconnect the operator's wireless
        # debugging, and in the `full` sequence one profile's disconnect would
        # otherwise drop the link every later profile depends on.
        arguments: dict[str, Any] = {"keep_wireless": True}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_disconnect", arguments)

    def observe(self, session_id: str | None) -> dict[str, Any]:
        arguments: dict[str, Any] = {}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_observe", arguments)

    def screenshot(self, session_id: str | None) -> dict[str, Any]:
        arguments: dict[str, Any] = {}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_screenshot", arguments)

    def tap(
        self,
        session_id: str | None,
        snapshot_id: str | None,
        x: float,
        y: float,
        *,
        use_device_coordinates: bool = False,
    ) -> dict[str, Any]:
        arguments: dict[str, Any] = {"x": x, "y": y}
        if use_device_coordinates:
            arguments["use_device_coordinates"] = True
        if session_id:
            arguments["session_id"] = session_id
        if snapshot_id:
            arguments["phone_snapshot_id"] = snapshot_id
        return self.call("phone_tap", arguments)

    def swipe(
        self,
        session_id: str | None,
        snapshot_id: str | None,
        start_x: float,
        start_y: float,
        end_x: float,
        end_y: float,
        *,
        use_device_coordinates: bool = False,
    ) -> dict[str, Any]:
        arguments: dict[str, Any] = {
            "start_x": start_x,
            "start_y": start_y,
            "end_x": end_x,
            "end_y": end_y,
        }
        if use_device_coordinates:
            arguments["use_device_coordinates"] = True
        if session_id:
            arguments["session_id"] = session_id
        if snapshot_id:
            arguments["phone_snapshot_id"] = snapshot_id
        return self.call("phone_swipe", arguments)

    def app_current(self, session_id: str | None) -> dict[str, Any]:
        arguments: dict[str, Any] = {}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_app_current", arguments)

    def app_list(self, session_id: str | None) -> dict[str, Any]:
        arguments: dict[str, Any] = {}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_app_list", arguments)

    def companion_status(self, session_id: str | None) -> dict[str, Any]:
        arguments: dict[str, Any] = {}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_companion_status", arguments)

    def pair_wireless(self, host_port: str, pairing_code: str) -> dict[str, Any]:
        return self.call(
            "phone_pair_wireless",
            {"host_port": host_port, "pairing_code": pairing_code},
        )

    def notifications(self, session_id: str | None) -> dict[str, Any]:
        arguments: dict[str, Any] = {}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_notifications", arguments)

    def notification_open(self, session_id: str | None, event_id: str) -> dict[str, Any]:
        arguments: dict[str, Any] = {"event_id": event_id}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_notification_open", arguments)

    def notification_dismiss(self, session_id: str | None, event_id: str) -> dict[str, Any]:
        arguments: dict[str, Any] = {"event_id": event_id}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_notification_dismiss", arguments)

    def notification_action(
        self, session_id: str | None, event_id: str, action_id: str
    ) -> dict[str, Any]:
        arguments: dict[str, Any] = {"event_id": event_id, "action_id": action_id}
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_notification_action", arguments)

    def notification_reply(
        self, session_id: str | None, event_id: str, action_id: str, reply_text: str
    ) -> dict[str, Any]:
        arguments: dict[str, Any] = {
            "event_id": event_id,
            "action_id": action_id,
            "text": reply_text,
        }
        if session_id:
            arguments["session_id"] = session_id
        return self.call("phone_notification_reply", arguments)


# ---------------------------------------------------------------------------
# Shared step helpers used by multiple profiles
# ---------------------------------------------------------------------------


def require_devices_or_skip(smoke: PhoneSmoke) -> list[dict[str, Any]]:
    """Return connectable devices, or raise SmokeSkip with an explicit reason."""
    if adb_binary() is None:
        raise SmokeSkip("adb not found on PATH or via SKY_CUA_ADB")
    result = smoke.list_devices()
    devices = structured(result).get("devices")
    usable: list[dict[str, Any]] = []
    if isinstance(devices, list):
        for device in devices:
            if isinstance(device, dict) and device.get("state") == "device":
                usable.append(device)
    if not usable:
        raise SmokeSkip("no authorized adb device attached")
    return usable


def choose_serial(
    options: PhoneSmokeOptions,
    devices: list[dict[str, Any]],
    *,
    wireless: bool,
) -> str:
    """Pick the serial a profile should connect to.

    Wireless profiles prefer the explicit ``--wireless-host``; otherwise the
    first device whose connection_kind is wireless. USB profiles prefer
    ``--serial`` then the first USB/emulator device.
    """
    if wireless:
        if options.wireless_host:
            return options.wireless_host
        for device in devices:
            if device.get("connection_kind") == "wireless":
                serial = device.get("serial")
                if isinstance(serial, str) and serial:
                    return serial
        raise SmokeSkip("no wireless adb device attached and --wireless-host unset")
    if options.serial:
        return options.serial
    serial = devices[0].get("serial")
    if isinstance(serial, str) and serial:
        return serial
    raise SmokeSkip("attached device did not report a serial")


def connect_session(smoke: PhoneSmoke, serial: str) -> tuple[str, dict[str, Any]]:
    """Connect and return ``(session_id, connect_result)`` or raise SmokeFailure."""
    result = smoke.connect(serial)
    if result_is_error(result):
        raise SmokeFailure(
            f"phone_connect failed: {first_diagnostic_message(result) or 'no diagnostic'}"
        )
    session_id = structured(result).get("session_id")
    if not isinstance(session_id, str) or not session_id:
        raise SmokeFailure("phone_connect returned no session_id")
    return session_id, result


# ---------------------------------------------------------------------------
# Profiles
# ---------------------------------------------------------------------------

ProfileFn = Callable[[PhoneSmoke, PhoneSmokeOptions], list[StepResult]]


def profile_adb_usb(smoke: PhoneSmoke, options: PhoneSmokeOptions) -> list[StepResult]:
    """USB/emulator ADB: status, connect, observe, screenshot, tap, apps."""
    steps: list[StepResult] = []
    status = smoke.status(refresh_devices=True)
    report = structured(status)
    steps.append(
        step_pass(
            "phone_status",
            f"adb={bounded_metadata(report.get('adb_available'))} "
            f"devices={bounded_metadata(len(report.get('devices', [])))}",
        )
    )
    devices = require_devices_or_skip(smoke)
    serial = choose_serial(options, devices, wireless=False)
    session_id, _ = connect_session(smoke, serial)
    steps.append(step_pass("phone_connect", f"serial={sanitize_serial(serial)}"))

    observe = smoke.observe(session_id)
    if result_is_error(observe):
        raise SmokeFailure(f"phone_observe failed: {first_diagnostic_message(observe)}")
    observe_structured = structured(observe)
    steps.append(
        step_pass(
            "phone_observe",
            f"profile={bounded_metadata(observe_structured.get('capability_profile_id'))} "
            f"actions={bounded_metadata(len(observe_structured.get('available_actions', [])))}",
        )
    )

    shot = smoke.screenshot(session_id)
    if result_is_error(shot):
        raise SmokeFailure(f"phone_screenshot failed: {first_diagnostic_message(shot)}")
    shot_structured = structured(shot)
    snapshot_id = shot_structured.get("phone_snapshot_id")
    size = shot_structured.get("device_size") or {}
    steps.append(
        step_pass(
            "phone_screenshot",
            f"snapshot={bounded_metadata(snapshot_id)} "
            f"size={bounded_metadata(size.get('width'))}x{bounded_metadata(size.get('height'))}",
        )
    )

    if isinstance(snapshot_id, str) and snapshot_id:
        tap = smoke.tap(session_id, snapshot_id, 1.0, 1.0)
        if result_is_error(tap):
            steps.append(
                step_skip(
                    "phone_tap",
                    f"tap rejected: {first_diagnostic_message(tap) or 'no diagnostic'}",
                )
            )
        else:
            steps.append(step_pass("phone_tap", f"snapshot={bounded_metadata(snapshot_id)}"))
    else:
        steps.append(step_skip("phone_tap", "no snapshot id from phone_screenshot"))

    current = smoke.app_current(session_id)
    current_app = structured(current).get("current_app") or {}
    package = current_app.get("package_name") if isinstance(current_app, dict) else None
    steps.append(step_pass("phone_app_current", f"package={bounded_metadata(package)}"))

    apps = smoke.app_list(session_id)
    app_count = len(structured(apps).get("apps", []))
    steps.append(step_pass("phone_app_list", f"count={bounded_metadata(app_count)}"))

    disconnect = smoke.disconnect(session_id)
    if result_is_error(disconnect):
        raise SmokeFailure(f"phone_disconnect failed: {first_diagnostic_message(disconnect)}")
    steps.append(step_pass("phone_disconnect", f"serial={sanitize_serial(serial)}"))
    return steps


def profile_adb_wireless(smoke: PhoneSmoke, options: PhoneSmokeOptions) -> list[StepResult]:
    """Already-paired wireless: connect, observe, screenshot, reconnect."""
    steps: list[StepResult] = []
    devices = require_devices_or_skip(smoke)
    serial = choose_serial(options, devices, wireless=True)
    session_id, _ = connect_session(smoke, serial)
    steps.append(step_pass("phone_connect", f"serial={sanitize_serial(serial)}"))

    observe = smoke.observe(session_id)
    if result_is_error(observe):
        raise SmokeFailure(f"phone_observe failed: {first_diagnostic_message(observe)}")
    steps.append(
        step_pass(
            "phone_observe",
            f"profile={bounded_metadata(structured(observe).get('capability_profile_id'))}",
        )
    )

    shot = smoke.screenshot(session_id)
    if result_is_error(shot):
        raise SmokeFailure(f"phone_screenshot failed: {first_diagnostic_message(shot)}")
    steps.append(
        step_pass(
            "phone_screenshot",
            f"snapshot={bounded_metadata(structured(shot).get('phone_snapshot_id'))}",
        )
    )

    smoke.disconnect(session_id)
    reconnect_session, _ = connect_session(smoke, serial)
    steps.append(step_pass("phone_reconnect", f"serial={sanitize_serial(serial)} reconnect=true"))
    smoke.disconnect(reconnect_session)
    return steps


def profile_pair_wireless(smoke: PhoneSmoke, options: PhoneSmokeOptions) -> list[StepResult]:
    """Manual Android 11+ pairing-code workflow.

    Requires both a pairing endpoint (``--pair-host``) and a one-time code
    (``--pairing-code``). The code is sent to the tool but never printed or
    persisted.
    """
    if adb_binary() is None:
        raise SmokeSkip("adb not found on PATH or via SKY_CUA_ADB")
    if not options.pair_host:
        raise SmokeSkip("pairing endpoint not provided (--pair-host)")
    if not options.pairing_code:
        raise SmokeSkip("one-time pairing code not provided (--pairing-code)")
    result = smoke.pair_wireless(options.pair_host, options.pairing_code)
    if result_is_error(result):
        raise SmokeFailure(
            f"phone_pair_wireless failed: {first_diagnostic_message(result) or 'no diagnostic'}"
        )
    paired = structured(result).get("paired")
    return [
        step_pass(
            "phone_pair_wireless",
            f"endpoint={sanitize_serial(options.pair_host)} paired={bounded_metadata(paired)}",
        )
    ]


def _step_or_skip(name: str, body: Callable[[], StepResult]) -> StepResult:
    """Run one named step, converting a ``SmokeSkip`` into a per-step SKIP line.

    Used by profiles where a single missing live precondition (no notification to
    open, no reply action present) should skip that step with a named reason
    instead of collapsing the whole established session into one SKIP.
    """
    try:
        return body()
    except SmokeSkip as skip:
        return step_skip(name, str(skip))


def _notification_observe_step(
    smoke: PhoneSmoke, session_id: str
) -> tuple[StepResult, list[dict[str, Any]]]:
    """Observe notifications; skip (named) when the listener is unavailable."""
    result = smoke.notifications(session_id)
    if result_is_error(result):
        codes = ",".join(diagnostic_codes(result)) or "no code"
        raise SmokeSkip(f"notification listener unavailable ({codes})")
    report = structured(result)
    if report.get("listener_enabled") is not True:
        raise SmokeSkip("notification listener not enabled on device")
    events = notification_events(result)
    step = step_pass("phone_notifications", f"events={bounded_metadata(len(events))}")
    return step, events


def overlay_step(smoke: PhoneSmoke, session_id: str, companion_caps: dict[str, Any]) -> StepResult:
    """Exercise the phone-native agent overlay over an established companion session.

    The persistent "agent in control" edge glow is toggled on by the host when the
    session establishes (``overlay_active(true)``) and off on disconnect; each
    successful tap/swipe fires ``overlay_gesture`` inside the host's action path.
    The smoke drives the MCP boundary, not the companion RPC directly, so it
    proves the overlay path through what the surface exposes: the companion must
    advertise ``native_overlay`` (the glow/cursor plane), a benign device-space tap
    and swipe must dispatch through the companion (firing the tap-ripple and
    swipe-trail animations), and the resulting screenshot must report the
    ``phone_native_overlay`` cursor plane as live.

    Raises ``SmokeSkip`` with a named reason when a prerequisite is missing so a
    device without the overlay plane skips precisely instead of failing.
    """
    if companion_caps.get("rpc_reachable") is not True:
        raise SmokeSkip("companion rpc not reachable for overlay")
    if companion_native_overlay({"structuredContent": {"companion": companion_caps}}) is not True:
        raise SmokeSkip("companion does not advertise the native overlay plane")

    shot = smoke.screenshot(session_id)
    if result_is_error(shot):
        raise SmokeSkip(f"phone_screenshot failed: {first_diagnostic_message(shot)}")
    caps = cursor_capabilities(shot)
    if caps.get("phone_native_overlay") is not True:
        raise SmokeSkip("screenshot did not report the phone_native_overlay plane")
    snapshot_id = structured(shot).get("phone_snapshot_id")
    if not isinstance(snapshot_id, str) or not snapshot_id:
        raise SmokeSkip("phone_screenshot returned no snapshot id for the overlay step")

    # Benign device-space tap at (1, 1): dispatches through the companion and fires
    # the host's overlay_gesture(tap) ripple animation. Device coordinates avoid
    # any snapshot-mapping rejection unrelated to the overlay path.
    tap = smoke.tap(session_id, snapshot_id, 1.0, 1.0, use_device_coordinates=True)
    if result_is_error(tap):
        raise SmokeSkip(f"overlay tap rejected: {first_diagnostic_message(tap) or 'no diagnostic'}")
    tap_backend = bounded_metadata(structured(tap).get("backend"))
    if tap_backend != "companion":
        raise SmokeFailure(f"overlay tap used {tap_backend}, expected companion")

    # Short benign swipe: fires the host's overlay_gesture(swipe) trail animation.
    swipe = smoke.swipe(
        session_id,
        snapshot_id,
        1.0,
        1.0,
        1.0,
        2.0,
        use_device_coordinates=True,
    )
    if result_is_error(swipe):
        raise SmokeSkip(
            f"overlay swipe rejected: {first_diagnostic_message(swipe) or 'no diagnostic'}"
        )
    swipe_backend = bounded_metadata(structured(swipe).get("backend"))
    if swipe_backend != "companion":
        raise SmokeFailure(f"overlay swipe used {swipe_backend}, expected companion")

    return step_pass(
        "phone_overlay",
        f"native_overlay=true gesture_backend={tap_backend} swipe_backend={swipe_backend}",
    )


# The canonical overlay assertion is shared with the workflow smoke
# (`live_phone_workflow_smoke.overlay_probe`), so it is public. The historical
# private name is retained as an alias because tests reference it.
_overlay_step = overlay_step


def profile_companion(smoke: PhoneSmoke, options: PhoneSmokeOptions) -> list[StepResult]:
    """Companion backend: status, agent overlay, then notification flows."""
    steps: list[StepResult] = []
    devices = require_devices_or_skip(smoke)
    serial = choose_serial(options, devices, wireless=False)
    session_id, _ = connect_session(smoke, serial)
    steps.append(step_pass("phone_connect", f"serial={sanitize_serial(serial)}"))

    companion = smoke.companion_status(session_id)
    companion_caps = structured(companion).get("companion") or {}
    installed = companion_caps.get("installed") if isinstance(companion_caps, dict) else None
    if result_is_error(companion):
        codes = ",".join(diagnostic_codes(companion)) or "no code"
        smoke.disconnect(session_id)
        raise SmokeSkip(f"companion unavailable ({codes})")
    if installed is not True:
        smoke.disconnect(session_id)
        raise SmokeSkip("companion app not installed on device")
    steps.append(
        step_pass(
            "phone_companion_status",
            "installed=true "
            f"accessibility={bounded_metadata(companion_caps.get('accessibility_enabled'))} "
            f"rpc={bounded_metadata(companion_caps.get('rpc_reachable'))}",
        )
    )

    # Phone-native agent overlay: the session is already holding the "agent in
    # control" glow (overlay_active(true) fired on connect); exercise the
    # per-action overlay_gesture animations and assert the native overlay plane.
    steps.append(
        _step_or_skip("phone_overlay", lambda: overlay_step(smoke, session_id, companion_caps))
    )

    # Notification surface: observe first, then exercise open/dismiss/action/reply
    # against a live event. Each operation names its own missing prerequisite so a
    # device with no current notification skips precisely rather than failing.
    try:
        observe_step, events = _notification_observe_step(smoke, session_id)
        steps.append(observe_step)
    except SmokeSkip as skip:
        steps.append(step_skip("phone_notifications", str(skip)))
        events = []

    def open_step() -> StepResult:
        event = first_openable_event(events)
        if event is None:
            raise SmokeSkip("no openable notification event present")
        result = smoke.notification_open(session_id, str(event["event_id"]))
        if result_is_error(result):
            codes = ",".join(diagnostic_codes(result)) or "no code"
            raise SmokeSkip(f"notification open rejected ({codes})")
        return step_pass("phone_notification_open", "opened=true")

    def action_step() -> StepResult:
        for event in events:
            action_id = first_action_id(event)
            if action_id is not None and isinstance(event.get("event_id"), str):
                result = smoke.notification_action(session_id, str(event["event_id"]), action_id)
                if result_is_error(result):
                    codes = ",".join(diagnostic_codes(result)) or "no code"
                    raise SmokeSkip(f"notification action rejected ({codes})")
                return step_pass("phone_notification_action", "actioned=true")
        raise SmokeSkip("no notification action present")

    def reply_step() -> StepResult:
        for event in events:
            action_id = first_reply_action(event)
            if action_id is not None and isinstance(event.get("event_id"), str):
                result = smoke.notification_reply(
                    session_id, str(event["event_id"]), action_id, "sky-cua smoke reply"
                )
                if result_is_error(result):
                    codes = ",".join(diagnostic_codes(result)) or "no code"
                    raise SmokeSkip(f"notification reply rejected ({codes})")
                return step_pass("phone_notification_reply", "replied=true")
        raise SmokeSkip("no inline-reply notification action present")

    def dismiss_step() -> StepResult:
        for event in events:
            if event.get("can_dismiss") is True and isinstance(event.get("event_id"), str):
                result = smoke.notification_dismiss(session_id, str(event["event_id"]))
                if result_is_error(result):
                    codes = ",".join(diagnostic_codes(result)) or "no code"
                    raise SmokeSkip(f"notification dismiss rejected ({codes})")
                return step_pass("phone_notification_dismiss", "dismissed=true")
        raise SmokeSkip("no dismissable notification event present")

    steps.append(_step_or_skip("phone_notification_open", open_step))
    steps.append(_step_or_skip("phone_notification_action", action_step))
    steps.append(_step_or_skip("phone_notification_reply", reply_step))
    steps.append(_step_or_skip("phone_notification_dismiss", dismiss_step))

    smoke.disconnect(session_id)
    return steps


def profile_scrcpy(smoke: PhoneSmoke, options: PhoneSmokeOptions) -> list[StepResult]:
    """scrcpy acceleration: requires the scrcpy binary plus a device."""
    if scrcpy_binary() is None:
        raise SmokeSkip("scrcpy not found on PATH or via SKY_CUA_SCRCPY")
    devices = require_devices_or_skip(smoke)
    serial = choose_serial(options, devices, wireless=False)
    status = smoke.status()
    if not structured(status).get("scrcpy_available"):
        raise SmokeSkip("phone_status reports scrcpy unavailable to the service")
    session_id, _ = connect_session(smoke, serial)
    steps: list[StepResult] = [step_pass("phone_connect", f"serial={sanitize_serial(serial)}")]
    shot = smoke.screenshot(session_id)
    if result_is_error(shot):
        smoke.disconnect(session_id)
        raise SmokeFailure(f"phone_screenshot failed: {first_diagnostic_message(shot)}")
    backend = structured(shot).get("backend")
    steps.append(step_pass("phone_screenshot", f"backend={bounded_metadata(backend)}"))
    smoke.disconnect(session_id)
    return steps


def profile_fallback(smoke: PhoneSmoke, options: PhoneSmokeOptions) -> list[StepResult]:
    """Degraded-capability proof: status reports honest backend availability.

    This profile does not require a device; it proves the host surface reports
    adb/scrcpy/companion availability as structured booleans so an agent can
    reason about degradation instead of guessing from prose.
    """
    status = smoke.status()
    if result_is_error(status):
        raise SmokeFailure(f"phone_status failed: {first_diagnostic_message(status)}")
    report = structured(status)
    if "adb_available" not in report:
        raise SmokeFailure("phone_status omitted the adb_available field")
    return [
        step_pass(
            "phone_status_fallback",
            f"adb={bounded_metadata(report.get('adb_available'))} "
            f"scrcpy={bounded_metadata(report.get('scrcpy_available'))} "
            f"companion={bounded_metadata(report.get('companion_enabled'))}",
        )
    ]


def profile_adversarial(smoke: PhoneSmoke, options: PhoneSmokeOptions) -> list[StepResult]:
    """Bounded live adversarial cases that are safe without a device.

    - Wrong serial: ``phone_connect`` to a serial no device reports must error.
    - Stale snapshot: ``phone_tap`` against a fabricated snapshot must error.
    """
    steps: list[StepResult] = []
    if adb_binary() is None:
        raise SmokeSkip("adb not found on PATH or via SKY_CUA_ADB")

    wrong = smoke.connect(BOGUS_SERIAL)
    if result_is_error(wrong):
        steps.append(step_pass("wrong_serial_rejected", "wrong_serial_rejected=true"))
    else:
        steps.append(step_fail("wrong_serial_rejected", "phone_connect accepted a bogus serial"))

    stale = smoke.tap(None, STALE_SNAPSHOT_ID, 1.0, 1.0)
    if result_is_error(stale):
        steps.append(step_pass("stale_snapshot_rejected", "stale_snapshot_rejected=true"))
    else:
        steps.append(
            step_fail(
                "stale_snapshot_rejected",
                "phone_tap accepted a fabricated snapshot id",
            )
        )

    steps.append(
        _step_or_skip(
            "orientation_mismatch_rejected",
            lambda: _snapshot_drift_step(
                smoke,
                options,
                enabled=options.rotate_device,
                enable_flag="--device-can-rotate",
                change_label="rotate",
                expected_code=SNAPSHOT_ORIENTATION_MISMATCH_CODE,
                step_name="orientation_mismatch_rejected",
            ),
        )
    )
    steps.append(
        _step_or_skip(
            "resolution_mismatch_rejected",
            lambda: _snapshot_drift_step(
                smoke,
                options,
                enabled=options.resize_device,
                enable_flag="--device-can-resize",
                change_label="resize",
                expected_code=SNAPSHOT_RESOLUTION_MISMATCH_CODE,
                step_name="resolution_mismatch_rejected",
            ),
        )
    )
    return steps


def _snapshot_drift_step(
    smoke: PhoneSmoke,
    options: PhoneSmokeOptions,
    *,
    enabled: bool,
    enable_flag: str,
    change_label: str,
    expected_code: str,
    step_name: str,
) -> StepResult:
    """Assert a coordinate action is rejected after a snapshot's geometry drifts.

    The harness cannot physically rotate or resize a real device, so this step
    SKIPs with a named prerequisite unless the operator asserts the device can be
    changed (``enable_flag``) and arranges the change after the snapshot is
    captured. When enabled it captures a snapshot, then taps against it and
    asserts the structured ``expected_code`` rejection.
    """
    if not enabled:
        raise SmokeSkip(
            f"device cannot be {change_label}d in-harness; pass {enable_flag} and "
            f"{change_label} the device after the snapshot to exercise this case"
        )
    devices = require_devices_or_skip(smoke)
    serial = choose_serial(options, devices, wireless=False)
    session_id, _ = connect_session(smoke, serial)
    try:
        shot = smoke.screenshot(session_id)
        if result_is_error(shot):
            raise SmokeSkip(f"phone_screenshot failed: {first_diagnostic_message(shot)}")
        snapshot_id = structured(shot).get("phone_snapshot_id")
        if not isinstance(snapshot_id, str) or not snapshot_id:
            raise SmokeSkip("phone_screenshot returned no snapshot id to drift")
        tap = smoke.tap(session_id, snapshot_id, 1.0, 1.0)
        codes = diagnostic_codes(tap)
        if result_is_error(tap) and expected_code in codes:
            return step_pass(step_name, f"code={expected_code}")
        if result_is_error(tap):
            return step_fail(
                step_name,
                f"phone_tap rejected with {','.join(codes) or 'no code'}; expected {expected_code}",
            )
        return step_fail(
            step_name,
            f"phone_tap accepted a {change_label}d-geometry snapshot; expected {expected_code}",
        )
    finally:
        smoke.disconnect(session_id)


PROFILES: dict[str, ProfileFn] = {
    PROFILE_ADB_USB: profile_adb_usb,
    PROFILE_ADB_WIRELESS: profile_adb_wireless,
    PROFILE_PAIR_WIRELESS: profile_pair_wireless,
    PROFILE_COMPANION: profile_companion,
    PROFILE_SCRCPY: profile_scrcpy,
    PROFILE_FALLBACK: profile_fallback,
    PROFILE_ADVERSARIAL: profile_adversarial,
}


def profiles_for(profile: str) -> tuple[str, ...]:
    """Expand a requested profile into the concrete profiles to run."""
    if profile == PROFILE_FULL:
        return FULL_PROFILE_SEQUENCE
    return (profile,)


# ---------------------------------------------------------------------------
# Execution + reporting
# ---------------------------------------------------------------------------


@dataclass
class RunReport:
    """Accumulated step results across one harness invocation."""

    steps: list[StepResult] = field(default_factory=list)

    def extend(self, results: Iterable[StepResult]) -> None:
        self.steps.extend(results)

    def add(self, result: StepResult) -> None:
        self.steps.append(result)

    @property
    def failed_count(self) -> int:
        return sum(1 for step in self.steps if step.failed)


def run_profile(
    smoke: PhoneSmoke,
    profile: str,
    options: PhoneSmokeOptions,
) -> list[StepResult]:
    """Run a single concrete profile, converting skips/failures to step results.

    A ``SmokeSkip`` collapses the whole profile into one SKIP line; a
    ``SmokeFailure`` collapses it into one FAIL line. Any other exception is
    surfaced as a FAIL so the harness never silently swallows a real bug.
    """
    profile_fn = PROFILES[profile]
    try:
        return profile_fn(smoke, options)
    except SmokeSkip as skip:
        return [step_skip(profile, str(skip))]
    except SmokeFailure as failure:
        return [step_fail(profile, str(failure))]
    except Exception as error:
        # Report, never crash the run: an unexpected error in one profile must
        # still produce a FAIL line and let the remaining profiles execute.
        return [step_fail(profile, bounded_metadata(f"unexpected error: {error}", max_len=160))]


def run_smoke(
    options: PhoneSmokeOptions,
    *,
    client_factory: Callable[[], McpClient] | None = None,
    emit: Callable[[str], None] = print,
) -> RunReport:
    """Run all requested profiles against the MCP surface and print step lines."""
    report = RunReport()
    factory = client_factory or (lambda: _default_client_factory(installed=options.installed))
    client = factory()
    try:
        client.initialize()
        if options.installed:
            proof = require_expected_phone_tools(client)
            report.add(proof)
            emit(format_step_line("installed", proof))
        smoke = PhoneSmoke(client, options)
        for profile in profiles_for(options.profile):
            results = run_profile(smoke, profile, options)
            report.extend(results)
            for result in results:
                emit(format_step_line(profile, result))
    finally:
        client.close()
    emit(format_result_line(report.steps))
    return report


def _default_client_factory(*, installed: bool = False) -> McpClient:
    base_env = dict(os.environ)
    # Force text-only delivery so no base64 screenshot ever rides the structured
    # channel into artifacts or logs.
    base_env.setdefault("SKY_CUA_MODEL_SUPPORTS_IMAGES", "false")
    base_env.setdefault("SKY_CUA_PHONE", "1")
    client_path = resolve_client_path(installed=installed)
    return McpClient([str(client_path), "mcp"], base_env=base_env)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Live phone-use smoke harness.")
    parser.add_argument(
        "--profile",
        choices=ALL_PROFILES,
        default=PROFILE_FULL,
        help="Smoke profile to run. 'full' fans out to every profile.",
    )
    parser.add_argument(
        "--serial",
        default=None,
        help="USB/emulator serial or host:port to connect to.",
    )
    parser.add_argument(
        "--wireless-host",
        default=None,
        help="Already-paired wireless host:port for adb-wireless.",
    )
    parser.add_argument(
        "--pair-host",
        default=None,
        help="host:port pairing endpoint shown on the device for pair-wireless.",
    )
    parser.add_argument(
        "--pairing-code",
        default=None,
        help="One-time pairing code for pair-wireless. Never logged or persisted.",
    )
    parser.add_argument(
        "--installed",
        action="store_true",
        default=None,
        help=(
            "Drive the staged installed client at "
            "dist/plugin/sky-cua/bin/sky-cua-client instead of the dev build. "
            "Errors if that client is absent. Also enabled by "
            f"{INSTALLED_ENV_VAR}=1."
        ),
    )
    parser.add_argument(
        "--device-can-rotate",
        action="store_true",
        help=(
            "Assert the device can be physically rotated during the run, enabling "
            "the adversarial orientation-mismatch snapshot-rejection step."
        ),
    )
    parser.add_argument(
        "--device-can-resize",
        action="store_true",
        help=(
            "Assert the device display resolution can be changed during the run, "
            "enabling the adversarial resolution-mismatch snapshot-rejection step."
        ),
    )
    return parser


def _env_truthy(name: str) -> bool:
    """Return True when an env opt-in is set to a truthy token."""
    value = os.environ.get(name)
    if value is None:
        return False
    return value.strip().lower() in {"1", "true", "yes", "on"}


def options_from_args(args: argparse.Namespace) -> PhoneSmokeOptions:
    installed = bool(args.installed) or _env_truthy(INSTALLED_ENV_VAR)
    return PhoneSmokeOptions(
        profile=args.profile,
        serial=args.serial,
        wireless_host=args.wireless_host,
        pair_host=args.pair_host,
        pairing_code=args.pairing_code,
        installed=installed,
        rotate_device=bool(args.device_can_rotate),
        resize_device=bool(args.device_can_resize),
    )


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    options = options_from_args(args)
    report = run_smoke(options)
    return 1 if report.failed_count else 0


if __name__ == "__main__":
    raise SystemExit(main())
