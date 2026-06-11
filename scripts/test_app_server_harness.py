"""Tests for rich app-server harness helpers."""

from __future__ import annotations

from _app_server_harness import build_schema_accept_value, response_contains_computer_use_server


def test_build_schema_accept_value_prefers_required_fields_and_enums() -> None:
    value = build_schema_accept_value(
        {
            "type": "object",
            "required": ["decision", "count", "flags"],
            "properties": {
                "decision": {"type": "string", "enum": ["accept", "decline"]},
                "count": {"type": "integer"},
                "flags": {
                    "type": "array",
                    "minItems": 2,
                    "items": {"type": "boolean"},
                },
                "optional": {"type": "string"},
            },
        }
    )

    assert value == {
        "decision": "accept",
        "count": 1,
        "flags": [True, True],
    }


def test_response_contains_computer_use_server_accepts_common_shapes() -> None:
    assert response_contains_computer_use_server(
        {"result": {"servers": [{"name": "computer-use"}]}}
    )
    assert response_contains_computer_use_server({"result": {"data": [{"name": "computer-use"}]}})
    assert response_contains_computer_use_server(
        {"result": {"items": [{"server": "computer-use"}]}}
    )
    assert not response_contains_computer_use_server({"result": {"servers": []}})
