"""Tests for shared live-smoke configuration and desktop smoke helpers."""

from __future__ import annotations

import json
from argparse import Namespace
from pathlib import Path
from typing import Any, cast

import pytest

import _agent_mcp_smoke
import live_agent_mcp_smoke
import live_agentic_loop_smoke
import live_desktop_smoke
import live_openclaw_mcp_smoke
import live_portal_downgrade_smoke
import live_wayland_pointer_smoke
from _codex_exec import DEFAULT_MODEL, DEFAULT_REASONING_EFFORT
from _pointer_geometry import adjusted_origin_for_visible_monitor
from _smoke_config import LIVE_SMOKE_MODEL, LIVE_SMOKE_REASONING_EFFORT


def test_live_smoke_model_config_is_centralized() -> None:
    assert LIVE_SMOKE_MODEL == "gpt-5.5"
    assert LIVE_SMOKE_REASONING_EFFORT == "low"
    assert DEFAULT_MODEL == LIVE_SMOKE_MODEL
    assert DEFAULT_REASONING_EFFORT == LIVE_SMOKE_REASONING_EFFORT


def test_agentic_loop_default_uses_tool_evidence_enforced_agent() -> None:
    assert live_agentic_loop_smoke.DEFAULT_AGENT in {"opencode", "pi"}
    assert set(live_agentic_loop_smoke.ACCEPTANCE_AGENTS) == {"opencode", "pi"}


def test_agent_smoke_fixtures_are_dialog_dismissal_flows() -> None:
    assert set(live_agent_mcp_smoke.FIXTURES) == {"kdialog", "zenity"}


def test_pointer_fixture_adjusts_origin_when_fullscreen_allocation_is_clipped() -> None:
    assert adjusted_origin_for_visible_monitor(
        origin_x=0,
        origin_y=0,
        allocation_width=1280,
        allocation_height=955,
        monitor_width=1280,
        monitor_height=800,
    ) == (0, -78)


def test_pointer_fixture_keeps_origin_when_allocation_fits_monitor() -> None:
    assert adjusted_origin_for_visible_monitor(
        origin_x=12,
        origin_y=34,
        allocation_width=1280,
        allocation_height=800,
        monitor_width=1280,
        monitor_height=800,
    ) == (12, 34)


def test_x11_click_target_falls_back_to_native_root_window() -> None:
    snapshot = {
        "snapshot_id": "snapshot-root-only",
        "elements": [
            {
                "bounds": {"height": 52.0, "width": 128.0, "x": 895.0, "y": 526.0},
                "element_index": 0,
                "parent_index": None,
                "role": "window",
                "state_flags": ["native_window_fallback", "physical_target"],
            }
        ],
    }

    target = live_desktop_smoke.pick_x11_click_target(snapshot)

    assert target["element_index"] == 0
    assert live_desktop_smoke.x11_click_arguments(snapshot, target) == {
        "x": 959.0,
        "y": 565.52,
    }


def test_x11_click_target_prefers_lowest_leaf_region_when_available() -> None:
    snapshot = {
        "snapshot_id": "snapshot-with-leaves",
        "elements": [
            {
                "bounds": {"height": 80.0, "width": 160.0, "x": 0.0, "y": 0.0},
                "element_index": 0,
                "parent_index": None,
                "role": "window",
                "state_flags": ["native_window_fallback"],
            },
            {
                "bounds": {"height": 20.0, "width": 80.0, "x": 10.0, "y": 10.0},
                "element_index": 1,
                "parent_index": 0,
                "role": "x11_leaf_region",
            },
            {
                "bounds": {"height": 16.0, "width": 64.0, "x": 20.0, "y": 44.0},
                "element_index": 2,
                "parent_index": 0,
                "role": "x11_action_region",
            },
        ],
    }

    target = live_desktop_smoke.pick_x11_click_target(snapshot)

    assert target["element_index"] == 2
    assert live_desktop_smoke.x11_click_arguments(snapshot, target) == {
        "snapshot_id": "snapshot-with-leaves",
        "element_index": 2,
    }
    live_desktop_smoke.require_x11_action_region_hints(snapshot, "X11")


def test_x11_action_region_hints_reject_root_only_snapshot() -> None:
    snapshot = {
        "snapshot_id": "snapshot-root-only",
        "elements": [
            {
                "bounds": {"height": 52.0, "width": 128.0, "x": 895.0, "y": 526.0},
                "element_index": 0,
                "parent_index": None,
                "role": "window",
                "state_flags": ["native_window_fallback", "physical_target"],
            }
        ],
    }

    with pytest.raises(RuntimeError, match="did not recover any child X11 regions"):
        live_desktop_smoke.require_x11_action_region_hints(snapshot, "X11")


def test_portal_downgrade_accepts_restored_session_diagnostic() -> None:
    diagnostics: list[dict[str, object]] = [
        {"code": "PipeWireStreamFailed"},
        {"code": "CaptureBackendDowngraded"},
        {"code": "PortalSessionRestored"},
    ]

    assert live_portal_downgrade_smoke.has_portal_session_diagnostic(diagnostics)
    assert live_portal_downgrade_smoke.diagnostic_codes(diagnostics) >= {
        "PipeWireStreamFailed",
        "CaptureBackendDowngraded",
    }


def test_portal_downgrade_summary_accepts_restored_session_text() -> None:
    summary = "Reused a persisted RemoteDesktop approval token for the combined portal session."

    assert live_portal_downgrade_smoke.summary_mentions_portal_session(summary)


def test_wayland_pointer_smoke_requires_gnome_eis_diagnostics() -> None:
    success = {
        "structuredContent": {
            "diagnostics": [{"code": "PortalEisInputUsed"}],
        },
    }
    live_wayland_pointer_smoke.require_gnome_eis_input_used(success, "click", is_gnome=True)
    live_wayland_pointer_smoke.require_gnome_eis_input_used(
        {"structuredContent": {"diagnostics": []}}, "click", is_gnome=False
    )

    fallback = {
        "structuredContent": {
            "diagnostics": [{"code": "PortalEisInputFallback"}],
        },
    }
    import os

    # Without SKY_CUA_REQUIRE_EIS, fallback is a warning, not a hard failure
    live_wayland_pointer_smoke.require_gnome_eis_input_used(fallback, "click", is_gnome=True)

    # With SKY_CUA_REQUIRE_EIS=1, fallback is a hard failure
    os.environ["SKY_CUA_REQUIRE_EIS"] = "1"
    with pytest.raises(RuntimeError, match="PortalEisInputFallback"):
        live_wayland_pointer_smoke.require_gnome_eis_input_used(fallback, "click", is_gnome=True)
    del os.environ["SKY_CUA_REQUIRE_EIS"]

    # Without SKY_CUA_REQUIRE_EIS, missing EIS is also a warning
    live_wayland_pointer_smoke.require_gnome_eis_input_used(
        {"structuredContent": {"diagnostics": []}}, "click", is_gnome=True
    )

    # With SKY_CUA_REQUIRE_EIS=1, missing EIS is a hard failure
    os.environ["SKY_CUA_REQUIRE_EIS"] = "1"
    with pytest.raises(RuntimeError, match="did not use GNOME RemoteDesktop EIS input"):
        live_wayland_pointer_smoke.require_gnome_eis_input_used(
            {"structuredContent": {"diagnostics": []}}, "click", is_gnome=True
        )
    del os.environ["SKY_CUA_REQUIRE_EIS"]


def test_agent_smoke_accepts_opencode_tool_use_event_shape() -> None:
    event = {
        "type": "tool_use",
        "part": {
            "type": "tool",
            "tool": "sky_cua_click",
            "state": {
                "status": "completed",
                "output": "Invoked the element semantically through AT-SPI.",
            },
        },
    }

    assert live_agent_mcp_smoke._tool_evidence_from_stdout_line(json.dumps(event)) is True


def test_agent_smoke_accepts_action_tool_evidence(tmp_path: Path) -> None:
    stdout = tmp_path / "agent.stdout.log"
    stdout.write_text(
        json.dumps(
            {
                "type": "tool_use",
                "part": {
                    "type": "tool",
                    "tool": "sky_cua_click",
                    "state": {"status": "completed", "output": "clicked"},
                },
            }
        )
        + "\n",
        encoding="utf-8",
    )

    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(stdout) is True


def test_agent_smoke_accepts_compact_action_tool_evidence(tmp_path: Path) -> None:
    stdout = tmp_path / "agent.stdout.log"
    stdout.write_text(
        json.dumps(
            {
                "type": "tool_use",
                "part": {
                    "type": "tool",
                    "tool": "sky_cua_desktop_pointer",
                    "state": {"status": "completed", "output": "clicked"},
                },
            }
        )
        + "\n",
        encoding="utf-8",
    )

    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(stdout) is True


def test_agent_smoke_rejects_read_only_tool_as_action_evidence(tmp_path: Path) -> None:
    stdout = tmp_path / "agent.stdout.log"
    stdout.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "type": "tool_execution_start",
                        "tool": "sky_cua_get_app_state",
                        "toolCallId": "tool-1",
                    }
                ),
                json.dumps(
                    {
                        "type": "tool_execution_end",
                        "result": {"redacted": True},
                        "toolCallId": "tool-1",
                    }
                ),
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(stdout) is True
    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(stdout) is False


def test_agent_smoke_rejects_server_only_tool_result(tmp_path: Path) -> None:
    stdout = tmp_path / "agent.stdout.log"
    stdout.write_text(
        json.dumps(
            {
                "type": "tool_execution_end",
                "server": "sky_cua",
                "result": {"redacted": True},
            }
        )
        + "\n",
        encoding="utf-8",
    )

    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(stdout) is False


def test_agent_smoke_redacted_opencode_event_preserves_tool_evidence(tmp_path: Path) -> None:
    raw = tmp_path / "opencode.raw.jsonl"
    redacted = tmp_path / "opencode.redacted.jsonl"
    raw.write_text(
        json.dumps(
            {
                "type": "tool_use",
                "part": {
                    "type": "tool",
                    "tool": "sky_cua_click",
                    "state": {
                        "status": "completed",
                        "output": "secret desktop text",
                    },
                },
            }
        )
        + "\n",
        encoding="utf-8",
    )

    _agent_mcp_smoke.redact_pi_json_stdout(raw, redacted)

    redacted_text = redacted.read_text(encoding="utf-8")
    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(redacted) is True
    assert "secret desktop text" not in redacted_text
    assert "sky_cua_click" in redacted_text


def test_opencode_neutral_cwd_gets_installed_project_config(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    home = tmp_path / "home"
    generated = home / ".local" / "share" / "sky-cua" / "opencode.json"
    generated.parent.mkdir(parents=True)
    generated.write_text('{"mcp":{"sky_cua":{"command":["/vm/client","mcp"]}}}\n', encoding="utf-8")
    neutral = tmp_path / "neutral"
    neutral.mkdir()
    monkeypatch.setenv("HOME", str(home))

    _agent_mcp_smoke.install_opencode_project_config(neutral)

    assert (neutral / "opencode.json").read_text(encoding="utf-8") == generated.read_text(
        encoding="utf-8"
    )


def test_pi_smoke_default_model_uses_openai_codex_auth() -> None:
    assert _agent_mcp_smoke.DEFAULT_PI_SMOKE_MODEL == "openai-codex/gpt-5.5"
    assert "OPENAI_API_KEY" not in _agent_mcp_smoke.model_auth_environment_keys(
        "pi", _agent_mcp_smoke.DEFAULT_PI_SMOKE_MODEL
    )


def test_pi_smoke_cwd_has_git_head() -> None:
    cwd = _agent_mcp_smoke.prepare_pi_smoke_cwd()
    try:
        assert (
            _agent_mcp_smoke.subprocess.run(
                ["git", "rev-parse", "--verify", "HEAD"],
                cwd=cwd,
                capture_output=True,
                text=True,
                check=True,
            ).stdout.strip()
            != ""
        )
    finally:
        _agent_mcp_smoke.shutil.rmtree(cwd, ignore_errors=True)


def test_agent_smoke_accepts_payload_after_non_payload_state() -> None:
    event = {
        "type": "tool_use",
        "part": {
            "type": "tool",
            "tool": "sky_cua_click",
            "state": {"status": "completed"},
            "toolResult": {"content": [{"type": "text", "text": "clicked"}]},
        },
    }

    assert live_agent_mcp_smoke._tool_evidence_from_stdout_line(json.dumps(event)) is True


def test_agent_smoke_rejects_status_only_completed_state() -> None:
    event = {
        "type": "tool_use",
        "tool": "sky_cua_click",
        "state": "completed",
    }

    assert live_agent_mcp_smoke._tool_evidence_from_stdout_line(json.dumps(event)) is False


def test_agent_smoke_rejects_opencode_failed_tool_use_event_shape() -> None:
    event = {
        "type": "tool_use",
        "part": {
            "type": "tool",
            "tool": "sky_cua_click",
            "state": {
                "status": "failed",
                "error": "boom",
            },
        },
    }

    assert live_agent_mcp_smoke._tool_evidence_from_stdout_line(json.dumps(event)) is False


def test_agent_smoke_redacted_top_level_error_rejects_tool_evidence(tmp_path: Path) -> None:
    raw = tmp_path / "opencode.raw.jsonl"
    redacted = tmp_path / "opencode.redacted.jsonl"
    raw.write_text(
        json.dumps(
            {
                "type": "tool_use",
                "tool": "sky_cua_click",
                "error": "boom",
                "result": {"content": [{"type": "text", "text": "clicked"}]},
            }
        )
        + "\n",
        encoding="utf-8",
    )

    _agent_mcp_smoke.redact_pi_json_stdout(raw, redacted)

    redacted_text = redacted.read_text(encoding="utf-8")
    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(redacted) is False
    assert "boom" not in redacted_text
    assert "clicked" not in redacted_text
    assert "result_declares_failure" in redacted_text


@pytest.mark.parametrize(
    "state",
    [
        {"status": "failed", "output": "boom"},
        {"status": "failed", "content": [{"type": "text", "text": "boom"}]},
        {"isError": True, "output": "boom"},
    ],
)
def test_agent_smoke_rejects_opencode_failed_state_with_payload(
    state: dict[str, object],
) -> None:
    event = {
        "type": "tool_use",
        "part": {
            "type": "tool",
            "tool": "sky_cua_click",
            "state": state,
        },
    }

    assert live_agent_mcp_smoke._tool_evidence_from_stdout_line(json.dumps(event)) is False


def test_agent_smoke_accepts_pi_split_tool_start_and_end_events(tmp_path: Path) -> None:
    stdout = tmp_path / "pi.stdout.log"
    stdout.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "server": "sky_cua",
                        "tool": "sky_cua_get_app_state",
                        "toolCallId": "tool-1",
                        "toolName": "mcp",
                        "type": "tool_execution_start",
                    }
                ),
                json.dumps(
                    {
                        "isError": False,
                        "result": {"redacted": True},
                        "toolCallId": "tool-1",
                        "toolName": "mcp",
                        "type": "tool_execution_end",
                    }
                ),
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(stdout) is True


def test_agent_smoke_rejects_mismatched_split_tool_completion(tmp_path: Path) -> None:
    stdout = tmp_path / "pi.stdout.log"
    stdout.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "tool": "sky_cua_click",
                        "toolCallId": "sky",
                        "type": "tool_execution_start",
                    }
                ),
                json.dumps(
                    {
                        "result": {"redacted": True},
                        "toolCallId": "other",
                        "type": "tool_execution_end",
                    }
                ),
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(stdout) is False


def test_agent_smoke_rejects_pi_failed_split_tool_before_unrelated_success(
    tmp_path: Path,
) -> None:
    stdout = tmp_path / "pi.stdout.log"
    stdout.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "server": "sky_cua",
                        "tool": "sky_cua_get_app_state",
                        "toolCallId": "tool-1",
                        "toolName": "mcp",
                        "type": "tool_execution_start",
                    }
                ),
                json.dumps(
                    {
                        "isError": True,
                        "result": {"redacted": True, "result_declares_failure": True},
                        "toolCallId": "tool-1",
                        "toolName": "mcp",
                        "type": "tool_execution_end",
                    }
                ),
                json.dumps(
                    {
                        "isError": False,
                        "result": {"redacted": True},
                        "toolName": "mcp",
                        "type": "tool_execution_end",
                    }
                ),
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(stdout) is False


def test_agent_smoke_fails_without_action_tool_evidence(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    class ClosedFixtureProcess:
        def poll(self) -> int:
            return 0

    def fake_popen(_argv: list[str]) -> ClosedFixtureProcess:
        return ClosedFixtureProcess()

    def fake_run_agent(
        agent: str,
        _prompt: str,
        artifact_dir: Path,
        **_kwargs: object,
    ) -> object:
        stdout = artifact_dir / f"{agent}.stdout.log"
        stdout.write_text(
            json.dumps(
                {
                    "type": "tool_use",
                    "tool": "sky_cua_get_app_state",
                    "result": {"redacted": True},
                }
            )
            + "\n",
            encoding="utf-8",
        )
        return live_agent_mcp_smoke.subprocess.CompletedProcess([agent], returncode=0)

    monkeypatch.setattr(live_agent_mcp_smoke, "make_artifact_dir", lambda *_args: tmp_path)
    monkeypatch.setattr(live_agent_mcp_smoke.subprocess, "Popen", fake_popen)
    monkeypatch.setattr(live_agent_mcp_smoke, "run_agent", fake_run_agent)

    assert live_agent_mcp_smoke.run_fixture_smoke(agent="pi", fixture_name="zenity") == 1

    result = json.loads((tmp_path / "result.json").read_text(encoding="utf-8"))
    assert result["tool_evidence"] is True
    assert result["action_tool_evidence"] is False


def test_openclaw_smoke_show_config_accepts_installed_approve_mode(tmp_path: Path) -> None:
    client = tmp_path / "sky-cua-client"
    client.write_text("", encoding="utf-8")
    config = {
        "enabled": True,
        "command": str(client),
        "args": ["mcp"],
        "env": {"SKY_CUA_REPO_ROOT": str(tmp_path)},
        "codex": {"defaultToolsApprovalMode": "approve"},
    }

    assert live_openclaw_mcp_smoke.check_show_config(config) == []


def test_openclaw_smoke_show_config_rejects_approve_mode_and_disabled(tmp_path: Path) -> None:
    client = tmp_path / "sky-cua-client"
    client.write_text("", encoding="utf-8")
    config = {
        "enabled": False,
        "command": str(client),
        "env": {"SKY_CUA_REPO_ROOT": str(tmp_path)},
        "codex": {"defaultToolsApprovalMode": "auto"},
    }

    failures = live_openclaw_mcp_smoke.check_show_config(config)

    assert any("enabled: false" in failure for failure in failures)
    assert any("defaultToolsApprovalMode is 'auto'" in failure for failure in failures)


def test_openclaw_smoke_show_config_reports_missing_binary_and_env(tmp_path: Path) -> None:
    config = {
        "command": str(tmp_path / "missing-client"),
        "codex": {"defaultToolsApprovalMode": "approve"},
    }

    failures = live_openclaw_mcp_smoke.check_show_config(config)

    assert any("does not exist" in failure for failure in failures)
    assert any("SKY_CUA_REPO_ROOT" in failure for failure in failures)


def test_openclaw_smoke_probe_requires_browser_and_desktop_tools() -> None:
    all_tools = list(live_openclaw_mcp_smoke.REQUIRED_TOOLS)
    probe = {
        "servers": {"sky_cua": {"tools": len(all_tools)}},
        "tools": all_tools,
        "diagnostics": [],
    }
    assert live_openclaw_mcp_smoke.check_probe_result(probe) == []

    missing_browser = {
        "servers": {"sky_cua": {"tools": 1}},
        "tools": ["sky_cua__doctor"],
        "diagnostics": [],
    }
    failures = live_openclaw_mcp_smoke.check_probe_result(missing_browser)
    assert any("sky_cua__browser_status" in failure for failure in failures)

    disconnected = {"servers": {}, "tools": [], "diagnostics": []}
    failures = live_openclaw_mcp_smoke.check_probe_result(disconnected)
    assert any("did not connect" in failure for failure in failures)


def test_openclaw_smoke_extracts_agent_report_from_json_output() -> None:
    report = {"tool_called": True, "tools_visible": True, "error": None}
    direct = json.dumps({"reply": {"sky_cua_smoke": report}})
    assert live_openclaw_mcp_smoke.extract_smoke_report(direct) == report

    embedded_reply = json.dumps(
        {"payloads": [{"text": "Here you go: " + json.dumps({"sky_cua_smoke": report})}]}
    )
    assert live_openclaw_mcp_smoke.extract_smoke_report(embedded_reply) == report

    assert live_openclaw_mcp_smoke.extract_smoke_report("no structured output") is None


def test_openclaw_smoke_detects_browser_status_tool_event() -> None:
    pending_event = json.dumps(
        {
            "type": "session_update",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-1",
                "title": "sky_cua__browser_status",
                "status": "pending",
            },
        }
    )
    completed_event = json.dumps(
        {
            "type": "session_update",
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "tool-1",
                "status": "completed",
            },
        }
    )
    result_event = json.dumps(
        {
            "type": "session_update",
            "update": {
                "sessionUpdate": "tool_result",
                "toolCallId": "tool-1",
                "content": "ok",
            },
        }
    )
    completed_batch_with_pending_call = json.dumps(
        {
            "type": "tool_batch",
            "status": "completed",
            "events": [
                {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tool-2",
                    "title": "sky_cua__browser_status",
                    "status": "pending",
                }
            ],
        }
    )

    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        "\n".join([pending_event, completed_event])
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        "\n".join([pending_event, result_event])
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        json.dumps(
            {
                "message": {
                    "role": "toolResult",
                    "toolName": "sky_cua__browser_status",
                    "toolCallId": "call-1",
                    "isError": False,
                    "content": [{"type": "toolResult", "text": "ok"}],
                }
            }
        )
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        json.dumps(
            {
                "type": "toolResult",
                "toolName": "sky_cua__browser_status",
                "toolCallId": "call-1",
                "isError": False,
                "content": "ok",
            }
        )
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        json.dumps(
            {
                "type": "tool.result",
                "data": {
                    "name": "sky_cua__browser_status",
                    "toolCallId": "call-1",
                    "status": "completed",
                    "output": "ok",
                },
            }
        )
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        json.dumps(
            {
                "type": "tool.result",
                "toolName": "sky_cua__browser_status",
                "toolCallId": "call-1",
                "data": {"output": "ok"},
            }
        )
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        "[tool result] sky_cua__browser_status (completed)\n"
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        "[tool result] sky_cua__browser_status failed before completed output\n"
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        "[tool result] sky_cua__browser_status error: unsuccessful\n"
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        "[tool result] sky_cua__browser_status not completed\n"
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        "[tool result] sky_cua__browser_status not ok\n"
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        json.dumps(
            {
                "finalAssistantVisibleText": json.dumps(
                    {"sky_cua_smoke": {"tool_called": True, "tools_visible": True, "error": None}}
                ),
                "completion": {"stopReason": "stop"},
                "toolSummary": {
                    "calls": 1,
                    "failures": 0,
                    "tools": ["sky_cua.browser_status"],
                },
            }
        )
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        json.dumps(
            {
                "toolSummary": {
                    "calls": 0,
                    "failures": 0,
                    "tools": ["sky_cua.browser_status"],
                }
            }
        )
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        json.dumps(
            {
                "reply": {
                    "sky_cua_smoke": {"tool_called": True, "tools_visible": True, "error": None},
                    "toolSummary": {
                        "calls": 1,
                        "failures": 0,
                        "tools": ["sky_cua.browser_status"],
                    },
                }
            }
        )
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        json.dumps(
            {
                "toolSummary": {
                    "calls": 1,
                    "failures": 1,
                    "tools": ["sky_cua.browser_status"],
                }
            }
        )
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(pending_event)
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        pending_event.replace('"pending"', '"failed"')
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        completed_batch_with_pending_call
    )

    failed_wrapper_with_successful_child = json.dumps(
        {
            "type": "session_batch",
            "status": "failed",
            "events": [
                {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "tool-3",
                    "title": "sky_cua__browser_status",
                    "status": "pending",
                },
                {
                    "sessionUpdate": "tool_result",
                    "toolCallId": "tool-3",
                    "content": "ok",
                },
            ],
        }
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(
        failed_wrapper_with_successful_child
    )

    report_only = json.dumps(
        {"reply": {"sky_cua_smoke": {"tool_called": True, "tools_visible": True, "error": None}}}
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_browser_status_tool_event(report_only)


def test_opencode_agent_runner_preserves_status_and_redacts_stdout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    captured: dict[str, Any] = {}

    def fake_run(argv: list[str], **kwargs: Any) -> object:
        captured["argv"] = argv
        captured["env"] = kwargs["env"]
        stdout = cast(Any, kwargs["stdout"])
        stdout.write(
            json.dumps(
                {
                    "type": "tool_use",
                    "tool": "sky_cua_click",
                    "output": "secret desktop text",
                }
            )
            + "\n"
        )
        return _agent_mcp_smoke.subprocess.CompletedProcess(argv, returncode=7)

    monkeypatch.setenv("OPENCODE_API_KEY", "opencode-secret")
    monkeypatch.setenv("MOONSHOT_API_KEY", "moonshot-secret")
    monkeypatch.setenv("CONTEXT7_API_KEY", "context-secret")
    monkeypatch.setenv("SSH_AUTH_SOCK", "/tmp/agent.sock")
    monkeypatch.delenv("SKY_CUA_SMOKE_KEEP_RAW_AGENT_LOG", raising=False)
    monkeypatch.setattr(_agent_mcp_smoke.subprocess, "run", fake_run)

    proc = _agent_mcp_smoke.run_agent(
        "opencode", "use sky cua", tmp_path, model="opencode-go/test-model", gate_deploy=False
    )

    assert proc.returncode == 7
    assert captured["argv"][:3] == ["script", "-q", "-e"]
    assert "--model opencode-go/test-model" in captured["argv"][4]
    env = cast(dict[str, str], captured["env"])
    assert env["OPENCODE_API_KEY"] == "opencode-secret"
    assert env["MOONSHOT_API_KEY"] == "moonshot-secret"
    assert env["CONTEXT7_API_KEY"] == "context-secret"
    assert "SSH_AUTH_SOCK" not in env
    stdout = (tmp_path / "opencode.stdout.log").read_text(encoding="utf-8")
    assert "secret desktop text" not in stdout
    assert '"result": {"redacted": true}' in stdout


def test_openclaw_agent_runner_preserves_state_and_auth_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    captured: dict[str, Any] = {}

    def fake_run(argv: list[str], **kwargs: Any) -> object:
        captured["argv"] = argv
        captured["env"] = kwargs["env"]
        return _agent_mcp_smoke.subprocess.CompletedProcess(argv, returncode=0)

    monkeypatch.setenv("OPENCLAW_STATE_DIR", "/tmp/openclaw-state")
    monkeypatch.setenv("OPENCLAW_CONFIG_PATH", "/tmp/openclaw-state/openclaw.json")
    monkeypatch.setenv("OPENCLAW_GATEWAY_PASSWORD", "gateway-secret")
    monkeypatch.setenv("SSH_AUTH_SOCK", "/tmp/agent.sock")
    monkeypatch.setattr(_agent_mcp_smoke.subprocess, "run", fake_run)

    proc = _agent_mcp_smoke.run_agent("openclaw", "use sky cua", tmp_path, gate_deploy=False)

    assert proc.returncode == 0
    assert captured["argv"][:2] == ["openclaw", "agent"]
    env = cast(dict[str, str], captured["env"])
    assert env["OPENCLAW_STATE_DIR"] == "/tmp/openclaw-state"
    assert env["OPENCLAW_CONFIG_PATH"] == "/tmp/openclaw-state/openclaw.json"
    assert env["OPENCLAW_GATEWAY_PASSWORD"] == "gateway-secret"
    assert "SSH_AUTH_SOCK" not in env


def test_openclaw_smoke_gateway_auth_fallback(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv("OPENCLAW_GATEWAY_TOKEN", raising=False)
    monkeypatch.delenv("OPENCLAW_GATEWAY_PASSWORD", raising=False)

    assert live_openclaw_mcp_smoke.gateway_auth_environment(tmp_path) == {}

    (tmp_path / "gateway.systemd.env").write_text(
        'OTHER_KEY=abc\nOPENCLAW_GATEWAY_PASSWORD="hunter2"\n', encoding="utf-8"
    )
    assert live_openclaw_mcp_smoke.gateway_auth_environment(tmp_path) == {
        "OPENCLAW_GATEWAY_PASSWORD": "hunter2"
    }

    # Values already exported in the environment take precedence over the file.
    monkeypatch.setenv("OPENCLAW_GATEWAY_TOKEN", "tok")
    assert live_openclaw_mcp_smoke.gateway_auth_environment(tmp_path) == {}


def test_openclaw_smoke_agent_report_verdicts() -> None:
    assert (
        live_openclaw_mcp_smoke.check_agent_report(
            {"tool_called": True, "tools_visible": True, "error": None}
        )
        == []
    )

    failures = live_openclaw_mcp_smoke.check_agent_report(
        {"tool_called": False, "tools_visible": False, "error": "tools missing"}
    )
    assert any("not visible" in failure for failure in failures)
    assert any("tools missing" in failure for failure in failures)

    failures = live_openclaw_mcp_smoke.check_agent_report(None)
    assert any("no structured smoke report" in failure for failure in failures)


def test_openclaw_smoke_timeout_yields_failed_result_with_artifacts(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import subprocess

    def fake_run(argv: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(argv, 60, output="partial out", stderr="partial err")

    monkeypatch.setattr(live_openclaw_mcp_smoke.subprocess, "run", fake_run)

    proc = live_openclaw_mcp_smoke.run_openclaw(
        "openclaw", ["mcp", "show"], tmp_path, "mcp-show", None
    )

    assert proc.returncode == live_openclaw_mcp_smoke.TIMEOUT_RETURNCODE
    assert (tmp_path / "mcp-show.stdout.log").read_text(encoding="utf-8") == "partial out"
    stderr_log = (tmp_path / "mcp-show.stderr.log").read_text(encoding="utf-8")
    assert "partial err" in stderr_log
    assert "timed out" in stderr_log


def test_openclaw_smoke_agent_turn_timeout_keeps_stage_timeout(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import subprocess

    def fake_run(argv: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(argv, 60, output="", stderr="")

    monkeypatch.setattr(live_openclaw_mcp_smoke.subprocess, "run", fake_run)
    args = Namespace(openclaw_bin="openclaw", openclaw_dir=None, agent=None, session_key=None)

    stage, failures = live_openclaw_mcp_smoke.run_agent_turn_stage(args, tmp_path)

    assert stage["ok"] is False
    assert stage["timeout"] is True
    assert stage["returncode"] == live_openclaw_mcp_smoke.TIMEOUT_RETURNCODE
    assert failures == [
        f"agent turn timed out after {live_openclaw_mcp_smoke.AGENT_TURN_TIMEOUT_SECONDS} seconds"
    ]


def test_openclaw_smoke_agent_turn_requires_tool_event(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import subprocess

    report = {"sky_cua_smoke": {"tool_called": True, "tools_visible": True, "error": None}}

    def fake_run(argv: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(argv, 0, json.dumps({"reply": report}), "")

    monkeypatch.setattr(live_openclaw_mcp_smoke.subprocess, "run", fake_run)
    args = Namespace(openclaw_bin="openclaw", openclaw_dir=None, agent=None, session_key=None)

    stage, failures = live_openclaw_mcp_smoke.run_agent_turn_stage(args, tmp_path)

    assert stage["ok"] is False
    assert stage["tool_result_seen"] is False
    assert any(
        "did not show a completed sky_cua__browser_status result" in failure for failure in failures
    )


def test_openclaw_smoke_agent_turn_accepts_tool_event_and_report(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import subprocess

    tool_event = {
        "type": "session_update",
        "update": {
            "sessionUpdate": "tool_call",
            "toolCallId": "tool-1",
            "title": "sky_cua__browser_status",
            "status": "pending",
        },
    }
    tool_result = {
        "type": "session_update",
        "update": {
            "sessionUpdate": "tool_result",
            "toolCallId": "tool-1",
            "content": "ok",
        },
    }
    report = {"sky_cua_smoke": {"tool_called": True, "tools_visible": True, "error": None}}
    stdout = "\n".join(
        [json.dumps(tool_event), json.dumps(tool_result), json.dumps({"reply": report})]
    )

    def fake_run(argv: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(argv, 0, stdout, "")

    monkeypatch.setattr(live_openclaw_mcp_smoke.subprocess, "run", fake_run)
    args = Namespace(openclaw_bin="openclaw", openclaw_dir=None, agent=None, session_key=None)

    stage, failures = live_openclaw_mcp_smoke.run_agent_turn_stage(args, tmp_path)

    assert failures == []
    assert stage["ok"] is True
    assert stage["tool_result_seen"] is True


def test_openclaw_smoke_codex_home_stage_validates_pins(tmp_path: Path) -> None:
    state_dir = tmp_path / "openclaw"
    client = tmp_path / "sky-cua-client"
    client.write_text("", encoding="utf-8")

    good = state_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    good.parent.mkdir(parents=True)
    good.write_text(
        f'[mcp_servers.sky_cua]\ncommand = "{client}"\nargs = ["mcp"]\n', encoding="utf-8"
    )
    failures, checked = live_openclaw_mcp_smoke.check_codex_home_pins(state_dir)
    assert failures == []
    assert checked == 1

    missing_pin = state_dir / "agents" / "esther" / "agent" / "codex-home" / "config.toml"
    missing_pin.parent.mkdir(parents=True)
    missing_pin.write_text('model = "gpt-5.5"\n', encoding="utf-8")
    broken = state_dir / "agents" / "luke" / "agent" / "codex-home" / "config.toml"
    broken.parent.mkdir(parents=True)
    broken.write_text("browser = ", encoding="utf-8")
    dead_command = state_dir / "agents" / "main" / "agent" / "codex-home" / "config.toml"
    dead_command.parent.mkdir(parents=True)
    dead_command.write_text(
        '[mcp_servers.sky_cua]\ncommand = "/missing/sky-cua-client"\n', encoding="utf-8"
    )

    failures, checked = live_openclaw_mcp_smoke.check_codex_home_pins(state_dir)
    assert checked == 4
    assert len(failures) == 3
    assert any("missing [mcp_servers.sky_cua] pin" in failure for failure in failures)
    assert any("invalid TOML" in failure for failure in failures)
    assert any("pinned command does not exist" in failure for failure in failures)

    # No agents directory: vacuously clean with zero configs checked.
    failures, checked = live_openclaw_mcp_smoke.check_codex_home_pins(tmp_path / "empty")
    assert failures == []
    assert checked == 0
