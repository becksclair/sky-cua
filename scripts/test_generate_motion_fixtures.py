"""Tests for the motion fixture generator.

These tests exercise generation freshness, byte idempotence, the Kotlin Float
emulation helper, and the branch-flip sample-selection rule without requiring
a device, desktop, or Android runtime. The cross-language byte-sync guard for
the two fixture copies also lives in `test_overlay_spec_codegen.py`.
"""

from __future__ import annotations

import json
import math
import subprocess
import sys
from pathlib import Path

import pytest

import generate_motion_fixtures as gmf

REPO_ROOT = Path(__file__).resolve().parent.parent
GENERATOR = REPO_ROOT / "scripts" / "generate_motion_fixtures.py"
CANONICAL = REPO_ROOT / "resources" / "overlay" / "agent_overlay_motion_fixtures.json"
ANDROID_COPY = (
    REPO_ROOT
    / "android"
    / "phone-companion"
    / "app"
    / "src"
    / "test"
    / "resources"
    / "overlay"
    / "agent_overlay_motion_fixtures.json"
)

# Snapshot of the hand-maintained families that predate the generator, in file
# order. The generator must never rewrite or reorder them.
PRE_EXISTING_FAMILIES = (
    "breathing_intensity",
    "wave_phase",
    "halo_breathing",
    "pulse_intensity",
    "ripple_radius",
    "ripple_alpha",
    "click_scale",
    "trail_alpha",
    "no_no_wiggle",
    "path_sampling",
)

GENERATED_FAMILIES = ("mover_trajectory", "approach_angle", "wrap_radians", "trail_resample")


def _run_generator(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    cmd = [sys.executable, str(GENERATOR), *args]
    return subprocess.run(
        cmd,
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=check,
    )


@pytest.fixture
def restore_fixture_copies():
    """Restore both fixture copies after a test mutates them."""
    canonical = CANONICAL.read_bytes()
    android = ANDROID_COPY.read_bytes()
    try:
        yield
    finally:
        CANONICAL.write_bytes(canonical)
        ANDROID_COPY.write_bytes(android)


def test_check_passes_when_fresh() -> None:
    _run_generator()
    result = _run_generator("--check")
    assert result.returncode == 0
    assert "up to date" in result.stdout


def test_check_fails_when_android_copy_diverges(restore_fixture_copies: None) -> None:
    _run_generator()
    ANDROID_COPY.write_bytes(ANDROID_COPY.read_bytes() + b"\n")
    result = _run_generator("--check", check=False)
    assert result.returncode == 1
    assert "stale or missing" in result.stderr


def test_check_fails_when_generated_family_is_hand_edited(restore_fixture_copies: None) -> None:
    _run_generator()
    text = CANONICAL.read_text(encoding="utf-8")
    mutated = text.replace('"name": "straight_arrival"', '"name": "straight_arrivalx"', 1)
    assert mutated != text
    CANONICAL.write_text(mutated, encoding="utf-8")
    result = _run_generator("--check", check=False)
    assert result.returncode == 1
    assert "stale or missing" in result.stderr


def test_regeneration_is_byte_idempotent() -> None:
    _run_generator()
    first_canonical = CANONICAL.read_bytes()
    first_android = ANDROID_COPY.read_bytes()
    _run_generator()
    assert CANONICAL.read_bytes() == first_canonical
    assert ANDROID_COPY.read_bytes() == first_android


def test_both_copies_are_written_byte_identical() -> None:
    _run_generator()
    assert CANONICAL.read_bytes() == ANDROID_COPY.read_bytes()


def test_f32_round_trips_known_binary32_values() -> None:
    assert gmf.f32(0.1) == 0.10000000149011612
    assert gmf.f32(1.5) == 1.5
    assert gmf.f32(math.pi) == 3.1415927410125732
    # Smallest positive subnormal survives the round trip.
    assert gmf.f32(2.0**-149) == 2.0**-149
    # Negative zero keeps its sign bit.
    assert math.copysign(1.0, gmf.f32(-0.0)) == -1.0


def test_flip_rule_rejects_sample_next_to_branch_flip() -> None:
    branches = ["snap"] + ["integrate"] * 10 + ["pass_target"] + ["settle"] * 10
    # Flips land at steps 12 (integrate -> pass_target) and 13 (-> settle).
    assert gmf.flip_steps(branches) == [12, 13]
    with pytest.raises(ValueError, match="branch flip"):
        gmf.assert_samples_clear_of_flips(branches, [11])
    with pytest.raises(ValueError, match="branch flip"):
        gmf.assert_samples_clear_of_flips(branches, [15])


def test_flip_rule_accepts_samples_clear_of_flips() -> None:
    branches = ["snap"] + ["integrate"] * 10 + ["pass_target"] + ["settle"] * 10
    gmf.assert_samples_clear_of_flips(branches, [1, 5, 9, 16, 22])
    # A snap -> integrate transition is not a settle/land flip.
    gmf.assert_samples_clear_of_flips(["snap"] + ["integrate"] * 5, [1, 2])


def test_new_families_present_and_pre_existing_untouched() -> None:
    _run_generator()
    data = json.loads(CANONICAL.read_text(encoding="utf-8"))

    families = list(data["fixtures"].keys())
    assert families == list(PRE_EXISTING_FAMILIES + GENERATED_FAMILIES)

    # Spot-check hand-maintained content survives regeneration verbatim.
    assert data["fixtures"]["breathing_intensity"][0] == {"elapsed_ms": 0, "expected": 0.55}
    assert data["fixtures"]["no_no_wiggle"][4] == {
        "progress": 0.5,
        "amplitude_deg": 20.0,
        "expected": -20.0,
    }
    assert data["fixtures"]["path_sampling"][1]["expected"] == {"x": 3.0, "y": 4.0}

    assert data["tolerance"] == {"default": 1e-4, "loose": 0.01, "mover": 0.002}


def test_generated_families_have_expected_shape() -> None:
    _run_generator()
    data = json.loads(CANONICAL.read_text(encoding="utf-8"))
    fixtures = data["fixtures"]

    trajectory_names = [case["name"] for case in fixtures["mover_trajectory"]]
    assert trajectory_names == [
        "straight_arrival",
        "homing_convergence",
        "redirect_mid_flight",
        "bounds_clamp",
        "dt_clamp",
        "first_step_snaps",
    ]
    for case in fixtures["mover_trajectory"]:
        total_steps = sum(seg["steps"] for seg in case["segments"])
        assert case["samples"], case["name"]
        assert all(1 <= sample["step"] <= total_steps for sample in case["samples"])

    snap_case = fixtures["mover_trajectory"][-1]
    assert "start" not in snap_case
    snap_sample = snap_case["samples"][0]
    assert snap_sample["x"] == 321.5
    assert snap_sample["y"] == 87.25
    assert snap_sample["speed"] == 0.0

    settled = [case for case in fixtures["mover_trajectory"] if "settled_step" in case]
    assert {case["name"] for case in settled} == {"straight_arrival", "homing_convergence"}
    for case in settled:
        landing = [s for s in case["samples"] if s["step"] >= case["settled_step"]]
        target = case["segments"][-1]["target"]
        assert landing
        assert all(
            s["x"] == target["x"] and s["y"] == target["y"] and s["speed"] == 0.0 for s in landing
        )

    assert len(fixtures["approach_angle"]) == 5
    assert len(fixtures["wrap_radians"]) == 4

    for case in fixtures["trail_resample"]:
        assert case["sample_count"] == 12
        assert len(case["expected"]) == case["sample_count"]
        # The resample spans start to the swept head.
        assert case["expected"][0] == case["points"][0]
