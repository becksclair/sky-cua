#!/usr/bin/env python3
"""Probe the sky-cua MCP tool surface through real stdio transport.

The probe is intentionally small and host-safe: it verifies advertised tools,
profile isolation, compact response envelopes, and degraded-but-structured
status branches without needing a particular desktop app, browser tab, or
attached Android device.
"""

from __future__ import annotations

import argparse
import json
import os
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from _mcp_stdio import McpClient

REPO_ROOT = Path(__file__).resolve().parents[1]
DEV_CLIENT = REPO_ROOT / "bin" / "sky-cua-client"
INSTALLED_CLIENT = REPO_ROOT / "dist" / "plugin" / "sky-cua" / "bin" / "sky-cua-client"

COMPACT_REQUIRED_TOOLS: frozenset[str] = frozenset(
    {
        "doctor",
        "status",
        "list_resources",
        "observe",
        "capture_screen",
        "capture_desktop",
        "setup_desktop",
        "session_presence",
        "desktop_pointer",
        "desktop_keyboard",
        "desktop_action",
        "browser_input",
        "phone_connection",
        "phone_pointer",
        "phone_keyboard",
        "phone_app_action",
        "phone_notifications",
        "phone_accessibility_tree",
    }
)

LEGACY_SENTINELS: frozenset[str] = frozenset(
    {
        "list_apps",
        "get_app_state",
        "click",
        "type_text",
        "browser_list_tabs",
        "browser_click",
        "phone_connect",
        "phone_status",
    }
)

LEGACY_REQUIRED_TOOLS: frozenset[str] = frozenset(
    {
        "list_apps",
        "get_app_state",
        "click",
        "type_text",
        "browser_status",
        "browser_list_tabs",
        "phone_status",
        "phone_list_devices",
        "phone_connect",
    }
)

COMPACT_SENTINELS: frozenset[str] = frozenset(
    {
        "status",
        "list_resources",
        "desktop_pointer",
        "browser_input",
        "phone_connection",
    }
)


class ProbeFailure(Exception):
    """Raised when the MCP surface violates the requested contract."""


@dataclass(frozen=True)
class ProbeStep:
    status: str
    name: str
    detail: str = ""


def step_pass(name: str, detail: str = "") -> ProbeStep:
    return ProbeStep("PASS", name, detail)


def format_step(step: ProbeStep) -> str:
    if step.detail:
        return f"{step.status} {step.name} {step.detail}"
    return f"{step.status} {step.name}"


def tool_names(tools: Iterable[dict[str, Any]]) -> set[str]:
    names: set[str] = set()
    for tool in tools:
        name = tool.get("name")
        if isinstance(name, str):
            names.add(name)
    return names


def require_tools(names: set[str], required: frozenset[str], *, profile: str) -> None:
    missing = sorted(required - names)
    if missing:
        raise ProbeFailure(f"{profile} tools/list is missing required tools: {missing!r}")


def forbid_tools(names: set[str], forbidden: frozenset[str], *, profile: str) -> None:
    present = sorted(forbidden & names)
    if present:
        raise ProbeFailure(f"{profile} tools/list advertised inactive tools: {present!r}")


def compact_payload(result: dict[str, Any], *, tool: str, branch: str) -> dict[str, Any]:
    payload = result.get("structuredContent")
    if not isinstance(payload, dict):
        raise ProbeFailure(f"{tool} returned no compact structuredContent: {result!r}")
    expected = {
        "profile": "compact",
        "tool": tool,
        "branch": branch,
    }
    for key, value in expected.items():
        if payload.get(key) != value:
            raise ProbeFailure(
                f"{tool} compact envelope has wrong {key}: "
                f"expected {value!r}, got {payload.get(key)!r}"
            )
    if "legacy_tool" not in payload:
        raise ProbeFailure(f"{tool} compact envelope omitted legacy_tool: {payload!r}")
    if not isinstance(payload.get("result"), dict):
        raise ProbeFailure(f"{tool} compact envelope omitted result map: {payload!r}")
    return payload


def compact_error_payload(result: dict[str, Any], *, tool: str, code: str) -> dict[str, Any]:
    if result.get("isError") is not True:
        raise ProbeFailure(f"{tool} invalid branch did not set isError: {result!r}")
    payload = result.get("structuredContent")
    if not isinstance(payload, dict):
        raise ProbeFailure(f"{tool} invalid branch returned no structuredContent: {result!r}")
    if payload.get("profile") != "compact" or payload.get("tool") != tool:
        raise ProbeFailure(f"{tool} invalid branch returned wrong compact identity: {payload!r}")
    error = payload.get("error")
    if not isinstance(error, dict) or error.get("code") != code:
        raise ProbeFailure(f"{tool} invalid branch returned wrong error code: {payload!r}")
    return payload


def tool_error_code(response: dict[str, Any]) -> str | None:
    error = response.get("error")
    if isinstance(error, dict):
        data = error.get("data")
        if isinstance(data, dict) and isinstance(data.get("code"), str):
            return data["code"]
    result = response.get("result")
    if isinstance(result, dict):
        structured = result.get("structuredContent")
        if isinstance(structured, dict) and isinstance(structured.get("code"), str):
            return structured["code"]
    return None


def resolve_client(installed: bool) -> Path:
    client = INSTALLED_CLIENT if installed else DEV_CLIENT
    if not client.exists():
        raise FileNotFoundError(f"MCP client binary not found: {client}")
    return client


def make_client(profile: str, *, installed: bool, phone_enabled: bool) -> McpClient:
    env = dict(os.environ)
    env["SKY_CUA_MCP_TOOL_PROFILE"] = profile
    env.setdefault("SKY_CUA_MODEL_SUPPORTS_IMAGES", "false")
    if phone_enabled:
        env.setdefault("SKY_CUA_PHONE", "1")
    return McpClient(
        [str(resolve_client(installed)), "mcp"],
        base_env=env,
        client_name=f"mcp-tool-surface-{profile}-probe",
    )


def probe_compact(*, installed: bool, phone_enabled: bool) -> list[ProbeStep]:
    steps: list[ProbeStep] = []
    client = make_client("compact", installed=installed, phone_enabled=phone_enabled)
    try:
        client.initialize()
        names = tool_names(client.tools_list())
        require_tools(names, COMPACT_REQUIRED_TOOLS, profile="compact")
        forbid_tools(names, LEGACY_SENTINELS, profile="compact")
        steps.append(step_pass("compact.tools_list", f"tools={len(names)}"))

        invalid = client.tools_call(10, "status", {"component": "__invalid__"})
        compact_error_payload(invalid, tool="status", code="InvalidRequest")
        steps.append(step_pass("compact.invalid_branch", "code=InvalidRequest"))

        for request_id, tool, arguments, branch in (
            (11, "status", {"component": "browser"}, "browser"),
            (12, "status", {"component": "phone"}, "phone"),
            (13, "status", {"component": "session_presence"}, "session_presence"),
            (14, "list_resources", {"surface": "desktop", "resource": "apps"}, "desktop/apps"),
            (15, "list_resources", {"surface": "phone", "resource": "devices"}, "phone/devices"),
        ):
            result = client.tools_call(request_id, tool, arguments)
            payload = compact_payload(result, tool=tool, branch=branch)
            legacy_tool = payload.get("legacy_tool")
            result_error = bool(payload["result"].get("isError"))
            steps.append(
                step_pass(
                    f"compact.{tool}.{branch.replace('/', '_')}",
                    f"legacy={legacy_tool} result_error={str(result_error).lower()}",
                )
            )

        legacy_call = client.call_raw(30, "tools/call", {"name": "phone_status", "arguments": {}})
        if tool_error_code(legacy_call.raw) != "ToolNotInActiveProfile":
            raise ProbeFailure(
                f"compact profile did not reject inactive legacy phone_status: {legacy_call.raw!r}"
            )
        steps.append(step_pass("compact.inactive_legacy_rejected", "code=ToolNotInActiveProfile"))
    finally:
        client.close()
    return steps


def probe_legacy(*, installed: bool, phone_enabled: bool) -> list[ProbeStep]:
    steps: list[ProbeStep] = []
    client = make_client("legacy", installed=installed, phone_enabled=phone_enabled)
    try:
        client.initialize()
        names = tool_names(client.tools_list())
        require_tools(names, LEGACY_REQUIRED_TOOLS, profile="legacy")
        forbid_tools(names, COMPACT_SENTINELS, profile="legacy")
        steps.append(step_pass("legacy.tools_list", f"tools={len(names)}"))

        compact_call = client.call_raw(
            40,
            "tools/call",
            {"name": "status", "arguments": {"component": "phone"}},
        )
        if tool_error_code(compact_call.raw) != "ToolNotInActiveProfile":
            raise ProbeFailure(
                f"legacy profile did not reject inactive compact status: {compact_call.raw!r}"
            )
        steps.append(step_pass("legacy.inactive_compact_rejected", "code=ToolNotInActiveProfile"))
    finally:
        client.close()
    return steps


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("compact", "legacy", "both"), default="both")
    parser.add_argument(
        "--installed",
        action="store_true",
        help="Probe dist/plugin/sky-cua/bin/sky-cua-client instead of bin/sky-cua-client.",
    )
    parser.add_argument(
        "--no-phone",
        action="store_true",
        help="Do not set SKY_CUA_PHONE=1 for the probe process.",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit the result as JSON instead of PASS lines.",
    )
    return parser


def run(profile: str, *, installed: bool, phone_enabled: bool) -> list[ProbeStep]:
    steps: list[ProbeStep] = []
    if profile in {"compact", "both"}:
        steps.extend(probe_compact(installed=installed, phone_enabled=phone_enabled))
    if profile in {"legacy", "both"}:
        steps.extend(probe_legacy(installed=installed, phone_enabled=phone_enabled))
    return steps


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    steps = run(args.profile, installed=bool(args.installed), phone_enabled=not args.no_phone)
    if args.json:
        print(json.dumps({"ok": True, "steps": [step.__dict__ for step in steps]}, indent=2))
    else:
        for step in steps:
            print(format_step(step))
        print(f"RESULT mcp_tool_surface_probe passed={len(steps)} failed=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
