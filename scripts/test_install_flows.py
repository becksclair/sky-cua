"""Tests for plugin and MCP server install flows."""

from __future__ import annotations

import json
import stat
import subprocess
import sys
from pathlib import Path
from typing import cast

import pytest

import _kwin_effect as kwin_effect
import _plugin_bundle as plugin_bundle
import install_mcp_server
import install_plugin
from _plugin_bundle import (
    current_runtime_platform,
    runtime_binary_names,
    runtime_binary_path,
    runtime_binary_source_name,
)
from _test_support import (
    write_minimal_bundle,
)


def test_install_bundle_uses_runtime_binary_paths(tmp_path: Path) -> None:
    source = tmp_path / "source"
    destination = tmp_path / "installed"
    write_minimal_bundle(source, binaries=runtime_binary_names())

    install_plugin.install_bundle(source, destination, symlink=False)

    platform_id = current_runtime_platform()
    for binary_name in plugin_bundle.platform_runtime_binary_base_names(platform_id):
        binary_path = destination / runtime_binary_path(platform_id, binary_name)
        assert binary_path.exists()
        if not binary_path.name.endswith(".exe"):
            assert binary_path.stat().st_mode & 0o111


def test_install_plugin_skips_browser_preflight_on_non_linux(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    destination = tmp_path / "installed"
    preflight = destination / "resources" / "chrome_preflight.py"
    preflight.parent.mkdir(parents=True)
    preflight.write_text("raise SystemExit(99)", encoding="utf-8")
    calls: list[list[str]] = []

    def fake_run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(install_plugin.sys, "platform", "win32")
    monkeypatch.setattr(install_plugin.subprocess, "run", fake_run)

    install_plugin.run_browser_preflight(destination, tmp_path / "codex-home")

    assert calls == []


def test_generic_mcp_install_copies_all_current_platform_binaries(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    release_root = repo_root / "target" / "release"
    release_root.mkdir(parents=True)
    target_dir = tmp_path / "installed"
    platform_id = install_mcp_server.current_platform()

    for binary_name in install_mcp_server.platform_runtime_binary_base_names(platform_id):
        source_name = runtime_binary_source_name(platform_id, binary_name)
        (release_root / source_name).write_text(binary_name, encoding="utf-8")

    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    client_path = install_mcp_server.install_binaries(target_dir)

    assert client_path == target_dir / install_mcp_server.entrypoint_path(
        platform_id, "sky-cua-client"
    )
    for binary_name in install_mcp_server.platform_runtime_binary_base_names(platform_id):
        binary_path = target_dir / install_mcp_server.entrypoint_path(platform_id, binary_name)
        assert binary_path.exists()
        if not binary_path.name.endswith(".exe"):
            assert binary_path.stat().st_mode & 0o111


def test_generic_mcp_bin_links_use_platform_entrypoint_names(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    bin_dir = tmp_path / "bin"
    for name in install_mcp_server.platform_runtime_binary_base_names("windows-x64"):
        binary = target_dir / install_mcp_server.entrypoint_path("windows-x64", name)
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_text(name, encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "current_platform", lambda: "windows-x64")

    install_mcp_server.link_current_platform_binaries(target_dir, bin_dir)

    assert (bin_dir / "sky-cua-client.exe").readlink() == target_dir / "bin" / "sky-cua-client.exe"
    assert (
        bin_dir / "sky-cua-service.exe"
    ).readlink() == target_dir / "bin" / "sky-cua-service.exe"
    assert (bin_dir / "sky-cua-overlay-host.exe").readlink() == (
        target_dir / "bin" / "sky-cua-overlay-host.exe"
    )
    assert not (bin_dir / "sky-cua-client").exists()


def test_generic_mcp_bin_links_copy_when_symlinks_are_unavailable(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    bin_dir = tmp_path / "bin"
    binary = target_dir / install_mcp_server.entrypoint_path("windows-x64", "sky-cua-client")
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_text("client", encoding="utf-8")
    service = target_dir / install_mcp_server.entrypoint_path("windows-x64", "sky-cua-service")
    service.write_text("service", encoding="utf-8")
    overlay = target_dir / install_mcp_server.entrypoint_path("windows-x64", "sky-cua-overlay-host")
    overlay.write_text("overlay", encoding="utf-8")

    def fake_symlink_to(self: Path, target: Path) -> None:
        _ = self, target
        raise OSError("symlinks unavailable")

    monkeypatch.setattr(install_mcp_server, "current_platform", lambda: "windows-x64")
    monkeypatch.setattr(Path, "symlink_to", fake_symlink_to)

    install_mcp_server.link_current_platform_binaries(target_dir, bin_dir)

    assert (bin_dir / "sky-cua-client.exe").read_text(encoding="utf-8") == "client"
    assert (bin_dir / "sky-cua-service.exe").read_text(encoding="utf-8") == "service"
    assert (bin_dir / "sky-cua-overlay-host.exe").read_text(encoding="utf-8") == "overlay"


def test_generic_mcp_restart_runtime_stops_installed_processes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    calls: list[list[Path]] = []

    def fake_stop_unix_runtime_processes(search_roots: list[Path]) -> None:
        calls.append(search_roots)

    monkeypatch.setattr(
        install_mcp_server,
        "stop_unix_runtime_processes",
        fake_stop_unix_runtime_processes,
    )
    monkeypatch.setattr(install_mcp_server, "stop_windows_cache_processes", lambda _root: None)

    install_mcp_server.restart_runtime_processes(target_dir)

    assert calls == [[target_dir]]


def test_generic_mcp_main_can_restart_runtime_after_install(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    client_path = target_dir / "bin" / "sky-cua-client"
    restarted: list[Path] = []

    monkeypatch.setattr(
        sys, "argv", ["install_mcp_server.py", "--target-dir", str(target_dir), "--restart-runtime"]
    )
    monkeypatch.setattr(install_mcp_server, "install_binaries", lambda _target_dir: client_path)
    monkeypatch.setattr(
        install_mcp_server,
        "write_mcp_json",
        lambda target, _config: target / ".mcp.json",
    )
    monkeypatch.setattr(
        install_mcp_server,
        "restart_runtime_processes",
        lambda target: restarted.append(target),
    )
    monkeypatch.setattr(install_mcp_server, "print_next_steps", lambda *_args: None)

    assert install_mcp_server.main() == 0
    assert restarted == [target_dir.resolve()]


def test_generic_mcp_main_stops_windows_runtime_before_binary_copy(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    client_path = target_dir / "bin" / "sky-cua-client.exe"
    events: list[str] = []

    monkeypatch.setattr(
        sys,
        "argv",
        ["install_mcp_server.py", "--target-dir", str(target_dir), "--restart-runtime"],
    )
    monkeypatch.setattr(install_mcp_server.sys, "platform", "win32")
    monkeypatch.setattr(
        install_mcp_server,
        "restart_runtime_processes",
        lambda _target: events.append("restart"),
    )

    def fake_install_binaries(_target_dir: Path) -> Path:
        events.append("install")
        return client_path

    monkeypatch.setattr(install_mcp_server, "install_binaries", fake_install_binaries)
    monkeypatch.setattr(
        install_mcp_server,
        "write_mcp_json",
        lambda target, _config: target / ".mcp.json",
    )
    monkeypatch.setattr(install_mcp_server, "print_next_steps", lambda *_args: None)

    assert install_mcp_server.main() == 0
    assert events == ["restart", "install", "restart"]


def test_generic_mcp_next_steps_document_restart_runtime(
    capsys: pytest.CaptureFixture[str],
) -> None:
    target_dir = Path("/tmp/sky-cua-install")
    client_path = target_dir / "bin" / "sky-cua-client"
    config_path = target_dir / "opencode.json"

    install_mcp_server.print_next_steps("opencode", target_dir, client_path, config_path)
    install_mcp_server.print_next_steps("pi", target_dir, client_path, target_dir / "pi_mcp.json")
    install_mcp_server.print_next_steps(
        "openclaw", target_dir, client_path, target_dir / "openclaw_mcp.json"
    )

    output = capsys.readouterr().out
    assert "--restart-runtime" in output
    assert "Restart or reload the OpenCode session" in output
    assert "Restart Pi or run /reload" in output
    assert "configured OpenClaw workspace" in output
    assert "~/.openclaw/workspace/skills" not in output


def test_opencode_install_configures_browser_tools_without_enable_flag(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(install_mcp_server.BROWSER_SELECTION_ENV, raising=False)
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"

    config_path = install_mcp_server.install_opencode(target_dir, client_path)

    config = json.loads(config_path.read_text(encoding="utf-8"))
    env = config["mcp"]["sky_cua"]["environment"]
    assert env["SKY_CUA_REPO_ROOT"] == str(install_mcp_server.REPO_ROOT)
    assert install_mcp_server.BROWSER_SELECTION_ENV not in env


def test_opencode_install_preserves_browser_selection_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(install_mcp_server.BROWSER_SELECTION_ENV, "brave")
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"

    config_path = install_mcp_server.install_opencode(target_dir, client_path)

    config = json.loads(config_path.read_text(encoding="utf-8"))
    env = config["mcp"]["sky_cua"]["environment"]
    assert env[install_mcp_server.BROWSER_SELECTION_ENV] == "brave"


def test_claude_desktop_install_preserves_browser_selection_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(install_mcp_server.BROWSER_SELECTION_ENV, "brave")
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"

    config_path = install_mcp_server.install_claude_desktop(target_dir, client_path)

    config = json.loads(config_path.read_text(encoding="utf-8"))
    env = config["mcpServers"]["computer-use"]["env"]
    assert env["SKY_CUA_REPO_ROOT"] == str(install_mcp_server.REPO_ROOT)
    assert env[install_mcp_server.BROWSER_SELECTION_ENV] == "brave"


def test_pi_install_merges_mcp_config_and_copies_sky_cua_skills(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(install_mcp_server.BROWSER_SELECTION_ENV, "brave")
    repo_root = tmp_path / "repo"
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    agent_dir = tmp_path / "pi-agent"
    agent_dir.mkdir()
    (agent_dir / "mcp.json").write_text(
        json.dumps({"mcpServers": {"context7": {"command": "context7"}}}),
        encoding="utf-8",
    )
    stale_skill = agent_dir / "skills" / install_mcp_server.SKY_CUA_SKILLS[0]
    stale_skill.mkdir(parents=True)
    (stale_skill / "obsolete.md").write_text("old", encoding="utf-8")
    unrelated_skill = agent_dir / "skills" / "other-skill"
    unrelated_skill.mkdir(parents=True)
    (unrelated_skill / "SKILL.md").write_text("# other\n", encoding="utf-8")

    snippet_path = install_mcp_server.install_pi(target_dir, client_path, agent_dir)

    wrapper = target_dir / "pi_mcp_wrapper.sh"
    wrapper_text = wrapper.read_text(encoding="utf-8")
    assert f"export {install_mcp_server.BROWSER_SELECTION_ENV}=brave" in wrapper_text
    assert snippet_path == target_dir / "pi_mcp.json"
    snippet = json.loads(snippet_path.read_text(encoding="utf-8"))
    assert snippet["mcpServers"]["sky_cua"]["command"] == str(wrapper)

    merged = json.loads((agent_dir / "mcp.json").read_text(encoding="utf-8"))
    assert merged["mcpServers"]["context7"] == {"command": "context7"}
    assert merged["mcpServers"]["sky_cua"]["command"] == str(wrapper)
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        assert (agent_dir / "skills" / skill_name / "SKILL.md").read_text(
            encoding="utf-8"
        ) == f"# {skill_name}\n"
    assert not (stale_skill / "obsolete.md").exists()
    assert (unrelated_skill / "SKILL.md").read_text(encoding="utf-8") == "# other\n"


def test_pi_install_preserves_symlinked_mcp_config(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    agent_dir = tmp_path / "pi-agent"
    agent_dir.mkdir()
    real_config = tmp_path / "real-mcp.json"
    real_config.write_text(
        json.dumps({"mcpServers": {"context7": {"command": "context7"}}}) + "\n",
        encoding="utf-8",
    )
    config_link = agent_dir / "mcp.json"
    try:
        config_link.symlink_to(real_config)
    except OSError as error:
        pytest.skip(f"symlink creation is unavailable: {error}")

    install_mcp_server.install_pi(target_dir, client_path, agent_dir)

    assert config_link.is_symlink()
    merged = json.loads(real_config.read_text(encoding="utf-8"))
    assert merged["mcpServers"]["context7"] == {"command": "context7"}
    assert merged["mcpServers"]["sky_cua"]["command"] == str(target_dir / "pi_mcp_wrapper.sh")


def test_openclaw_install_sets_mcp_config_and_copies_sky_cua_skills(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(install_mcp_server.BROWSER_SELECTION_ENV, "brave")
    repo_root = tmp_path / "repo"
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)
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

    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    openclaw_dir = tmp_path / "openclaw"
    (openclaw_dir / "workspace" / "skills" / install_mcp_server.SKY_CUA_SKILLS[0]).mkdir(
        parents=True
    )
    (
        openclaw_dir / "workspace" / "skills" / install_mcp_server.SKY_CUA_SKILLS[0] / "obsolete.md"
    ).write_text("old", encoding="utf-8")

    config_path = install_mcp_server.install_openclaw(
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
    assert server["env"][install_mcp_server.BROWSER_SELECTION_ENV] == "brave"
    assert server["enabled"] is True
    # "approve" defers MCP tool calls to the codex app-server approval policy,
    # which blocks every call during unattended agent turns; "auto" keeps the
    # tools callable.
    assert server["codex"]["defaultToolsApprovalMode"] == "auto"

    assert len(calls) == 2
    command = cast(list[str], calls[0]["command"])
    assert command[:4] == ["openclaw", "mcp", "set", "sky_cua"]
    assert json.loads(command[4]) == server
    assert calls[0]["check"] is True
    assert calls[0]["timeout"] == install_mcp_server.OPENCLAW_MCP_SET_TIMEOUT_SECONDS
    env = cast(dict[str, str], calls[0]["env"])
    assert env["OPENCLAW_STATE_DIR"] == str(openclaw_dir)
    assert env["OPENCLAW_CONFIG_PATH"] == str(openclaw_dir / "openclaw.json")
    reload_command = cast(list[str], calls[1]["command"])
    assert reload_command == ["openclaw", "mcp", "reload"]
    reload_env = cast(dict[str, str], calls[1]["env"])
    assert reload_env["OPENCLAW_STATE_DIR"] == str(openclaw_dir)

    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        assert (openclaw_dir / "workspace" / "skills" / skill_name / "SKILL.md").read_text(
            encoding="utf-8"
        ) == f"# {skill_name}\n"
    assert not (
        openclaw_dir / "workspace" / "skills" / install_mcp_server.SKY_CUA_SKILLS[0] / "obsolete.md"
    ).exists()


def test_openclaw_install_reports_registration_timeout(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    for skill_name in install_mcp_server.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    def fake_run(
        command: list[str],
        *,
        check: bool,
        env: dict[str, str],
        timeout: int,
    ) -> subprocess.CompletedProcess[str]:
        raise subprocess.TimeoutExpired(command, timeout)

    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    with pytest.raises(TimeoutError, match="timed out registering sky-cua with OpenClaw"):
        install_mcp_server.install_openclaw(
            tmp_path / "installed",
            tmp_path / "installed" / "bin" / "sky-cua-client",
            openclaw_dir=tmp_path / "openclaw",
            openclaw_bin="openclaw",
        )


def test_generic_mcp_main_can_install_openclaw_host(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    openclaw_dir = tmp_path / "openclaw"
    client_path = target_dir / "bin" / "sky-cua-client"
    installed: list[tuple[Path, Path, Path]] = []

    monkeypatch.setattr(
        sys,
        "argv",
        [
            "install_mcp_server.py",
            "--target-dir",
            str(target_dir),
            "--host",
            "openclaw",
            "--openclaw-dir",
            str(openclaw_dir),
        ],
    )
    monkeypatch.setattr(install_mcp_server, "install_binaries", lambda _target_dir: client_path)
    monkeypatch.setattr(
        install_mcp_server,
        "install_openclaw",
        lambda target, client, openclaw_dir, openclaw_bin="openclaw": (
            installed.append((target, client, openclaw_dir)) or target / "openclaw_mcp.json"
        ),
    )
    monkeypatch.setattr(install_mcp_server, "print_next_steps", lambda *_args: None)

    assert install_mcp_server.main() == 0
    assert installed == [(target_dir.resolve(), client_path, openclaw_dir.resolve())]


def test_pi_mcp_config_merge_keeps_existing_file_when_replace_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    config_path = tmp_path / "mcp.json"
    original = json.dumps({"mcpServers": {"context7": {"command": "context7"}}}) + "\n"
    config_path.write_text(original, encoding="utf-8")

    def fail_replace(_source: Path, _destination: Path) -> None:
        raise OSError("replace failed")

    monkeypatch.setattr(install_mcp_server.os, "replace", fail_replace)

    with pytest.raises(OSError, match="replace failed"):
        install_mcp_server.merge_pi_mcp_config(
            config_path,
            {"mcpServers": {"sky_cua": {"command": "/tmp/sky-cua-client"}}},
        )

    assert config_path.read_text(encoding="utf-8") == original
    assert not list(tmp_path.glob(".mcp.json.tmp-*"))


def test_pi_mcp_config_merge_preserves_existing_file_permissions(tmp_path: Path) -> None:
    config_path = tmp_path / "mcp.json"
    config_path.write_text(
        json.dumps({"mcpServers": {"context7": {"command": "context7"}}}) + "\n",
        encoding="utf-8",
    )
    config_path.chmod(0o600)

    install_mcp_server.merge_pi_mcp_config(
        config_path,
        {"mcpServers": {"sky_cua": {"command": "/tmp/sky-cua-client"}}},
    )

    assert stat.S_IMODE(config_path.stat().st_mode) == 0o600


def test_pi_skill_install_keeps_existing_skill_when_copy_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    skill_name = "computer-use"
    monkeypatch.setattr(install_mcp_server, "SKY_CUA_SKILLS", (skill_name,))
    repo_root = tmp_path / "repo"
    source = repo_root / "skills" / skill_name
    source.mkdir(parents=True)
    (source / "SKILL.md").write_text("# new\n", encoding="utf-8")
    monkeypatch.setattr(install_mcp_server, "REPO_ROOT", repo_root)

    skills_dir = tmp_path / "skills"
    destination = skills_dir / skill_name
    destination.mkdir(parents=True)
    (destination / "SKILL.md").write_text("# old\n", encoding="utf-8")

    def fail_copytree(_source: Path, destination: Path) -> None:
        destination.mkdir(parents=True)
        (destination / "partial.md").write_text("partial\n", encoding="utf-8")
        raise OSError("copy failed")

    monkeypatch.setattr(install_mcp_server.shutil, "copytree", fail_copytree)

    with pytest.raises(OSError, match="copy failed"):
        install_mcp_server.install_pi_skills(skills_dir)

    assert (destination / "SKILL.md").read_text(encoding="utf-8") == "# old\n"
    assert not (destination / "partial.md").exists()
    assert not list(skills_dir.glob(f".{skill_name}.tmp-*"))


def test_replace_tree_atomically_restores_file_destination_when_replace_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source = tmp_path / "source"
    source.mkdir()
    (source / "SKILL.md").write_text("# new\n", encoding="utf-8")
    destination = tmp_path / "skill"
    destination.write_text("# old-file\n", encoding="utf-8")
    real_replace = install_mcp_server.os.replace

    def fail_new_tree_replace(source_path: Path, destination_path: Path) -> None:
        if (
            Path(source_path).name.startswith(".skill.tmp-")
            and Path(destination_path) == destination
        ):
            raise OSError("replace failed")
        real_replace(source_path, destination_path)

    monkeypatch.setattr(install_mcp_server.os, "replace", fail_new_tree_replace)

    with pytest.raises(OSError, match="replace failed"):
        install_mcp_server.replace_tree_atomically(source, destination)

    assert destination.read_text(encoding="utf-8") == "# old-file\n"
    assert not list(tmp_path.glob(".skill.tmp-*"))
    assert not list(tmp_path.glob(".skill.backup-*"))


def test_install_mcp_server_kwin_effect_flag_invokes_helper(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: dict[str, object] = {}

    def fake_deploy(**kwargs: object) -> kwin_effect.ReloadOutcome:
        calls.update(kwargs)
        return kwin_effect.ReloadOutcome(
            converged=True,
            loaded=True,
            expected_build_id="abc",
            running_build_id="abc",
        )

    monkeypatch.setattr(install_mcp_server, "deploy_kwin_effect", fake_deploy)
    monkeypatch.setattr(
        install_mcp_server, "install_binaries", lambda target: target / "bin" / "sky-cua-client"
    )
    monkeypatch.setattr(
        install_mcp_server,
        "write_mcp_json",
        lambda target, config: target / ".mcp.json",
    )
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "install_mcp_server.py",
            "--target-dir",
            str(tmp_path / "install"),
            "--kwin-effect",
        ],
    )

    assert install_mcp_server.main() == 0
    assert calls["build_dir"] == (tmp_path / "install").resolve() / "kwin-effect-build"
