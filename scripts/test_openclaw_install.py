"""Tests for the OpenClaw MCP server install flow."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import cast

import pytest

import _install_shared
import _openclaw_install
from _install_shared import BROWSER_SELECTION_ENV, SKY_CUA_SKILLS


def test_openclaw_install_sets_mcp_config_without_touching_workspace_skills(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(BROWSER_SELECTION_ENV, "brave")
    repo_root = tmp_path / "repo"
    repo_root.mkdir()
    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)
    calls: list[dict[str, object]] = []

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
        capture_output: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        calls.append({"command": command, "check": check, "env": env, "timeout": timeout})
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    openclaw_dir = tmp_path / "openclaw"
    (openclaw_dir / "workspace" / "skills" / SKY_CUA_SKILLS[0]).mkdir(parents=True)
    (openclaw_dir / "workspace" / "skills" / SKY_CUA_SKILLS[0] / "obsolete.md").write_text(
        "old", encoding="utf-8"
    )

    config_path = _openclaw_install.install_openclaw(
        target_dir,
        client_path,
        openclaw_dir=openclaw_dir,
        openclaw_bin="openclaw",
    )

    assert config_path == target_dir / "openclaw_mcp.json"
    snippet = json.loads(config_path.read_text(encoding="utf-8"))
    server = snippet["mcp"]["servers"]["sky_cua"]
    assert server["command"] == str(client_path)
    assert server["args"] == ["mcp"]
    assert server["cwd"] == str(target_dir)
    assert server["env"]["SKY_CUA_REPO_ROOT"] == str(repo_root)
    assert server["env"][_openclaw_install.MCP_CALLER_PROVENANCE_ENV] == "openclaw"
    assert BROWSER_SELECTION_ENV not in server["env"]
    assert server["enabled"] is True
    # Codex "approve" mode approves every tool call without user interaction;
    # "auto" still prompts for annotated destructive/open-world sky-cua calls.
    assert server["codex"]["defaultToolsApprovalMode"] == "approve"

    assert len(calls) == 2
    command = cast(list[str], calls[0]["command"])
    assert command[:4] == ["openclaw", "mcp", "set", "sky_cua"]
    assert json.loads(command[4]) == server
    assert calls[0]["check"] is True
    assert calls[0]["timeout"] == _openclaw_install.OPENCLAW_MCP_SET_TIMEOUT_SECONDS
    env = cast(dict[str, str], calls[0]["env"])
    assert env["OPENCLAW_STATE_DIR"] == str(openclaw_dir)
    assert env["OPENCLAW_CONFIG_PATH"] == str(openclaw_dir / "openclaw.json")
    reload_command = cast(list[str], calls[1]["command"])
    assert reload_command == ["openclaw", "mcp", "reload"]
    reload_env = cast(dict[str, str], calls[1]["env"])
    assert reload_env["OPENCLAW_STATE_DIR"] == str(openclaw_dir)

    # The workspace-skills copy was retired (2026-07-03): the installer must
    # leave any pre-existing workspace skills exactly as it found them.
    assert (openclaw_dir / "workspace" / "skills" / SKY_CUA_SKILLS[0] / "obsolete.md").read_text(
        encoding="utf-8"
    ) == "old"
    assert not (openclaw_dir / "workspace" / "skills" / SKY_CUA_SKILLS[0] / "SKILL.md").exists()


def test_openclaw_bundle_mode_pins_bundle_resource_root(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import tomllib

    repo_root = tmp_path / "package-root"
    for skill_name in SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)
    monkeypatch.setattr(
        _openclaw_install.subprocess,
        "run",
        lambda command, **kwargs: subprocess.CompletedProcess(command, 0),
    )

    resource_root = tmp_path / "package-root" / "plugin" / "sky-cua"
    app_index = resource_root / "resources" / "app-instructions" / "index.json"
    app_index.parent.mkdir(parents=True)
    app_index.write_text('{"entries":[]}\n', encoding="utf-8")
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    openclaw_dir = tmp_path / "openclaw"
    agent_config = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    agent_config.parent.mkdir(parents=True)
    agent_config.write_text('model = "gpt-5.5"\n', encoding="utf-8")

    config_path = _openclaw_install.install_openclaw(
        target_dir,
        client_path,
        openclaw_dir=openclaw_dir,
        resource_root=resource_root,
    )

    server = json.loads(config_path.read_text(encoding="utf-8"))["mcp"]["servers"]["sky_cua"]
    assert Path(server["env"]["SKY_CUA_REPO_ROOT"]) == resource_root.resolve()
    parsed = tomllib.loads(agent_config.read_text(encoding="utf-8"))
    pinned_root = Path(parsed["mcp_servers"]["sky_cua"]["env"]["SKY_CUA_REPO_ROOT"])
    assert pinned_root == resource_root.resolve()
    assert (pinned_root / "resources" / "app-instructions" / "index.json").exists()


def test_openclaw_codex_home_toml_upsert_is_idempotent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(BROWSER_SELECTION_ENV, "brave")
    config_path = tmp_path / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text('model = "gpt-5.5"\n', encoding="utf-8")
    client_path = tmp_path / "bin" / "sky-cua-client"

    _openclaw_install.install_openclaw_agent_codex_mcp_servers(tmp_path, client_path)
    first = config_path.read_text(encoding="utf-8")
    assert 'model = "gpt-5.5"' in first
    assert "[mcp_servers.sky_cua]" in first
    assert f'command = "{client_path}"' in first
    assert BROWSER_SELECTION_ENV not in first
    assert "SKY_CUA_REPO_ROOT" in first
    assert first.count('SKY_CUA_MCP_CALLER_PROVENANCE = "openclaw"') == 1

    # Re-running replaces the managed block instead of appending a duplicate.
    monkeypatch.delenv(BROWSER_SELECTION_ENV, raising=False)
    _openclaw_install.install_openclaw_agent_codex_mcp_servers(tmp_path, client_path)
    second = config_path.read_text(encoding="utf-8")
    assert second.count("[mcp_servers.sky_cua]") == 1
    assert second.count('SKY_CUA_MCP_CALLER_PROVENANCE = "openclaw"') == 1
    assert BROWSER_SELECTION_ENV not in second.split("[mcp_servers")[1]

    import tomllib

    parsed = tomllib.loads(second)
    assert parsed["mcp_servers"]["sky_cua"]["args"] == ["mcp"]
    # Always-allow at the codex layer: "approve" never prompts; "auto" would
    # still prompt for destructive/open-world sky-cua tools.
    assert parsed["mcp_servers"]["sky_cua"]["default_tools_approval_mode"] == "approve"


def test_openclaw_provenance_is_consistent_idempotent_and_preserves_other_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    agent_config = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    agent_config.parent.mkdir(parents=True)
    agent_config.write_text('model = "gpt-5.5"\n', encoding="utf-8")
    client_path = target_dir / "bin" / "sky-cua-client"
    calls: list[list[str]] = []

    def fake_run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)
    launch_env = {
        "KEEP_ME": "unchanged",
        "SKY_CUA_MCP_HOST": "../../legacy-wrong-value",
        _openclaw_install.MCP_CALLER_PROVENANCE_ENV: "wrong-new-value",
    }

    first_path = _openclaw_install.install_openclaw(
        target_dir,
        client_path,
        openclaw_dir=openclaw_dir,
        launch_env=launch_env,
    )
    first_snippet = first_path.read_bytes()
    first_codex_config = agent_config.read_bytes()
    second_path = _openclaw_install.install_openclaw(
        target_dir,
        client_path,
        openclaw_dir=openclaw_dir,
        launch_env=launch_env,
    )

    assert second_path.read_bytes() == first_snippet
    assert agent_config.read_bytes() == first_codex_config
    server = json.loads(first_snippet)["mcp"]["servers"]["sky_cua"]
    assert server["env"][_openclaw_install.MCP_CALLER_PROVENANCE_ENV] == "openclaw"
    assert server["env"]["KEEP_ME"] == "unchanged"
    assert server["env"]["SKY_CUA_MCP_HOST"] == "../../legacy-wrong-value"
    assert all(
        json.loads(command[4])["env"][_openclaw_install.MCP_CALLER_PROVENANCE_ENV] == "openclaw"
        for command in calls
        if command[:4] == ["openclaw", "mcp", "set", "sky_cua"]
    )

    import tomllib

    codex_env = tomllib.loads(first_codex_config.decode())["mcp_servers"]["sky_cua"]["env"]
    assert codex_env[_openclaw_install.MCP_CALLER_PROVENANCE_ENV] == "openclaw"
    assert codex_env["KEEP_ME"] == "unchanged"
    assert codex_env["SKY_CUA_MCP_HOST"] == "../../legacy-wrong-value"
    assert len(calls) == 4


def test_openclaw_forwards_optional_browser_control_env_without_default(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    agent_config = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    agent_config.parent.mkdir(parents=True)
    agent_config.write_text('model = "gpt-5.5"\n', encoding="utf-8")
    client_path = target_dir / "bin" / "sky-cua-client"
    monkeypatch.setattr(
        _openclaw_install.subprocess,
        "run",
        lambda command, **_kwargs: subprocess.CompletedProcess(command, 0),
    )
    for name in _openclaw_install.OPTIONAL_MCP_RUNTIME_ENV:
        monkeypatch.delenv(name, raising=False)

    default_path = _openclaw_install.install_openclaw(
        target_dir, client_path, openclaw_dir=openclaw_dir
    )
    default_server = json.loads(default_path.read_text(encoding="utf-8"))["mcp"]["servers"][
        "sky_cua"
    ]
    assert all(
        name not in default_server["env"] for name in _openclaw_install.OPTIONAL_MCP_RUNTIME_ENV
    )

    monkeypatch.setenv("SKY_CUA_BROWSER_CONTROL_MODE", "strict")
    monkeypatch.setenv(
        "SKY_CUA_CODEX_BROWSER_SOCKET_PATH", "/run/user/1000/sky-cua/codex-browser.sock"
    )
    configured_path = _openclaw_install.install_openclaw(
        target_dir, client_path, openclaw_dir=openclaw_dir
    )
    first_snippet = configured_path.read_bytes()
    first_codex_config = agent_config.read_bytes()
    _openclaw_install.install_openclaw(target_dir, client_path, openclaw_dir=openclaw_dir)

    assert configured_path.read_bytes() == first_snippet
    assert agent_config.read_bytes() == first_codex_config
    server_env = json.loads(first_snippet)["mcp"]["servers"]["sky_cua"]["env"]
    assert server_env["SKY_CUA_BROWSER_CONTROL_MODE"] == "strict"
    assert (
        server_env["SKY_CUA_CODEX_BROWSER_SOCKET_PATH"]
        == "/run/user/1000/sky-cua/codex-browser.sock"
    )

    import tomllib

    codex_env = tomllib.loads(first_codex_config.decode())["mcp_servers"]["sky_cua"]["env"]
    assert codex_env["SKY_CUA_BROWSER_CONTROL_MODE"] == "strict"
    assert (
        codex_env["SKY_CUA_CODEX_BROWSER_SOCKET_PATH"]
        == "/run/user/1000/sky-cua/codex-browser.sock"
    )
    assert "SKY_CUA_BROWSER_BRIDGE_TRANSPORT" not in first_snippet.decode()
    assert "SKY_CUA_BROWSER_BRIDGE_TRANSPORT" not in first_codex_config.decode()


def test_openclaw_codex_home_pin_refusal_fails_install_without_success_message(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    config_path = tmp_path / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    user_config = f"{_openclaw_install.CODEX_MCP_SERVER_TOML_BEGIN}\nuser_key = 1\n"
    config_path.write_text(user_config, encoding="utf-8")

    with pytest.raises(RuntimeError, match="refused to update OpenClaw agent codex-home"):
        _openclaw_install.install_openclaw_agent_codex_mcp_servers(
            tmp_path, tmp_path / "sky-cua-client"
        )

    captured = capsys.readouterr()
    assert "corrupt sky-cua marker block" in captured.err
    assert "Pinned sky_cua mcp_servers entry" not in captured.out
    assert config_path.read_text(encoding="utf-8") == user_config


def test_openclaw_codex_home_pin_refusal_keeps_batch_unmodified(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    valid_path = tmp_path / "agents" / "alpha" / "agent" / "codex-home" / "config.toml"
    corrupt_path = tmp_path / "agents" / "zulu" / "agent" / "codex-home" / "config.toml"
    valid_path.parent.mkdir(parents=True)
    corrupt_path.parent.mkdir(parents=True)
    valid_config = 'model = "gpt-5.5"\n'
    corrupt_config = f"{_openclaw_install.CODEX_MCP_SERVER_TOML_BEGIN}\nuser_key = 1\n"
    valid_path.write_text(valid_config, encoding="utf-8")
    corrupt_path.write_text(corrupt_config, encoding="utf-8")

    with pytest.raises(RuntimeError, match="refused to update OpenClaw agent codex-home"):
        _openclaw_install.install_openclaw_agent_codex_mcp_servers(
            tmp_path, tmp_path / "sky-cua-client"
        )

    captured = capsys.readouterr()
    assert "corrupt sky-cua marker block" in captured.err
    assert "Pinned sky_cua mcp_servers entry" not in captured.out
    assert valid_path.read_text(encoding="utf-8") == valid_config
    assert corrupt_path.read_text(encoding="utf-8") == corrupt_config


def test_openclaw_codex_home_write_failure_rolls_back_batch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    alpha_path = tmp_path / "agents" / "alpha" / "agent" / "codex-home" / "config.toml"
    zulu_path = tmp_path / "agents" / "zulu" / "agent" / "codex-home" / "config.toml"
    alpha_path.parent.mkdir(parents=True)
    zulu_path.parent.mkdir(parents=True)
    alpha_config = 'model = "gpt-5.5"\n'
    zulu_config = 'model = "gpt-5.5-mini"\n'
    alpha_path.write_text(alpha_config, encoding="utf-8")
    zulu_path.write_text(zulu_config, encoding="utf-8")
    real_write = _install_shared.write_text_atomically

    def fail_zulu(path: Path, text: str, mode: int | None = None) -> None:
        if path == zulu_path:
            raise OSError("write failed")
        real_write(path, text, mode=mode)

    monkeypatch.setattr(_openclaw_install, "write_text_atomically", fail_zulu)

    with pytest.raises(OSError, match="write failed"):
        _openclaw_install.install_openclaw_agent_codex_mcp_servers(
            tmp_path, tmp_path / "sky-cua-client"
        )

    assert "Pinned sky_cua mcp_servers entry" not in capsys.readouterr().out
    assert alpha_path.read_text(encoding="utf-8") == alpha_config
    assert zulu_path.read_text(encoding="utf-8") == zulu_config


def test_openclaw_install_preflights_codex_home_before_registration(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text(
        f"{_openclaw_install.CODEX_MCP_SERVER_TOML_BEGIN}\nuser_key = 1\n",
        encoding="utf-8",
    )
    calls: list[list[str]] = []

    def fake_run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    with pytest.raises(RuntimeError, match="refused to update OpenClaw agent codex-home"):
        _openclaw_install.install_openclaw(
            target_dir,
            target_dir / "bin" / "sky-cua-client",
            openclaw_dir=openclaw_dir,
            openclaw_bin="openclaw",
        )

    assert calls == []
    assert not (target_dir / "openclaw_mcp.json").exists()


def test_openclaw_install_codex_home_write_failure_precedes_registration(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    original_config = 'model = "gpt-5.5"\n'
    config_path.write_text(original_config, encoding="utf-8")
    calls: list[list[str]] = []

    def fake_run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0)

    real_write = _install_shared.write_text_atomically

    def fail_config_write(path: Path, text: str, mode: int | None = None) -> None:
        if path == config_path:
            raise OSError("codex-home write failed")
        real_write(path, text, mode=mode)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)
    monkeypatch.setattr(_openclaw_install, "write_text_atomically", fail_config_write)

    with pytest.raises(OSError, match="codex-home write failed"):
        _openclaw_install.install_openclaw(
            target_dir,
            target_dir / "bin" / "sky-cua-client",
            openclaw_dir=openclaw_dir,
            openclaw_bin="openclaw",
        )

    assert calls == []
    assert not (target_dir / "openclaw_mcp.json").exists()
    assert config_path.read_text(encoding="utf-8") == original_config


def test_openclaw_install_refuses_broken_codex_home_symlink_before_registration(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    broken_target = tmp_path / "missing" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.symlink_to(broken_target)
    calls: list[list[str]] = []

    def fake_run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    with pytest.raises(RuntimeError, match="refused to update OpenClaw agent codex-home"):
        _openclaw_install.install_openclaw(
            target_dir,
            target_dir / "bin" / "sky-cua-client",
            openclaw_dir=openclaw_dir,
            openclaw_bin="openclaw",
        )

    assert "broken symlink" in capsys.readouterr().err
    assert calls == []
    assert config_path.is_symlink()
    assert config_path.readlink() == broken_target
    assert not broken_target.exists()
    assert not (target_dir / "openclaw_mcp.json").exists()


def test_openclaw_codex_home_discovery_skips_missing_agents_dir(tmp_path: Path) -> None:
    assert _openclaw_install.openclaw_agent_codex_config_paths(tmp_path) == []

    config_path = tmp_path / "agents" / "esther" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text("", encoding="utf-8")
    assert _openclaw_install.openclaw_agent_codex_config_paths(tmp_path) == [config_path]


def test_openclaw_install_reports_registration_timeout(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    repo_root = tmp_path / "repo"
    for skill_name in SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
    ) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(command, timeout)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    original_config = 'model = "gpt-5.5"\n'
    config_path.write_text(original_config, encoding="utf-8")

    with pytest.raises(TimeoutError, match="timed out registering sky-cua with OpenClaw"):
        _openclaw_install.install_openclaw(
            target_dir,
            target_dir / "bin" / "sky-cua-client",
            openclaw_dir=openclaw_dir,
            openclaw_bin="openclaw",
        )
    assert "Pinned sky_cua mcp_servers entry" not in capsys.readouterr().out
    assert config_path.read_text(encoding="utf-8") == original_config
    assert not (target_dir / "openclaw_mcp.json").exists()


def test_openclaw_install_registration_error_rolls_back_snippet_without_pin_message(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    original_config = 'model = "gpt-5.5"\n'
    config_path.write_text(original_config, encoding="utf-8")

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
    ) -> subprocess.CompletedProcess[str]:
        raise subprocess.CalledProcessError(1, command, stderr=b"registration failed")

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    with pytest.raises(subprocess.CalledProcessError):
        _openclaw_install.install_openclaw(
            target_dir,
            target_dir / "bin" / "sky-cua-client",
            openclaw_dir=openclaw_dir,
            openclaw_bin="openclaw",
        )

    assert "Pinned sky_cua mcp_servers entry" not in capsys.readouterr().out
    assert config_path.read_text(encoding="utf-8") == original_config
    assert not (target_dir / "openclaw_mcp.json").exists()


def test_openclaw_install_registration_error_preserves_broken_snippet_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    snippet_path = target_dir / "openclaw_mcp.json"
    broken_target = tmp_path / "missing" / "openclaw_mcp.json"
    snippet_path.symlink_to(broken_target)
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    original_config = 'model = "gpt-5.5"\n'
    config_path.write_text(original_config, encoding="utf-8")

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
    ) -> subprocess.CompletedProcess[str]:
        raise subprocess.CalledProcessError(1, command, stderr=b"registration failed")

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    with pytest.raises(subprocess.CalledProcessError):
        _openclaw_install.install_openclaw(
            target_dir,
            target_dir / "bin" / "sky-cua-client",
            openclaw_dir=openclaw_dir,
            openclaw_bin="openclaw",
        )

    assert snippet_path.is_symlink()
    assert snippet_path.readlink() == broken_target
    assert not broken_target.exists()
    assert config_path.read_text(encoding="utf-8") == original_config


def test_openclaw_install_env_carries_gateway_credentials_from_file(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # A plain install shell rarely exports the gateway password; the installer
    # must fill it in from the gateway env file so `mcp set` / `mcp reload` can
    # authenticate to a password-protected gateway.
    monkeypatch.delenv("OPENCLAW_GATEWAY_PASSWORD", raising=False)
    monkeypatch.delenv("OPENCLAW_GATEWAY_TOKEN", raising=False)
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    openclaw_dir.mkdir(parents=True)
    (openclaw_dir / "gateway.systemd.env").write_text(
        'OPENCLAW_GATEWAY_PASSWORD="s3cret"\nOTHER_KEY=ignored\n', encoding="utf-8"
    )
    envs: list[dict[str, str]] = []

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
        capture_output: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        envs.append(env)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    _openclaw_install.install_openclaw(
        target_dir,
        target_dir / "bin" / "sky-cua-client",
        openclaw_dir=openclaw_dir,
        openclaw_bin="openclaw",
    )

    # Both the `mcp set` and `mcp reload` subprocesses get the password, and
    # only the two auth keys are lifted from the file.
    assert envs, "no openclaw subprocess was run"
    for env in envs:
        assert env["OPENCLAW_GATEWAY_PASSWORD"] == "s3cret"
        assert "OTHER_KEY" not in env


def test_resolve_gateway_auth_env_unquotes_single_and_double_and_bare(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # systemd EnvironmentFile permits single or double quotes; both must be
    # stripped, and a bare value left untouched.
    import _install_shared

    monkeypatch.delenv("OPENCLAW_GATEWAY_PASSWORD", raising=False)
    monkeypatch.delenv("OPENCLAW_GATEWAY_TOKEN", raising=False)
    openclaw_dir = tmp_path / "openclaw"
    openclaw_dir.mkdir(parents=True)
    (openclaw_dir / "gateway.systemd.env").write_text(
        "OPENCLAW_GATEWAY_PASSWORD='sng-quoted'\nOPENCLAW_GATEWAY_TOKEN=bare-token\n",
        encoding="utf-8",
    )
    resolved = _install_shared.resolve_gateway_auth_env(openclaw_dir)
    assert resolved == {
        "OPENCLAW_GATEWAY_PASSWORD": "sng-quoted",
        "OPENCLAW_GATEWAY_TOKEN": "bare-token",
    }


def test_openclaw_install_env_keeps_existing_gateway_credentials(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # A credential already exported by the shell always wins over the file.
    monkeypatch.setenv("OPENCLAW_GATEWAY_PASSWORD", "from-shell")
    openclaw_dir = tmp_path / "openclaw"
    openclaw_dir.mkdir(parents=True)
    (openclaw_dir / "gateway.systemd.env").write_text(
        'OPENCLAW_GATEWAY_PASSWORD="from-file"\n', encoding="utf-8"
    )
    envs: list[dict[str, str]] = []

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
        capture_output: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        envs.append(env)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    _openclaw_install.install_openclaw(
        tmp_path / "installed",
        tmp_path / "installed" / "bin" / "sky-cua-client",
        openclaw_dir=openclaw_dir,
        openclaw_bin="openclaw",
    )

    assert envs
    assert all(env["OPENCLAW_GATEWAY_PASSWORD"] == "from-shell" for env in envs)


def test_openclaw_install_survives_unspawnable_reload_after_registration(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    # The reload step is best-effort: if the openclaw binary cannot even be
    # spawned at reload time (OSError / FileNotFoundError), the already-committed
    # registration must stay committed and the install must complete with a
    # warning rather than aborting.
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text('model = "gpt-5.5"\n', encoding="utf-8")
    calls: list[list[str]] = []

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
        capture_output: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        if "reload" in command:
            raise FileNotFoundError("openclaw")
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    # No exception: the unspawnable reload is swallowed as non-fatal.
    config = _openclaw_install.install_openclaw(
        target_dir,
        target_dir / "bin" / "sky-cua-client",
        openclaw_dir=openclaw_dir,
        openclaw_bin="openclaw",
    )

    assert config == target_dir / "openclaw_mcp.json"
    assert len(calls) == 2
    assert calls[0][:4] == ["openclaw", "mcp", "set", "sky_cua"]
    assert calls[1] == ["openclaw", "mcp", "reload"]
    # Registration committed: the snippet and the codex-home pin both persist.
    assert (target_dir / "openclaw_mcp.json").exists()
    config_text = config_path.read_text(encoding="utf-8")
    assert "[mcp_servers.sky_cua]" in config_text
    assert f'command = "{target_dir / "bin" / "sky-cua-client"}"' in config_text
    # The failure was reported, not silently dropped.
    assert "openclaw mcp reload failed" in capsys.readouterr().err


def test_openclaw_codex_home_toml_escapes_special_path_characters(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import tomllib

    weird_repo = tmp_path / 'repo "with" quotes\\and-backslash'
    monkeypatch.setattr(_install_shared, "REPO_ROOT", weird_repo)
    monkeypatch.delenv(BROWSER_SELECTION_ENV, raising=False)
    config_path = tmp_path / "config.toml"
    client_path = tmp_path / 'bin "x"' / "sky-cua-client"

    assert _openclaw_install.upsert_codex_mcp_server_toml(config_path, client_path)

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["mcp_servers"]["sky_cua"]["command"] == str(client_path)
    assert parsed["mcp_servers"]["sky_cua"]["env"]["SKY_CUA_REPO_ROOT"] == str(weird_repo)


def test_openclaw_codex_home_toml_escapes_del_character(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import tomllib

    weird_repo = tmp_path / "repo\x7fdel"
    monkeypatch.setattr(_install_shared, "REPO_ROOT", weird_repo)
    monkeypatch.delenv(BROWSER_SELECTION_ENV, raising=False)
    config_path = tmp_path / "config.toml"
    client_path = tmp_path / "sky-cua-client"

    assert _openclaw_install.upsert_codex_mcp_server_toml(config_path, client_path)

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["mcp_servers"]["sky_cua"]["env"]["SKY_CUA_REPO_ROOT"] == str(weird_repo)


def test_openclaw_codex_home_toml_refuses_unmanaged_duplicate_table(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    config_path = tmp_path / "config.toml"
    config_path.write_text('[mcp_servers.sky_cua]\ncommand = "/old/by-hand"\n', encoding="utf-8")

    assert not _openclaw_install.upsert_codex_mcp_server_toml(
        config_path, tmp_path / "sky-cua-client"
    )

    assert "outside the managed block" in capsys.readouterr().err
    # The hand-written config is untouched.
    assert config_path.read_text(encoding="utf-8").count("[mcp_servers.sky_cua]") == 1


def test_openclaw_codex_home_toml_allows_marker_text_in_comments_and_strings(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    import tomllib

    config_path = tmp_path / "config.toml"
    user_config = '# [mcp_servers.sky_cua]\nnote = "[mcp_servers.sky_cua]"\n'
    config_path.write_text(user_config, encoding="utf-8")

    assert _openclaw_install.upsert_codex_mcp_server_toml(config_path, tmp_path / "sky-cua-client")

    assert capsys.readouterr().err == ""
    text = config_path.read_text(encoding="utf-8")
    assert user_config in text
    parsed = tomllib.loads(text)
    assert parsed["note"] == "[mcp_servers.sky_cua]"
    assert parsed["mcp_servers"]["sky_cua"]["args"] == ["mcp"]


def test_openclaw_codex_home_toml_refuses_orphaned_marker(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    config_path = tmp_path / "config.toml"
    user_config = f"{_openclaw_install.CODEX_MCP_SERVER_TOML_BEGIN}\nuser_key = 1\n"
    config_path.write_text(user_config, encoding="utf-8")

    # First run must refuse rather than appending a block whose END marker
    # would make a later run splice out the user content between the orphan
    # BEGIN and the appended END.
    assert not _openclaw_install.upsert_codex_mcp_server_toml(
        config_path, tmp_path / "sky-cua-client"
    )

    assert "corrupt sky-cua marker block" in capsys.readouterr().err
    assert config_path.read_text(encoding="utf-8") == user_config


def test_openclaw_codex_home_toml_refuses_stray_marker_outside_managed_block(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.delenv(BROWSER_SELECTION_ENV, raising=False)
    config_path = tmp_path / "config.toml"
    client_path = tmp_path / "sky-cua-client"
    assert _openclaw_install.upsert_codex_mcp_server_toml(config_path, client_path)

    # A duplicate END marker after the managed block would otherwise survive
    # every rewrite; the planner must refuse the same way it does for a
    # corrupt marker pair.
    managed = config_path.read_text(encoding="utf-8")
    config_path.write_text(
        f"{managed}\n{_openclaw_install.CODEX_MCP_SERVER_TOML_END}\n", encoding="utf-8"
    )
    corrupted = config_path.read_text(encoding="utf-8")

    assert not _openclaw_install.upsert_codex_mcp_server_toml(config_path, client_path)

    assert "corrupt sky-cua marker block" in capsys.readouterr().err
    assert config_path.read_text(encoding="utf-8") == corrupted

    # A stray BEGIN marker after the managed block is refused the same way.
    config_path.write_text(
        f"{managed}\n{_openclaw_install.CODEX_MCP_SERVER_TOML_BEGIN}\n", encoding="utf-8"
    )
    assert not _openclaw_install.upsert_codex_mcp_server_toml(config_path, client_path)
    assert "corrupt sky-cua marker block" in capsys.readouterr().err


def test_openclaw_install_interrupt_during_registration_rolls_back(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    original_config = 'model = "gpt-5.5"\n'
    config_path.write_text(original_config, encoding="utf-8")

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
    ) -> subprocess.CompletedProcess[str]:
        raise KeyboardInterrupt

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    with pytest.raises(KeyboardInterrupt):
        _openclaw_install.install_openclaw(
            target_dir,
            target_dir / "bin" / "sky-cua-client",
            openclaw_dir=openclaw_dir,
            openclaw_bin="openclaw",
        )

    # An operator Ctrl-C mid-registration must not leave half-pinned state.
    assert config_path.read_text(encoding="utf-8") == original_config
    assert not (target_dir / "openclaw_mcp.json").exists()


def test_openclaw_install_post_commit_timeout_keeps_committed_state(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo_root = tmp_path / "repo"
    for skill_name in SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    config_path = openclaw_dir / "agents" / "sky" / "agent" / "codex-home" / "config.toml"
    config_path.parent.mkdir(parents=True)
    config_path.write_text('model = "gpt-5.5"\n', encoding="utf-8")

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
        capture_output: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(_openclaw_install.subprocess, "run", fake_run)

    def raising_reload(openclaw_bin: str, env: dict[str, str]) -> None:
        # A post-commit timeout must not roll back the committed
        # registration and pins, and must not be mislabeled as a
        # registration timeout.
        raise subprocess.TimeoutExpired(["openclaw", "mcp", "reload"], 30)

    monkeypatch.setattr(_openclaw_install, "reload_openclaw_mcp_runtimes", raising_reload)

    with pytest.raises(subprocess.TimeoutExpired):
        _openclaw_install.install_openclaw(
            target_dir,
            target_dir / "bin" / "sky-cua-client",
            openclaw_dir=openclaw_dir,
            openclaw_bin="openclaw",
        )

    assert "[mcp_servers.sky_cua]" in config_path.read_text(encoding="utf-8")
    assert (target_dir / "openclaw_mcp.json").exists()
