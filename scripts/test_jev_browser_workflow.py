"""Offline checks for observed browser targets and independent TodoMVC outcomes."""

from __future__ import annotations

import json
from typing import Any

import pytest

from _jev_browser_workflow import (
    candidates_for,
    page_text_for,
    paired_summary,
    public_candidates,
    state_proof,
    target_correct,
    workflow_proof,
)


def element(index: int = 0, **extra: Any) -> dict[str, Any]:
    return {
        "index": index,
        "ref": f"opaque-dom-ref-{index}",
        "name": "New Todo Input",
        "tag": "input",
        "role": None,
        "href": None,
        "bounds": {"x": -5, "y": 10, "width": 200, "height": 40},
        "disabled": False,
        **extra,
    }


def test_projection_uses_only_observed_semantics_and_bounds() -> None:
    snapshot = {
        "text": "Actual observed task label\n" + "x" * 5000,
        "elements": [
            element(),
            element(1, name=None, bounds={"width": 0, "height": 30}),
            element(2, role="checkbox", disabled=True),
            element(3, name="Active", tag="a", href="http://localhost:8000/#/active"),
        ],
    }
    candidates = candidates_for(snapshot)
    assert [candidate["id"] for candidate in candidates] == ["c0", "c1", "c2", "c3"]
    assert candidates[0]["role"] == "input"  # No synthetic textbox ARIA role.
    assert candidates[0]["bounds"]["x"] == -5
    assert candidates[0]["visible"] is True
    assert candidates[1]["label"] == ""
    assert candidates[1]["visible"] is False
    assert candidates[2]["role"] == "checkbox"
    assert candidates[2]["disabled"] is True
    # Shared page text is sent once in the payload state, not per candidate.
    assert candidates[3]["context"] == "href: http://localhost:8000/#/active"
    assert candidates[0]["context"] == ""
    assert page_text_for(snapshot) == snapshot["text"][:4000]
    assert candidates[0]["ref"] == "opaque-dom-ref-0"
    candidates[0]["oracle"] = "private test-only metadata"
    public = public_candidates(candidates)
    assert set(public[0]) == {"id", "label", "role", "context", "bounds", "disabled", "visible"}
    assert all(
        not {"ref", "index", "tag", "href", "oracle"} & candidate.keys() for candidate in public
    )
    assert not any("parent" in key for candidate in candidates for key in candidate)


def test_target_oracles_require_actual_control_label_tag_and_availability() -> None:
    candidates = candidates_for(
        {
            "elements": [
                element(),
                element(1, name="Confirmed completion label"),
                element(2, name="Active", tag="a", href="http://localhost/#/active"),
            ]
        }
    )
    for stage, candidate in zip(("add", "complete", "filter_active"), candidates, strict=True):
        options: dict[str, Any] = {"complete_label": "Confirmed completion label"}
        assert target_correct(stage, candidate, **options)
        assert not target_correct(stage, {**candidate, "disabled": True}, **options)
        assert not target_correct(stage, {**candidate, "visible": False}, **options)
        assert not target_correct(stage, {**candidate, "tag": "button"}, **options)
        assert not target_correct(stage, {**candidate, "label": "Unrelated"}, **options)
        assert not target_correct(stage, None, **options)
    assert not target_correct("complete", candidates[1])
    assert not target_correct("complete", candidates[1], complete_label="Toggle Todo")
    assert not target_correct("filter_active", {**candidates[2], "href": "#/completed"})
    assert not target_correct("filter_active", {**candidates[2], "href": None})
    with pytest.raises(ValueError, match="Unknown"):
        target_correct("invented-stage", candidates[0])


def test_unnamed_completion_oracle_distinguishes_individual_from_bulk_toggle() -> None:
    individual_bounds = {"x": 234, "y": 205.0625, "width": 40, "height": 40}
    candidates = candidates_for(
        {
            "elements": [
                element(1, name=None, bounds={"x": 238, "y": 132.667, "width": 40, "height": 60}),
                element(2, name=None, bounds=individual_bounds),
            ]
        }
    )
    matches = [
        candidate
        for candidate in candidates
        if target_correct("complete", candidate, complete_bounds=individual_bounds)
    ]
    assert [candidate["id"] for candidate in matches] == ["c2"]
    assert all(
        candidate["label"] == "" and candidate["role"] == "input" for candidate in candidates
    )
    assert not target_correct("complete", candidates[1])
    assert target_correct(
        "complete",
        {**candidates[1], "bounds": {**individual_bounds, "x": 235}},
        complete_bounds=individual_bounds,
    )
    assert not target_correct(
        "complete",
        {**candidates[1], "bounds": {**individual_bounds, "x": 235.01}},
        complete_bounds=individual_bounds,
    )
    for invalid_bounds in (
        {},
        {**individual_bounds, "width": 0},
        {**individual_bounds, "height": -1},
        {**individual_bounds, "x": float("nan")},
        {**individual_bounds, "y": True},
    ):
        assert not target_correct("complete", candidates[1], complete_bounds=invalid_bounds)
    assert "complete_bounds" not in public_candidates(candidates)[1]


def task(title: str, completed: bool) -> dict[str, Any]:
    return {"id": title, "title": title, "completed": completed}


def prove(tasks: list[Any], **page_updates: Any) -> dict[str, Any]:
    return state_proof(
        {"arbitrary-real-app-key": json.dumps(tasks), "unrelated-setting": "not JSON"},
        {"hash": "#/active", "visible_tasks": ["B"], **page_updates},
        "A",
        "B",
    )


def test_workflow_proof_verifies_dom_state_without_claiming_persistence() -> None:
    observed = [task("B", False), task("A", True)]
    proof = workflow_proof(observed, {"hash": "#/active", "visible_tasks": ["B"]}, "A", "B")
    assert proof["success"]
    assert proof["tasks_before_filter"] == observed
    assert proof["tasks_match"] and proof["hash_matches"] and proof["visible_tasks_match"]
    assert not any("storage" in key or "persist" in key for key in proof)


@pytest.mark.parametrize(
    "tasks",
    [
        [task("B", False)],
        [task("A", False), task("B", False)],
        [task("A", True), task("B", True)],
        [task("A", True), task("B", False), task("Unexpected", False)],
        [task("A", True), {"title": "B", "completed": 0}],
    ],
)
def test_workflow_proof_rejects_missing_extra_or_wrong_dom_tasks(
    tasks: list[dict[str, Any]],
) -> None:
    proof = workflow_proof(tasks, {"hash": "#/active", "visible_tasks": ["B"]}, "A", "B")
    assert not proof["tasks_match"]
    assert not proof["success"]


@pytest.mark.parametrize(
    "page",
    [
        {"hash": "#/", "visible_tasks": ["B"]},
        {"hash": "#/active", "visible_tasks": ["A", "B"]},
        {"hash": "#/active", "visible_tasks": []},
    ],
)
def test_workflow_proof_requires_final_hash_and_visible_tasks(page: dict[str, Any]) -> None:
    proof = workflow_proof([task("A", True), task("B", False)], page, "A", "B")
    assert proof["tasks_match"]
    assert not proof["success"]


def test_state_proof_uses_persisted_titles_flags_and_actual_active_view() -> None:
    proof = prove([task("B", False), task("A", True)])
    assert proof["success"] is True
    assert proof["storage_keys"] == ["arbitrary-real-app-key"]
    assert proof["storage_matches"] and proof["hash_matches"] and proof["visible_tasks_match"]


@pytest.mark.parametrize(
    "tasks",
    [
        [],
        [task("A", False), task("B", False)],
        [task("A", True), task("B", True)],
        [task("A", True), task("B", False), task("Unknown extra task", False)],
        [task("A", True), task("A", True)],
        [task("A", True), {"title": "B", "completed": 0}],
        [task("A", True), {"title": "B"}],
        [task("A", True), "broken task"],
    ],
)
def test_state_proof_rejects_extra_incorrect_duplicate_or_malformed_tasks(tasks: list[Any]) -> None:
    proof = prove(tasks)
    assert not proof["storage_matches"]
    assert not proof["success"]


@pytest.mark.parametrize(
    "page", [{"hash": "#/"}, {"visible_tasks": ["A", "B"]}, {"visible_tasks": []}]
)
def test_persistence_alone_cannot_pass_with_wrong_visible_view(page: dict[str, Any]) -> None:
    proof = prove([task("A", True), task("B", False)], **page)
    assert proof["storage_matches"]
    assert not proof["success"]


def test_state_proof_does_not_hide_tasks_in_another_storage_array() -> None:
    proof = state_proof(
        {
            "todos-one": json.dumps([task("A", True), task("B", False)]),
            "todos-two": json.dumps([task("Extra", False)]),
            "unrelated-array": json.dumps(["theme", "dark"]),
        },
        {"hash": "#/active", "visible_tasks": ["B"]},
        "A",
        "B",
    )
    assert proof["storage_keys"] == ["todos-one", "todos-two"]
    assert len(proof["tasks"]) == 3
    assert not proof["success"]


def records() -> list[dict[str, Any]]:
    return [
        {
            "pair": pair,
            "route": route,
            "total_ms": 100 if route == "baseline" else 80,
            "success": True,
            "status": "ok",
            "wrong_targets": 0,
            "terra_calls": 4 if route == "baseline" else 1,
            "jev_calls": 0 if route == "baseline" else 4,
        }
        for pair in range(3)
        for route in ("baseline", "hybrid")
    ]


def test_pair_gate_accepts_exact_twenty_percent_and_counts_actual_calls() -> None:
    summary = paired_summary(records())
    assert summary["passed"]
    assert summary["correct_tasks"] == summary["total_tasks"] == 6
    assert summary["all_task_proofs_correct"]
    assert summary["savings_fraction"] == pytest.approx(0.2)
    assert summary["baseline_total_ms"] == 300
    assert summary["hybrid_total_ms"] == 240
    assert summary["terra_calls"] == 15
    assert summary["jev_calls"] == 12
    assert summary["calls_by_route"]["hybrid"] == {
        "terra_calls": 3,
        "jev_calls": 12,
        "wrong_targets": 0,
    }


@pytest.mark.parametrize(
    "change",
    [
        {"wrong_targets": 1},
        {"success": False},
        {"status": "error"},
        {"total_ms": 80.001},
    ],
)
def test_pair_gate_requires_every_outcome_and_total_savings(change: dict[str, Any]) -> None:
    measured = records()
    measured[1].update(change)
    assert not paired_summary(measured)["passed"]


def test_pair_gate_uses_total_wall_time_instead_of_mean_pair_percentages() -> None:
    measured = records()
    measured[0]["total_ms"], measured[1]["total_ms"] = 1000, 1000
    measured[2]["total_ms"], measured[3]["total_ms"] = 100, 10
    measured[4]["total_ms"], measured[5]["total_ms"] = 100, 10
    summary = paired_summary(measured)
    assert summary["savings_fraction"] == pytest.approx(0.15)
    assert not summary["passed"]


@pytest.mark.parametrize("value", [0, -1, float("nan"), float("inf"), True])
def test_pair_gate_rejects_invalid_timing(value: Any) -> None:
    measured = records()
    measured[0]["total_ms"] = value
    with pytest.raises(ValueError, match="timing"):
        paired_summary(measured)


def test_pair_gate_rejects_missing_pairs_duplicate_routes_and_invalid_counts() -> None:
    with pytest.raises(ValueError, match="count"):
        paired_summary(records()[:-1])
    duplicate = records()
    duplicate[1]["route"] = "baseline"
    with pytest.raises(ValueError, match="pair"):
        paired_summary(duplicate)
    invalid = records()
    invalid[0]["terra_calls"] = True
    with pytest.raises(ValueError, match="counts"):
        paired_summary(invalid)
