from __future__ import annotations

import hashlib
import json
import stat
from pathlib import Path

import pytest

import _hermes_config


def _fixture_install(root: Path) -> Path:
    for name in ("sky-cua-client", "node_repl"):
        path = root / "bin" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/bin/sh\n", encoding="utf-8")
        path.chmod(0o755)
    return root


def _managed_server(text: str, name: str) -> dict[str, object]:
    prefix = f"  {name}: "
    line = next(line for line in text.splitlines() if line.startswith(prefix))
    value = json.loads(line.removeprefix(prefix))
    assert isinstance(value, dict)
    return value


def test_merge_preserves_unrelated_config_and_replaces_managed_servers() -> None:
    original = (
        "model:\n"
        "  default: example\n"
        "mcp_servers:\n"
        "  honcho:\n"
        "    url: https://example.invalid/mcp\n"
        "  sky_cua:\n"
        "    command: old-client\n"
        "display:\n"
        "  interface: tui\n"
    )
    servers = {
        "sky_cua": {"command": "/fixed/sky-cua-client", "args": ["mcp"]},
        "node_repl": {"command": "/fixed/node_repl", "args": []},
    }

    merged = _hermes_config.merge_hermes_config(original, servers)

    assert "  honcho:\n    url: https://example.invalid/mcp\n" in merged
    assert "model:\n  default: example\n" in merged
    assert "display:\n  interface: tui\n" in merged
    assert "old-client" not in merged
    assert _managed_server(merged, "sky_cua") == servers["sky_cua"]
    assert _managed_server(merged, "node_repl") == servers["node_repl"]
    assert _hermes_config.merge_hermes_config(merged, servers) == merged


def test_merge_rejects_corrupt_managed_markers() -> None:
    text = f"mcp_servers:\n{_hermes_config.HERMES_MANAGED_START}\n"
    servers = {"sky_cua": {}, "node_repl": {}}

    with pytest.raises(ValueError, match="corrupt Sky CUA managed MCP markers"):
        _hermes_config.merge_hermes_config(text, servers)


def test_merge_handles_four_space_children_and_root_aligned_comments() -> None:
    original = (
        "mcp_servers:\n"
        "    sky_cua:\n"
        "        command: old-client\n"
        "# This comment does not end the YAML mapping.\n"
        "    unrelated:\n"
        "        command: keep-me\n"
        "display:\n"
        "    interface: tui\n"
    )
    servers = {"sky_cua": {"command": "new-client"}, "node_repl": {"command": "node"}}

    merged = _hermes_config.merge_hermes_config(original, servers)

    assert "old-client" not in merged
    assert "    unrelated:\n        command: keep-me\n" in merged
    assert "# This comment does not end the YAML mapping.\n" not in merged
    assert "    sky_cua: " in merged
    assert "    node_repl: " in merged


@pytest.mark.parametrize("quote", ['"', "'"])
def test_merge_replaces_quoted_managed_keys(quote: str) -> None:
    original = (
        "mcp_servers:\n"
        f"  {quote}sky_cua{quote}:\n"
        "    command: old-client\n"
        f"  {quote}node_repl{quote}:\n"
        "    command: old-node\n"
    )
    servers = {"sky_cua": {"command": "new-client"}, "node_repl": {"command": "new-node"}}

    merged = _hermes_config.merge_hermes_config(original, servers)

    assert "old-client" not in merged
    assert "old-node" not in merged
    assert merged.count("  sky_cua:") == 1
    assert merged.count("  node_repl:") == 1


def test_agents_merge_preserves_unrelated_content_and_is_idempotent() -> None:
    original = "# Team context\n\nKeep this instruction.\n"

    merged = _hermes_config.merge_hermes_agents(original)

    assert original.rstrip() in merged
    assert "mcp__node_repl__js" in merged
    assert "nodeRepl.write(value)" in merged
    assert "node:process" in merged
    assert _hermes_config.merge_hermes_agents(merged) == merged


def test_agents_merge_rejects_corrupt_markers() -> None:
    with pytest.raises(ValueError, match="corrupt Sky CUA managed markers"):
        _hermes_config.merge_hermes_agents(_hermes_config.HERMES_AGENTS_START + "\n")


def test_install_backs_up_then_idempotently_updates_existing_config(tmp_path: Path) -> None:
    install_root = _fixture_install(tmp_path / "installed")
    home = tmp_path / "home"
    config_path = home / ".hermes" / "config.yaml"
    config_path.parent.mkdir(parents=True)
    original = "mcp_servers:\n  honcho:\n    url: https://example.invalid/mcp\n"
    config_path.write_text(original, encoding="utf-8")
    config_path.chmod(0o640)

    first = _hermes_config.install_hermes_config(
        install_root,
        home=home,
        env={"DISPLAY": ":1"},
    )
    first_bytes = config_path.read_bytes()
    second = _hermes_config.install_hermes_config(
        install_root,
        home=home,
        env={"DISPLAY": ":1"},
    )

    assert first.status == "updated"
    assert first.backup_path is not None
    assert first.backup_path.read_text(encoding="utf-8") == original
    assert stat.S_IMODE(first.backup_path.stat().st_mode) == 0o640
    assert stat.S_IMODE(config_path.stat().st_mode) == 0o640
    assert second.status == "unchanged"
    assert second.backup_path is None
    assert config_path.read_bytes() == first_bytes
    sky_cua = _managed_server(first_bytes.decode(), "sky_cua")
    assert sky_cua["command"] == str(install_root / "bin/sky-cua-client")
    assert sky_cua["env"] == {
        "DISPLAY": ":1",
        "SKY_CUA_MCP_CALLER_PROVENANCE": "hermes",
        "SKY_CUA_PRESENCE_ENABLED": "1",
        "SKY_CUA_REPO_ROOT": str(install_root),
    }
    assert _managed_server(first_bytes.decode(), "node_repl")["command"] == str(
        install_root / "bin/node_repl"
    )


def test_install_respects_hermes_home_and_explicit_creation(tmp_path: Path) -> None:
    install_root = _fixture_install(tmp_path / "installed")
    configured_home = tmp_path / "profile"

    absent = _hermes_config.install_hermes_config(
        install_root,
        home=tmp_path / "home",
        env={"HERMES_HOME": str(configured_home)},
    )
    created = _hermes_config.install_hermes_config(
        install_root,
        home=tmp_path / "home",
        env={"HERMES_HOME": str(configured_home)},
        create=True,
    )

    assert absent.status == "no_global_config"
    assert absent.config_path is None
    assert created.status == "updated"
    assert created.config_path == configured_home / "config.yaml"


def test_install_agents_backs_up_and_preserves_existing_instructions(tmp_path: Path) -> None:
    home = tmp_path / "home"
    agents_path = home / ".hermes/AGENTS.md"
    agents_path.parent.mkdir(parents=True)
    agents_path.write_text("# Existing\n\nKeep me.\n", encoding="utf-8")

    first = _hermes_config.install_hermes_agents(home=home, env={})
    first_bytes = agents_path.read_bytes()
    second = _hermes_config.install_hermes_agents(home=home, env={})

    assert first.status == "updated"
    assert first.backup_path is not None
    assert first.backup_path.read_text(encoding="utf-8") == "# Existing\n\nKeep me.\n"
    assert second.status == "unchanged"
    assert agents_path.read_bytes() == first_bytes


def test_backup_identity_hashes_exact_line_endings(tmp_path: Path) -> None:
    install_root = _fixture_install(tmp_path / "installed")
    home = tmp_path / "home"
    config_path = home / ".hermes/config.yaml"
    config_path.parent.mkdir(parents=True)
    crlf = b"mcp_servers:\r\n  unrelated:\r\n    command: keep\r\n"
    lf = crlf.replace(b"\r\n", b"\n")

    config_path.write_bytes(crlf)
    first = _hermes_config.install_hermes_config(install_root, home=home, env={})
    config_path.write_bytes(lf)
    second = _hermes_config.install_hermes_config(install_root, home=home, env={})

    assert first.backup_path is not None
    assert second.backup_path is not None
    assert first.backup_path != second.backup_path
    assert first.backup_path.read_bytes() == crlf
    assert second.backup_path.read_bytes() == lf
    assert hashlib.sha256(crlf).hexdigest() in first.backup_path.name
    assert hashlib.sha256(lf).hexdigest() in second.backup_path.name
