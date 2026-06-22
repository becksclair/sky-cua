#!/usr/bin/env python3
"""Probe the sky-cua MCP tool surface through real stdio transport.

The probe is intentionally small and host-safe: it verifies advertised tools,
canonical response envelopes, and degraded-but-structured status branches
without needing a particular desktop app, browser tab, or attached Android
device.
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

CANONICAL_TOOLS: frozenset[str] = frozenset(
    {
        "activate_window",
        "browser_claim_tab",
        "browser_input",
        "browser_move_mouse",
        "browser_navigate",
        "browser_open",
        "browser_scroll",
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
        "desktop_scroll",
        "desktop_semantic",
        "desktop_set_value",
        "desktop_toggle",
        "phone_connection",
        "phone_pair_wireless",
        "phone_setup",
        "phone_app_force_stop",
        "phone_pointer",
        "phone_keyboard",
        "phone_app_action",
        "phone_app_install",
        "phone_notification_action",
        "phone_notification_reply",
        "phone_notifications",
        "phone_accessibility_tree",
    }
)

BROWSER_EVAL_TOOL = "browser_eval"


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


def require_exact_canonical_tools(names: set[str]) -> None:
    expected = set(CANONICAL_TOOLS)
    if BROWSER_EVAL_TOOL in names:
        expected.add(BROWSER_EVAL_TOOL)
    missing = sorted(expected - names)
    extra = sorted(names - expected)
    if missing or extra:
        details = []
        if missing:
            details.append(f"missing={missing!r}")
        if extra:
            details.append(f"extra={extra!r}")
        raise ProbeFailure("tools/list does not match canonical surface: " + " ".join(details))


def require_canonical_action_shape(tools: Iterable[dict[str, Any]]) -> None:
    by_name: dict[str, dict[str, Any]] = {}
    for tool in tools:
        name = tool.get("name")
        if isinstance(name, str):
            by_name[name] = tool
    doctor = by_name.get("doctor")
    if not isinstance(doctor, dict):
        raise ProbeFailure("tools/list omitted doctor")
    doctor_annotations = doctor.get("annotations")
    if (
        not isinstance(doctor_annotations, dict)
        or doctor_annotations.get("readOnlyHint") is not True
    ):
        raise ProbeFailure(f"doctor must be read-only diagnostics: {doctor!r}")

    pointer = _tool_schema(by_name, "desktop_pointer")
    pointer_description = str(by_name["desktop_pointer"].get("description", ""))
    if "do not call with only operation" not in pointer_description:
        raise ProbeFailure("desktop_pointer description must reject operation-only calls")
    if not _all_of_has_conditional_required(pointer, "click", ["x", "y"]):
        raise ProbeFailure("desktop_pointer click branch must require coordinates or selector")

    action = _tool_schema(by_name, "desktop_action")
    action_description = str(by_name["desktop_action"].get("description", ""))
    if "do not call with only operation" not in action_description:
        raise ProbeFailure("desktop_action description must reject operation-only calls")
    if not _schema_has_any_required(
        action, ["element_index", "element_identifier", "name", "text"]
    ):
        raise ProbeFailure("desktop_action must require a concrete selector")

    keyboard = _tool_schema(by_name, "desktop_keyboard")
    if not _all_of_has_then_required(keyboard, "press_key", ["key"]):
        raise ProbeFailure("desktop_keyboard press_key branch must require key")
    if not _all_of_has_then_required(keyboard, "type_text", ["text"]):
        raise ProbeFailure("desktop_keyboard type_text branch must require text")


def _tool_schema(by_name: dict[str, dict[str, Any]], name: str) -> dict[str, Any]:
    tool = by_name.get(name)
    if not isinstance(tool, dict):
        raise ProbeFailure(f"tools/list omitted {name}")
    schema = tool.get("inputSchema")
    if not isinstance(schema, dict):
        raise ProbeFailure(f"{name} omitted object inputSchema")
    return schema


def _all_of(schema: dict[str, Any]) -> list[dict[str, Any]]:
    value = schema.get("allOf")
    if not isinstance(value, list):
        return []
    return [entry for entry in value if isinstance(entry, dict)]


def _all_of_has_conditional_required(
    schema: dict[str, Any], operation: str, required: list[str]
) -> bool:
    return any(
        _conditional_operation(entry) == operation
        and _schema_has_any_required(entry.get("then"), required)
        for entry in _all_of(schema)
    )


def _all_of_has_then_required(schema: dict[str, Any], operation: str, required: list[str]) -> bool:
    return any(
        _conditional_operation(entry) == operation
        and isinstance(entry.get("then"), dict)
        and entry["then"].get("required") == required
        for entry in _all_of(schema)
    )


def _conditional_operation(schema: dict[str, Any]) -> str | None:
    condition = schema.get("if")
    if not isinstance(condition, dict):
        return None
    properties = condition.get("properties")
    if not isinstance(properties, dict):
        return None
    operation = properties.get("operation")
    if not isinstance(operation, dict):
        return None
    value = operation.get("const")
    return value if isinstance(value, str) else None


def _schema_has_any_required(schema: object, required: list[str]) -> bool:
    if not isinstance(schema, dict):
        return False
    all_of = schema.get("allOf")
    if isinstance(all_of, list) and any(
        _schema_has_any_required(entry, required) for entry in all_of
    ):
        return True
    any_of = schema.get("anyOf")
    if isinstance(any_of, list):
        return any(
            isinstance(entry, dict)
            and isinstance(entry.get("required"), list)
            and any(name in entry["required"] for name in required)
            for entry in any_of
        )
    return isinstance(schema.get("required"), list) and any(
        name in schema["required"] for name in required
    )


def canonical_payload(result: dict[str, Any], *, tool: str, branch: str) -> dict[str, Any]:
    payload = result.get("structuredContent")
    if not isinstance(payload, dict):
        raise ProbeFailure(f"{tool} returned no canonical structuredContent: {result!r}")
    expected = {
        "tool": tool,
        "branch": branch,
    }
    for key, value in expected.items():
        if payload.get(key) != value:
            raise ProbeFailure(
                f"{tool} canonical envelope has wrong {key}: "
                f"expected {value!r}, got {payload.get(key)!r}"
            )
    if not isinstance(payload.get("result"), dict):
        raise ProbeFailure(f"{tool} canonical envelope omitted result map: {payload!r}")
    return payload


def canonical_error_payload(result: dict[str, Any], *, tool: str, code: str) -> dict[str, Any]:
    if result.get("isError") is not True:
        raise ProbeFailure(f"{tool} invalid branch did not set isError: {result!r}")
    payload = result.get("structuredContent")
    if not isinstance(payload, dict):
        raise ProbeFailure(f"{tool} invalid branch returned no structuredContent: {result!r}")
    if payload.get("tool") != tool:
        raise ProbeFailure(f"{tool} invalid branch returned wrong canonical identity: {payload!r}")
    error = payload.get("error")
    if not isinstance(error, dict) or error.get("code") != code:
        raise ProbeFailure(f"{tool} invalid branch returned wrong error code: {payload!r}")
    return payload


def resolve_client(installed: bool) -> Path:
    client = INSTALLED_CLIENT if installed else DEV_CLIENT
    if not client.exists():
        raise FileNotFoundError(f"MCP client binary not found: {client}")
    return client


def make_client(*, installed: bool, phone_enabled: bool) -> McpClient:
    env = dict(os.environ)
    env.pop("SKY_CUA_MCP_TOOL_PROFILE", None)
    env.setdefault("SKY_CUA_MODEL_SUPPORTS_IMAGES", "false")
    if phone_enabled:
        env.setdefault("SKY_CUA_PHONE", "1")
    return McpClient(
        [str(resolve_client(installed)), "mcp"],
        base_env=env,
        client_name="mcp-tool-surface-probe",
    )


def probe_canonical(*, installed: bool, phone_enabled: bool) -> list[ProbeStep]:
    steps: list[ProbeStep] = []
    client = make_client(installed=installed, phone_enabled=phone_enabled)
    try:
        client.initialize()
        tools = client.tools_list()
        names = tool_names(tools)
        require_exact_canonical_tools(names)
        require_canonical_action_shape(tools)
        steps.append(step_pass("canonical.tools_list", f"tools={len(names)}"))

        invalid = client.tools_call(10, "status", {"component": "__invalid__"})
        canonical_error_payload(invalid, tool="status", code="InvalidRequest")
        steps.append(step_pass("canonical.invalid_branch", "code=InvalidRequest"))

        for request_id, tool, arguments, branch in (
            (11, "status", {"component": "browser"}, "browser"),
            (12, "status", {"component": "phone"}, "phone"),
            (13, "status", {"component": "session_presence"}, "session_presence"),
            (14, "list_resources", {"surface": "desktop", "resource": "apps"}, "desktop/apps"),
            (15, "list_resources", {"surface": "phone", "resource": "devices"}, "phone/devices"),
        ):
            result = client.tools_call(request_id, tool, arguments)
            payload = canonical_payload(result, tool=tool, branch=branch)
            result_error = bool(payload["result"].get("isError"))
            steps.append(
                step_pass(
                    f"canonical.{tool}.{branch.replace('/', '_')}",
                    f"result_error={str(result_error).lower()}",
                )
            )

    finally:
        client.close()
    return steps


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
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


def run(*, installed: bool, phone_enabled: bool) -> list[ProbeStep]:
    return probe_canonical(installed=installed, phone_enabled=phone_enabled)


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    steps = run(installed=bool(args.installed), phone_enabled=not args.no_phone)
    if args.json:
        print(json.dumps({"ok": True, "steps": [step.__dict__ for step in steps]}, indent=2))
    else:
        for step in steps:
            print(format_step(step))
        print(f"RESULT mcp_tool_surface_probe passed={len(steps)} failed=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
