"""Offline decision-route contracts: no browser, credentials, or provider calls."""

from __future__ import annotations

import json
from collections.abc import Iterator
from pathlib import Path
from typing import Any
from unittest.mock import Mock

import pytest

import jev_browser_pilot as pilot


def candidate(
    candidate_id: str = "observed-1",
    *,
    label: str = "Save",
    disabled: bool = False,
    visible: bool = True,
) -> dict[str, Any]:
    return {
        "id": candidate_id,
        "label": label,
        "context": "Document editor",
        "role": "button",
        "disabled": disabled,
        "visible": visible,
    }


@pytest.fixture
def decision_harness(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> Iterator[tuple[pilot.Decisions, Mock, Mock]]:
    client = Mock(last_elapsed_ms=12.0)
    monkeypatch.setattr(pilot, "jev_client", Mock(return_value=client))
    monkeypatch.setattr(pilot, "prepare_judge_codex_home", Mock(return_value=tmp_path / "home"))
    monkeypatch.setattr(
        pilot,
        "run_codex_exec",
        Mock(side_effect=AssertionError("Offline test attempted a live Luna request")),
    )
    manifest = {
        "cases": [
            {
                "id": "editor-01",
                "intent": "Save the document",
                "exact_name": "Save",
                "expected_key": "private-oracle-key",
                "controls": [{"key": "private-control-key"}],
            }
        ],
        "threshold": 0.7,
    }
    decisions = pilot.Decisions(tmp_path, manifest)
    luna = Mock(return_value={"choice": "observed-2", "elapsed_ms": 30.0, "usage": []})
    monkeypatch.setattr(decisions, "luna", luna)
    yield decisions, client, luna
    decisions.close()


def request(route: str = "hybrid", *, exact_match: bool = False) -> dict[str, Any]:
    return {
        "case_id": "editor-01",
        "route": route,
        "observation": {
            "candidates": [
                candidate(label="Save" if exact_match else "Write changes"),
                candidate("observed-2", label="Cancel"),
            ]
        },
    }


@pytest.mark.parametrize(
    ("candidates", "expected"),
    [
        ([candidate()], "observed-1"),
        ([candidate(disabled=True)], None),
        ([candidate(visible=False)], None),
        ([candidate(label="Save as")], None),
        ([candidate(label="save")], None),
        ([candidate(), candidate("observed-2")], None),
        ([candidate(), candidate("observed-2", disabled=True)], "observed-1"),
        ([candidate(), candidate("observed-2", visible=False)], "observed-1"),
        ([], None),
    ],
)
def test_exact_rule_requires_one_enabled_visible_exact_label(
    candidates: list[dict[str, Any]], expected: str | None
) -> None:
    assert pilot.exact_rule({"exact_name": "Save"}, {"candidates": candidates}) == expected


def test_case_without_exact_name_does_not_match_by_intent() -> None:
    assert pilot.exact_rule({"intent": "Save"}, {"candidates": [candidate()]}) is None


@pytest.mark.parametrize("route", ["baseline", "hybrid"])
def test_exact_match_skips_both_models(
    decision_harness: tuple[pilot.Decisions, Mock, Mock], route: str
) -> None:
    decisions, client, luna = decision_harness
    result = decisions.decide(request(route, exact_match=True))
    assert result["status"] == "ok"
    assert result["source"] == "rules"
    assert result["choice"] == "observed-1"
    assert result["jev"] is None and result["luna"] is None
    client.call.assert_not_called()
    luna.assert_not_called()


def test_baseline_without_rule_match_goes_directly_to_luna(
    decision_harness: tuple[pilot.Decisions, Mock, Mock],
) -> None:
    decisions, client, luna = decision_harness
    incoming = request("baseline")
    result = decisions.decide(incoming)
    assert result["status"] == "ok"
    assert result["source"] == "luna"
    assert result["choice"] == "observed-2"
    assert result["jev"] is None
    client.call.assert_not_called()
    luna.assert_called_once_with(
        pilot.payload_for("Save the document", incoming["observation"]),
        decisions.out / "decisions/editor-01-baseline/luna",
    )


@pytest.mark.parametrize("probability", [0.7, 0.95])
def test_hybrid_accepts_observed_non_abstain_at_or_above_threshold(
    decision_harness: tuple[pilot.Decisions, Mock, Mock], probability: float
) -> None:
    decisions, client, luna = decision_harness
    response = {
        "answers": {
            "target": {"choice": "observed-1", "probabilities": {"observed-1": probability}}
        }
    }
    client.call.return_value = response
    result = decisions.decide(request())
    assert result["status"] == "ok"
    assert result["source"] == "jev"
    assert result["choice"] == "observed-1"
    assert result["jev"] == {"response": response, "elapsed_ms": 12.0}
    assert result["luna"] is None
    client.call.assert_called_once()
    luna.assert_not_called()


@pytest.mark.parametrize(
    ("choice", "probability"),
    [("observed-1", 0.699999), ("abstain", 0.99), ("invented-target", 0.99)],
)
def test_hybrid_rejected_choice_falls_back_to_luna(
    decision_harness: tuple[pilot.Decisions, Mock, Mock], choice: str, probability: float
) -> None:
    decisions, client, luna = decision_harness
    response = {"answers": {"target": {"choice": choice, "probabilities": {choice: probability}}}}
    client.call.return_value = response
    result = decisions.decide(request())
    assert result["status"] == "ok"
    assert result["source"] == "luna"
    assert result["choice"] == "observed-2"
    assert result["jev"]["response"] == response
    assert result["luna"] == luna.return_value
    client.call.assert_called_once()
    luna.assert_called_once()
    assert luna.call_args.args[0] == client.call.call_args.args[0]


def test_saved_and_submitted_request_exclude_case_private_ground_truth(
    decision_harness: tuple[pilot.Decisions, Mock, Mock],
) -> None:
    decisions, client, luna = decision_harness
    client.call.return_value = {
        "answers": {"target": {"choice": "abstain", "probabilities": {"abstain": 1.0}}}
    }
    incoming = request()
    decisions.decide(incoming)
    raw = (decisions.out / "decisions/editor-01-hybrid/request.json").read_text()
    saved = json.loads(raw)
    assert set(saved) == {"model", "state", "questions"}
    assert saved["state"] == {
        "intent": "Save the document",
        "observation": incoming["observation"],
    }
    assert set(saved["questions"]["target"]["criteria"]) == {
        "observed-1",
        "observed-2",
        "abstain",
    }
    for private in ('"expected_key"', "private-oracle-key", '"controls"', "private-control-key"):
        assert private not in raw
    assert client.call.call_args.args[0] == saved
    assert luna.call_args.args[0] == saved


def test_duplicate_inference_is_rejected_before_any_more_model_calls(
    decision_harness: tuple[pilot.Decisions, Mock, Mock],
) -> None:
    decisions, client, luna = decision_harness
    incoming = request("baseline")
    decisions.decide(incoming)
    folder = decisions.out / "decisions/editor-01-baseline"
    before = {path.name: path.read_bytes() for path in folder.iterdir()}
    with pytest.raises(FileExistsError, match="Refusing repeated inference"):
        decisions.decide(incoming)
    assert {path.name: path.read_bytes() for path in folder.iterdir()} == before
    client.call.assert_not_called()
    luna.assert_called_once()
