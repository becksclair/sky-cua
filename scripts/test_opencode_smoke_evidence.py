from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Any

import pytest

import _agent_mcp_smoke
import live_agent_mcp_smoke


def _execute_event() -> dict[str, Any]:
    return {
        "type": "tool_use",
        "part": {
            "tool": "execute",
            "state": {
                "status": "completed",
                "input": {"code": "return await tools.sky_cua.desktop_action({secret:'input'});"},
                "output": json.dumps(
                    {
                        "tool": "desktop_action",
                        "branch": "activate",
                        "result": {"success": True, "code": "Ok", "message": "private output"},
                    }
                ),
                "metadata": {
                    "metadata": {
                        "toolCalls": [
                            {
                                "tool": "sky_cua.desktop_action",
                                "status": "completed",
                                "input": {"private": "arguments"},
                            }
                        ],
                    },
                },
            },
        },
    }


@pytest.mark.parametrize("sanitize", [False, True])
def test_opencode_code_mode_accepts_returned_action_and_final_text(
    tmp_path: Path,
    sanitize: bool,
) -> None:
    raw = tmp_path / "agent.jsonl"
    raw.write_text(
        json.dumps(_execute_event())
        + "\n"
        + json.dumps(
            {
                "type": "text",
                "part": {"type": "text", "text": '{"dismissed":true,"private":"dialog text"}'},
            }
        )
        + "\n"
    )
    stdout = raw
    if sanitize:
        stdout = tmp_path / "redacted.jsonl"
        _agent_mcp_smoke.redact_pi_json_stdout(raw, stdout)
        text = stdout.read_text()
        for private in ("private", "arguments", "dialog text", "return await"):
            assert private not in text
    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(stdout) is True
    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(stdout) is True
    assert live_agent_mcp_smoke._parse_dismissed_from_stdout(stdout) is True


@pytest.mark.parametrize(
    "case",
    [
        "source_only",
        "search",
        "pending",
        "failed",
        "missing_output",
        "wrong_returned_tool",
        "action_failure",
        "result_error",
        "outer_failure",
    ],
)
def test_opencode_code_mode_rejects_unproven_actions(tmp_path: Path, case: str) -> None:
    event = _execute_event()
    state = event["part"]["state"]
    call = state["metadata"]["metadata"]["toolCalls"][0]
    if case == "source_only":
        state["metadata"] = {}
    elif case == "search":
        call["tool"] = "search"
    elif case == "pending":
        call["status"] = "running"
    elif case == "failed":
        call["status"] = "failed"
    elif case == "missing_output":
        state.pop("output")
    elif case == "wrong_returned_tool":
        state["output"] = json.dumps({"tool": "observe", "result": {"success": True}})
    elif case == "action_failure":
        state["output"] = json.dumps({"tool": "desktop_action", "result": {"success": False}})
    elif case == "result_error":
        state["output"] = json.dumps({"tool": "desktop_action", "result": {"isError": True}})
    elif case == "outer_failure":
        state["status"] = "error"
    assert _agent_mcp_smoke.opencode_code_mode_tool_results(event) == []
    raw = tmp_path / "raw.jsonl"
    raw.write_text(json.dumps(event) + "\n")
    redacted = tmp_path / "redacted.jsonl"
    _agent_mcp_smoke.redact_pi_json_stdout(raw, redacted)
    for stdout in (raw, redacted):
        assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(stdout) is False


def test_opencode_code_mode_handles_returned_observation_without_action(tmp_path: Path) -> None:
    event = _execute_event()
    state = event["part"]["state"]
    state["metadata"]["metadata"]["toolCalls"][0]["tool"] = "sky_cua.observe"
    state["output"] = json.dumps({"tool": "observe", "result": {"appshot_id": "private"}})
    stdout = tmp_path / "raw.jsonl"
    stdout.write_text(json.dumps(event) + "\n")
    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(stdout) is True
    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(stdout) is False


def test_opencode_code_mode_matches_all_settled_returned_envelopes() -> None:
    event = _execute_event()
    original = copy.deepcopy(event)
    output = json.loads(event["part"]["state"]["output"])
    event["part"]["state"]["output"] = json.dumps([{"status": "fulfilled", "value": output}])
    assert _agent_mcp_smoke.opencode_code_mode_tool_results(event) == (
        _agent_mcp_smoke.opencode_code_mode_tool_results(original)
    )


def test_opencode_code_mode_matches_envelopes_returned_under_named_keys() -> None:
    event = _execute_event()
    original = copy.deepcopy(event)
    output = json.loads(event["part"]["state"]["output"])
    event["part"]["state"]["output"] = json.dumps({"activate": output})
    assert _agent_mcp_smoke.opencode_code_mode_tool_results(event) == (
        _agent_mcp_smoke.opencode_code_mode_tool_results(original)
    )


def test_opencode_code_mode_ignores_rejected_settled_records() -> None:
    event = _execute_event()
    output = json.loads(event["part"]["state"]["output"])
    event["part"]["state"]["output"] = json.dumps([{"status": "rejected", "reason": output}])
    assert _agent_mcp_smoke.opencode_code_mode_tool_results(event) == []
