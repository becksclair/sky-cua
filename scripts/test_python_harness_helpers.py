from __future__ import annotations

from _app_server_harness import build_schema_accept_value
from _plugin_bundle import (
    ensure_apps_feature_disabled,
    ensure_fast_service_tier,
    ensure_plugin_enabled,
    ensure_plugins_feature_enabled,
)


def test_codex_config_helpers_update_existing_sections() -> None:
    config = "\n".join(
        [
            'service_tier = "flex"',
            "",
            "[features]",
            "plugins = false",
            "apps = true",
            "",
            '[plugins."sky-cua@debug"]',
            "enabled = false",
            "",
            "[profiles.default]",
            'service_tier = "flex"',
            "",
        ]
    )

    config = ensure_plugins_feature_enabled(config)
    config = ensure_apps_feature_disabled(config)
    config = ensure_fast_service_tier(config)
    config = ensure_plugin_enabled(config)

    assert 'service_tier = "fast"' in config
    assert "plugins = true" in config
    assert "apps = false" in config
    assert "enabled = true" in config
    assert "[profiles.default]\n" in config
    assert 'profiles.default]\nservice_tier = "flex"' not in config


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
