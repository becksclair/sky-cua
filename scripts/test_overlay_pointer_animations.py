"""Tests for the pure scenario logic of the overlay pointer-animation harness.

The capture path itself is hardware-dependent (a device, adb, ffmpeg, montage);
these tests cover only the device-free move builders and timing math.
"""

from __future__ import annotations

import overlay_pointer_animations as opa

WIDTH = 1440
HEIGHT = 3120


def _in_bounds(moves: list[opa.Move]) -> bool:
    return all(0.0 <= x <= WIDTH and 0.0 <= y <= HEIGHT for move in moves for x, y in move.points)


def test_corner_moves_are_taps_in_bounds_with_center_last() -> None:
    moves = opa.corner_moves(WIDTH, HEIGHT)
    assert len(moves) == 5
    assert all(move.kind == "tap" and len(move.points) == 1 for move in moves)
    assert _in_bounds(moves)
    assert moves[-1].points[0] == (WIDTH / 2.0, HEIGHT / 2.0)


def test_fan_moves_alternate_ring_and_center() -> None:
    moves = opa.fan_moves(WIDTH, HEIGHT, count=8)
    assert len(moves) == 16  # ring tap + return-to-center, per spoke
    assert _in_bounds(moves)
    # Every other move returns to the centre.
    centers = moves[1::2]
    assert all(move.points[0] == (WIDTH / 2.0, HEIGHT / 2.0) for move in centers)


def test_swipe_moves_have_two_points() -> None:
    moves = opa.swipe_moves(WIDTH, HEIGHT)
    assert moves and all(move.kind == "swipe" and len(move.points) == 2 for move in moves)
    assert _in_bounds(moves)


def test_redirect_moves_fire_faster_than_a_glide_settles() -> None:
    moves = opa.redirect_moves(WIDTH, HEIGHT)
    assert _in_bounds(moves)
    # The whole point is rapid retargeting: the dwell is well under a glide.
    assert all(move.pause_s < 0.5 for move in moves)


def test_moves_for_concatenates_selected_scenarios() -> None:
    combined = opa.moves_for(["corners", "swipes"], WIDTH, HEIGHT)
    assert len(combined) == len(opa.corner_moves(WIDTH, HEIGHT)) + len(
        opa.swipe_moves(WIDTH, HEIGHT)
    )


def test_recording_seconds_covers_moves_with_lead_and_tail() -> None:
    moves = opa.corner_moves(WIDTH, HEIGHT)
    seconds = opa.recording_seconds(moves)
    assert seconds >= sum(move.pause_s for move in moves)
    assert 3 <= seconds <= 180


def test_options_default_scenarios() -> None:
    parser = opa.build_parser()
    opts = opa.options_from_args(parser.parse_args(["--serial", "dev:1"]))
    assert opts.scenarios == ["corners", "redirect", "swipes"]
    assert opts.serial == "dev:1"
