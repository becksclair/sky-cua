from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Any

import pytest

from _opencode_config import (
    OPENCODE_MANAGED_SERVERS,
    _build_opencode_servers,
    _merge_opencode_servers,
    install_opencode_config,
)


def test_native_server_definitions_use_phase_timeouts_and_fixed_root(tmp_path: Path) -> None:
    install_root = tmp_path / "installed"
    servers = _build_opencode_servers(
        install_root,
        timeout_ms=45_000,
        env={
            "SKY_CUA_BROWSER_CONTROL_MODE": " cdp ",
            "XDG_RUNTIME_DIR": "/old/session",
            "DISPLAY": ":99",
        },
    )
    for name, executable, arguments in (
        ("sky_cua", "sky-cua-client", ["mcp"]),
        ("node_repl", "node_repl", []),
    ):
        assert servers[name] == {
            "type": "local",
            "command": [str(install_root / "bin" / executable), *arguments],
            "cwd": str(install_root),
            "environment": {"SKY_CUA_BROWSER_CONTROL_MODE": "cdp"},
            "disabled": False,
            "timeout": {"startup": 45_000, "catalog": 45_000, "execution": 45_000},
        }


def test_native_merge_removes_managed_legacy_conflicts_and_preserves_other_config(
    tmp_path: Path,
) -> None:
    existing: dict[str, Any] = {
        "model": "provider/model",
        "provider": {"private": {"options": {"baseURL": "https://example.test"}}},
        "mcp": {
            "timeout": {"execution": 90_000},
            "sky_cua": {"command": ["legacy-sky"], "timeout": 999_999},
            "node_repl": {"command": ["legacy-node"]},
            "legacy_other": {"type": "local", "command": ["keep-legacy"]},
            "servers": {
                "sky_cua": {
                    "type": "local",
                    "command": ["stale-native-sky"],
                    "cwd": "/stale",
                    "environment": {"DISPLAY": ":99"},
                    "disabled": True,
                    "timeout": {"execution": 600_000},
                    "codemode": False,
                    "protocol": "auto",
                },
                "node_repl": {"command": ["stale-native-node"]},
                "other": {"type": "remote", "url": "https://example.test/mcp"},
            },
        },
    }
    original = copy.deepcopy(existing)
    definitions = _build_opencode_servers(tmp_path, env={})
    merged = _merge_opencode_servers(existing, definitions)
    assert existing == original
    assert merged["model"] == original["model"]
    assert merged["provider"] == original["provider"]
    mcp = merged["mcp"]
    assert isinstance(mcp, dict)
    assert set(mcp) == {"timeout", "legacy_other", "servers"}
    assert mcp["timeout"] == original["mcp"]["timeout"]
    assert mcp["legacy_other"] == original["mcp"]["legacy_other"]
    assert mcp["servers"]["other"] == original["mcp"]["servers"]["other"]
    sky = mcp["servers"]["sky_cua"]
    sky_definition = definitions["sky_cua"]
    assert isinstance(sky_definition, dict)
    assert sky == {
        **sky_definition,
        "timeout": {"startup": 30_000, "catalog": 30_000, "execution": 600_000},
        "codemode": False,
        "protocol": "auto",
    }
    assert mcp["servers"]["node_repl"] == definitions["node_repl"]


@pytest.mark.parametrize("name", OPENCODE_MANAGED_SERVERS)
def test_native_timeout_preserves_only_valid_phase_values(tmp_path: Path, name: str) -> None:
    definitions = _build_opencode_servers(tmp_path, timeout_ms=50_000, env={})
    merged = _merge_opencode_servers(
        {
            "mcp": {
                "servers": {
                    name: {
                        "timeout": {
                            "startup": 1_000,
                            "catalog": 120_000,
                            "execution": 600_000,
                            "unknown": 10,
                        },
                    },
                },
            },
        },
        definitions,
    )
    mcp = merged["mcp"]
    assert isinstance(mcp, dict)
    assert mcp["servers"][name]["timeout"] == {
        "startup": 1_000,
        "catalog": 120_000,
        "execution": 600_000,
    }


@pytest.mark.parametrize(
    "invalid",
    [
        None,
        30_000,
        {"execution": True},
        {"execution": 0},
        {"execution": -1},
        {"execution": 1.5},
        {"execution": "600000"},
    ],
)
def test_invalid_native_timeout_values_use_defaults(tmp_path: Path, invalid: object) -> None:
    definitions = _build_opencode_servers(tmp_path, env={})
    merged = _merge_opencode_servers(
        {"mcp": {"servers": {"sky_cua": {"timeout": invalid}}}}, definitions
    )
    mcp = merged["mcp"]
    assert isinstance(mcp, dict)
    assert mcp["servers"]["sky_cua"]["timeout"] == {
        "startup": 30_000,
        "catalog": 30_000,
        "execution": 30_000,
    }


def test_native_install_migrates_with_backup_and_is_idempotent(tmp_path: Path) -> None:
    config_path = tmp_path / ".config/opencode/opencode.jsonc"
    config_path.parent.mkdir(parents=True)
    original = b'{\n// old config\n"mcp": {"sky_cua": {"enabled": true}}\n}\n'
    config_path.write_bytes(original)
    config_path.chmod(0o640)
    install_root = tmp_path / "runtime"
    report = install_opencode_config(install_root, home=tmp_path, env={})
    assert report["status"] == "updated"
    assert Path(str(report["backup_path"])).read_bytes() == original
    assert config_path.stat().st_mode & 0o777 == 0o640
    payload = json.loads(config_path.read_text())
    assert set(payload["mcp"]) == {"servers"}
    assert payload["mcp"]["servers"] == _build_opencode_servers(install_root, env={})
    installed = config_path.read_bytes()
    second = install_opencode_config(install_root, home=tmp_path, env={})
    assert second["status"] == "unchanged"
    assert config_path.read_bytes() == installed
    assert len(list(config_path.parent.glob(".sky-cua-backups/*"))) == 1


@pytest.mark.parametrize("invalid", [True, 0, -1, 1.5])
def test_native_server_builder_rejects_invalid_timeout(tmp_path: Path, invalid: Any) -> None:
    with pytest.raises(ValueError, match="positive integer"):
        _build_opencode_servers(tmp_path, timeout_ms=invalid, env={})
