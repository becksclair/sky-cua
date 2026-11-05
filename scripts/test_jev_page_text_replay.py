"""Offline checks for the page-text layout replay; no model calls."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from jev_page_text_replay import observations, payload_for, split_legacy, summarize


def legacy(cid: str, text: str, href: str | None = None) -> dict[str, Any]:
    context = f"href: {href}\npage text: {text}" if href else f"page text: {text}"
    return {
        "id": cid,
        "ref": f"ref-{cid}",
        "label": cid,
        "role": "a" if href else "input",
        "context": context,
        "bounds": {"x": 0, "y": 0, "width": 10, "height": 10},
        "disabled": False,
        "visible": True,
    }


def test_split_legacy_moves_page_text_out_of_every_context() -> None:
    shared, text = split_legacy(
        [legacy("c0", "todos\nActive"), legacy("c1", "todos\nActive", "http://x/#/active")]
    )
    assert text == "todos\nActive"
    assert [c["context"] for c in shared] == ["", "href: http://x/#/active"]
    with pytest.raises(ValueError, match="disagree"):
        split_legacy([legacy("c0", "a"), legacy("c1", "b")])
    with pytest.raises(ValueError, match="lacks"):
        split_legacy([{**legacy("c0", "a"), "context": ""}])


def test_observations_require_verified_targets_and_build_both_layouts(tmp_path: Path) -> None:
    run = tmp_path / "pair-0-hybrid"
    (run / "add-first").mkdir(parents=True)
    text = "page " * 40
    (run / "add-first/candidates.json").write_text(
        json.dumps([legacy("c0", text), legacy("c1", text)])
    )
    step = {"stage": "add", "intent": "Add", "choice": "c0", "target_correct": True}
    (run / "result.json").write_text(json.dumps({"steps": [step]}))
    (tmp_path / "warmup-hybrid").mkdir()  # Warm-ups are excluded by name.
    [observation] = observations(tmp_path)
    assert observation["name"] == "pair-0-hybrid/add-first"
    old, new = payload_for(observation, "legacy"), payload_for(observation, "shared")
    assert json.dumps(old).count(text) == 4  # two candidates, state plus criteria
    assert json.dumps(new).count(text) == 1
    assert "ref-c0" not in json.dumps(new)
    (run / "result.json").write_text(json.dumps({"steps": [{**step, "target_correct": False}]}))
    with pytest.raises(ValueError, match="verified"):
        observations(tmp_path)


def record(layout: str, choice: str, probability: float, stage: str = "add") -> dict[str, Any]:
    return {
        "layout": layout,
        "stage": stage,
        "correct": "c0",
        "choice": choice,
        "probability": probability,
        "accepted": choice != "abstain" and probability >= 0.8,
        "request_bytes": 100 if layout == "shared" else 400,
        "input_tokens": 50 if layout == "shared" else 200,
        "elapsed_ms": 5.0,
    }


def test_summary_gate_rejects_wrong_acceptance_and_lost_coverage() -> None:
    base = [record("legacy", "c0", 0.9), record("legacy", "c0", 0.9)]
    summary = summarize([*base, record("shared", "c0", 0.95), record("shared", "c1", 0.5)])
    assert summary["passed"] is True
    assert summary["shared"]["overall"]["accepted_correct"] == 1
    assert summary["shared"]["overall"]["raw_correct"] == 1
    assert summary["shared"]["overall"]["mean_input_tokens"] == 50
    assert summarize([*base, record("shared", "c1", 0.9)])["passed"] is False
    lost = [*base, record("legacy", "c0", 0.9), record("shared", "c0", 0.5)]
    assert summarize(lost)["passed"] is False
