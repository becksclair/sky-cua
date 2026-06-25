"""Tests for the overlay spec code generator.

These tests exercise normal generation, idempotence, stale-file detection,
and strict validation without requiring a live desktop or Android runtime.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent
GENERATOR = REPO_ROOT / "scripts" / "generate_overlay_spec.py"
SPEC_PATH = REPO_ROOT / "resources" / "overlay" / "agent_overlay_spec.toml"
RUST_OUT = REPO_ROOT / "crates" / "sky-cua-platform" / "src" / "overlay_spec_generated.rs"
KT_OUT = (
    REPO_ROOT
    / "android"
    / "phone-companion"
    / "app"
    / "src"
    / "main"
    / "java"
    / "com"
    / "skycua"
    / "phonecompanion"
    / "overlay"
    / "OverlaySpec.kt"
)


def _run_generator(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    cmd = [sys.executable, str(GENERATOR), *args]
    return subprocess.run(
        cmd,
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=check,
    )


def _load_spec() -> str:
    return SPEC_PATH.read_text(encoding="utf-8")


def _write_spec(content: str) -> None:
    SPEC_PATH.write_text(content, encoding="utf-8")


@pytest.fixture
def restore_spec():
    """Restore the canonical spec after the test mutates it."""
    original = _load_spec()
    try:
        yield
    finally:
        _write_spec(original)


def test_generator_check_passes_when_files_are_fresh() -> None:
    # Ensure files are generated and current.
    _run_generator()
    result = _run_generator("--check")
    assert result.returncode == 0
    assert "up to date" in result.stdout


def test_generator_is_idempotent() -> None:
    _run_generator()
    first_rust = RUST_OUT.read_text(encoding="utf-8")
    first_kt = KT_OUT.read_text(encoding="utf-8")
    _run_generator()
    second_rust = RUST_OUT.read_text(encoding="utf-8")
    second_kt = KT_OUT.read_text(encoding="utf-8")
    assert first_rust == second_rust
    assert first_kt == second_kt


def test_check_fails_when_spec_changes(restore_spec: None) -> None:
    _run_generator()
    spec = _load_spec()
    # Mutate a value that does not violate range checks.
    spec = spec.replace("breathe_period_ms = 1600", "breathe_period_ms = 1700")
    _write_spec(spec)
    result = _run_generator("--check", check=False)
    assert result.returncode == 1
    assert "stale or missing" in result.stderr


def test_check_fails_when_generated_files_missing(restore_spec: None, tmp_path: Path) -> None:
    rust_backup = tmp_path / "overlay_spec_generated.rs.bak"
    kt_backup = tmp_path / "OverlaySpec.kt.bak"
    _run_generator()
    shutil.copy(RUST_OUT, rust_backup)
    shutil.copy(KT_OUT, kt_backup)
    try:
        RUST_OUT.unlink()
        result = _run_generator("--check", check=False)
        assert result.returncode == 1
    finally:
        shutil.copy(rust_backup, RUST_OUT)
        shutil.copy(kt_backup, KT_OUT)


def test_rejects_unknown_top_level_key(restore_spec: None) -> None:
    spec = _load_spec()
    spec += "\n[unknown_section]\nfoo = 1\n"
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "unknown top-level keys" in result.stderr


def test_rejects_unknown_key_in_section(restore_spec: None) -> None:
    spec = _load_spec()
    spec = spec.replace(
        "[shared.timing]\n",
        "[shared.timing]\nunknown_timing_key_ms = 1\n",
    )
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "unknown keys" in result.stderr


def test_rejects_invalid_schema_version(restore_spec: None) -> None:
    spec = _load_spec()
    spec = spec.replace("schema_version = 1", "schema_version = 2")
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "schema_version must be 1" in result.stderr


def test_rejects_missing_required_key(restore_spec: None) -> None:
    spec = _load_spec()
    spec = spec.replace("min_gesture_duration_ms = 120\n", "")
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "missing required keys" in result.stderr


def test_rejects_missing_required_section(restore_spec: None) -> None:
    spec = _load_spec()
    start = spec.index("[shared.motion]\n")
    end = spec.index("\n[shared.effects]\n")
    spec = spec[:start] + spec[end + 1 :]
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "missing required keys in [shared]" in result.stderr
    assert "motion" in result.stderr


def test_rejects_negative_duration(restore_spec: None) -> None:
    spec = _load_spec()
    spec = spec.replace("min_gesture_duration_ms = 120", "min_gesture_duration_ms = -1")
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "min_gesture_duration_ms" in result.stderr


def test_rejects_alpha_out_of_range(restore_spec: None) -> None:
    spec = _load_spec()
    spec = spec.replace(
        "glow_baseline_min_alpha_0_1 = 0.55",
        "glow_baseline_min_alpha_0_1 = 1.5",
    )
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "glow_baseline_min_alpha_0_1" in result.stderr


def test_rejects_nonfinite_float(restore_spec: None) -> None:
    spec = _load_spec()
    spec = spec.replace(
        "cursor_max_speed_dp_per_s = 950.0",
        "cursor_max_speed_dp_per_s = nan",
    )
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "cursor_max_speed_dp_per_s" in result.stderr
    assert "finite" in result.stderr


def test_rejects_inconsistent_geometry(restore_spec: None) -> None:
    spec = _load_spec()
    spec = spec.replace(
        "ripple_min_logical_px = 20.0",
        "ripple_min_logical_px = 200.0",
    )
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "ripple_min_logical_px must be <= ripple_max_logical_px" in result.stderr


def test_rejects_excessive_gesture_points(restore_spec: None) -> None:
    spec = _load_spec()
    spec = spec.replace("max_gesture_points = 16", "max_gesture_points = 9999")
    _write_spec(spec)
    result = _run_generator(check=False)
    assert result.returncode == 1
    assert "max_gesture_points" in result.stderr


def test_generated_rust_contains_expected_constants() -> None:
    _run_generator()
    rust = RUST_OUT.read_text(encoding="utf-8")
    assert "pub mod overlay_spec" in rust
    assert "pub const SCHEMA_VERSION: u32 = 1;" in rust
    assert "pub const MIN_GESTURE_DURATION_MS: u64 = 120;" in rust
    assert "pub const CURSOR_NOSE_DEG: f64 = -135.0;" in rust


def test_generated_kt_contains_expected_constants() -> None:
    _run_generator()
    kt = KT_OUT.read_text(encoding="utf-8")
    assert "object OverlaySpec" in kt
    assert "const val SCHEMA_VERSION: Int = 1" in kt
    assert "const val MIN_GESTURE_DURATION_MS: Long = 120L" in kt
    assert "const val CURSOR_NOSE_DEG: Double = -135.0" in kt


def test_generator_uses_canonical_toml_by_default() -> None:
    result = _run_generator("--help", check=True)
    assert "TOML" in result.stdout or "overlay" in result.stdout
