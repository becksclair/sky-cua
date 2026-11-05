"""Pure loopback tests: no browser, model, API key, or desktop dependency."""

from __future__ import annotations

import json
from collections import Counter
from html.parser import HTMLParser
from typing import Any
from urllib.error import HTTPError
from urllib.request import Request, urlopen

import pytest

from _jev_browser_fixture import Fixture, build_cases


class Buttons(HTMLParser):
    def __init__(self, page: str) -> None:
        super().__init__()
        self.buttons: list[dict[str, str | None]] = []
        self.feed(page)

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag == "button":
            self.buttons.append(dict(attrs))


def get(fixture: Fixture, path: str) -> str:
    with urlopen(fixture.origin + path, timeout=3) as response:
        assert response.headers["Cache-Control"] == "no-store"
        return response.read().decode()


def state(fixture: Fixture, run: str) -> dict[str, Any]:
    return json.loads(get(fixture, "/state/" + run))


def post(fixture: Fixture, run: str, token: str) -> dict[str, Any]:
    request = Request(
        fixture.origin + "/action",
        data=json.dumps({"run": run, "token": token}).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urlopen(request, timeout=3) as response:
        return json.loads(response.read())


def test_twenty_frozen_cases_are_balanced_and_disjoint() -> None:
    cases = build_cases()
    assert cases == build_cases()
    assert len(cases) == len({c["id"] for c in cases}) == 20
    assert len({c["intent"] for c in cases}) == 20
    assert Counter(c["family"] for c in cases) == {
        "editor": 5,
        "notification setup": 5,
        "record management": 5,
        "file workflow": 5,
    }
    for family in {c["family"] for c in cases}:
        assert Counter(c["kind"] for c in cases if c["family"] == family) == {
            "exact": 1,
            "semantic": 3,
            "negative": 1,
        }
    keys = [control["key"] for c in cases for control in c["controls"]]
    assert len(keys) == len(set(keys))
    for case in cases:
        assert len(case["controls"]) >= 3
        assert ("exact_name" in case) == (case["kind"] == "exact")
        if case["kind"] == "negative":
            assert case["expected_key"] is None
        else:
            selected = [c for c in case["controls"] if c["key"] == case["expected_key"]]
            assert len(selected) == 1
            assert not selected[0].get("disabled", False)
    cases[0]["controls"][0]["label"] = "Changed locally"
    assert build_cases()[0]["controls"][0]["label"] != "Changed locally"


def test_all_pages_have_only_visible_context_and_opaque_controls() -> None:
    with Fixture() as fixture:
        assert json.loads(get(fixture, "/health")) == {"ok": True}
        for case in fixture.cases:
            page = get(fixture, f"/case/{case['id']}?run=inspect-{case['id']}")
            buttons = Buttons(page).buttons
            assert len(buttons) == len(case["controls"])
            assert len({b["data-ref"] for b in buttons}) == len(buttons)
            assert all(set(b) <= {"data-ref", "disabled"} for b in buttons)
            for control, button in zip(case["controls"], buttons, strict=True):
                assert control["key"] not in page
                assert control["label"] not in str(button["data-ref"])
                assert ("disabled" in button) == control.get("disabled", False)
            for private in ("expected", "correct", "target", "exact_name", "semantic", "negative"):
                assert private not in page
            assert "<legend>" in page
            assert "Action recorded." in page


def test_actions_record_actual_tokens_and_reset_per_run() -> None:
    with Fixture() as fixture:
        case = fixture.cases[0]
        page = get(fixture, f"/case/{case['id']}?run=first")
        buttons = Buttons(page).buttons
        assert state(fixture, "first")["outcome"] == "pending"
        # Deliberately select a wrong control: server must never substitute the oracle.
        assert post(fixture, "first", str(buttons[0]["data-ref"])) == {"ok": True}
        result = state(fixture, "first")
        assert result["clicked_keys"] == [case["controls"][0]["key"]]
        assert result["outcome"] == "mismatched"
        assert result["acknowledged"] is True
        get(fixture, f"/case/{case['id']}?run=first")
        assert state(fixture, "first")["clicked_keys"] == []
        post(fixture, "first", str(buttons[1]["data-ref"]))
        assert state(fixture, "first")["outcome"] == "matched"
        # Repeated clicks remain visible; success requires exactly the intended action.
        post(fixture, "first", str(buttons[1]["data-ref"]))
        assert len(state(fixture, "first")["clicked_keys"]) == 2
        assert state(fixture, "first")["outcome"] == "mismatched"


def test_tokens_are_stable_per_run_but_do_not_transfer_between_runs() -> None:
    with Fixture() as fixture:
        path = "/case/editor-02?run="
        first = Buttons(get(fixture, path + "one")).buttons
        assert first == Buttons(get(fixture, path + "one")).buttons
        second = Buttons(get(fixture, path + "two")).buttons
        assert {b["data-ref"] for b in first}.isdisjoint(b["data-ref"] for b in second)
        with pytest.raises(HTTPError) as error:
            post(fixture, "two", str(first[0]["data-ref"]))
        assert error.value.code == 409
        assert state(fixture, "two")["clicked_keys"] == []
        assert state(fixture, "one")["clicked_keys"] == []


def test_disabled_controls_reject_even_direct_post_and_negative_requires_no_click() -> None:
    with Fixture() as fixture:
        case = next(c for c in fixture.cases if any(b.get("disabled") for b in c["controls"]))
        page = get(fixture, f"/case/{case['id']}?run=disabled")
        buttons = Buttons(page).buttons
        disabled = next(b for b in buttons if "disabled" in b)
        assert state(fixture, "disabled")["outcome"] == "abstained"
        with pytest.raises(HTTPError) as error:
            post(fixture, "disabled", str(disabled["data-ref"]))
        assert error.value.code == 409
        assert state(fixture, "disabled")["clicked_keys"] == []
        enabled = next(b for b in buttons if "disabled" not in b)
        post(fixture, "disabled", str(enabled["data-ref"]))
        assert state(fixture, "disabled")["outcome"] == "mismatched"


def test_unknown_routes_runs_and_tokens_do_not_create_actions() -> None:
    with Fixture() as fixture:
        for path in ("/unknown", "/case/unknown?run=x", "/case/editor-01", "/state/missing"):
            with pytest.raises(HTTPError) as error:
                get(fixture, path)
            assert error.value.code == 404
        with pytest.raises(HTTPError) as error:
            post(fixture, "missing", "invented")
        assert error.value.code == 404
        get(fixture, "/case/editor-01?run=known")
        with pytest.raises(HTTPError) as error:
            post(fixture, "known", "invented")
        assert error.value.code == 409
        assert state(fixture, "known")["clicked_keys"] == []
    fixture.close()  # idempotent after context manager exit
