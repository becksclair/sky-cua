"""Tests for shared live-smoke configuration and desktop smoke helpers."""

from __future__ import annotations

import json
from argparse import Namespace
from pathlib import Path
from typing import Any, cast

import pytest

import _agent_mcp_smoke
import _agent_perf_judge
import _cua_coverage
import _model_profiles
import live_agent_mcp_smoke
import live_agentic_loop_smoke
import live_desktop_smoke
import live_fallback_anchor_smoke
import live_openclaw_mcp_smoke
import live_portal_downgrade_smoke
import live_targeted_screenshot_smoke
import live_wayland_pointer_smoke
from _codex_exec import DEFAULT_MODEL, DEFAULT_REASONING_EFFORT
from _pointer_geometry import adjusted_origin_for_visible_monitor
from _smoke_config import LIVE_SMOKE_MODEL, LIVE_SMOKE_REASONING_EFFORT


def test_live_smoke_model_config_is_centralized() -> None:
    configured = _model_profiles.model_profile("codex_exec")
    assert configured.model == LIVE_SMOKE_MODEL
    assert configured.reasoning_effort == LIVE_SMOKE_REASONING_EFFORT
    assert DEFAULT_MODEL == LIVE_SMOKE_MODEL
    assert DEFAULT_REASONING_EFFORT == LIVE_SMOKE_REASONING_EFFORT


def test_agentic_loop_default_uses_tool_evidence_enforced_agent() -> None:
    assert live_agentic_loop_smoke.DEFAULT_AGENT in {"opencode", "pi"}
    assert set(live_agentic_loop_smoke.ACCEPTANCE_AGENTS) == {"opencode", "pi"}


def test_agent_smoke_fixtures_are_dialog_dismissal_flows() -> None:
    assert set(live_agent_mcp_smoke.FIXTURES) == {"kdialog", "zenity"}


def test_agentic_loop_fixture_choices_include_fallback_anchor() -> None:
    # fallback-anchor is a distinct flow (fallback-anchor proof, not a
    # dialog-dismiss task), so it stays out of FIXTURES but must still be a
    # selectable --fixture value.
    assert "fallback-anchor" in live_agentic_loop_smoke.FIXTURE_CHOICES
    assert set(live_agentic_loop_smoke.FIXTURE_CHOICES) == {
        "kdialog",
        "zenity",
        "fallback-anchor",
    }
    assert live_agentic_loop_smoke.DEFAULT_FIXTURE == "zenity"


def test_scroll_region_prefers_visible_scroll_pane_over_oversized_content() -> None:
    selected = live_desktop_smoke.find_scroll_region(
        {
            "elements": [
                {
                    "element_index": 21,
                    "role": "panel",
                    "name": "Scroll region",
                    "bounds": {"height": 1595},
                },
                {
                    "element_index": 20,
                    "role": "scroll pane",
                    "name": "Scroll region",
                    "bounds": {"height": 334},
                },
            ]
        }
    )
    assert selected["element_index"] == 20


def test_live_desktop_requires_top_level_appshot_capture_provenance() -> None:
    live_desktop_smoke.require_live_wayland_image_backend(
        {
            "capture_backend": "portal_pipe_wire",
            "image_backend": "portal_pipe_wire",
            "diagnostics": [],
        },
        "fixture",
    )

    with pytest.raises(RuntimeError, match="actual image backend"):
        live_desktop_smoke.require_live_wayland_image_backend(
            {"capture": {"image_backend": "portal_pipe_wire"}, "diagnostics": []},
            "legacy fixture",
        )


def test_live_desktop_normalizes_canonical_appshot_without_losing_fences() -> None:
    normalized = live_desktop_smoke.normalized_appshot(
        {
            "appshot_id": "appshot-1",
            "action_snapshot": {"snapshot_id": "snapshot-1"},
            "semantic_projection": {
                "elements": [{"element_index": 0}],
                "focused_app": {"name": "fixture"},
                "accessibility": {"backend": "atspi"},
            },
            "image_backend": "portal_pipe_wire",
        }
    )

    assert normalized["elements"] == [{"element_index": 0}]
    assert normalized["focused_app"] == {"name": "fixture"}
    assert normalized["accessibility"] == {"backend": "atspi"}
    assert normalized["snapshot_id"] == "snapshot-1"
    assert normalized["appshot_id"] == "appshot-1"
    assert live_desktop_smoke.appshot_action_fences(normalized) == {
        "appshot_id": "appshot-1",
        "snapshot_id": "snapshot-1",
    }


def test_targeted_screenshot_click_carries_both_action_fences() -> None:
    arguments = live_targeted_screenshot_smoke.targeted_click_arguments(
        {
            "appshot_id": "appshot-1",
            "action_snapshot": {"snapshot_id": "snapshot-1"},
        },
        {"pixel_size": {"width": 1000, "height": 500}},
    )

    assert arguments == {
        "operation": "click",
        "appshot_id": "appshot-1",
        "snapshot_id": "snapshot-1",
        "x": 730.0,
        "y": 407.5,
    }


def test_mpv_launch_argv_idles_with_distinctive_title() -> None:
    argv = live_fallback_anchor_smoke.build_launch_argv()

    assert argv[0] == live_fallback_anchor_smoke.MPV_BIN
    assert "--idle" in argv
    assert "--force-window" in argv
    assert "--no-config" in argv
    expected_title_arg = f"--title={live_fallback_anchor_smoke.FIXTURE_WINDOW_TITLE}"
    assert expected_title_arg in argv


@pytest.mark.parametrize(
    ("payload", "expected"),
    [
        pytest.param(
            {
                "elements": [
                    {
                        "element_index": 0,
                        "role": "window",
                        "state_flags": ["vision_anchor", "physical_target"],
                    }
                ]
            },
            True,
            id="single_vision_anchor_window",
        ),
        pytest.param(
            {
                "elements": [
                    {
                        "element_index": 0,
                        "role": "window",
                        "state_flags": ["native_window_fallback", "physical_target"],
                    }
                ]
            },
            False,
            id="fallback_root_without_vision_anchor_flag",
        ),
        pytest.param(
            {
                "elements": [
                    {"element_index": 0, "role": "window", "state_flags": ["vision_anchor"]},
                    {"element_index": 1, "role": "push_button", "state_flags": ["focusable"]},
                ]
            },
            False,
            id="vision_anchor_plus_rich_atspi_child_is_not_fallback_only",
        ),
        pytest.param(
            {"elements": []},
            False,
            id="empty_elements",
        ),
        pytest.param(
            {"focused_app": {"name": "mpv"}},
            False,
            id="payload_without_elements_key",
        ),
        pytest.param(
            "not a dict",
            False,
            id="non_dict_payload",
        ),
        pytest.param(
            {
                "elements": [
                    {
                        "element_index": 0,
                        "role": "x11_leaf_region",
                        "state_flags": ["vision_anchor", "x11_fallback"],
                    }
                ]
            },
            True,
            id="native_fallback_role_other_than_window_still_counts",
        ),
    ],
)
def test_observe_payload_proves_fallback_table(payload: object, expected: bool) -> None:
    assert live_fallback_anchor_smoke.observe_payload_proves_fallback(payload) is expected


def test_stdout_proves_fallback_scans_raw_jsonl_including_embedded_json_strings(
    tmp_path: Path,
) -> None:
    fallback_element = {
        "elements": [
            {
                "element_index": 0,
                "role": "window",
                "state_flags": ["vision_anchor", "physical_target"],
            }
        ]
    }
    lines = [
        json.dumps({"type": "tool_use_start", "tool": "sky_cua_observe"}),
        # Result payload embedded as a JSON string within a text content
        # block, the shape pi/opencode raw transcripts actually use.
        json.dumps(
            {
                "type": "tool_result",
                "content": [{"type": "text", "text": json.dumps(fallback_element)}],
            }
        ),
        "not json at all",
    ]
    stdout_path = tmp_path / "pi.stdout.log"
    stdout_path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    assert live_fallback_anchor_smoke.stdout_proves_fallback(stdout_path) is True


def test_stdout_proves_fallback_false_without_matching_evidence(tmp_path: Path) -> None:
    stdout_path = tmp_path / "pi.stdout.log"
    stdout_path.write_text(
        json.dumps({"type": "tool_use_start", "tool": "sky_cua_observe"}) + "\n",
        encoding="utf-8",
    )

    assert live_fallback_anchor_smoke.stdout_proves_fallback(stdout_path) is False


def test_stdout_proves_fallback_false_for_missing_file(tmp_path: Path) -> None:
    assert live_fallback_anchor_smoke.stdout_proves_fallback(tmp_path / "missing.log") is False


def test_text_proves_fallback_true_on_real_captured_line() -> None:
    # Verbatim from a live opencode run: sky-cua's observe result logged as
    # the tool's TEXT-summary content block, with no structured JSON at all.
    line = (
        "... states=native_window_fallback,physical_target,vision_anchor,"
        "container,content_like,focused,active "
        "bounds=(563.0,336.0 652.0x394.0 DesktopLogical) ..."
    )
    assert live_fallback_anchor_smoke.text_proves_fallback(line) is True


def test_text_proves_fallback_false_without_native_window_fallback_flag() -> None:
    line = "... states=physical_target,vision_anchor,container,focused ..."
    assert live_fallback_anchor_smoke.text_proves_fallback(line) is False


def test_text_proves_fallback_false_without_vision_anchor_flag() -> None:
    line = "... states=native_window_fallback,physical_target,container,focused ..."
    assert live_fallback_anchor_smoke.text_proves_fallback(line) is False


def test_text_proves_fallback_false_without_either_flag() -> None:
    line = "... states=physical_target,container,content_like,focused,active ..."
    assert live_fallback_anchor_smoke.text_proves_fallback(line) is False


def test_text_proves_fallback_false_on_empty_or_garbage_text() -> None:
    assert live_fallback_anchor_smoke.text_proves_fallback("") is False
    assert live_fallback_anchor_smoke.text_proves_fallback("not a states line at all") is False


def test_stdout_proves_fallback_true_on_text_only_states_line(tmp_path: Path) -> None:
    # No structured JSON anywhere in the file, only the text-summary form
    # sky-cua's observe result actually takes in some agent CLIs' raw logs.
    stdout_path = tmp_path / "opencode.stdout.log"
    stdout_path.write_text(
        "role=window states=native_window_fallback,physical_target,vision_anchor,"
        "container,content_like,focused,active "
        "bounds=(563.0,336.0 652.0x394.0 DesktopLogical)\n",
        encoding="utf-8",
    )

    assert live_fallback_anchor_smoke.stdout_proves_fallback(stdout_path) is True


def test_kill_fallback_anchor_mpv_matches_command_line_markers(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    title = live_fallback_anchor_smoke.FIXTURE_WINDOW_TITLE
    pgrep_output = (
        f"1234 /usr/bin/mpv --idle --force-window --no-config --title {title}\n"
        f"1235 /usr/lib/mpv/helper --type=cache --parent-title={title}\n"
        "9999 /usr/bin/mpv --idle --force-window --title some-other-window\n"
    )
    killed_pids: list[int] = []

    def fake_run(argv: list[str], **_kwargs: object) -> object:
        assert argv == ["pgrep", "-af", "mpv"]
        return live_fallback_anchor_smoke.subprocess.CompletedProcess(
            argv, returncode=0, stdout=pgrep_output, stderr=""
        )

    def fake_kill(pid: int, _sig: int) -> None:
        killed_pids.append(pid)

    monkeypatch.setattr(live_fallback_anchor_smoke.subprocess, "run", fake_run)
    monkeypatch.setattr(live_fallback_anchor_smoke.os, "kill", fake_kill)

    live_fallback_anchor_smoke.kill_fallback_anchor_mpv()

    assert sorted(killed_pids) == [1234, 1235]


def test_kill_fallback_anchor_mpv_tolerates_missing_pgrep(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def fake_run(*_args: object, **_kwargs: object) -> object:
        raise FileNotFoundError("pgrep")

    monkeypatch.setattr(live_fallback_anchor_smoke.subprocess, "run", fake_run)

    live_fallback_anchor_smoke.kill_fallback_anchor_mpv()  # must not raise


def test_run_fallback_anchor_smoke_rejects_agents_without_tool_evidence() -> None:
    with pytest.raises(ValueError, match="opencode or pi"):
        live_fallback_anchor_smoke.run_fallback_anchor_smoke(agent="claude")


def test_run_fallback_anchor_smoke_passes_when_fallback_proved(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _wire_fallback_anchor_smoke_fakes(
        tmp_path,
        monkeypatch,
        fallback_element_present=True,
    )

    assert live_fallback_anchor_smoke.run_fallback_anchor_smoke(agent="pi") == 0

    result = json.loads((tmp_path / "result.json").read_text(encoding="utf-8"))
    assert result["fallback_proved"] is True
    assert result["ok"] is True


def test_run_fallback_anchor_smoke_fails_without_fallback_evidence(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    _wire_fallback_anchor_smoke_fakes(
        tmp_path,
        monkeypatch,
        fallback_element_present=False,
    )

    assert live_fallback_anchor_smoke.run_fallback_anchor_smoke(agent="pi") == 1

    result = json.loads((tmp_path / "result.json").read_text(encoding="utf-8"))
    assert result["fallback_proved"] is False
    assert result["ok"] is False


def test_run_fallback_anchor_smoke_tears_down_mpv_process(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    teardown_calls: list[str] = []

    monkeypatch.setattr(
        live_fallback_anchor_smoke,
        "kill_fallback_anchor_mpv",
        lambda: teardown_calls.append("kill"),
    )
    _wire_fallback_anchor_smoke_fakes(
        tmp_path,
        monkeypatch,
        fallback_element_present=True,
        patch_teardown=False,
    )

    live_fallback_anchor_smoke.run_fallback_anchor_smoke(agent="pi")

    assert teardown_calls == ["kill"]


def _wire_fallback_anchor_smoke_fakes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    *,
    fallback_element_present: bool,
    patch_teardown: bool = True,
) -> None:
    class FakeLaunchProcess:
        def poll(self) -> int | None:
            return None

        def terminate(self) -> None:
            return None

    def fake_run_agent(
        agent: str,
        _prompt: str,
        artifact_dir: Path,
        **_kwargs: object,
    ) -> object:
        stdout = artifact_dir / f"{agent}.stdout.log"
        if fallback_element_present:
            body = {
                "elements": [
                    {
                        "element_index": 0,
                        "role": "window",
                        "state_flags": ["vision_anchor", "physical_target"],
                    }
                ]
            }
        else:
            body = {"elements": []}
        stdout.write_text(
            json.dumps({"type": "tool_result", "result": body}) + "\n",
            encoding="utf-8",
        )
        return live_fallback_anchor_smoke.subprocess.CompletedProcess([agent], returncode=0)

    monkeypatch.setattr(live_fallback_anchor_smoke, "make_artifact_dir", lambda *_args: tmp_path)
    monkeypatch.setattr(
        live_fallback_anchor_smoke.subprocess, "Popen", lambda *_a, **_k: FakeLaunchProcess()
    )
    monkeypatch.setattr(live_fallback_anchor_smoke, "run_agent", fake_run_agent)
    monkeypatch.setattr(live_fallback_anchor_smoke.time, "sleep", lambda *_a: None)
    if patch_teardown:
        monkeypatch.setattr(live_fallback_anchor_smoke, "kill_fallback_anchor_mpv", lambda: None)


def test_agent_smoke_prompt_tells_agents_to_return_after_action() -> None:
    prompt = live_agent_mcp_smoke.build_agent_prompt(
        agent="opencode",
        fixture_title="sky-cua agent smoke",
        prompt_suffix="dismiss it by confirming OK",
    )

    assert "After a successful sky-cua action, return immediately" in prompt


def test_agent_smoke_pi_prompt_documents_generic_mcp_wrapper() -> None:
    prompt = live_agent_mcp_smoke.build_agent_prompt(
        agent="pi",
        fixture_title="sky-cua agent smoke",
        prompt_suffix="dismiss it by confirming OK",
    )

    assert "Pi's generic mcp tool" in prompt
    assert "args set to a JSON string" in prompt
    assert "args set to a JSON object" not in prompt
    assert 'args "{\\"operation\\":\\"press_key\\",\\"key\\":\\"Enter\\"}"' in prompt
    assert "Do not call desktop list_resources with title_contains" in prompt


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
        "appshot_id": "appshot-root-only",
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
        "appshot_id": "appshot-root-only",
        "snapshot_id": "snapshot-root-only",
        "x": 959.0,
        "y": 565.52,
    }


def test_x11_click_target_prefers_lowest_leaf_region_when_available() -> None:
    snapshot = {
        "appshot_id": "appshot-with-leaves",
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
        "appshot_id": "appshot-with-leaves",
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
            "tool": "sky_cua_desktop_pointer",
            "state": {
                "status": "completed",
                "output": "Invoked the element semantically through AT-SPI.",
            },
        },
    }

    assert live_agent_mcp_smoke._tool_evidence_from_stdout_line(json.dumps(event)) is True


def test_agent_smoke_accepts_grouped_action_tool_evidence(tmp_path: Path) -> None:
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
                        "tool": "sky_cua_observe",
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
                    "tool": "sky_cua_desktop_pointer",
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
    assert "sky_cua_desktop_pointer" in redacted_text


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


def test_pi_smoke_default_model_is_free_opencode_model() -> None:
    assert _model_profiles.model_profile("pi_mcp").model == (
        _agent_mcp_smoke.DEFAULT_PI_SMOKE_MODEL
    )
    assert (
        _model_profiles.model_profile("opencode_mcp").model
        == _agent_mcp_smoke.DEFAULT_OPENCODE_SMOKE_MODEL
    )
    assert "OPENAI_API_KEY" not in _agent_mcp_smoke.model_auth_environment_keys(
        "pi", _agent_mcp_smoke.DEFAULT_PI_SMOKE_MODEL
    )
    assert "OPENCODE_API_KEY" in _agent_mcp_smoke.model_auth_environment_keys(
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
            "tool": "sky_cua_desktop_pointer",
            "state": {"status": "completed"},
            "toolResult": {"content": [{"type": "text", "text": "clicked"}]},
        },
    }

    assert live_agent_mcp_smoke._tool_evidence_from_stdout_line(json.dumps(event)) is True


def test_agent_smoke_rejects_status_only_completed_state() -> None:
    event = {
        "type": "tool_use",
        "tool": "sky_cua_desktop_pointer",
        "state": "completed",
    }

    assert live_agent_mcp_smoke._tool_evidence_from_stdout_line(json.dumps(event)) is False


def test_agent_smoke_rejects_opencode_failed_tool_use_event_shape() -> None:
    event = {
        "type": "tool_use",
        "part": {
            "type": "tool",
            "tool": "sky_cua_desktop_pointer",
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
                "tool": "sky_cua_desktop_pointer",
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
            "tool": "sky_cua_desktop_pointer",
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
                        "tool": "sky_cua_observe",
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


def test_agent_smoke_accepts_pi_anonymous_split_action_events(tmp_path: Path) -> None:
    stdout = tmp_path / "pi.stdout.log"
    stdout.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "args": {"tool": "sky_cua_desktop_action"},
                        "tool": "sky_cua_desktop_action",
                        "toolName": "mcp",
                        "type": "tool_execution_start",
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

    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(stdout) is True


def test_pi_redactor_summarizes_ctx_execute_stdio_mcp_intent_without_evidence(
    tmp_path: Path,
) -> None:
    raw = tmp_path / "pi.raw.jsonl"
    redacted = tmp_path / "pi.stdout.log"
    raw.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "type": "tool_execution_start",
                        "toolName": "ctx_execute",
                        "args": {
                            "language": "javascript",
                            "code": (
                                "spawn('/home/bex/.local/share/sky-cua/pi_mcp_wrapper.sh');"
                                "JSON.stringify({method:'tools/call',params:{name:'desktop_keyboard',"
                                "arguments:{operation:'press_key',key:'Enter'}}});"
                            ),
                        },
                    }
                ),
                json.dumps(
                    {
                        "type": "tool_execution_end",
                        "toolName": "ctx_execute",
                        "isError": False,
                        "result": {"stdout": "dismissed"},
                    }
                ),
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    _agent_mcp_smoke.redact_pi_json_stdout(raw, redacted)

    text = redacted.read_text(encoding="utf-8")
    assert "pi_mcp_wrapper.sh" not in text
    assert "press_key" not in text
    assert "ctx_execute_mcp_intent" in text
    assert live_agent_mcp_smoke._stdout_has_sky_cua_tool_evidence(redacted) is False
    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(redacted) is False


def test_pi_redactor_rejects_ctx_execute_without_known_mcp_wrapper(tmp_path: Path) -> None:
    raw = tmp_path / "pi.raw.jsonl"
    redacted = tmp_path / "pi.stdout.log"
    raw.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "type": "tool_execution_start",
                        "toolName": "ctx_execute",
                        "args": {
                            "language": "javascript",
                            "code": (
                                "require('child_process').spawn('xdotool', ['key', 'Enter']);"
                                "JSON.stringify({method:'tools/call',params:{name:'desktop_keyboard'}});"
                            ),
                        },
                    }
                ),
                json.dumps(
                    {
                        "type": "tool_execution_end",
                        "toolName": "ctx_execute",
                        "isError": False,
                        "result": {"stdout": "dismissed"},
                    }
                ),
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    _agent_mcp_smoke.redact_pi_json_stdout(raw, redacted)

    assert "ctx_execute_mcp_intent" not in redacted.read_text(encoding="utf-8")
    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(redacted) is False


def test_pi_redactor_rejects_ctx_execute_with_generic_client_name_only(tmp_path: Path) -> None:
    raw = tmp_path / "pi.raw.jsonl"
    redacted = tmp_path / "pi.stdout.log"
    raw.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "type": "tool_execution_start",
                        "toolName": "ctx_execute",
                        "args": {
                            "language": "javascript",
                            "code": (
                                "const note = 'sky-cua-client';"
                                "JSON.stringify({method:'tools/call',params:{name:'desktop_keyboard'}});"
                            ),
                        },
                    }
                ),
                json.dumps(
                    {
                        "type": "tool_execution_end",
                        "toolName": "ctx_execute",
                        "isError": False,
                        "result": {"stdout": "dismissed"},
                    }
                ),
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    _agent_mcp_smoke.redact_pi_json_stdout(raw, redacted)

    assert "ctx_execute_mcp_intent" not in redacted.read_text(encoding="utf-8")
    assert live_agent_mcp_smoke._stdout_has_sky_cua_action_tool_evidence(redacted) is False


def test_agent_smoke_rejects_mismatched_split_tool_completion(tmp_path: Path) -> None:
    stdout = tmp_path / "pi.stdout.log"
    stdout.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "tool": "sky_cua_desktop_pointer",
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


def test_agent_smoke_rejects_pi_failed_anonymous_action_before_success(
    tmp_path: Path,
) -> None:
    stdout = tmp_path / "pi.stdout.log"
    stdout.write_text(
        "\n".join(
            [
                json.dumps(
                    {
                        "tool": "sky_cua_desktop_action",
                        "toolName": "mcp",
                        "type": "tool_execution_start",
                    }
                ),
                json.dumps(
                    {
                        "isError": False,
                        "result": {"redacted": True},
                        "result_declares_failure": True,
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
                        "tool": "sky_cua_observe",
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
                    "tool": "sky_cua_observe",
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
    assert any("sky_cua__status" in failure for failure in failures)

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


def test_openclaw_smoke_detects_status_tool_event() -> None:
    pending_event = json.dumps(
        {
            "type": "session_update",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-1",
                "title": "sky_cua__status",
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
                    "title": "sky_cua__status",
                    "status": "pending",
                }
            ],
        }
    )

    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        "\n".join([pending_event, completed_event])
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        "\n".join([pending_event, result_event])
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        json.dumps(
            {
                "message": {
                    "role": "toolResult",
                    "toolName": "sky_cua__status",
                    "toolCallId": "call-1",
                    "isError": False,
                    "content": [{"type": "toolResult", "text": "ok"}],
                }
            }
        )
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        json.dumps(
            {
                "type": "toolResult",
                "toolName": "sky_cua__status",
                "toolCallId": "call-1",
                "isError": False,
                "content": "ok",
            }
        )
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        json.dumps(
            {
                "type": "tool.result",
                "data": {
                    "name": "sky_cua__status",
                    "toolCallId": "call-1",
                    "status": "completed",
                    "output": "ok",
                },
            }
        )
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        json.dumps(
            {
                "type": "tool.result",
                "toolName": "sky_cua__status",
                "toolCallId": "call-1",
                "data": {"output": "ok"},
            }
        )
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        "[tool result] sky_cua__status (completed)\n"
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        "[tool result] sky_cua__status failed before completed output\n"
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        "[tool result] sky_cua__status error: unsuccessful\n"
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        "[tool result] sky_cua__status not completed\n"
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        "[tool result] sky_cua__status not ok\n"
    )
    assert live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        json.dumps(
            {
                "finalAssistantVisibleText": json.dumps(
                    {"sky_cua_smoke": {"tool_called": True, "tools_visible": True, "error": None}}
                ),
                "completion": {"stopReason": "stop"},
                "toolSummary": {
                    "calls": 1,
                    "failures": 0,
                    "tools": ["sky_cua.status"],
                },
            }
        )
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        json.dumps(
            {
                "toolSummary": {
                    "calls": 0,
                    "failures": 0,
                    "tools": ["sky_cua.status"],
                }
            }
        )
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        json.dumps(
            {
                "reply": {
                    "sky_cua_smoke": {"tool_called": True, "tools_visible": True, "error": None},
                    "toolSummary": {
                        "calls": 1,
                        "failures": 0,
                        "tools": ["sky_cua.status"],
                    },
                }
            }
        )
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        json.dumps(
            {
                "toolSummary": {
                    "calls": 1,
                    "failures": 1,
                    "tools": ["sky_cua.status"],
                }
            }
        )
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(pending_event)
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        pending_event.replace('"pending"', '"failed"')
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
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
                    "title": "sky_cua__status",
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
    assert live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(
        failed_wrapper_with_successful_child
    )

    report_only = json.dumps(
        {"reply": {"sky_cua_smoke": {"tool_called": True, "tools_visible": True, "error": None}}}
    )
    assert not live_openclaw_mcp_smoke.agent_turn_has_status_tool_event(report_only)


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
                    "tool": "sky_cua_desktop_pointer",
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


def test_opencode_agent_runner_uses_configured_default_model(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    captured: dict[str, Any] = {}

    def fake_run(argv: list[str], **kwargs: Any) -> object:
        captured["argv"] = argv
        captured["env"] = kwargs["env"]
        return _agent_mcp_smoke.subprocess.CompletedProcess(argv, returncode=0)

    monkeypatch.setenv("OPENCODE_API_KEY", "opencode-secret")
    monkeypatch.delenv("SKY_CUA_SMOKE_OPENCODE_MODEL", raising=False)
    monkeypatch.setattr(_agent_mcp_smoke.subprocess, "run", fake_run)

    proc = _agent_mcp_smoke.run_agent("opencode", "use sky cua", tmp_path, gate_deploy=False)

    assert proc.returncode == 0
    assert f"--model {_agent_mcp_smoke.DEFAULT_OPENCODE_SMOKE_MODEL}" in captured["argv"][4]
    env = cast(dict[str, str], captured["env"])
    assert env["OPENCODE_API_KEY"] == "opencode-secret"


def test_claude_agent_runner_explicit_model_precedes_environment(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    captured: dict[str, Any] = {}

    def fake_run(argv: list[str], **kwargs: Any) -> object:
        captured["argv"] = argv
        return _agent_mcp_smoke.subprocess.CompletedProcess(argv, returncode=0)

    monkeypatch.setenv("SKY_CUA_SMOKE_CLAUDE_MODEL", "environment-model")
    monkeypatch.setattr(_agent_mcp_smoke.shutil, "which", lambda _name: "/usr/bin/claude")
    monkeypatch.setattr(_agent_mcp_smoke.subprocess, "run", fake_run)

    proc = _agent_mcp_smoke.run_agent(
        "claude", "use sky cua", tmp_path, model="explicit-model", gate_deploy=False
    )

    assert proc.returncode == 0
    argv = cast(list[str], captured["argv"])
    assert argv[argv.index("--model") + 1] == "explicit-model"


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
    assert any("did not show a completed sky_cua__status result" in failure for failure in failures)


def test_openclaw_smoke_agent_turn_accepts_tool_event_and_report(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import subprocess

    tool_event = {
        "type": "session_update",
        "update": {
            "sessionUpdate": "tool_call",
            "toolCallId": "tool-1",
            "title": "sky_cua__status",
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
    missing_pin.write_text('model = "gpt-5.6-luna"\n', encoding="utf-8")
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


def _cua_call(call_id: str, tool: str, **arguments: Any) -> dict[str, Any]:
    return {
        "type": "mcp_tool_call",
        "id": call_id,
        "server": "computer-use",
        "tool": tool,
        "arguments": arguments,
        "status": "completed",
    }


def _full_coverage_calls() -> list[dict[str, Any]]:
    calls = [
        _cua_call("a1", "mcp__computer_use__observe", surface="desktop"),
        _cua_call("a2", "observe", surface="browser", tab_id="t1"),
        _cua_call("a3", "capture_desktop"),
        _cua_call("a4", "capture_screen", surface="browser", tab_id="t1"),
        _cua_call("a5", "activate_window", window_id="w1"),
        _cua_call("a6", "desktop_pointer", operation="click"),
        _cua_call("a7", "desktop_pointer", operation="secondary_click"),
        _cua_call("a8", "desktop_pointer", operation="drag"),
        _cua_call("a9", "desktop_keyboard", operation="type_text"),
        _cua_call("a10", "desktop_keyboard", operation="press_key"),
        _cua_call("a11", "desktop_semantic", operation="select"),
        _cua_call("a12", "desktop_action", operation="activate"),
        _cua_call("a13", "desktop_set_value", value="x"),
        _cua_call("a14", "desktop_scroll", direction="down"),
        _cua_call("a15", "browser_open"),
        _cua_call("a16", "browser_navigate", url="http://x"),
        _cua_call("a17", "browser_claim_tab", tab_id="t1"),
        _cua_call("a18", "browser_move_mouse", tab_id="t1"),
        _cua_call("a19", "browser_scroll", tab_id="t1"),
        _cua_call("a20", "browser_input", operation="click"),
        _cua_call("a21", "browser_input", operation="type_text"),
        _cua_call("a22", "browser_input", operation="press_key"),
    ]
    return calls


def test_cua_bare_tool_name_strips_namespaces() -> None:
    assert _cua_coverage.bare_tool_name("mcp__computer_use__observe") == "observe"
    assert _cua_coverage.bare_tool_name("sky_cua_desktop_pointer") == "desktop_pointer"
    assert _cua_coverage.bare_tool_name("browser_open") == "browser_open"


def test_cua_coverage_full_pass() -> None:
    report = _cua_coverage.analyze_coverage(_full_coverage_calls())
    assert report.missing_tools == []
    assert report.missing_operations == []
    assert report.missing_surfaces == []
    assert report.errors == []
    assert report.ok is True
    # observe was exercised on both surfaces.
    assert report.surfaces_seen["observe"] == ["browser", "desktop"]


def test_cua_coverage_reports_missing_and_errors() -> None:
    calls = [
        _cua_call("b1", "observe", surface="desktop"),
        _cua_call("b2", "desktop_pointer", operation="click"),
        {
            "type": "mcp_tool_call",
            "id": "b3",
            "server": "computer-use",
            "tool": "browser_open",
            "arguments": {},
            "status": "failed",
            "error": "BrowserBridgeDisconnected",
        },
    ]
    report = _cua_coverage.analyze_coverage(calls)
    assert report.ok is False
    # missing the bulk of required tools, the other pointer operations, browser surfaces.
    assert "capture_desktop" in report.missing_tools
    assert "desktop_pointer:drag" in report.missing_operations
    assert "capture_screen:browser" in report.missing_surfaces
    # browser_open never succeeds again, so the error is unrecovered and fatal.
    assert report.errors == [
        {"tool": "browser_open", "excerpt": "BrowserBridgeDisconnected", "recovered": False}
    ]
    assert report.unrecovered_errors == report.errors


def test_cua_coverage_recovered_error_does_not_fail_gate() -> None:
    # A transient error followed by a successful retry of the same operation is
    # recovery, not failure: it stays reported but does not flip ``ok``.
    calls = [
        *_full_coverage_calls(),
        {
            "type": "mcp_tool_call",
            "id": "drag-fail",
            "server": "computer-use",
            "tool": "desktop_pointer",
            "arguments": {"operation": "drag", "duration_ms": 500},
            "status": "failed",
            "error": "Invalid desktop_pointer request: duration_ms not in schema",
        },
        _cua_call("drag-retry", "desktop_pointer", operation="drag"),
    ]
    report = _cua_coverage.analyze_coverage(calls)
    assert report.errors == [
        {
            "tool": "desktop_pointer",
            "excerpt": "Invalid desktop_pointer request: duration_ms not in schema",
            "recovered": True,
        }
    ]
    assert report.unrecovered_errors == []
    assert report.ok is True
    assert "unrecovered" not in " ".join(report.problems())


def test_cua_coverage_merges_started_and_completed_items() -> None:
    # The started item carries the arguments (operation), the completed item the
    # status; coverage must union them rather than letting completed clobber args.
    started = {
        "type": "mcp_tool_call",
        "id": "c1",
        "server": "computer-use",
        "tool": "desktop_pointer",
        "arguments": {"operation": "drag"},
    }
    completed = {
        "type": "mcp_tool_call",
        "id": "c1",
        "server": "computer-use",
        "tool": "desktop_pointer",
        "status": "completed",
    }
    report = _cua_coverage.analyze_coverage([started, completed])
    assert report.operations_seen["desktop_pointer"] == ["drag"]
    assert report.tools_seen["desktop_pointer"] == 1


def test_condense_transcript_strips_images_and_records_errors(tmp_path: Path) -> None:
    image_blob = "B" * 5000
    events = [
        {
            "type": "item.started",
            "item": {
                "type": "mcp_tool_call",
                "id": "1",
                "server": "computer-use",
                "tool": "mcp__computer_use__capture_desktop",
                "arguments": {"surface": "desktop"},
            },
        },
        {
            "type": "item.completed",
            "item": {
                "type": "mcp_tool_call",
                "id": "1",
                "server": "computer-use",
                "tool": "mcp__computer_use__capture_desktop",
                "status": "completed",
                "result": {"content": [{"type": "image", "data": image_blob}]},
            },
        },
        {
            "type": "item.completed",
            "item": {
                "type": "mcp_tool_call",
                "id": "2",
                "server": "computer-use",
                "tool": "browser_open",
                "arguments": {},
                "status": "failed",
                "error": "BrowserBridgeDisconnected",
            },
        },
    ]
    transcript = tmp_path / "codex-output.jsonl"
    transcript.write_text("\n".join(json.dumps(event) for event in events) + "\n", encoding="utf-8")

    condensed = _agent_perf_judge.condense_transcript(transcript)

    serialized = json.dumps(condensed)
    assert image_blob not in serialized  # the dominant token sink is stripped
    by_tool = {entry["tool"]: entry for entry in condensed}
    assert by_tool["capture_desktop"]["status"] == "ok"
    assert by_tool["capture_desktop"]["arguments"]  # merged from the started item
    assert by_tool["browser_open"]["status"] == "error"
    assert by_tool["browser_open"]["error"] == "BrowserBridgeDisconnected"
    assert all("seq" in entry for entry in condensed)
