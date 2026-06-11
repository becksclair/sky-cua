"""Tests for shared live-smoke configuration and desktop smoke helpers."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

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


def test_openclaw_smoke_show_config_accepts_installed_auto_mode(tmp_path: Path) -> None:
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
