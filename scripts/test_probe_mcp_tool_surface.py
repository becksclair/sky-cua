"""Unit tests for the grouped MCP tool-surface probe helpers."""

from __future__ import annotations

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
                "allOf": [
                    {
                        "if": {"properties": {"operation": {"const": "click"}}},
                        "then": {"anyOf": [{"required": ["x", "y"]}]},
                    }
                ]
            },
        },
        {
            "name": "desktop_action",
            "description": "do not call with only operation",
            "inputSchema": {
                "allOf": [
                    {"anyOf": [{"required": ["element_index"]}]},
                ]
            },
        },
        {
            "name": "desktop_keyboard",
            "inputSchema": {
                "allOf": [
                    {
                        "if": {"properties": {"operation": {"const": "press_key"}}},
                        "then": {"required": ["key"]},
                    },
                    {
                        "if": {"properties": {"operation": {"const": "type_text"}}},
                        "then": {"required": ["text"]},
                    },
                ]
            },
        },
        {
            "name": "phone_pointer",
            "inputSchema": {
                "allOf": [
                    {
                        "if": {"properties": {"operation": {"const": "tap"}}},
                        "then": {
                            "required": ["session_id", "x", "y"],
                            "anyOf": [
                                {"required": ["phone_snapshot_id"]},
                                {
                                    "required": ["use_device_coordinates"],
                                    "properties": {"use_device_coordinates": {"const": True}},
                                },
                            ],
                        },
                    },
                    {
                        "if": {"properties": {"operation": {"const": "swipe"}}},
                        "then": {
                            "required": [
                                "session_id",
                                "start_x",
                                "start_y",
                                "end_x",
                                "end_y",
                            ],
                            "anyOf": [
                                {"required": ["phone_snapshot_id"]},
                                {
                                    "required": ["use_device_coordinates"],
                                    "properties": {"use_device_coordinates": {"const": True}},
                                },
                            ],
                        },
                    },
                ]
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
                "properties": {"backend": {"enum": ["auto", "adb", "companion", "scrcpy"]}},
                "allOf": [
                    {
                        "if": {"properties": {"operation": {"const": "disconnect"}}},
                        "then": {"required": ["session_id"]},
                    }
                ],
            },
        },
        {
            "name": "capture_desktop",
            "inputSchema": {
                "properties": {
                    "display_id": {"minLength": 1},
                    "display_name": {"minLength": 1},
                },
                "allOf": [
                    {"not": {"anyOf": [{"required": ["window_id", "display_id"]}]}},
                    {"not": {"anyOf": [{"required": ["capture_all_displays", "display_id"]}]}},
                ],
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

    vague_pointer = [dict(tool) for tool in base_tools]
    pointer_index = next(
        index for index, tool in enumerate(vague_pointer) if tool["name"] == "desktop_pointer"
    )
    vague_pointer[pointer_index] = {
        "name": "desktop_pointer",
        "description": "Click things",
        "inputSchema": {"type": "object"},
    }
    with pytest.raises(probe.ProbeFailure, match="desktop_pointer"):
        probe.require_grouped_action_shape(vague_pointer)
