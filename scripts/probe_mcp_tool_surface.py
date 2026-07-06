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
from _plugin_bundle import DEFAULT_CODEX_HOME, installed_plugin_root

REPO_ROOT = Path(__file__).resolve().parents[1]
DEV_CLIENT = REPO_ROOT / "bin" / "sky-cua-client"

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
        "desktop_launch_app",
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


def require_no_top_level_composition(tools: Iterable[dict[str, Any]]) -> None:
    """Every advertised inputSchema must be flat at the top level.

    Top-level allOf/oneOf/anyOf/not on an advertised inputSchema is exactly what
    makes the Anthropic Messages API / Claude Code drop the tool. The rich
    per-branch constraints live in a separate, non-advertised validation schema
    and stay enforced at runtime.
    """
    for tool in tools:
        name = tool.get("name")
        if not isinstance(name, str):
            continue
        schema = tool.get("inputSchema")
        if not isinstance(schema, dict):
            continue
        for keyword in ("allOf", "oneOf", "anyOf", "not"):
            if keyword in schema:
                raise ProbeFailure(f"{name} inputSchema must not advertise top-level {keyword}")


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
    pointer_description = str(by_name["desktop_pointer"].get("description", "")).lower()
    if "do not call with only operation" not in pointer_description:
        raise ProbeFailure("desktop_pointer description must reject operation-only calls")
    pointer_props = _properties(pointer)
    for field in ("x", "y"):
        if field not in pointer_props:
            raise ProbeFailure(f"desktop_pointer must advertise {field} coordinate")

    action = _tool_schema(by_name, "desktop_action")
    action_description = str(by_name["desktop_action"].get("description", "")).lower()
    if "do not call with only operation" not in action_description:
        raise ProbeFailure("desktop_action description must reject operation-only calls")
    action_props = _properties(action)
    if not any(
        field in action_props
        for field in ("element_index", "element_identifier", "name", "action_name", "text")
    ):
        raise ProbeFailure("desktop_action must advertise a concrete selector field")

    keyboard = _tool_schema(by_name, "desktop_keyboard")
    keyboard_props = _properties(keyboard)
    for field in ("key", "text"):
        if field not in keyboard_props:
            raise ProbeFailure(f"desktop_keyboard must advertise {field}")

    require_hardened_schema_shape(by_name)


def require_hardened_schema_shape(by_name: dict[str, dict[str, Any]]) -> None:
    status_props = _properties(_tool_schema(by_name, "status"))
    if "refresh_devices" not in status_props:
        raise ProbeFailure("status must expose refresh_devices")
    if "session_id" not in status_props:
        raise ProbeFailure("status must expose session_id")

    browser_move = _tool_schema(by_name, "browser_move_mouse")
    browser_input = _tool_schema(by_name, "browser_input")
    browser_scroll = _tool_schema(by_name, "browser_scroll")
    if "wait_for_arrival" not in _properties(browser_move):
        raise ProbeFailure("browser_move_mouse must expose wait_for_arrival")
    for name, schema in [("browser_input", browser_input), ("browser_scroll", browser_scroll)]:
        if "wait_for_arrival" in _properties(schema):
            raise ProbeFailure(f"{name} must not expose wait_for_arrival")

    browser_open = _tool_schema(by_name, "browser_open")
    if not _optional_url_schema_accepts_only_http_about_or_absent(
        _properties(browser_open).get("url", {})
    ):
        raise ProbeFailure(
            "browser_open URL schema must allow only anchored HTTP/about:blank or absent sentinels"
        )

    phone_pointer_props = _properties(_tool_schema(by_name, "phone_pointer"))
    for field in (
        "session_id",
        "x",
        "y",
        "start_x",
        "start_y",
        "end_x",
        "end_y",
        "phone_snapshot_id",
    ):
        if field not in phone_pointer_props:
            raise ProbeFailure(f"phone_pointer must advertise {field}")

    observe_backend = _properties(_tool_schema(by_name, "observe")).get("backend", {})
    capture_backend = _properties(_tool_schema(by_name, "capture_screen")).get("backend", {})
    connect_backend = _properties(_tool_schema(by_name, "phone_connection")).get("backend", {})
    for name, backend in [("observe", observe_backend), ("capture_screen", capture_backend)]:
        enum = _schema_enum_values(backend)
        if not isinstance(enum, list) or "none" in enum or "scrcpy" in enum:
            raise ProbeFailure(f"{name} backend request enum must exclude none and scrcpy")
    connect_enum = _schema_enum_values(connect_backend)
    if not isinstance(connect_enum, list) or "none" in connect_enum:
        raise ProbeFailure("phone_connection backend request enum must exclude none")
    if "session_id" not in _properties(_tool_schema(by_name, "phone_connection")):
        raise ProbeFailure("phone_connection must advertise session_id")

    desktop_capture = _tool_schema(by_name, "capture_desktop")
    desktop_capture_props = _properties(desktop_capture)
    for name in ["display_id", "display_name"]:
        if not _string_schema_branch_has_min_length(desktop_capture_props.get(name), 1):
            raise ProbeFailure(f"capture_desktop {name} must reject empty strings")
    for name in ["window_id", "display_id"]:
        if name not in desktop_capture_props:
            raise ProbeFailure(f"capture_desktop must advertise {name} selector")
    if "capture_all_displays" in desktop_capture_props:
        raise ProbeFailure("capture_desktop must not advertise capture_all_displays")

    desktop_scroll_props = _properties(_tool_schema(by_name, "desktop_scroll"))
    if "pages" not in desktop_scroll_props or desktop_scroll_props["pages"].get("minimum") != 1:
        raise ProbeFailure("desktop_scroll must expose positive pages")
    if "steps" in desktop_scroll_props or "delta_y" in desktop_scroll_props:
        raise ProbeFailure("desktop_scroll must not expose legacy magnitude fields")

    launch_app = _tool_schema(by_name, "desktop_launch_app")
    launch_app_description = str(by_name["desktop_launch_app"].get("description", ""))
    if "private isolated desktop" not in launch_app_description:
        raise ProbeFailure("desktop_launch_app description must state isolated-desktop scope")
    launch_app_props = _properties(launch_app)
    if not _required_string_schema_rejects_empty(launch_app_props.get("command")):
        raise ProbeFailure("desktop_launch_app command must reject empty strings")
    args_schema = launch_app_props.get("args")
    if not isinstance(args_schema, dict) or args_schema.get("type") != "array":
        raise ProbeFailure("desktop_launch_app args must be an array")
    args_items = args_schema.get("items")
    if not _required_string_schema_rejects_empty(args_items):
        raise ProbeFailure("desktop_launch_app args items must reject empty strings")
    if launch_app.get("required") != ["command"]:
        raise ProbeFailure("desktop_launch_app must require command")

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


def _properties(schema: dict[str, Any]) -> dict[str, Any]:
    properties = schema.get("properties")
    if isinstance(properties, dict):
        return properties
    return {}


def _optional_url_schema_accepts_only_http_about_or_absent(schema: object) -> bool:
    if not isinstance(schema, dict):
        return False
    any_of = schema.get("anyOf")
    if not isinstance(any_of, list):
        return schema.get("pattern") == r"^(https?://[^\s]+|about:blank)$"
    has_url_pattern = any(
        isinstance(entry, dict)
        and entry.get("type") == "string"
        and entry.get("pattern") == r"^(https?://[^\s]+|about:blank)$"
        for entry in any_of
    )
    has_empty = any(
        isinstance(entry, dict) and entry.get("type") == "string" and entry.get("const") == ""
        for entry in any_of
    )
    has_null = any(isinstance(entry, dict) and entry.get("type") == "null" for entry in any_of)
    return has_url_pattern and has_empty and has_null


def _string_schema_branch_has_min_length(schema: object, min_length: int) -> bool:
    if not isinstance(schema, dict):
        return False
    if schema.get("type") == "string" and schema.get("minLength") == min_length:
        return True
    any_of = schema.get("anyOf")
    if isinstance(any_of, list):
        return any(_string_schema_branch_has_min_length(entry, min_length) for entry in any_of)
    return False


def _required_string_schema_rejects_empty(schema: object) -> bool:
    if not isinstance(schema, dict):
        return False
    any_of = schema.get("anyOf")
    if isinstance(any_of, list):
        return bool(any_of) and all(
            _required_string_schema_rejects_empty(entry) for entry in any_of
        )
    if schema.get("type") != "string":
        return False
    if "const" in schema:
        const = schema.get("const")
        return isinstance(const, str) and const != ""
    enum = schema.get("enum")
    if isinstance(enum, list):
        return bool(enum) and all(isinstance(value, str) and value != "" for value in enum)
    min_length = schema.get("minLength")
    return isinstance(min_length, int) and min_length >= 1


def _schema_enum_values(schema: object) -> list[Any] | None:
    if not isinstance(schema, dict):
        return None
    enum = schema.get("enum")
    if isinstance(enum, list):
        return enum
    any_of = schema.get("anyOf")
    if isinstance(any_of, list):
        for entry in any_of:
            if isinstance(entry, dict) and isinstance(entry.get("enum"), list):
                return entry["enum"]
    return None


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


def grouped_error_payload(
    result: dict[str, Any], *, tool: str, code: str, branch: str | None = None
) -> dict[str, Any]:
    if result.get("isError") is not True:
        raise ProbeFailure(f"{tool} invalid branch did not set isError: {result!r}")
    payload = result.get("structuredContent")
    if not isinstance(payload, dict):
        raise ProbeFailure(f"{tool} invalid branch returned no structuredContent: {result!r}")
    if payload.get("tool") != tool:
        raise ProbeFailure(f"{tool} invalid branch returned wrong grouped identity: {payload!r}")
    if payload.get("branch") != branch:
        raise ProbeFailure(
            f"{tool} invalid branch returned wrong branch: "
            f"expected {branch!r}, got {payload.get('branch')!r}; payload={payload!r}"
        )
    error = payload.get("error")
    if branch is not None and not isinstance(error, dict):
        error = payload.get("result")
    if not isinstance(error, dict) or error.get("code") != code:
        raise ProbeFailure(f"{tool} invalid branch returned wrong error code: {payload!r}")
    return payload


def resolve_client(installed: bool, codex_home: Path) -> Path:
    client = (
        installed_plugin_root(codex_home.expanduser().resolve()) / "bin" / "sky-cua-client"
        if installed
        else DEV_CLIENT
    )
    if not client.exists():
        raise FileNotFoundError(f"MCP client binary not found: {client}")
    return client


def make_client(*, installed: bool, phone_enabled: bool, codex_home: Path) -> McpClient:
    env = dict(os.environ)
    env.pop("SKY_CUA_MCP_TOOL_PROFILE", None)
    env.setdefault("SKY_CUA_MODEL_SUPPORTS_IMAGES", "false")
    env["SKY_CUA_ISOLATED_DESKTOP"] = "0"
    if phone_enabled:
        env.setdefault("SKY_CUA_PHONE", "1")
    return McpClient(
        [str(resolve_client(installed, codex_home)), "mcp"],
        base_env=env,
        client_name="mcp-tool-surface-probe",
    )


def probe_grouped(*, installed: bool, phone_enabled: bool, codex_home: Path) -> list[ProbeStep]:
    steps: list[ProbeStep] = []
    client = make_client(installed=installed, phone_enabled=phone_enabled, codex_home=codex_home)
    try:
        client.initialize()
        tools = client.tools_list()
        names = tool_names(tools)
        require_exact_grouped_tools(names)
        require_no_top_level_composition(tools)
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
            (14, "desktop_launch_app", {"command": "true"}),
        ]
        for request_id, tool, arguments in invalid_cases:
            invalid = client.tools_call(request_id, tool, arguments)
            expected_code = (
                "IsolatedDesktopRequired" if tool == "desktop_launch_app" else "InvalidRequest"
            )
            expected_branch = "default" if tool == "desktop_launch_app" else None
            grouped_error_payload(invalid, tool=tool, code=expected_code, branch=expected_branch)
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
        help="Probe the deployed Codex plugin cache payload instead of bin/sky-cua-client.",
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=DEFAULT_CODEX_HOME,
        help=f"Codex home for --installed resolution (default: {DEFAULT_CODEX_HOME}).",
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


def run(
    *, installed: bool, phone_enabled: bool, codex_home: Path = DEFAULT_CODEX_HOME
) -> list[ProbeStep]:
    return probe_grouped(installed=installed, phone_enabled=phone_enabled, codex_home=codex_home)


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    steps = run(
        installed=bool(args.installed),
        phone_enabled=not args.no_phone,
        codex_home=args.codex_home,
    )
    if args.json:
        print(json.dumps({"ok": True, "steps": [step.__dict__ for step in steps]}, indent=2))
    else:
        for step in steps:
            print(format_step(step))
        print(f"RESULT mcp_tool_surface_probe passed={len(steps)} failed=0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
