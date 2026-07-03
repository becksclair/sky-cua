"""Tests for the pure logic of the desktop overlay motion harness.

The capture path itself is desktop-dependent (a Wayland session, the ScreenCast
portal, gstreamer, spectacle, the overlay host binary); these tests cover only
the desktop-free pieces: scenario builders, timing math, screen-geometry
parsing, the gst pipeline argv, restore-token persistence, stills naming, and
the offline frame grouping. No portal, no gst, no host is spawned.
"""

from __future__ import annotations

from pathlib import Path

import pytest

import _contact_sheets
import _kde_screencast as ks
import overlay_motion_animations as oma

SIZES = [(2560.0, 1440.0), (1920.0, 1080.0), (1024.0, 768.0)]


def _in_bounds(moves: list[oma.Move], w: float, h: float) -> bool:
    return all(0.0 <= x <= w and 0.0 <= y <= h for move in moves for x, y in move.points)


@pytest.mark.parametrize(("w", "h"), SIZES)
def test_corner_moves_are_glides_in_bounds_with_center_last(w: float, h: float) -> None:
    moves = oma.corner_moves(w, h)
    assert len(moves) == 5
    assert all(move.kind == "glide" and len(move.points) == 1 for move in moves)
    assert _in_bounds(moves, w, h)
    assert moves[-1].points[0] == (w / 2.0, h / 2.0)


@pytest.mark.parametrize(("w", "h"), SIZES)
def test_redirect_moves_fire_faster_than_a_glide_settles(w: float, h: float) -> None:
    moves = oma.redirect_moves(w, h)
    assert _in_bounds(moves, w, h)
    # The whole point is mid-flight retargeting: the dwell is well under a glide.
    assert all(move.kind == "glide" and move.pause_s < 0.5 for move in moves)


@pytest.mark.parametrize(("w", "h"), SIZES)
def test_swipe_moves_are_two_point_swipes(w: float, h: float) -> None:
    moves = oma.swipe_moves(w, h)
    assert moves
    assert all(move.kind == "swipe" and len(move.points) == 2 for move in moves)
    assert all(move.duration_ms > 0 for move in moves)
    assert _in_bounds(moves, w, h)


@pytest.mark.parametrize(("w", "h"), SIZES)
def test_fan_moves_alternate_ring_and_center(w: float, h: float) -> None:
    moves = oma.fan_moves(w, h, count=8)
    assert len(moves) == 16  # ring glide + return-to-center, per spoke
    assert _in_bounds(moves, w, h)
    centers = moves[1::2]
    assert all(move.points[0] == (w / 2.0, h / 2.0) for move in centers)


@pytest.mark.parametrize(("w", "h"), SIZES)
def test_tap_settle_pairs_are_colocated_glide_then_tap(w: float, h: float) -> None:
    moves = oma.tap_settle_moves(w, h)
    assert moves and len(moves) % 2 == 0
    assert _in_bounds(moves, w, h)
    for glide, tap in zip(moves[0::2], moves[1::2], strict=True):
        # The far retarget and the Tap aim at the SAME point, back to back, so
        # the ripple visibly waits for the mover's arrival on video.
        assert glide.kind == "glide" and tap.kind == "tap"
        assert glide.points[0] == tap.points[0]
        assert glide.pause_s == 0.0
        assert tap.pause_s > 1.0


@pytest.mark.parametrize(("w", "h"), SIZES)
def test_fast_flick_moves_are_rapid_glides(w: float, h: float) -> None:
    moves = oma.fast_flick_moves(w, h)
    assert _in_bounds(moves, w, h)
    assert all(move.kind == "glide" and move.pause_s <= 0.2 for move in moves)


def test_moves_for_concatenates_selected_scenarios() -> None:
    combined = oma.moves_for(["corners", "swipes"], 1920.0, 1080.0)
    assert len(combined) == len(oma.corner_moves(1920.0, 1080.0)) + len(
        oma.swipe_moves(1920.0, 1080.0)
    )


def test_recording_seconds_covers_moves_with_lead_and_tail() -> None:
    moves = oma.moves_for(list(oma.DEFAULT_SCENARIOS), 2560.0, 1440.0)
    seconds = oma.recording_seconds(moves)
    assert seconds >= sum(move.pause_s for move in moves)
    assert seconds >= oma.recording_seconds([])  # lead-in + tail floor
    assert 3 <= seconds <= 600


def test_options_default_scenario_set() -> None:
    opts = oma.options_from_args(oma.build_parser().parse_args([]))
    assert opts.scenarios == ["corners", "redirect", "swipes", "tap_settle"]
    assert opts.recorder == "auto"
    assert not opts.offline


def test_options_reject_width_without_height() -> None:
    with pytest.raises(SystemExit):
        oma.options_from_args(oma.build_parser().parse_args(["--width", "1920"]))


def test_union_logical_geometry_unions_enabled_outputs_and_divides_by_scale() -> None:
    outputs = [
        {"enabled": True, "pos": {"x": 0, "y": 0}, "size": {"width": 2560, "height": 1440}},
        {
            "enabled": True,
            "pos": {"x": -1280, "y": 100},
            "size": {"width": 1920, "height": 1080},
            "scale": 1.5,
        },
        {"enabled": False, "pos": {"x": 9000, "y": 0}, "size": {"width": 800, "height": 600}},
    ]
    geometry = oma.union_logical_geometry(outputs)
    assert geometry.x == -1280.0
    assert geometry.y == 0.0
    assert geometry.width == 2560.0 + 1280.0
    assert geometry.height == 1440.0


def test_union_logical_geometry_without_enabled_outputs_fails_honestly() -> None:
    with pytest.raises(SystemExit):
        oma.union_logical_geometry([{"enabled": False}])


def test_cursor_state_carries_logical_points_snapshot_and_fresh_timestamp() -> None:
    state = oma.cursor_state((120.5, 480.25), 7)
    assert state["visible"] is True
    assert state["sequence"] == 7
    for key in ("model_point", "native_point"):
        assert state[key] == {"x": 120.5, "y": 480.25, "coordinate_space": "desktop_logical"}
    assert state["snapshot_id"] == oma.SNAPSHOT_ID
    assert state["updated_at_ms"] > 0


def test_gst_pipeline_args_shape() -> None:
    args = ks.gst_pipeline_args(9, 42, Path("/tmp/out.mp4"))
    assert args[0] == "gst-launch-1.0"
    assert args[1] == "-e"  # SIGINT must flush EOS so the MP4 muxes cleanly
    assert "fd=9" in args
    assert "path=42" in args
    assert "location=/tmp/out.mp4" in args
    for element in ("pipewiresrc", "videoconvert", "x264enc", "h264parse", "mp4mux", "filesink"):
        assert element in args


def test_restore_token_round_trips(tmp_path: Path) -> None:
    token_path = tmp_path / "nested" / ".screencast-restore-token"
    assert ks.read_restore_token(token_path) is None
    ks.write_restore_token(token_path, "token-123")
    assert ks.read_restore_token(token_path) == "token-123"


def test_read_restore_token_ignores_blank_files(tmp_path: Path) -> None:
    token_path = tmp_path / ".screencast-restore-token"
    token_path.write_text("  \n", encoding="utf-8")
    assert ks.read_restore_token(token_path) is None


def test_stills_frame_naming() -> None:
    assert oma.stills_frame_path(Path("/tmp/frames"), 7) == Path("/tmp/frames/frame_0007.png")
    assert oma.stills_frame_path(Path("/tmp/frames"), 1234).name == "frame_1234.png"


def test_scenario_group_strips_trailing_frame_tag() -> None:
    # The motion dump's contract: <scenario>-f<NN>.rgba frames.
    assert oma.scenario_group("corner_glide-f07") == "corner_glide"
    assert oma.scenario_group("arrival_gated_tap-f21") == "arrival_gated_tap"
    # The gesture dump's <scenario>_<index> form groups too.
    assert oma.scenario_group("corner_glide_0007") == "corner_glide"
    assert oma.scenario_group("redirect_12") == "redirect"
    # Non-frame stems stay themselves.
    assert oma.scenario_group("manifest") == "manifest"
    assert oma.scenario_group("swipe-chase") == "swipe-chase"


def test_own_sheets_matches_only_this_stems_pagination_pages(tmp_path: Path) -> None:
    sheet = tmp_path / "contact-corners.png"
    sheet.write_bytes(b"png")
    (tmp_path / "contact-corners-0.png").write_bytes(b"png")
    (tmp_path / "contact-corners-1.png").write_bytes(b"png")
    (tmp_path / "contact-corners-redirect-0.png").write_bytes(b"png")
    names = [page.name for page in _contact_sheets.own_sheets(sheet)]
    assert names == ["contact-corners-0.png", "contact-corners-1.png", "contact-corners.png"]
