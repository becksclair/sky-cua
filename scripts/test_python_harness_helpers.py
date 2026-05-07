from __future__ import annotations

from _app_server_harness import build_schema_accept_value, response_contains_computer_use_server
from _plugin_bundle import (
    ensure_apps_feature_disabled,
    ensure_fast_service_tier,
    ensure_plugin_enabled,
    ensure_plugins_feature_enabled,
    executable_name,
    runtime_binary_names,
)
from _tidal_workflow import tidal_playlist_prompt
from live_app_server_tidal_image_ab import DEFAULT_VARIANTS, playlist_name_for_variant


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


def test_runtime_binary_names_match_host_platform() -> None:
    suffix = ".exe" if executable_name("tool").endswith(".exe") else ""

    assert runtime_binary_names() == [
        f"sky-cua-client{suffix}",
        f"sky-cua-service{suffix}",
    ]


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


def test_tidal_prompt_uses_custom_playlist_name() -> None:
    prompt = tidal_playlist_prompt(
        app_server=True, playlist_name="Codex Favorites AB test webp-q85"
    )

    assert "Codex Favorites AB test webp-q85" in prompt


def test_tidal_ab_playlist_names_are_variant_scoped() -> None:
    names = [playlist_name_for_variant("20260424T120000Z", variant) for variant in DEFAULT_VARIANTS]

    assert len(names) == len(set(names))
    assert all(name.startswith("Codex Favorites AB 20260424T120000Z ") for name in names)
