"""Unit tests for the canonical MCP tool-surface probe helpers."""

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


def test_require_exact_canonical_tools_report_contract_violations() -> None:
    probe.require_exact_canonical_tools(set(probe.CANONICAL_TOOLS))
    probe.require_exact_canonical_tools(set(probe.CANONICAL_TOOLS) | {probe.BROWSER_EVAL_TOOL})

    missing_phone = set(probe.CANONICAL_TOOLS)
    missing_phone.remove("phone_connection")
    with pytest.raises(probe.ProbeFailure, match=r"missing=.*phone_connection"):
        probe.require_exact_canonical_tools(missing_phone)

    with pytest.raises(probe.ProbeFailure, match=r"extra=.*unexpected_tool"):
        probe.require_exact_canonical_tools(set(probe.CANONICAL_TOOLS) | {"unexpected_tool"})


def test_canonical_payload_requires_identity_and_result() -> None:
    payload = probe.canonical_payload(
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
        probe.canonical_payload(
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


def test_canonical_error_payload_requires_structured_error_code() -> None:
    payload = probe.canonical_error_payload(
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
        probe.canonical_error_payload(
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


def test_require_canonical_action_shape_rejects_vague_action_tools() -> None:
    base_tools = [
        {
            "name": "doctor",
            "annotations": {"readOnlyHint": True},
            "inputSchema": {"type": "object"},
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
    ]

    probe.require_canonical_action_shape(base_tools)

    vague_pointer = [dict(tool) for tool in base_tools]
    vague_pointer[1] = {
        "name": "desktop_pointer",
        "description": "Click things",
        "inputSchema": {"type": "object"},
    }
    with pytest.raises(probe.ProbeFailure, match="desktop_pointer"):
        probe.require_canonical_action_shape(vague_pointer)
