"""Unit tests for the grouped MCP tool-surface probe helpers."""

from __future__ import annotations

import copy
from pathlib import Path

import pytest

import probe_mcp_tool_surface as probe


def test_tool_names_extracts_only_string_names() -> None:
    names = probe.tool_names(
        [
            {"name": "status"},
            {"name": "phone_connection"},
            {"name": 3},
            {"description": "missing"},
        ]
    )

    assert names == {"status", "phone_connection"}


def test_resolve_installed_client_uses_codex_plugin_cache(tmp_path: Path) -> None:
    client = (
        tmp_path / "plugins" / "cache" / "local" / "sky-cua" / "local" / "bin" / "sky-cua-client"
    )
    client.parent.mkdir(parents=True)
    client.write_text("#!/bin/sh\n", encoding="utf-8")

    assert probe.resolve_client(installed=True, codex_home=tmp_path) == client


def test_require_exact_grouped_tools_report_contract_violations() -> None:
    probe.require_exact_grouped_tools(set(probe.GROUPED_TOOLS))
    probe.require_exact_grouped_tools(set(probe.GROUPED_TOOLS) | {probe.BROWSER_EVAL_TOOL})

    missing_phone = set(probe.GROUPED_TOOLS)
    missing_phone.remove("phone_connection")
    with pytest.raises(probe.ProbeFailure, match=r"missing=.*phone_connection"):
        probe.require_exact_grouped_tools(missing_phone)

    with pytest.raises(probe.ProbeFailure, match=r"extra=.*unexpected_tool"):
        probe.require_exact_grouped_tools(set(probe.GROUPED_TOOLS) | {"unexpected_tool"})


def test_grouped_payload_requires_identity_and_result() -> None:
    payload = probe.grouped_payload(
        {
            "structuredContent": {
                "tool": "status",
                "branch": "phone",
                "result": {"structuredContent": {"adb_available": False}},
            }
        },
        tool="status",
        branch="phone",
    )

    assert payload["tool"] == "status"

    with pytest.raises(probe.ProbeFailure, match="wrong branch"):
        probe.grouped_payload(
            {
                "structuredContent": {
                    "tool": "status",
                    "branch": "browser",
                    "result": {},
                }
            },
            tool="status",
            branch="phone",
        )


def test_grouped_error_payload_requires_structured_error_code() -> None:
    payload = probe.grouped_error_payload(
        {
            "isError": True,
            "structuredContent": {
                "tool": "status",
                "branch": None,
                "error": {"code": "InvalidRequest", "message": "bad branch"},
            },
        },
        tool="status",
        code="InvalidRequest",
    )

    assert payload["error"]["code"] == "InvalidRequest"

    delegated_payload = probe.grouped_error_payload(
        {
            "isError": True,
            "structuredContent": {
                "tool": "desktop_launch_app",
                "branch": "default",
                "result": {"code": "IsolatedDesktopRequired", "message": "disabled"},
            },
        },
        tool="desktop_launch_app",
        code="IsolatedDesktopRequired",
        branch="default",
    )

    assert delegated_payload["result"]["code"] == "IsolatedDesktopRequired"

    with pytest.raises(probe.ProbeFailure, match="wrong error code"):
        probe.grouped_error_payload(
            {
                "isError": True,
                "structuredContent": {
                    "tool": "status",
                    "branch": None,
                    "result": {"code": "InvalidRequest", "message": "wrong slot"},
                },
            },
            tool="status",
            code="InvalidRequest",
        )

    with pytest.raises(probe.ProbeFailure, match="wrong error code"):
        probe.grouped_error_payload(
            {
                "isError": True,
                "structuredContent": {
                    "tool": "status",
                    "error": {"code": "Other"},
                },
            },
            tool="status",
            code="InvalidRequest",
        )


def test_require_grouped_action_shape_rejects_vague_action_tools() -> None:
    base_tools = [
        {
            "name": "doctor",
            "annotations": {"readOnlyHint": True},
            "inputSchema": {"type": "object"},
        },
        {
            "name": "status",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "component": {"type": "string"},
                    "refresh_devices": {"type": "boolean"},
                    "session_id": {"type": "string"},
                },
            },
        },
        {
            "name": "browser_move_mouse",
            "inputSchema": {"properties": {"wait_for_arrival": {"type": "boolean"}}},
        },
        {
            "name": "browser_input",
            "inputSchema": {
                "properties": {
                    "operation": {"enum": ["click", "type_text", "press_key"]},
                    "tab_id": {"type": "string"},
                    "text": {"type": "string"},
                    "x": {"type": "number"},
                    "y": {"type": "number"},
                }
            },
        },
        {
            "name": "browser_scroll",
            "inputSchema": {
                "properties": {
                    "tab_id": {"type": "string"},
                    "delta_y": {"type": "number"},
                    "x": {"type": "number"},
                    "y": {"type": "number"},
                }
            },
        },
        {
            "name": "browser_open",
            "inputSchema": {"properties": {"url": {"pattern": r"^(https?://[^\s]+|about:blank)$"}}},
        },
        {
            "name": "desktop_pointer",
            "description": "do not call with only operation",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "operation": {"type": "string"},
                    "x": {"type": "number"},
                    "y": {"type": "number"},
                },
            },
        },
        {
            "name": "desktop_action",
            "description": "do not call with only operation",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "operation": {"type": "string"},
                    "element_index": {"type": "integer"},
                    "action_name": {"type": "string"},
                },
            },
        },
        {
            "name": "desktop_keyboard",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "operation": {"type": "string"},
                    "key": {"type": "string"},
                    "text": {"type": "string"},
                },
            },
        },
        {
            "name": "desktop_launch_app",
            "description": "Launch an application into the agent's private isolated desktop.",
            "inputSchema": {
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {
                        "type": "string",
                        "minLength": 1,
                    },
                    "args": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "minLength": 1,
                        },
                    },
                },
            },
        },
        {
            "name": "phone_pointer",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "operation": {"type": "string"},
                    "session_id": {"type": "string"},
                    "x": {"type": "number"},
                    "y": {"type": "number"},
                    "start_x": {"type": "number"},
                    "start_y": {"type": "number"},
                    "end_x": {"type": "number"},
                    "end_y": {"type": "number"},
                    "phone_snapshot_id": {"type": "string"},
                },
            },
        },
        {
            "name": "observe",
            "inputSchema": {"properties": {"backend": {"enum": ["auto", "adb", "companion"]}}},
        },
        {
            "name": "capture_screen",
            "inputSchema": {"properties": {"backend": {"enum": ["auto", "adb", "companion"]}}},
        },
        {
            "name": "phone_connection",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "operation": {"type": "string"},
                    "session_id": {"type": "string"},
                    "backend": {"enum": ["auto", "adb", "companion", "scrcpy"]},
                },
            },
        },
        {
            "name": "capture_desktop",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "window_id": {"type": "string"},
                    "display_id": {
                        "anyOf": [
                            {"type": "string", "minLength": 1},
                            {"type": "string", "const": ""},
                            {"type": "null"},
                        ]
                    },
                    "display_name": {
                        "anyOf": [
                            {"type": "string", "minLength": 1},
                            {"type": "string", "const": ""},
                            {"type": "null"},
                        ]
                    },
                },
            },
        },
        {
            "name": "desktop_scroll",
            "inputSchema": {"properties": {"pages": {"minimum": 1}}},
        },
        {
            "name": "phone_app_install",
            "inputSchema": {
                "required": ["session_id", "apk_paths"],
                "properties": {"apk_paths": {"minItems": 1}},
            },
        },
        {
            "name": "phone_app_action",
            "inputSchema": {"properties": {"package_name": {"type": "string"}}},
        },
        {
            "name": "phone_setup",
            "annotations": {"destructiveHint": True, "idempotentHint": True},
            "inputSchema": {"type": "object"},
        },
        {
            "name": "phone_app_force_stop",
            "annotations": {"destructiveHint": True, "idempotentHint": True},
            "inputSchema": {"type": "object"},
        },
    ]

    probe.require_grouped_action_shape(base_tools)

    weak_launch_command = copy.deepcopy(base_tools)
    launch_index = next(
        index
        for index, tool in enumerate(weak_launch_command)
        if tool["name"] == "desktop_launch_app"
    )
    weak_launch_command[launch_index]["inputSchema"]["properties"]["command"] = {
        "anyOf": [
            {"type": "string", "minLength": 1},
            {"type": "string", "const": ""},
            {"type": "null"},
        ]
    }
    with pytest.raises(probe.ProbeFailure, match="desktop_launch_app command"):
        probe.require_grouped_action_shape(weak_launch_command)

    weak_launch_args = copy.deepcopy(base_tools)
    weak_launch_args[launch_index]["inputSchema"]["properties"]["args"]["items"] = {
        "anyOf": [
            {"type": "string", "minLength": 1},
            {"type": "string", "const": ""},
            {"type": "null"},
        ]
    }
    with pytest.raises(probe.ProbeFailure, match="desktop_launch_app args items"):
        probe.require_grouped_action_shape(weak_launch_args)

    vague_pointer = [dict(tool) for tool in base_tools]
    pointer_index = next(
        index for index, tool in enumerate(vague_pointer) if tool["name"] == "desktop_pointer"
    )
    vague_pointer[pointer_index] = {
        "name": "desktop_pointer",
        "description": "do not call with only operation",
        "inputSchema": {"type": "object", "properties": {"operation": {"type": "string"}}},
    }
    with pytest.raises(probe.ProbeFailure, match="desktop_pointer"):
        probe.require_grouped_action_shape(vague_pointer)


def test_require_no_top_level_composition_rejects_advertised_composition() -> None:
    probe.require_no_top_level_composition(
        [
            {"name": "status", "inputSchema": {"type": "object", "properties": {}}},
            {"name": "doctor", "inputSchema": {"type": "object"}},
        ]
    )

    with pytest.raises(probe.ProbeFailure, match=r"desktop_pointer.*allOf"):
        probe.require_no_top_level_composition(
            [
                {
                    "name": "desktop_pointer",
                    "inputSchema": {
                        "type": "object",
                        "allOf": [{"required": ["x", "y"]}],
                    },
                }
            ]
        )
