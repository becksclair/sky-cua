#!/usr/bin/env python3
"""Live phone-companion *setup* smoke for sky-cua.

Validates the end-to-end companion setup workflow from a COLD device — the flow
proven live on 2026-06-20: a freshly reset Android device (companion
uninstalled, accessibility + notification services disabled) is taken to a
fully reachable companion (RPC up, accessibility + notification services
enabled, `phone_observe` routed `backend=companion`).

Two drivers exercise the same workflow:

* ``--driver agent`` (default): an external agent CLI is told to set the
  companion up via the `phone_*` MCP tools — the "validate with an agent" path.
  The default agent is ``claude`` because Claude Code reliably surfaces the
  sky-cua phone MCP tools; it proves out by GROUND TRUTH (below). NOTE on agents:
  ``opencode`` does NOT currently expose the phone MCP tools to the agent (it
  sees the phone-use *skill* and falls back to repo exploration / bash), so it is
  unreliable here; the ground-truth gate still catches that honestly. Tool-call
  evidence is parsed from the transcript when the agent emits structured tool
  events (JSON-mode agents such as ``pi``); ``claude`` runs in plain-text mode so
  its tool-evidence is a soft skip (matching the desktop agent smokes), and the
  ground truth is the proof. Use ``--require-tool-evidence`` to additionally
  demand `phone_connect` + `phone_install_companion` in the transcript.
* ``--driver direct``: the harness drives the `phone_*` MCP tools itself
  (deterministic; no agent CLI required).

Either way, the proof is GROUND TRUTH independent of the driver's own claims:

1. ADB-level: companion package installed, `SkyAccessibilityService` bound,
   `SkyNotificationListenerService` bound, and the RPC port listening on-device.
2. A pure MCP probe with companion auto-install/operator-mode turned OFF, so
   `rpc_reachable` / `accessibility_enabled` / `notification_listener_enabled`
   can only pass when the *driver* already completed setup — the probe never
   sets anything up itself.

The harness is hardware-dependent: when a prerequisite is missing (no adb, no
device, no companion APK, no agent CLI) the run SKIPS with an explicit reason
rather than failing. It is destructive by design on the sky-cua companion only
(it uninstalls the companion and removes the companion's own service entries);
it never clobbers a user's other accessibility/notification services.

No screenshots, RPC tokens, notification bodies, or accessibility dumps are
persisted; only bounded sanitized metadata.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import shutil
import subprocess
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from _agent_mcp_smoke import make_artifact_dir, run_agent, tool_base_name
from _companion import build_and_stage_companion
from _mcp_stdio import McpClient

# Reuse the existing phone smoke's typed driver + outcome model + helpers so the
# two harnesses stay in lockstep on conventions (sanitization, step formatting,
# device/serial resolution, MCP result inspection).
from live_phone_use_smoke import (
    PhoneSmoke,
    PhoneSmokeOptions,
    StepResult,
    adb_binary,
    bounded_metadata,
    diagnostic_codes,
    first_diagnostic_message,
    resolve_client_path,
    result_is_error,
    sanitize_serial,
    step_fail,
    step_pass,
    step_skip,
    structured,
    summarize_counts,
)

REPO_ROOT = Path(__file__).resolve().parents[1]

COMPANION_PACKAGE = "com.skycua.phonecompanion"
ACCESSIBILITY_COMPONENT = f"{COMPANION_PACKAGE}/{COMPANION_PACKAGE}.service.SkyAccessibilityService"
NOTIFICATION_COMPONENT = (
    f"{COMPANION_PACKAGE}/{COMPANION_PACKAGE}.service.SkyNotificationListenerService"
)
# Companion RPC port 47683 = 0xBA43, as it appears in /proc/net/tcp[6].
RPC_PORT = 47683
RPC_PORT_HEX = f"{RPC_PORT:04X}"
STAGED_COMPANION_APK = REPO_ROOT / "resources" / "android" / "phone-companion.apk"

DRIVER_AGENT = "agent"
DRIVER_DIRECT = "direct"
DRIVER_CHOICES = (DRIVER_AGENT, DRIVER_DIRECT)

AGENT_CHOICES = ("claude", "opencode", "pi", "openclaw")
# Claude Code reliably surfaces the sky-cua phone MCP tools; opencode does not
# (it sees the phone-use skill and falls back to bash/exploration), so it is not
# the default. The ground-truth gate validates either way.
DEFAULT_AGENT = "claude"

COLD_RESET_FULL = "full"
COLD_RESET_SERVICES = "services"
COLD_RESET_NONE = "none"
COLD_RESET_CHOICES = (COLD_RESET_FULL, COLD_RESET_SERVICES, COLD_RESET_NONE)

# The two phone tools the agent MUST drive for an honest "set it up with an
# agent" claim. Ground truth proves the device ended up set up; tool-evidence
# proves the agent visibly used the MCP tools to get there.
REQUIRED_AGENT_TOOLS = ("phone_connect", "phone_install_companion")
SMOKE_NAME = "phone_companion_setup_smoke"
# Profile token for artifact dir naming; make_artifact_dir appends "-smoke".
ARTIFACT_PROFILE = "phone_companion_setup"


# ---------------------------------------------------------------------------
# Pure helpers (unit-tested; no device, agent, or MCP server required)
# ---------------------------------------------------------------------------


def agent_setup_prompt(serial: str) -> str:
    """The single-shot task prompt handed to the agent CLI."""
    return (
        "You have the sky-cua phone MCP tools (phone_status, phone_list_devices, "
        "phone_connect, phone_install_companion, phone_companion_status, phone_observe). "
        f"Set up the phone-use companion on the connected Android device with serial {serial} so "
        "phone-use is ready: the companion must be installed and reachable with its accessibility "
        "and notification-listener services enabled. Steps: (1) phone_connect(serial="
        f"'{serial}'); (2) phone_install_companion; (3) confirm with phone_companion_status that "
        "rpc_reachable is true and accessibility_enabled and notification_listener_enabled are "
        "true. Use only the sky-cua phone MCP tools — do not shell out to adb. Report the final "
        "companion status as a JSON object with keys rpc_reachable, accessibility_enabled, "
        "notification_listener_enabled (all booleans)."
    )


def _tool_names_in_json(value: Any) -> set[str]:
    """Recursively collect tool-name-like strings from a parsed JSON value.

    Agent transcripts vary by engine, so this is deliberately permissive: any
    string under a tool-identity key (``toolName``/``tool``/``name``/...) whose
    base name is a phone tool counts. Prefixes such as ``mcp__sky-cua__`` or
    ``sky_cua_`` are stripped to the bare phone tool name.
    """
    names: set[str] = set()
    if isinstance(value, dict):
        for key, item in value.items():
            normalized_key = str(key).lower().replace("-", "_")
            if normalized_key in {"toolname", "tool_name", "tool", "name"} and isinstance(
                item, str
            ):
                base = _phone_tool_base_name(item)
                if base is not None:
                    names.add(base)
            names |= _tool_names_in_json(item)
    elif isinstance(value, list):
        for item in value:
            names |= _tool_names_in_json(item)
    return names


def _phone_tool_base_name(value: str) -> str | None:
    """Strip any MCP/server namespace prefix and return the bare ``phone_*`` tool name."""
    base = tool_base_name(value)
    return base if base.startswith("phone_") else None


def phone_tools_invoked(transcript: str) -> set[str]:
    """Return the set of ``phone_*`` tool names the agent transcript shows invoked.

    Scans the transcript line-by-line (agent JSON/JSONL output) plus a whole-text
    parse fallback, collecting tool-identity fields. Pure over the transcript
    text so it is unit-testable without a live agent.
    """
    invoked: set[str] = set()
    for line in transcript.splitlines():
        stripped = line.strip()
        if not stripped:
            continue
        try:
            invoked |= _tool_names_in_json(json.loads(stripped))
        except json.JSONDecodeError:
            continue
    with contextlib.suppress(json.JSONDecodeError):
        invoked |= _tool_names_in_json(json.loads(transcript))
    return invoked


def proc_net_has_listening_port(proc_net_text: str, port_hex: str) -> bool:
    """Whether /proc/net/tcp[6] text contains a socket on ``port_hex``.

    Lines look like ``  0: 0100007F:BA43 00000000:0000 0A ...`` where ``0A`` is
    the TCP LISTEN state. Matching the local-address ``:<PORT>`` field is enough
    to prove the companion RPC server bound the port.
    """
    needle = f":{port_hex.upper()}"
    for line in proc_net_text.splitlines():
        fields = line.split()
        if len(fields) < 2:
            continue
        local_address = fields[1].upper()
        if local_address.endswith(needle):
            return True
    return False


def accessibility_service_bound(dumpsys_text: str, component: str) -> bool:
    """Whether ``dumpsys accessibility`` reports ``component`` as enabled/bound."""
    pkg = component.split("/", 1)[0]
    for line in dumpsys_text.splitlines():
        lowered = line.lower()
        if ("enabled services" in lowered or "bound services" in lowered) and pkg in line:
            return True
    return False


def notification_listener_bound(dumpsys_text: str, component: str) -> bool:
    """Whether ``dumpsys notification`` shows a live bound listener for ``component``.

    The framework prints a live binder proxy line referencing the component once
    the listener is actually bound (not merely allow-listed).
    """
    pkg = component.split("/", 1)[0]
    for line in dumpsys_text.splitlines():
        if pkg in line and ("inotificationlistener" in line.lower() or "proxy" in line.lower()):
            return True
    return False


def accessibility_list_without(existing: str, component: str) -> str:
    """The ``enabled_accessibility_services`` value with ``component`` removed.

    Used by the cold reset to disable ONLY the sky-cua companion's accessibility
    service, never a user's other services. ``settings get`` renders an unset
    value as the literal ``null``.
    """
    trimmed = existing.strip()
    if trimmed.lower() == "null" or not trimmed:
        return ""
    kept = [
        entry
        for entry in trimmed.split(":")
        if entry.strip() and not _components_match(entry.strip(), component)
    ]
    return ":".join(kept)


def _components_match(left: str, right: str) -> bool:
    """Compare two flattened ComponentNames, expanding a leading-dot short class."""

    def parts(value: str) -> tuple[str, str] | None:
        package, _, cls = value.partition("/")
        if not package or not cls:
            return None
        package = package.strip()
        cls = cls.strip()
        qualified = f"{package}.{cls[1:]}" if cls.startswith(".") else cls
        return package, qualified

    left_parts = parts(left)
    right_parts = parts(right)
    if left_parts is None or right_parts is None:
        return left.strip() == right.strip()
    return left_parts == right_parts


@dataclass(frozen=True)
class CompanionCapsCheck:
    """Whether the companion capabilities prove setup, plus any missing fields."""

    ok: bool
    missing: tuple[str, ...]


def companion_setup_complete(companion_caps: dict[str, Any]) -> CompanionCapsCheck:
    """Check the `phone_companion_status` companion map for full setup."""
    required = (
        "installed",
        "rpc_reachable",
        "accessibility_enabled",
        "notification_listener_enabled",
    )
    missing = tuple(field for field in required if companion_caps.get(field) is not True)
    return CompanionCapsCheck(ok=not missing, missing=missing)


def format_result_line(results: Iterable[StepResult]) -> str:
    """Render the final ``RESULT phone_companion_setup_smoke ...`` summary line."""
    passed, skipped, failed = summarize_counts(results)
    return f"RESULT {SMOKE_NAME} passed={passed} skipped={skipped} failed={failed}"


# ---------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------


@dataclass
class SetupSmokeOptions:
    """Operator-supplied targeting for the setup smoke."""

    driver: str = DRIVER_AGENT
    agent: str = DEFAULT_AGENT
    model: str | None = None
    serial: str | None = None
    cold_reset: str = COLD_RESET_FULL
    installed: bool = False
    build_companion: bool = True
    require_tool_evidence: bool = False
    artifacts_dir: Path | None = None
    agent_timeout: float = 300.0


# ---------------------------------------------------------------------------
# ADB helpers (device-side ground truth + cold reset)
# ---------------------------------------------------------------------------


def _run_adb(adb: str, serial: str, args: list[str], *, timeout: float = 30.0) -> str:
    """Run ``adb -s <serial> <args...>`` and return stdout (best-effort)."""
    proc = subprocess.run(
        [adb, "-s", serial, *args],
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    return proc.stdout


def resolve_serial(options: SetupSmokeOptions, adb: str) -> str:
    """Resolve the device serial: explicit ``--serial`` then the single device.

    Raises ``SmokeSkip``-style messaging via ValueError so the caller can record
    an honest skip when zero or several devices are attached with no selector.
    """
    if options.serial:
        return options.serial
    proc = subprocess.run(
        [adb, "devices"], capture_output=True, text=True, timeout=30.0, check=False
    )
    serials = [
        line.split("\t", 1)[0].strip()
        for line in proc.stdout.splitlines()[1:]
        if "\tdevice" in line
    ]
    if not serials:
        raise SetupSkip("no authorized adb device attached")
    if len(serials) > 1:
        raise SetupSkip(
            f"{len(serials)} devices attached; pass --serial to pick one "
            f"({', '.join(sanitize_serial(s) for s in serials)})"
        )
    return serials[0]


def cold_reset(adb: str, serial: str, mode: str) -> list[str]:
    """Reset the companion to a cold state. Returns bounded notes for logging.

    ``full`` uninstalls the companion and removes its service entries; ``services``
    keeps the APK installed but disables its accessibility + notification services;
    ``none`` is a no-op. Only the sky-cua companion's own entries are touched.
    """
    notes: list[str] = []
    if mode == COLD_RESET_NONE:
        return ["cold reset skipped (--cold-reset none)"]

    # Remove ONLY our accessibility component, preserving any others, then clear
    # the global flag is left alone (other services may still need it).
    existing = _run_adb(
        adb, serial, ["shell", "settings", "get", "secure", "enabled_accessibility_services"]
    )
    remaining = accessibility_list_without(existing, ACCESSIBILITY_COMPONENT)
    if remaining:
        _run_adb(
            adb,
            serial,
            ["shell", "settings", "put", "secure", "enabled_accessibility_services", remaining],
        )
    else:
        _run_adb(
            adb, serial, ["shell", "settings", "delete", "secure", "enabled_accessibility_services"]
        )
        _run_adb(adb, serial, ["shell", "settings", "put", "secure", "accessibility_enabled", "0"])
    notes.append("accessibility service entry removed")

    # `cmd notification disallow_listener` removes only our listener, additively.
    _run_adb(
        adb, serial, ["shell", "cmd", "notification", "disallow_listener", NOTIFICATION_COMPONENT]
    )
    notes.append("notification listener disallowed")

    if mode == COLD_RESET_FULL:
        _run_adb(adb, serial, ["uninstall", COMPANION_PACKAGE])
        notes.append("companion uninstalled")
    return notes


@dataclass(frozen=True)
class GroundTruth:
    """Device-side ground truth captured straight after the driver runs."""

    companion_installed: bool
    companion_version: str | None
    accessibility_bound: bool
    notification_bound: bool
    rpc_listening: bool

    @property
    def setup_complete(self) -> bool:
        return (
            self.companion_installed
            and self.accessibility_bound
            and self.notification_bound
            and self.rpc_listening
        )

    @property
    def missing(self) -> tuple[str, ...]:
        problems: list[str] = []
        if not self.companion_installed:
            problems.append("companion_installed")
        if not self.accessibility_bound:
            problems.append("accessibility_bound")
        if not self.notification_bound:
            problems.append("notification_bound")
        if not self.rpc_listening:
            problems.append("rpc_listening")
        return tuple(problems)


def capture_ground_truth(adb: str, serial: str) -> GroundTruth:
    """Read the device's companion install + service-binding + RPC-port state."""
    dump_pkg = _run_adb(adb, serial, ["shell", "dumpsys", "package", COMPANION_PACKAGE])
    installed = "versionName=" in dump_pkg
    version: str | None = None
    for line in dump_pkg.splitlines():
        marker = "versionName="
        if marker in line:
            version = line.split(marker, 1)[1].split()[0].strip() or None
            break
    a11y = accessibility_service_bound(
        _run_adb(adb, serial, ["shell", "dumpsys", "accessibility"]), ACCESSIBILITY_COMPONENT
    )
    notif = notification_listener_bound(
        _run_adb(adb, serial, ["shell", "dumpsys", "notification"]), NOTIFICATION_COMPONENT
    )
    proc_net = _run_adb(adb, serial, ["shell", "cat", "/proc/net/tcp", "/proc/net/tcp6"])
    rpc = proc_net_has_listening_port(proc_net, RPC_PORT_HEX)
    return GroundTruth(
        companion_installed=installed,
        companion_version=version,
        accessibility_bound=a11y,
        notification_bound=notif,
        rpc_listening=rpc,
    )


# ---------------------------------------------------------------------------
# Exceptions mirroring the phone-smoke outcome contract
# ---------------------------------------------------------------------------


class SetupSkip(Exception):
    """A prerequisite is missing; the whole run collapses to one SKIP line."""


class SetupFailure(Exception):
    """The setup workflow is proven broken; collapses to one FAIL line."""


# ---------------------------------------------------------------------------
# MCP probe env + verification (auto-install OFF: pure check, never sets up)
# ---------------------------------------------------------------------------


def _phone_runtime_env(adb: str, *, auto_install: bool) -> dict[str, str]:
    """Phone runtime env for an MCP server: pin adb + the staged companion APK.

    The absolute APK path overrides any `SKY_CUA_REPO_ROOT` so the runtime always
    resolves the freshly staged companion (and its metadata sidecar). Auto-install
    and operator mode are pinned explicitly in BOTH directions so the value is
    asserted, never inherited from an ambient export: the setup path forces them
    on, the verification probe forces them off so a connect cannot set anything up.
    """
    enabled = "1" if auto_install else "0"
    return {
        "SKY_CUA_PHONE": "1",
        "SKY_CUA_ADB": adb,
        "SKY_CUA_PHONE_COMPANION_APK": str(STAGED_COMPANION_APK),
        "SKY_CUA_MODEL_SUPPORTS_IMAGES": "false",
        "SKY_CUA_PHONE_COMPANION_AUTO_INSTALL": enabled,
        "SKY_CUA_PHONE_COMPANION_OPERATOR_MODE": enabled,
    }


def _client_factory(adb: str, *, installed: bool, auto_install: bool) -> McpClient:
    base_env = dict(os.environ)
    base_env.update(_phone_runtime_env(adb, auto_install=auto_install))
    client_path = resolve_client_path(installed=installed)
    client = McpClient([str(client_path), "mcp"], base_env=base_env, read_timeout=60.0)
    client.initialize()
    return client


def verify_via_mcp(adb: str, serial: str, options: SetupSmokeOptions) -> list[StepResult]:
    """Pure MCP verification: connect with auto-install OFF and read companion status.

    Because auto-install/operator-mode are off, a reachable companion with enabled
    services here can only be the result of the driver's earlier setup.
    """
    steps: list[StepResult] = []
    client = _client_factory(adb, installed=options.installed, auto_install=False)
    try:
        smoke = PhoneSmoke(client, PhoneSmokeOptions(profile="companion", serial=serial))
        connect = smoke.connect(serial)
        if result_is_error(connect):
            raise SetupFailure(
                f"verify phone_connect failed: {first_diagnostic_message(connect) or 'no diagnostic'}"
            )
        session_id = structured(connect).get("session_id")
        if not isinstance(session_id, str) or not session_id:
            raise SetupFailure("verify phone_connect returned no session_id")

        status = smoke.companion_status(session_id)
        companion = structured(status).get("companion")
        companion = companion if isinstance(companion, dict) else {}
        check = companion_setup_complete(companion)
        detail = (
            f"installed={bounded_metadata(companion.get('installed'))} "
            f"rpc={bounded_metadata(companion.get('rpc_reachable'))} "
            f"a11y={bounded_metadata(companion.get('accessibility_enabled'))} "
            f"notif={bounded_metadata(companion.get('notification_listener_enabled'))}"
        )
        if not check.ok:
            codes = ",".join(diagnostic_codes(status)) or "no code"
            steps.append(
                step_fail(
                    "verify_companion_status",
                    f"{detail} missing={','.join(check.missing)} diag={codes}",
                )
            )
        else:
            steps.append(step_pass("verify_companion_status", detail))

        observe = smoke.observe(session_id)
        backend = bounded_metadata(structured(observe).get("backend"))
        if result_is_error(observe):
            steps.append(
                step_fail(
                    "verify_observe", f"phone_observe failed: {first_diagnostic_message(observe)}"
                )
            )
        elif backend == "companion":
            steps.append(step_pass("verify_observe", f"backend={backend}"))
        else:
            steps.append(step_fail("verify_observe", f"backend={backend}; expected companion"))
        smoke.disconnect(session_id)
    finally:
        client.close()
    return steps


# ---------------------------------------------------------------------------
# Drivers
# ---------------------------------------------------------------------------


def drive_direct(adb: str, serial: str, options: SetupSmokeOptions) -> list[StepResult]:
    """Drive the setup directly through the MCP tools (deterministic)."""
    steps: list[StepResult] = []
    client = _client_factory(adb, installed=options.installed, auto_install=True)
    try:
        smoke = PhoneSmoke(client, PhoneSmokeOptions(profile="companion", serial=serial))
        connect = smoke.call("phone_connect", {"serial": serial, "install_companion": True})
        if result_is_error(connect):
            raise SetupFailure(
                f"phone_connect failed: {first_diagnostic_message(connect) or 'no diagnostic'}"
            )
        session_id = structured(connect).get("session_id")
        steps.append(step_pass("phone_connect", f"serial={sanitize_serial(serial)}"))

        install_args: dict[str, Any] = {}
        if isinstance(session_id, str) and session_id:
            install_args["session_id"] = session_id
        install = smoke.call("phone_install_companion", install_args)
        if result_is_error(install):
            raise SetupFailure(
                f"phone_install_companion failed: {first_diagnostic_message(install) or 'no diagnostic'}"
            )
        steps.append(step_pass("phone_install_companion", "ok"))
    finally:
        client.close()
    return steps


def drive_agent(
    adb: str, serial: str, options: SetupSmokeOptions, artifact_dir: Path
) -> list[StepResult]:
    """Drive the setup through an agent CLI; record tool-evidence."""
    if shutil.which(options.agent) is None and options.agent != "claude":
        raise SetupSkip(f"agent CLI '{options.agent}' not found on PATH")

    # Surface phone runtime config to the agent's MCP server via the allowlisted
    # env (the agent forwards inherited env to its MCP child). The absolute APK
    # path is the robust override regardless of the agent's configured repo root.
    for key, value in _phone_runtime_env(adb, auto_install=True).items():
        os.environ[key] = value
    os.environ.setdefault("SKY_CUA_REPO_ROOT", str(REPO_ROOT))

    prompt = agent_setup_prompt(serial)
    (artifact_dir / "prompt.txt").write_text(prompt + "\n", encoding="utf-8")
    proc = run_agent(
        options.agent,
        prompt,
        artifact_dir,
        timeout=options.agent_timeout,
        model=options.model,
    )

    stdout_path = artifact_dir / f"{options.agent}.stdout.log"
    transcript = stdout_path.read_text(encoding="utf-8") if stdout_path.exists() else ""
    invoked = phone_tools_invoked(transcript)
    missing_tools = [tool for tool in REQUIRED_AGENT_TOOLS if tool not in invoked]

    steps: list[StepResult] = []
    if proc.returncode != 0:
        steps.append(
            step_fail(
                "agent_run", f"{options.agent} exited rc={proc.returncode}; see {stdout_path.name}"
            )
        )
    else:
        steps.append(step_pass("agent_run", f"{options.agent} rc=0"))

    evidence_detail = f"invoked={','.join(sorted(invoked)) or 'none'}"
    if not missing_tools:
        steps.append(step_pass("agent_tool_evidence", evidence_detail))
    elif options.require_tool_evidence:
        steps.append(
            step_fail("agent_tool_evidence", f"{evidence_detail} missing={','.join(missing_tools)}")
        )
    else:
        # Ground truth is the real gate; missing native tool-evidence is a soft
        # signal (the agent may have used a workaround). Record it as a skip.
        steps.append(
            step_skip("agent_tool_evidence", f"{evidence_detail} missing={','.join(missing_tools)}")
        )
    return steps


# ---------------------------------------------------------------------------
# Orchestration
# ---------------------------------------------------------------------------


def ensure_companion_apk(options: SetupSmokeOptions) -> str:
    """Stage the companion APK, building it if needed. Returns a bounded note."""
    if STAGED_COMPANION_APK.exists() and not options.build_companion:
        return f"using staged APK {STAGED_COMPANION_APK.name}"
    if not options.build_companion:
        raise SetupSkip(f"companion APK missing at {STAGED_COMPANION_APK} and --no-build set")
    outcome = build_and_stage_companion()
    if outcome.status == "skipped_no_toolchain" and not STAGED_COMPANION_APK.exists():
        raise SetupSkip("no Android toolchain to build the companion APK and none staged")
    if not STAGED_COMPANION_APK.exists():
        raise SetupSkip(f"companion APK still missing after build ({outcome.status})")
    return f"companion APK {outcome.status}"


def run_setup_smoke(
    options: SetupSmokeOptions,
    *,
    emit: Callable[[str], None] = print,
) -> list[StepResult]:
    """Run the full cold→setup→verify workflow and emit step lines."""
    steps: list[StepResult] = []

    def record(result: StepResult) -> None:
        steps.append(result)
        emit(
            f"{result.status} {SMOKE_NAME}.{result.name}"
            + (f" {result.detail}" if result.detail.strip() else "")
        )

    try:
        adb = adb_binary()
        if adb is None:
            raise SetupSkip("adb not found on PATH or via SKY_CUA_ADB")
        serial = resolve_serial(options, adb)
        record(step_pass("device", f"serial={sanitize_serial(serial)}"))

        record(step_pass("companion_apk", ensure_companion_apk(options)))

        reset_notes = cold_reset(adb, serial, options.cold_reset)
        record(step_pass("cold_reset", bounded_metadata("; ".join(reset_notes), max_len=160)))

        if options.driver == DRIVER_AGENT:
            artifact_dir = options.artifacts_dir or make_artifact_dir(
                options.agent, ARTIFACT_PROFILE
            )
            for result in drive_agent(adb, serial, options, artifact_dir):
                record(result)
        else:
            for result in drive_direct(adb, serial, options):
                record(result)

        # Ground truth first — straight from the device, independent of any
        # verify-connect that could otherwise re-trigger setup.
        truth = capture_ground_truth(adb, serial)
        truth_detail = (
            f"installed={truth.companion_installed} version={bounded_metadata(truth.companion_version)} "
            f"a11y={truth.accessibility_bound} notif={truth.notification_bound} "
            f"rpc={truth.rpc_listening}"
        )
        if truth.setup_complete:
            record(step_pass("ground_truth", truth_detail))
        else:
            record(step_fail("ground_truth", f"{truth_detail} missing={','.join(truth.missing)}"))

        for result in verify_via_mcp(adb, serial, options):
            record(result)

    except SetupSkip as skip:
        record(step_skip(SMOKE_NAME, str(skip)))
    except SetupFailure as failure:
        record(step_fail(SMOKE_NAME, str(failure)))
    except Exception as error:
        # Never crash the run: any unexpected error becomes a FAIL line.
        record(step_fail(SMOKE_NAME, bounded_metadata(f"unexpected error: {error}", max_len=180)))

    emit(format_result_line(steps))
    return steps


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Live phone-companion setup smoke (cold device -> reachable companion)."
    )
    parser.add_argument(
        "--driver",
        choices=DRIVER_CHOICES,
        default=DRIVER_AGENT,
        help="How setup is driven: an agent CLI, or the MCP tools directly.",
    )
    parser.add_argument(
        "--agent",
        choices=AGENT_CHOICES,
        default=DEFAULT_AGENT,
        help="Agent CLI to drive setup (only used with --driver agent).",
    )
    parser.add_argument("--model", default=None, help="Agent model override.")
    parser.add_argument("--serial", default=None, help="Device serial; auto-detected when single.")
    parser.add_argument(
        "--cold-reset",
        choices=COLD_RESET_CHOICES,
        default=COLD_RESET_FULL,
        help="full: uninstall+disable services; services: disable only; none: skip.",
    )
    parser.add_argument(
        "--installed",
        action="store_true",
        help="Drive the staged installed client instead of the dev build.",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Do not build the companion APK; require it already staged.",
    )
    parser.add_argument(
        "--require-tool-evidence",
        action="store_true",
        help="Fail if the agent transcript lacks phone_connect + phone_install_companion.",
    )
    parser.add_argument(
        "--agent-timeout",
        type=float,
        default=300.0,
        help="Seconds to allow the agent CLI to run (default 300).",
    )
    return parser


def options_from_args(args: argparse.Namespace) -> SetupSmokeOptions:
    return SetupSmokeOptions(
        driver=args.driver,
        agent=args.agent,
        model=args.model,
        serial=args.serial,
        cold_reset=args.cold_reset,
        installed=bool(args.installed),
        build_companion=not bool(args.no_build),
        require_tool_evidence=bool(args.require_tool_evidence),
        agent_timeout=float(args.agent_timeout),
    )


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    options = options_from_args(args)
    results = run_setup_smoke(options)
    _, _, failed = summarize_counts(results)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
