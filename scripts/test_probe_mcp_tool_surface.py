"""Unit tests for the compact MCP tool-surface probe helpers."""

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


def test_require_and_forbid_tools_report_contract_violations() -> None:
    probe.require_tools({"status", "phone_connection"}, frozenset({"status"}), profile="x")
    probe.forbid_tools({"status"}, frozenset({"phone_status"}), profile="x")

    with pytest.raises(probe.ProbeFailure, match="missing required tools"):
        probe.require_tools({"status"}, frozenset({"status", "phone_connection"}), profile="x")

    with pytest.raises(probe.ProbeFailure, match="advertised inactive tools"):
        probe.forbid_tools({"status", "phone_status"}, frozenset({"phone_status"}), profile="x")


def test_compact_payload_requires_identity_and_result() -> None:
    payload = probe.compact_payload(
        {
            "structuredContent": {
                "profile": "compact",
                "tool": "status",
                "branch": "phone",
                "legacy_tool": "phone_status",
                "result": {"structuredContent": {"adb_available": False}},
            }
        },
        tool="status",
        branch="phone",
    )

    assert payload["legacy_tool"] == "phone_status"

    with pytest.raises(probe.ProbeFailure, match="wrong branch"):
        probe.compact_payload(
            {
                "structuredContent": {
                    "profile": "compact",
                    "tool": "status",
                    "branch": "browser",
                    "legacy_tool": "browser_status",
                    "result": {},
                }
            },
            tool="status",
            branch="phone",
        )


def test_compact_error_payload_requires_tool_error_code() -> None:
    payload = probe.compact_error_payload(
        {
            "isError": True,
            "structuredContent": {
                "profile": "compact",
                "tool": "status",
                "branch": None,
                "legacy_tool": None,
                "error": {"code": "InvalidRequest", "message": "bad branch"},
            },
        },
        tool="status",
        code="InvalidRequest",
    )

    assert payload["error"]["code"] == "InvalidRequest"

    with pytest.raises(probe.ProbeFailure, match="wrong error code"):
        probe.compact_error_payload(
            {
                "isError": True,
                "structuredContent": {
                    "profile": "compact",
                    "tool": "status",
                    "error": {"code": "Other"},
                },
            },
            tool="status",
            code="InvalidRequest",
        )


def test_tool_error_code_reads_json_rpc_data_code() -> None:
    assert (
        probe.tool_error_code(
            {"error": {"code": -32602, "data": {"code": "ToolNotInActiveProfile"}}}
        )
        == "ToolNotInActiveProfile"
    )
    assert probe.tool_error_code({"result": {}}) is None
