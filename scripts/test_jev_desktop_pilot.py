"""Offline desktop ranking and readback contracts; no GTK or MCP process required."""

from __future__ import annotations

import runpy
import sys
from pathlib import Path
from types import ModuleType, SimpleNamespace
from typing import Any
from unittest.mock import Mock

import pytest

import jev_desktop_pilot as pilot


def element(index: int = 7, **overrides: Any) -> dict[str, Any]:
    return {
        "element_index": index,
        "backend_ref": f"private-accessibility-ref-{index}",
        "name": "Physical click target",
        "description": "Interaction controls",
        "role": "button",
        "state_flags": ["showing", "enabled"],
        "bounds": {"x": 10, "y": 20, "width": 120, "height": 40},
        **overrides,
    }


def candidate(index: int = 7, **overrides: Any) -> dict[str, Any]:
    return pilot.candidates_for({"elements": [element(index, **overrides)]})[0]


def test_candidates_preserve_observed_identity_and_actionable_metadata() -> None:
    result = candidate()
    assert result == {
        "id": "e7",
        "backend_ref": "private-accessibility-ref-7",
        "element_index": 7,
        "label": "Physical click target",
        "role": "button",
        "context": "Interaction controls",
        "bounds": {"x": 10, "y": 20, "width": 120, "height": 40},
        "disabled": False,
        "visible": True,
    }


@pytest.mark.parametrize(
    "overrides",
    [
        {"state_flags": ["enabled"]},
        {"state_flags": []},
        {"role": "label"},
        {"role": "window"},
        {"role": "panel"},
        {"bounds": None},
        {"bounds": {}},
        {"bounds": {"x": -1, "y": 0, "width": 100, "height": 30}},
        {"bounds": {"x": 0, "y": -1, "width": 100, "height": 30}},
        {"bounds": {"x": 0, "y": 0, "width": 1, "height": 30}},
        {"bounds": {"x": 0, "y": 0, "width": 100, "height": 0}},
    ],
)
def test_candidates_exclude_hidden_nonactionable_and_invalid_bounds(
    overrides: dict[str, Any],
) -> None:
    snapshot = {"elements": [element(1, **overrides), element(9)]}
    assert [c["id"] for c in pilot.candidates_for(snapshot)] == ["e9"]


def test_disabled_candidates_remain_visible_but_are_marked_disabled() -> None:
    result = candidate(state_flags=["showing"], name=None, description=None)
    assert result["disabled"] is True
    assert result["label"] == ""
    assert result["context"] == ""


def test_public_candidates_strip_action_handles_without_mutating_private_candidates() -> None:
    private = [candidate()]
    public = pilot.public_candidates(private)
    assert public == [
        {
            "id": "e7",
            "label": "Physical click target",
            "role": "button",
            "context": "Interaction controls",
            "bounds": {"x": 10, "y": 20, "width": 120, "height": 40},
            "disabled": False,
            "visible": True,
        }
    ]
    assert private[0]["backend_ref"] == "private-accessibility-ref-7"
    assert private[0]["element_index"] == 7


@pytest.mark.parametrize(
    ("candidates", "expected"),
    [
        ([candidate()], "e7"),
        ([candidate(state_flags=["showing"])], None),
        ([candidate(name="physical click target")], None),
        ([candidate(name="Physical click target extra")], None),
        ([candidate(), candidate(8)], None),
        ([candidate(), candidate(8, state_flags=["showing"])], "e7"),
        ([], None),
    ],
)
def test_exact_choice_requires_one_enabled_exact_label(
    candidates: list[dict[str, Any]], expected: str | None
) -> None:
    assert pilot.exact_choice({"exact": "Physical click target"}, candidates) == expected


def test_exact_choice_does_not_infer_an_exact_label_from_intent() -> None:
    assert pilot.exact_choice({"intent": "Physical click target"}, [candidate()]) is None


@pytest.mark.parametrize("case_id", ["absent", "ambiguous"])
@pytest.mark.parametrize(
    ("choice", "changed", "success", "wrong_action"),
    [
        ("abstain", False, True, False),
        ("e7", False, False, True),
        ("abstain", True, False, False),
        ("e7", True, False, True),
    ],
)
def test_negative_cases_require_abstention_and_unchanged_readback(
    case_id: str, choice: str, changed: bool, success: bool, wrong_action: bool
) -> None:
    case = next(case for case in pilot.CASES if case["id"] == case_id)
    result = pilot.score(case, {"clicked": False}, {"clicked": changed}, choice)
    assert result == {
        "success": success,
        "wrong_action": wrong_action,
        "changed_fields": ["clicked"] if changed else [],
    }


@pytest.mark.parametrize(
    ("value", "success"),
    [(75, True), (74.01, True), (75.99, True), (74, False), (76, False), (0, False)],
)
def test_slider_score_requires_value_strictly_within_one_of_target(
    value: float, success: bool
) -> None:
    result = pilot.score(
        {"field": "slider_h_value"}, {"slider_h_value": 20}, {"slider_h_value": value}, "e7"
    )
    assert result == {
        "success": success,
        "wrong_action": False,
        "changed_fields": ["slider_h_value"],
    }


def test_unrelated_state_change_is_wrong_even_when_target_state_succeeds() -> None:
    result = pilot.score(
        {"field": "switch_active"},
        {"switch_active": False, "checkbox_toggled": False, "entry_focused": True},
        {"switch_active": True, "checkbox_toggled": True, "entry_focused": False},
        "e7",
    )
    assert result["success"] is True
    assert result["wrong_action"] is True
    assert set(result["changed_fields"]) == {
        "switch_active",
        "checkbox_toggled",
        "entry_focused",
    }


def test_entry_focus_side_effect_does_not_count_as_wrong_action() -> None:
    result = pilot.score(
        {"field": "clicked"},
        {"clicked": False, "entry_focused": True},
        {"clicked": True, "entry_focused": False},
        "e7",
    )
    assert result["success"] is True
    assert result["wrong_action"] is False


def test_wrong_state_change_does_not_substitute_for_requested_effect() -> None:
    result = pilot.score(
        {"field": "clicked"},
        {"clicked": False, "secondary_clicked": False},
        {"clicked": False, "secondary_clicked": True},
        "e7",
    )
    assert result["success"] is False
    assert result["wrong_action"] is True
    assert result["changed_fields"] == ["secondary_clicked"]


@pytest.mark.parametrize(
    ("field", "overrides", "expected"),
    [
        ("clicked", {}, True),
        ("clicked", {"name": "Secondary click target"}, False),
        ("clicked", {"role": "toggle button"}, False),
        ("checkbox_toggled", {"name": "Enable smoke option", "role": "check box"}, True),
        ("checkbox_toggled", {"name": "Other option", "role": "check box"}, False),
        ("expander_expanded", {"name": "Smoke details (expander)"}, True),
        ("entry_focused", {"role": "text"}, True),
        ("scroll_events", {"role": "scroll pane"}, True),
        ("slider_h_value", {"role": "slider"}, True),
        (
            "slider_h_value",
            {"role": "slider", "bounds": {"x": 10, "y": 20, "width": 40, "height": 120}},
            False,
        ),
        ("switch_active", {"role": "toggle button", "name": ""}, True),
        ("switch_active", {"role": "toggle button", "name": "Other toggle"}, False),
    ],
)
def test_target_correct_uses_observed_control_identity_and_slider_orientation(
    field: str, overrides: dict[str, Any], expected: bool
) -> None:
    assert pilot.target_correct({"field": field}, "e7", [candidate(**overrides)]) is expected


@pytest.mark.parametrize("choice", ["abstain", "invented-target"])
def test_positive_target_requires_an_observed_candidate(choice: str) -> None:
    assert pilot.target_correct({"field": "clicked"}, choice, [candidate()]) is False


@pytest.mark.parametrize(("choice", "expected"), [("abstain", True), ("e7", False)])
def test_negative_target_requires_abstention(choice: str, expected: bool) -> None:
    assert pilot.target_correct({"field": None}, choice, [candidate()]) is expected


def test_blind_fixture_status_does_not_expose_state_or_call_debug_oracle(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    base_update = Mock(side_effect=AssertionError("Debug oracle must never render"))

    class FakePointerWindow:
        update_status = base_update

    fixture_module = ModuleType("gtk_pointer_smoke_fixture")
    vars(fixture_module).update(
        Gtk=Mock(), PointerSmokeWindow=FakePointerWindow, install_dark_theme=Mock()
    )
    monkeypatch.setitem(sys.modules, "gtk_pointer_smoke_fixture", fixture_module)
    namespace = runpy.run_path(str(Path(pilot.__file__).with_name("jev_desktop_fixture.py")))
    status = Mock()
    window = SimpleNamespace(status=status, state={"clicked": True, "points": 999})
    namespace["BlindPointerWindow"].update_status(window)
    status.set_text.assert_called_once_with("Desktop interaction test")
    base_update.assert_not_called()
    fixture_module.Gtk.main.assert_not_called()
