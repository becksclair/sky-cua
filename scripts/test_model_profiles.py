from __future__ import annotations

import json
from pathlib import Path

import pytest

import _model_profiles


def test_runtime_harness_profiles_are_centralized_and_valid() -> None:
    codex = _model_profiles.model_profile("codex_exec")
    assert _model_profiles.model_profile("codex_cua") == codex
    assert _model_profiles.model_profile("performance_judge") == codex
    assert codex.reasoning_effort is not None
    assert _model_profiles.model_profile("opencode_mcp").reasoning_effort is None
    assert _model_profiles.model_profile("pi_mcp").reasoning_effort is None
    assert _model_profiles.model_profile("claude_mcp").reasoning_effort is None


def test_fallback_anchor_candidates_preserve_preference_order() -> None:
    candidates = _model_profiles.model_candidates("fallback_anchor")
    assert candidates
    assert len(candidates) == len(set(candidates))


def test_profile_shape_mismatch_fails_clearly() -> None:
    with pytest.raises(ValueError, match="defines candidates"):
        _model_profiles.model_profile("fallback_anchor")
    with pytest.raises(ValueError, match="defines one model"):
        _model_profiles.model_candidates("pi_mcp")
    with pytest.raises(KeyError, match="unknown model harness profile"):
        _model_profiles.model_profile("missing")


def test_profile_strings_are_normalized_and_collisions_fail(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    profile_path = tmp_path / "model_profiles.yaml"
    profile_path.write_text(
        json.dumps(
            {
                "version": 1,
                "harnesses": {
                    " demo ": {"model": " model/id ", "reasoning_effort": " medium "},
                    "fallback": {"candidates": [" first ", " second "]},
                },
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setattr(_model_profiles, "MODEL_PROFILES_PATH", profile_path)
    _model_profiles._load_harnesses.cache_clear()
    try:
        assert _model_profiles.model_profile("demo") == _model_profiles.ModelProfile(
            "model/id", "medium"
        )
        assert _model_profiles.model_candidates("fallback") == ("first", "second")

        profile_path.write_text(
            json.dumps(
                {
                    "version": 1,
                    "harnesses": {"demo": {"model": "one"}, " demo ": {"model": "two"}},
                }
            ),
            encoding="utf-8",
        )
        _model_profiles._load_harnesses.cache_clear()
        with pytest.raises(ValueError, match="duplicate normalized harness name"):
            _model_profiles.model_profile("demo")
    finally:
        _model_profiles._load_harnesses.cache_clear()


def test_runtime_scripts_do_not_duplicate_configured_model_ids() -> None:
    single_model_harnesses = (
        "codex_exec",
        "codex_cua",
        "performance_judge",
        "opencode_mcp",
        "pi_mcp",
        "claude_mcp",
    )
    model_ids = {_model_profiles.model_profile(harness).model for harness in single_model_harnesses}
    model_ids.update(_model_profiles.model_candidates("fallback_anchor"))

    scripts_dir = Path(__file__).parent
    for path in (*scripts_dir.rglob("*.py"), *scripts_dir.rglob("*.sh")):
        if path.name.startswith("test_") or "__pycache__" in path.parts:
            continue
        text = path.read_text(encoding="utf-8")
        duplicated = sorted(model for model in model_ids if model in text)
        assert not duplicated, (
            f"{path} duplicates model ids owned by model_profiles.yaml: {duplicated}"
        )
