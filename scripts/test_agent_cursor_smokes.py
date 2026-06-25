"""Tests for agent-cursor KDE and X11 overlay smoke helpers."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import cast

import pytest

import live_agent_cursor_kde_smoke
import live_agent_cursor_x11_overlay_smoke


def test_kwin_effect_static_mode_requires_explicit_install_flag(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "live_agent_cursor_kde_smoke.py",
            "--mode",
            "kwin-effect-static",
        ],
    )

    with pytest.raises(SystemExit) as exc_info:
        live_agent_cursor_kde_smoke.main()

    message = str(exc_info.value)
    assert "kwin-effect-static installs and loads a user-level KWin cursor-hide shim" in message
    assert "--allow-kwin-effect-install" in message


def test_agent_cursor_smoke_x11_mode_forces_x11_backend(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    captured: dict[str, object] = {}

    class FakePopen:
        def __init__(self, args: list[str], **kwargs: object) -> None:
            captured["args"] = args
            captured.update(kwargs)

    monkeypatch.setattr(live_agent_cursor_kde_smoke.subprocess, "Popen", FakePopen)

    process = live_agent_cursor_kde_smoke.start_service(
        tmp_path / "svc.sock", tmp_path, mode="x11-debug-visible"
    )

    assert isinstance(process, FakePopen)
    env = cast(dict[str, str], captured["env"])
    assert env["SKY_CUA_OVERLAY_BACKEND"] == "x11"
    assert env["SKY_CUA_SCREENSHOT_CURSOR"] == "never"
    assert env["SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE"] == "never"


def test_agent_cursor_smoke_layer_shell_click_through_mode_forces_visible_overlay_env(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    captured: dict[str, object] = {}

    class FakePopen:
        def __init__(self, args: list[str], **kwargs: object) -> None:
            captured["args"] = args
            captured.update(kwargs)

    monkeypatch.setattr(live_agent_cursor_kde_smoke.subprocess, "Popen", FakePopen)

    process = live_agent_cursor_kde_smoke.start_service(
        tmp_path / "svc.sock", tmp_path, mode="layer-shell-click-through"
    )

    assert isinstance(process, FakePopen)
    env = cast(dict[str, str], captured["env"])
    assert env["SKY_CUA_OVERLAY_BACKEND"] == "wayland-layer-shell"
    assert env["SKY_CUA_SCREENSHOT_CURSOR"] == "never"
    assert env["SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE"] == "never"


def test_x11_overlay_smoke_forces_true_x11_backend_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("WAYLAND_DISPLAY", "wayland-0")

    env = live_agent_cursor_x11_overlay_smoke.x11_overlay_env(":42", tmp_path)

    assert env["DISPLAY"] == ":42"
    assert env["XDG_SESSION_TYPE"] == "x11"
    assert env["XDG_RUNTIME_DIR"] == str(tmp_path)
    assert env["SKY_CUA_OVERLAY_BACKEND"] == "x11"
    assert "WAYLAND_DISPLAY" not in env


def test_x11_overlay_smoke_cursor_message_uses_stream_pixels() -> None:
    message = live_agent_cursor_x11_overlay_smoke.cursor_message((330.0, 240.0), sequence=7)

    assert message == {
        "version": 1,
        "kind": "set_cursor",
        "state": {
            "visible": True,
            "sequence": 7,
            "model_point": {
                "x": 330.0,
                "y": 240.0,
                "coordinate_space": "stream_pixels",
            },
            "source_action": "click",
            "updated_at_ms": 0,
        },
    }


def test_x11_overlay_smoke_show_message_reuses_state_and_forces_visible() -> None:
    hidden_reply = {
        "ok": True,
        "state": {
            "visible": False,
            "sequence": 7,
            "model_point": {
                "x": 330.0,
                "y": 240.0,
                "coordinate_space": "stream_pixels",
            },
            "source_action": "click",
            "updated_at_ms": 0,
        },
    }

    message = live_agent_cursor_x11_overlay_smoke.show_cursor_message(hidden_reply)

    assert message["version"] == 1
    assert message["kind"] == "show"
    assert message["state"]["visible"] is True
    assert message["state"]["sequence"] == 7
    assert hidden_reply["state"]["visible"] is False


def test_x11_overlay_smoke_show_message_requires_state() -> None:
    with pytest.raises(RuntimeError, match="did not include cursor state"):
        live_agent_cursor_x11_overlay_smoke.show_cursor_message({"ok": True})


def test_x11_overlay_smoke_rejects_non_x11_overlay_reply() -> None:
    with pytest.raises(RuntimeError, match="x11_shaped_window"):
        live_agent_cursor_x11_overlay_smoke.require_x11_overlay_reply(
            {
                "ok": True,
                "capabilities": {
                    "backend": "none",
                    "visible_overlay": False,
                    "click_through": False,
                    "system_cursor_hide_supported": False,
                    "system_cursor_hidden": False,
                },
            }
        )


def test_x11_overlay_smoke_visible_cursor_reply_requires_visible_state() -> None:
    with pytest.raises(RuntimeError, match="visible cursor"):
        live_agent_cursor_x11_overlay_smoke.require_visible_cursor_reply(
            {"ok": True, "state": {"visible": False}},
            context="show",
        )

    live_agent_cursor_x11_overlay_smoke.require_visible_cursor_reply(
        {"ok": True, "state": {"visible": True}},
        context="show",
    )


def test_x11_overlay_smoke_system_cursor_reply_tracks_hide_state() -> None:
    live_agent_cursor_x11_overlay_smoke.require_system_cursor_reply(
        {
            "ok": True,
            "capabilities": {
                "system_cursor_hide_supported": True,
                "system_cursor_hidden": True,
            },
        },
        hidden=True,
        context="set",
    )
    live_agent_cursor_x11_overlay_smoke.require_system_cursor_reply(
        {
            "ok": True,
            "capabilities": {
                "system_cursor_hide_supported": True,
                "system_cursor_hidden": False,
            },
        },
        hidden=False,
        context="hide",
    )
    assert (
        live_agent_cursor_x11_overlay_smoke.capability_bool(
            {
                "capabilities": {
                    "system_cursor_hidden": True,
                }
            },
            "system_cursor_hidden",
        )
        is True
    )


def test_x11_overlay_current_display_requires_real_x11_session(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("DISPLAY", ":0")
    monkeypatch.setenv("XDG_SESSION_TYPE", "wayland")
    monkeypatch.setenv("WAYLAND_DISPLAY", "wayland-0")

    with pytest.raises(RuntimeError, match="real X11 session"):
        live_agent_cursor_x11_overlay_smoke.require_current_x11_display()


def test_x11_overlay_current_display_accepts_x11_without_wayland(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("DISPLAY", ":7")
    monkeypatch.setenv("XDG_SESSION_TYPE", "x11")
    monkeypatch.delenv("WAYLAND_DISPLAY", raising=False)

    assert live_agent_cursor_x11_overlay_smoke.require_current_x11_display() == ":7"


def test_kwin_effect_list_parser_ignores_blank_lines() -> None:
    assert live_agent_cursor_kde_smoke.parse_kwin_effect_list(
        "\nblur\n\nsky-cua-agent-cursor\n  showfps  \n"
    ) == ["blur", "sky-cua-agent-cursor", "showfps"]


def test_kde_smoke_names_expected_visible_overlay_backend_by_mode() -> None:
    assert (
        live_agent_cursor_kde_smoke.expected_overlay_backend("layer-shell-debug-visible")
        == "wayland_layer_shell"
    )
    assert (
        live_agent_cursor_kde_smoke.expected_overlay_backend("layer-shell-hide-for-capture")
        == "wayland_layer_shell"
    )
    assert (
        live_agent_cursor_kde_smoke.expected_overlay_backend("layer-shell-click-through")
        == "wayland_layer_shell"
    )
    assert (
        live_agent_cursor_kde_smoke.expected_overlay_backend("x11-debug-visible")
        == "x11_shaped_window"
    )
    assert live_agent_cursor_kde_smoke.expected_overlay_backend("synthetic") is None


def test_kde_smoke_rejects_visible_overlay_mode_without_expected_backend() -> None:
    with pytest.raises(RuntimeError, match="wayland_layer_shell"):
        live_agent_cursor_kde_smoke.require_cursor_backend_capabilities(
            {
                "capabilities": {
                    "backend": "screenshot_synthetic",
                    "visible_overlay": False,
                    "click_through": False,
                }
            },
            expected_backend="wayland_layer_shell",
        )


def test_kde_smoke_accepts_expected_visible_overlay_capabilities() -> None:
    live_agent_cursor_kde_smoke.require_cursor_backend_capabilities(
        {
            "capabilities": {
                "backend": "wayland_layer_shell",
                "renderer_backend": "wgpu",
                "visible_overlay": True,
                "click_through": True,
                "pointer_tracking_backend": "kwin_effect_signal",
                "pointer_tracking_exact": True,
                "coverage": "full",
                "active_output_count": 2,
                "rendered_output_count": 2,
                "adapter_name": "llvmpipe",
            }
        },
        expected_backend="wayland_layer_shell",
        expected_renderer="wgpu",
        expected_pointer_tracking="kwin_effect_signal",
        expected_pointer_tracking_exact=True,
    )

    with pytest.raises(RuntimeError, match="output coverage"):
        live_agent_cursor_kde_smoke.require_cursor_backend_capabilities(
            {
                "capabilities": {
                    "backend": "wayland_layer_shell",
                    "renderer_backend": "wgpu",
                    "visible_overlay": True,
                    "click_through": True,
                    "coverage": "full",
                    "active_output_count": 2,
                    "rendered_output_count": 1,
                    "adapter_name": "llvmpipe",
                }
            },
            expected_backend="wayland_layer_shell",
        )

    with pytest.raises(RuntimeError, match="adapter_name"):
        live_agent_cursor_kde_smoke.require_cursor_backend_capabilities(
            {
                "capabilities": {
                    "backend": "wayland_layer_shell",
                    "renderer_backend": "wgpu",
                    "visible_overlay": True,
                    "click_through": True,
                    "coverage": "full",
                    "active_output_count": 1,
                    "rendered_output_count": 1,
                }
            },
            expected_backend="wayland_layer_shell",
        )


def test_kde_smoke_accepts_kwin_system_cursor_split_capabilities() -> None:
    live_agent_cursor_kde_smoke.require_kwin_system_cursor_capabilities(
        {
            "capabilities": {
                "backend": "wayland_layer_shell",
                "visible_overlay": True,
                "click_through": True,
                "system_cursor_backend": "kwin_effect",
                "system_cursor_hide_supported": True,
                "system_cursor_hidden": True,
            }
        },
        hidden=True,
    )
    with pytest.raises(RuntimeError, match="system_cursor_backend"):
        live_agent_cursor_kde_smoke.require_kwin_system_cursor_capabilities(
            {
                "capabilities": {
                    "system_cursor_backend": "wayland_client_unsupported",
                    "system_cursor_hide_supported": False,
                    "system_cursor_hidden": False,
                }
            },
            hidden=True,
        )


def test_kde_smoke_native_point_for_display_target_capture_is_desktop_logical() -> None:
    point = live_agent_cursor_kde_smoke.native_point_from_capture(
        {
            "backend": "portal_pipe_wire",
            "pixel_size": {"width": 400, "height": 200},
            "logical_rect": {
                "x": 100,
                "y": 50,
                "width": 200,
                "height": 100,
                "space": "desktop_logical",
            },
            "mapping_id": "mapping",
        },
        (40.0, 50.0),
    )

    assert point == {
        "x": 120.0,
        "y": 75.0,
        "coordinate_space": "desktop_logical",
        "mapping_id": "mapping",
    }


def test_kde_smoke_native_point_for_stream_local_portal_capture_stays_stream_logical() -> None:
    point = live_agent_cursor_kde_smoke.native_point_from_capture(
        {
            "backend": "portal_pipe_wire",
            "pixel_size": {"width": 400, "height": 200},
            "logical_rect": {
                "x": 100,
                "y": 50,
                "width": 200,
                "height": 100,
                "space": "stream_logical",
            },
            "mapping_id": "mapping",
        },
        (40.0, 50.0),
    )

    assert point == {
        "x": 20.0,
        "y": 25.0,
        "coordinate_space": "stream_logical",
        "mapping_id": "mapping",
    }


def test_kde_smoke_selects_display_for_logical_point() -> None:
    snapshot = {
        "environment": {
            "displays": [
                {
                    "display_id": "left",
                    "name": "Left",
                    "logical_rect": {
                        "x": -1280,
                        "y": 0,
                        "width": 1280,
                        "height": 720,
                        "space": "desktop_logical",
                    },
                },
                {
                    "display_id": "right",
                    "name": "Right",
                    "logical_rect": {
                        "x": 0,
                        "y": 0,
                        "width": 1920,
                        "height": 1080,
                        "space": "desktop_logical",
                    },
                },
            ]
        }
    }

    display = live_agent_cursor_kde_smoke.display_for_logical_point(
        snapshot, {"x": -40.0, "y": 80.0}
    )

    assert display["display_id"] == "left"
    assert live_agent_cursor_kde_smoke.display_target_for_display(display) == {"display_id": "left"}
    assert live_agent_cursor_kde_smoke.native_point_in_display(
        {
            "x": -40.0,
            "y": 80.0,
            "coordinate_space": "desktop_logical",
        },
        display,
    )


def test_kde_smoke_display_rects_are_half_open_at_monitor_seams() -> None:
    left = {"x": -1280, "y": 0, "width": 1280, "height": 720}
    right = {"x": 0, "y": 0, "width": 1920, "height": 1080}

    assert not live_agent_cursor_kde_smoke.rect_contains_point(left, 0.0, 100.0)
    assert live_agent_cursor_kde_smoke.rect_contains_point(right, 0.0, 100.0)


def test_kde_smoke_maps_logical_fixture_point_to_model_pixels() -> None:
    point = live_agent_cursor_kde_smoke.model_point_from_logical_capture(
        {
            "backend": "portal_pipe_wire",
            "pixel_size": {"width": 800, "height": 400},
            "logical_rect": {
                "x": 100,
                "y": 50,
                "width": 400,
                "height": 200,
                "space": "desktop_logical",
            },
        },
        {"x": 300.0, "y": 100.0},
    )

    assert point == (400.0, 100.0)


def test_kde_smoke_rejects_fixture_point_outside_capture() -> None:
    with pytest.raises(RuntimeError, match="outside capture pixel bounds"):
        live_agent_cursor_kde_smoke.model_point_from_logical_capture(
            {
                "pixel_size": {"width": 800, "height": 400},
                "logical_rect": {
                    "x": 100,
                    "y": 50,
                    "width": 400,
                    "height": 200,
                    "space": "desktop_logical",
                },
            },
            {"x": 900.0, "y": 100.0},
        )


def test_kde_smoke_agent_probe_uses_native_point_when_available() -> None:
    point = live_agent_cursor_kde_smoke.agent_cursor_probe_point(
        {
            "agent_cursor": {
                "native_point": {
                    "x": 300.0,
                    "y": 100.0,
                    "coordinate_space": "desktop_logical",
                },
                "model_point": {
                    "x": 12.0,
                    "y": 34.0,
                    "coordinate_space": "stream_pixels",
                },
            },
        },
        {
            "pixel_size": {"width": 800, "height": 400},
            "logical_rect": {
                "x": 100,
                "y": 50,
                "width": 400,
                "height": 200,
                "space": "desktop_logical",
            },
        },
        (1.0, 2.0),
    )

    assert point == (400.0, 100.0)


def test_kde_smoke_agent_probe_fails_when_native_point_leaves_capture() -> None:
    point = live_agent_cursor_kde_smoke.agent_cursor_probe_point(
        {
            "agent_cursor": {
                "native_point": {
                    "x": 900.0,
                    "y": 100.0,
                    "coordinate_space": "desktop_logical",
                },
                "model_point": {
                    "x": 12.0,
                    "y": 34.0,
                    "coordinate_space": "stream_pixels",
                },
            },
        },
        {
            "pixel_size": {"width": 800, "height": 400},
            "logical_rect": {
                "x": 100,
                "y": 50,
                "width": 400,
                "height": 200,
                "space": "desktop_logical",
            },
        },
        (1.0, 2.0),
    )

    assert point is None


def test_kde_smoke_execute_click_request_uses_snapshot_and_stream_pixels() -> None:
    assert live_agent_cursor_kde_smoke.execute_click_request("snap-1", (12.5, 99.0)) == {
        "type": "execute_action",
        "request": {
            "action": "click",
            "snapshot_id": "snap-1",
            "arguments": {"x": 12.5, "y": 99.0},
        },
    }


def test_kde_smoke_fixture_point_requires_named_point() -> None:
    with pytest.raises(RuntimeError, match="click_button"):
        live_agent_cursor_kde_smoke.fixture_point({"points": {}}, "click_button")
