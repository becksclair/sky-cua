#!/usr/bin/env python3
"""Probe the sky-cua MCP tool surface through real stdio transport.

The probe is intentionally small and host-safe: it verifies advertised tools,
grouped response envelopes, and degraded-but-structured status branches
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

GROUPED_TOOLS: frozenset[str] = frozenset(
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


def require_exact_grouped_tools(names: set[str]) -> None:
    expected = set(GROUPED_TOOLS)
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
        raise ProbeFailure("tools/list does not match grouped surface: " + " ".join(details))


def require_grouped_action_shape(tools: Iterable[dict[str, Any]]) -> None:
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

    require_hardened_schema_shape(by_name)


def require_hardened_schema_shape(by_name: dict[str, dict[str, Any]]) -> None:
    browser_move = _tool_schema(by_name, "browser_move_mouse")
    browser_input = _tool_schema(by_name, "browser_input")
    browser_scroll = _tool_schema(by_name, "browser_scroll")
    if "wait_for_arrival" not in _properties(browser_move):
        raise ProbeFailure("browser_move_mouse must expose wait_for_arrival")
    for name, schema in [("browser_input", browser_input), ("browser_scroll", browser_scroll)]:
        if "wait_for_arrival" in _properties(schema):
            raise ProbeFailure(f"{name} must not expose wait_for_arrival")

    browser_open = _tool_schema(by_name, "browser_open")
    if (
        _properties(browser_open).get("url", {}).get("pattern")
        != r"^(https?://[^\s]+|about:blank)$"
    ):
        raise ProbeFailure("browser URL schema must use the anchored HTTP/about:blank pattern")

    pointer = _tool_schema(by_name, "phone_pointer")
    if not _all_of_has_then_required(pointer, "tap", ["session_id", "x", "y"]):
        raise ProbeFailure("phone_pointer tap must require session_id and tap coordinates")
    if not _conditional_then_has_snapshot_or_raw(pointer, "tap"):
        raise ProbeFailure("phone_pointer tap must require phone_snapshot_id or raw coordinates")
    if not _all_of_has_then_required(
        pointer, "swipe", ["session_id", "start_x", "start_y", "end_x", "end_y"]
    ):
        raise ProbeFailure("phone_pointer swipe must require session_id and swipe coordinates")
    if not _conditional_then_has_snapshot_or_raw(pointer, "swipe"):
        raise ProbeFailure("phone_pointer swipe must require phone_snapshot_id or raw coordinates")

    observe_backend = _properties(_tool_schema(by_name, "observe")).get("backend", {})
    capture_backend = _properties(_tool_schema(by_name, "capture_screen")).get("backend", {})
    connect_backend = _properties(_tool_schema(by_name, "phone_connection")).get("backend", {})
    for name, backend in [("observe", observe_backend), ("capture_screen", capture_backend)]:
        enum = backend.get("enum")
        if not isinstance(enum, list) or "none" in enum or "scrcpy" in enum:
            raise ProbeFailure(f"{name} backend request enum must exclude none and scrcpy")
    connect_enum = connect_backend.get("enum")
    if not isinstance(connect_enum, list) or "none" in connect_enum:
        raise ProbeFailure("phone_connection backend request enum must exclude none")
    if not _all_of_has_then_required(
        _tool_schema(by_name, "phone_connection"), "disconnect", ["session_id"]
    ):
        raise ProbeFailure("phone_connection disconnect must require session_id")

    desktop_capture = _tool_schema(by_name, "capture_desktop")
    desktop_capture_props = _properties(desktop_capture)
    for name in ["display_id", "display_name"]:
        if desktop_capture_props.get(name, {}).get("minLength") != 1:
            raise ProbeFailure(f"capture_desktop {name} must reject empty strings")
    if not _schema_contains_not_anyof_required(desktop_capture, "window_id", "display_id"):
        raise ProbeFailure("capture_desktop must reject mixed window/display selectors")
    if not _schema_contains_not_anyof_required(
        desktop_capture, "capture_all_displays", "display_id"
    ):
        raise ProbeFailure(
            "capture_desktop must reject capture_all_displays with display selectors"
        )

    desktop_scroll_props = _properties(_tool_schema(by_name, "desktop_scroll"))
    if "pages" not in desktop_scroll_props or desktop_scroll_props["pages"].get("minimum") != 1:
        raise ProbeFailure("desktop_scroll must expose positive pages")
    if "steps" in desktop_scroll_props or "delta_y" in desktop_scroll_props:
        raise ProbeFailure("desktop_scroll must not expose legacy magnitude fields")

    install_props = _properties(_tool_schema(by_name, "phone_app_install"))
    if "apk_path" in install_props:
        raise ProbeFailure("phone_app_install must not expose apk_path alias")
    install_required = _tool_schema(by_name, "phone_app_install").get("required")
    if install_required != ["session_id", "apk_paths"]:
        raise ProbeFailure("phone_app_install must require session_id and apk_paths")
    if "activity" in _properties(_tool_schema(by_name, "phone_app_action")):
        raise ProbeFailure("phone_app_action must not expose activity")

    for name in ["phone_setup", "phone_app_force_stop"]:
        annotations = by_name[name].get("annotations")
        if (
            not isinstance(annotations, dict)
            or annotations.get("destructiveHint") is not True
            or annotations.get("idempotentHint") is not True
        ):
            raise ProbeFailure(f"{name} must be destructive and idempotent")


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


def _properties(schema: dict[str, Any]) -> dict[str, Any]:
    properties = schema.get("properties")
    if isinstance(properties, dict):
        return properties
    return {}


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
        and _required_contains(entry["then"], required)
        for entry in _all_of(schema)
    )


def _required_contains(schema: dict[str, Any], required: list[str]) -> bool:
    actual = schema.get("required")
    return isinstance(actual, list) and set(required).issubset(actual)


def _conditional_then_has_snapshot_or_raw(schema: dict[str, Any], operation: str) -> bool:
    for entry in _all_of(schema):
        if _conditional_operation(entry) != operation:
            continue
        then = entry.get("then")
        if not isinstance(then, dict):
            return False
        any_of = then.get("anyOf")
        if not isinstance(any_of, list):
            return False
        has_snapshot = any(
            isinstance(branch, dict) and branch.get("required") == ["phone_snapshot_id"]
            for branch in any_of
        )
        has_raw = any(
            isinstance(branch, dict)
            and branch.get("required") == ["use_device_coordinates"]
            and _properties(branch).get("use_device_coordinates", {}).get("const") is True
            for branch in any_of
        )
        return has_snapshot and has_raw
    return False


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


def _schema_contains_not_anyof_required(schema: dict[str, Any], first: str, second: str) -> bool:
    for entry in _all_of(schema):
        rejected = entry.get("not")
        if not isinstance(rejected, dict):
            continue
        any_of = rejected.get("anyOf")
        if not isinstance(any_of, list):
            continue
        for branch in any_of:
            if not isinstance(branch, dict):
                continue
            required = branch.get("required")
            if isinstance(required, list) and first in required and second in required:
                return True
    return False


def grouped_payload(result: dict[str, Any], *, tool: str, branch: str) -> dict[str, Any]:
    payload = result.get("structuredContent")
    if not isinstance(payload, dict):
        raise ProbeFailure(f"{tool} returned no grouped structuredContent: {result!r}")
    expected = {
        "tool": tool,
        "branch": branch,
    }
    for key, value in expected.items():
        if payload.get(key) != value:
            raise ProbeFailure(
                f"{tool} grouped envelope has wrong {key}: "
                f"expected {value!r}, got {payload.get(key)!r}"
            )
    if not isinstance(payload.get("result"), dict):
        raise ProbeFailure(f"{tool} grouped envelope omitted result map: {payload!r}")
    return payload


def grouped_error_payload(result: dict[str, Any], *, tool: str, code: str) -> dict[str, Any]:
    if result.get("isError") is not True:
        raise ProbeFailure(f"{tool} invalid branch did not set isError: {result!r}")
    payload = result.get("structuredContent")
    if not isinstance(payload, dict):
        raise ProbeFailure(f"{tool} invalid branch returned no structuredContent: {result!r}")
    if payload.get("tool") != tool:
        raise ProbeFailure(f"{tool} invalid branch returned wrong grouped identity: {payload!r}")
    if payload.get("branch") is not None:
        raise ProbeFailure(f"{tool} invalid branch did not return branch=null: {payload!r}")
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


def probe_grouped(*, installed: bool, phone_enabled: bool) -> list[ProbeStep]:
    steps: list[ProbeStep] = []
    client = make_client(installed=installed, phone_enabled=phone_enabled)
    try:
        client.initialize()
        tools = client.tools_list()
        names = tool_names(tools)
        require_exact_grouped_tools(names)
        require_grouped_action_shape(tools)
        steps.append(step_pass("grouped.tools_list", f"tools={len(names)}"))

        invalid_cases = [
            (10, "status", {"component": "__invalid__"}),
            (11, "doctor", {"unexpected": True}),
            (
                12,
                "browser_input",
                {"operation": "type_text", "tab_id": "tab-1", "text": "hello", "x": 1, "y": 1},
            ),
            (
                13,
                "phone_pointer",
                {"operation": "tap", "session_id": "phone-1", "x": 1, "y": 1},
            ),
        ]
        for request_id, tool, arguments in invalid_cases:
            invalid = client.tools_call(request_id, tool, arguments)
            grouped_error_payload(invalid, tool=tool, code="InvalidRequest")
        steps.append(step_pass("grouped.invalid_branch", f"cases={len(invalid_cases)}"))

        for request_id, tool, arguments, branch in (
            (20, "status", {"component": "browser"}, "browser"),
            (21, "status", {"component": "phone"}, "phone"),
            (22, "status", {"component": "session_presence"}, "session_presence"),
            (23, "list_resources", {"surface": "desktop", "resource": "apps"}, "desktop/apps"),
            (24, "list_resources", {"surface": "phone", "resource": "devices"}, "phone/devices"),
        ):
            result = client.tools_call(request_id, tool, arguments)
            grouped_payload(result, tool=tool, branch=branch)
            result_error = result.get("isError") is True
            steps.append(
                step_pass(
                    f"grouped.{tool}.{branch.replace('/', '_')}",
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
    return probe_grouped(installed=installed, phone_enabled=phone_enabled)


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
