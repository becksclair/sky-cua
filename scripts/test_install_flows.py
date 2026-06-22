"""Tests for plugin and MCP server install flows."""

from __future__ import annotations

import json
import stat
import subprocess
import sys
from pathlib import Path

import pytest

import _codex_exec
import _install_shared
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

    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)

    client_path = install_mcp_server.install_binaries(target_dir)

    assert client_path == target_dir / install_mcp_server.entrypoint_path(
        platform_id, "sky-cua-client"
    )
    for binary_name in install_mcp_server.platform_runtime_binary_base_names(platform_id):
        binary_path = target_dir / install_mcp_server.entrypoint_path(platform_id, binary_name)
        assert binary_path.exists()
        if not binary_path.name.endswith(".exe"):
            assert binary_path.stat().st_mode & 0o111


def test_bundle_binary_install_copies_from_bundle_not_release_build(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    release_root = repo_root / "target" / "release"
    release_root.mkdir(parents=True)
    bundle_root = tmp_path / "bundle"
    target_dir = tmp_path / "installed"
    platform_id = install_mcp_server.current_platform()

    for binary_name in install_mcp_server.platform_runtime_binary_base_names(platform_id):
        source_name = runtime_binary_source_name(platform_id, binary_name)
        (release_root / source_name).write_text(f"release {binary_name}", encoding="utf-8")
        bundle_binary = bundle_root / runtime_binary_path(platform_id, binary_name)
        bundle_binary.parent.mkdir(parents=True, exist_ok=True)
        bundle_binary.write_text(f"bundle {binary_name}", encoding="utf-8")

    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)

    client_path = install_mcp_server.install_bundle_binaries(bundle_root, target_dir)

    assert client_path == target_dir / install_mcp_server.entrypoint_path(
        platform_id, "sky-cua-client"
    )
    for binary_name in install_mcp_server.platform_runtime_binary_base_names(platform_id):
        binary_path = target_dir / install_mcp_server.entrypoint_path(platform_id, binary_name)
        assert binary_path.read_text(encoding="utf-8") == f"bundle {binary_name}"


def test_bundle_binary_install_requires_bundle_binaries(
    tmp_path: Path,
) -> None:
    bundle_root = tmp_path / "bundle"
    bundle_root.mkdir()

    with pytest.raises(FileNotFoundError, match="bundle binary not found"):
        install_mcp_server.install_bundle_binaries(bundle_root, tmp_path / "installed")


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


def test_install_mcp_server_has_no_top_level_pwd_import() -> None:
    source = Path(install_mcp_server.__file__).read_text(encoding="utf-8")

    assert "\nimport pwd\n" not in source


def test_generic_mcp_restart_runtime_stops_installed_processes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    calls: list[list[Path]] = []
    atspi_refreshes = 0

    def fake_stop_unix_runtime_processes(search_roots: list[Path]) -> None:
        calls.append(search_roots)

    def fake_refresh_accessibility_bus() -> None:
        nonlocal atspi_refreshes
        atspi_refreshes += 1

    monkeypatch.setattr(
        install_mcp_server,
        "refresh_accessibility_bus",
        fake_refresh_accessibility_bus,
    )
    monkeypatch.setattr(
        install_mcp_server,
        "stop_unix_runtime_processes",
        fake_stop_unix_runtime_processes,
    )
    monkeypatch.setattr(install_mcp_server, "stop_windows_cache_processes", lambda _root: None)

    install_mcp_server.restart_runtime_processes(target_dir)

    assert atspi_refreshes == 1
    assert calls == [[target_dir]]


def test_refresh_accessibility_bus_restarts_user_atspi(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    calls: list[list[str]] = []

    def fake_which(name: str) -> str | None:
        return f"/usr/bin/{name}" if name in {"pkill", "systemctl"} else None

    def fake_run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        return subprocess.CompletedProcess(argv, returncode=0, stdout="", stderr="")

    monkeypatch.setattr(install_mcp_server.sys, "platform", "linux")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/run/user/1000")
    monkeypatch.setenv("USER", "bex")
    monkeypatch.setattr(install_mcp_server.shutil, "which", fake_which)
    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    install_mcp_server.refresh_accessibility_bus()

    assert calls == [
        [
            "/usr/bin/pkill",
            "-u",
            "bex",
            "-f",
            r"(^|/)at-spi2-registryd( |$)",
        ],
        ["/usr/bin/systemctl", "--user", "restart", "at-spi-dbus-bus.service"],
    ]
    assert "Refreshed user AT-SPI accessibility bus." in capsys.readouterr().out


def test_refresh_accessibility_bus_skips_without_user_session(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[list[str]] = []

    def fake_run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        return subprocess.CompletedProcess(argv, returncode=0, stdout="", stderr="")

    monkeypatch.setattr(install_mcp_server.sys, "platform", "linux")
    monkeypatch.delenv("DBUS_SESSION_BUS_ADDRESS", raising=False)
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/run/user/1000")
    monkeypatch.setattr(install_mcp_server.shutil, "which", lambda name: f"/usr/bin/{name}")
    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    install_mcp_server.refresh_accessibility_bus()

    assert calls == []


def test_refresh_accessibility_bus_skips_on_non_linux(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[list[str]] = []

    def fake_run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        return subprocess.CompletedProcess(argv, returncode=0, stdout="", stderr="")

    monkeypatch.setattr(install_mcp_server.sys, "platform", "darwin")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/run/user/1000")
    monkeypatch.setenv("USER", "bex")
    monkeypatch.setattr(install_mcp_server.shutil, "which", lambda name: f"/usr/bin/{name}")
    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    install_mcp_server.refresh_accessibility_bus()

    assert calls == []


def test_refresh_accessibility_bus_skips_pkill_when_systemctl_missing(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[list[str]] = []

    def fake_run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        return subprocess.CompletedProcess(argv, returncode=0, stdout="", stderr="")

    monkeypatch.setattr(install_mcp_server.sys, "platform", "linux")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/run/user/1000")
    monkeypatch.setenv("USER", "bex")
    monkeypatch.setattr(
        install_mcp_server.shutil,
        "which",
        lambda name: "/usr/bin/pkill" if name == "pkill" else None,
    )
    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    install_mcp_server.refresh_accessibility_bus()

    assert calls == []


def test_refresh_accessibility_bus_warns_when_systemctl_fails(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    calls: list[list[str]] = []

    def fake_run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        if argv[0] == "/usr/bin/systemctl":
            return subprocess.CompletedProcess(
                argv, returncode=1, stdout="", stderr="unit not found"
            )
        return subprocess.CompletedProcess(argv, returncode=0, stdout="", stderr="")

    monkeypatch.setattr(install_mcp_server.sys, "platform", "linux")
    monkeypatch.setenv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus")
    monkeypatch.setenv("XDG_RUNTIME_DIR", "/run/user/1000")
    monkeypatch.setenv("USER", "bex")
    monkeypatch.setattr(
        install_mcp_server.shutil,
        "which",
        lambda name: f"/usr/bin/{name}" if name in {"pkill", "systemctl"} else None,
    )
    monkeypatch.setattr(install_mcp_server.subprocess, "run", fake_run)

    install_mcp_server.refresh_accessibility_bus()

    assert calls == [
        [
            "/usr/bin/pkill",
            "-u",
            "bex",
            "-f",
            r"(^|/)at-spi2-registryd( |$)",
        ],
        ["/usr/bin/systemctl", "--user", "restart", "at-spi-dbus-bus.service"],
    ]
    outerr = capsys.readouterr()
    assert "warning: could not refresh user AT-SPI accessibility bus" in outerr.err
    assert "unit not found" in outerr.err


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
        lambda target, **_kwargs: restarted.append(target),
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
        lambda _target, **_kwargs: events.append("restart"),
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
    monkeypatch.delenv(_install_shared.BROWSER_SELECTION_ENV, raising=False)
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"

    config_path = install_mcp_server.install_opencode(target_dir, client_path)

    config = json.loads(config_path.read_text(encoding="utf-8"))
    env = config["mcp"]["sky_cua"]["environment"]
    assert env["SKY_CUA_REPO_ROOT"] == str(_install_shared.REPO_ROOT)
    assert _install_shared.BROWSER_SELECTION_ENV not in env


def test_bundle_mode_opencode_pins_bundle_resource_root(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(_install_shared.BROWSER_SELECTION_ENV, raising=False)
    bundle_root = tmp_path / "package" / "plugin" / "sky-cua"
    app_index = bundle_root / "resources" / "app-instructions" / "index.json"
    app_index.parent.mkdir(parents=True)
    app_index.write_text('{"entries":[]}\n', encoding="utf-8")
    client_path = tmp_path / "installed" / "bin" / "sky-cua-client"
    monkeypatch.setattr(
        install_mcp_server,
        "install_bundle_binaries",
        lambda _bundle, _target: client_path,
    )

    _client, config_path = install_mcp_server.install_local_mcp_server(
        tmp_path / "installed",
        "opencode",
        bundle_root=bundle_root,
    )

    env = json.loads(config_path.read_text(encoding="utf-8"))["mcp"]["sky_cua"]["environment"]
    resource_root = Path(env["SKY_CUA_REPO_ROOT"])
    assert resource_root == bundle_root.resolve()
    assert (resource_root / "resources" / "app-instructions" / "index.json").exists()


def test_bundle_mode_generic_mcp_config_pins_bundle_resource_root(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bundle_root = tmp_path / "package" / "plugin" / "sky-cua"
    app_index = bundle_root / "resources" / "app-instructions" / "index.json"
    app_index.parent.mkdir(parents=True)
    app_index.write_text('{"entries":[]}\n', encoding="utf-8")
    client_path = tmp_path / "installed" / "bin" / "sky-cua-client"
    monkeypatch.setattr(
        install_mcp_server,
        "install_bundle_binaries",
        lambda _bundle, _target: client_path,
    )

    _client, config_path = install_mcp_server.install_local_mcp_server(
        tmp_path / "installed",
        "generic",
        bundle_root=bundle_root,
    )

    server = json.loads(config_path.read_text(encoding="utf-8"))["mcpServers"]["computer-use"]
    resource_root = Path(server["env"]["SKY_CUA_REPO_ROOT"])
    assert resource_root == bundle_root.resolve()
    assert "SKY_CUA_REPO_ROOT" in server["env_vars"]
    assert (resource_root / "resources" / "app-instructions" / "index.json").exists()


def test_mcp_launch_policy_uses_cli_persisted_env_default_precedence(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    (target_dir / install_mcp_server.MCP_LAUNCH_POLICY_STATE).write_text(
        json.dumps(
            {
                "tool_profile": "compact",
                "browser_eval": "on",
                "model_supports_images": "false",
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.setenv(install_mcp_server.MCP_TOOL_PROFILE_ENV, "legacy")
    monkeypatch.setenv(install_mcp_server.MCP_BROWSER_EVAL_ENV, "off")
    monkeypatch.setenv(install_mcp_server.MCP_MODEL_SUPPORTS_IMAGES_ENV, "true")

    policy = install_mcp_server.resolve_mcp_launch_policy(
        target_dir,
        browser_eval="off",
    )

    assert policy.tool_profile == "compact"
    assert policy.browser_eval == "off"
    assert policy.model_supports_images == "false"


def test_mcp_launch_policy_rejects_invalid_recognized_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(install_mcp_server.MCP_TOOL_PROFILE_ENV, "tiny")

    with pytest.raises(ValueError, match="SKY_CUA_MCP_TOOL_PROFILE"):
        install_mcp_server.resolve_mcp_launch_policy(tmp_path / "installed")


def test_generic_mcp_config_pins_compact_launch_policy(tmp_path: Path) -> None:
    target_dir = tmp_path / "installed"
    client_path = target_dir / "bin" / "sky-cua-client"
    policy = install_mcp_server.McpLaunchPolicy(
        tool_profile="compact",
        browser_eval="on",
        model_supports_images="false",
    )

    config = install_mcp_server.generate_mcp_config(client_path, target_dir, launch_policy=policy)
    server = config["mcpServers"]["computer-use"]  # type: ignore[index]

    assert server["env"][install_mcp_server.MCP_TOOL_PROFILE_ENV] == "compact"  # type: ignore[index]
    assert server["env"][install_mcp_server.MCP_BROWSER_EVAL_ENV] == "on"  # type: ignore[index]
    assert server["env"][install_mcp_server.MCP_MODEL_SUPPORTS_IMAGES_ENV] == "false"  # type: ignore[index]
    for name in install_mcp_server.RECOGNIZED_MCP_LAUNCH_ENV:
        assert name in server["env_vars"]  # type: ignore[operator]


def test_codex_exec_plugin_mention_rejects_stale_compat_root(tmp_path: Path) -> None:
    codex_home = tmp_path / "codex-home"
    latest = codex_home / "plugins" / "cache" / "openai-bundled" / "computer-use" / "latest"
    latest.mkdir(parents=True)
    (latest / ".mcp.json").write_text(
        json.dumps(
            {
                "mcpServers": {
                    "computer-use": {
                        "command": str(tmp_path / "old" / "bin" / "sky-cua-client"),
                        "args": ["mcp"],
                    }
                }
            }
        ),
        encoding="utf-8",
    )

    assert _codex_exec.plugin_mention(codex_home) == _codex_exec.LOCAL_PLUGIN_MENTION

    payload = plugin_bundle.installed_plugin_root(codex_home)
    (payload / "bin").mkdir(parents=True)
    (latest / ".mcp.json").write_text(
        json.dumps(
            {
                "mcpServers": {
                    "computer-use": {
                        "command": str((payload / "bin" / "sky-cua-client").resolve()),
                        "args": ["mcp"],
                    }
                }
            }
        ),
        encoding="utf-8",
    )

    assert _codex_exec.plugin_mention(codex_home) == _codex_exec.COMPAT_PLUGIN_MENTION


def test_host_installers_do_not_inject_browser_selection_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Browser selection lives in the machine config file, not in every host
    # registration's environment.
    monkeypatch.setenv(_install_shared.BROWSER_SELECTION_ENV, "brave")
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"

    opencode_path = install_mcp_server.install_opencode(target_dir, client_path)
    desktop_path = install_mcp_server.install_claude_desktop(target_dir, client_path)

    opencode_env = json.loads(opencode_path.read_text(encoding="utf-8"))["mcp"]["sky_cua"][
        "environment"
    ]
    desktop_env = json.loads(desktop_path.read_text(encoding="utf-8"))["mcpServers"][
        "computer-use"
    ]["env"]
    assert _install_shared.BROWSER_SELECTION_ENV not in opencode_env
    assert _install_shared.BROWSER_SELECTION_ENV not in desktop_env


def test_claude_code_permissions_deny_builtin_and_approve_sky_cua(tmp_path: Path) -> None:
    claude_dir = tmp_path / ".claude"

    settings_path = install_mcp_server.configure_claude_code_permissions(claude_dir)

    assert settings_path == claude_dir / "settings.json"
    settings = json.loads((claude_dir / "settings.json").read_text(encoding="utf-8"))
    assert settings["permissions"]["deny"] == list(install_mcp_server.CLAUDE_CODE_DENY_RULES)
    assert settings["permissions"]["allow"] == list(install_mcp_server.CLAUDE_CODE_ALLOW_RULES)


def test_claude_code_permissions_preserve_existing_and_idempotent(tmp_path: Path) -> None:
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    settings_path = claude_dir / "settings.json"
    settings_path.write_text(
        json.dumps(
            {
                "model": "claude-fable-5",
                "permissions": {"deny": ["Bash(rm -rf /)"], "allow": ["Read"]},
            },
            indent=2,
        ),
        encoding="utf-8",
    )

    assert install_mcp_server.configure_claude_code_permissions(claude_dir) == settings_path
    settings = json.loads(settings_path.read_text(encoding="utf-8"))
    # Unrelated keys and pre-existing rules survive; new rules are appended.
    assert settings["model"] == "claude-fable-5"
    assert settings["permissions"]["deny"] == [
        "Bash(rm -rf /)",
        *install_mcp_server.CLAUDE_CODE_DENY_RULES,
    ]
    assert settings["permissions"]["allow"] == ["Read", *install_mcp_server.CLAUDE_CODE_ALLOW_RULES]

    # A second run adds no duplicates and does not rewrite the file.
    before = settings_path.read_text(encoding="utf-8")
    assert install_mcp_server.configure_claude_code_permissions(claude_dir) == settings_path
    assert settings_path.read_text(encoding="utf-8") == before


def test_claude_code_permissions_refuse_unparseable_settings(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    settings_path = claude_dir / "settings.json"
    settings_path.write_text("{ not json", encoding="utf-8")

    assert install_mcp_server.configure_claude_code_permissions(claude_dir) is None

    assert "fails JSON validation" in capsys.readouterr().err
    assert settings_path.read_text(encoding="utf-8") == "{ not json"


def test_claude_code_permissions_refuse_non_object_permissions(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    settings_path = claude_dir / "settings.json"
    settings_path.write_text(json.dumps({"permissions": ["not-an-object"]}), encoding="utf-8")
    original = settings_path.read_text(encoding="utf-8")

    assert install_mcp_server.configure_claude_code_permissions(claude_dir) is None

    assert "'permissions' is not a JSON object" in capsys.readouterr().err
    assert settings_path.read_text(encoding="utf-8") == original


def test_claude_code_permissions_refuse_non_utf8_settings(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    settings_path = claude_dir / "settings.json"
    settings_path.write_bytes(b"\xff\xfe not utf-8")

    assert install_mcp_server.configure_claude_code_permissions(claude_dir) is None

    assert "cannot read existing file" in capsys.readouterr().err
    assert settings_path.read_bytes() == b"\xff\xfe not utf-8"


def test_claude_code_permissions_refuse_non_list_rules(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    settings_path = claude_dir / "settings.json"
    settings_path.write_text(json.dumps({"permissions": {"deny": "all"}}), encoding="utf-8")
    original = settings_path.read_text(encoding="utf-8")

    assert install_mcp_server.configure_claude_code_permissions(claude_dir) is None

    assert "permissions.deny is not a JSON array" in capsys.readouterr().err
    assert settings_path.read_text(encoding="utf-8") == original


def test_claude_code_permissions_refuse_non_object_top_level(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    settings_path = claude_dir / "settings.json"
    settings_path.write_text(json.dumps([1, 2, 3]), encoding="utf-8")
    original = settings_path.read_text(encoding="utf-8")

    assert install_mcp_server.configure_claude_code_permissions(claude_dir) is None

    assert "top-level value is not a JSON object" in capsys.readouterr().err
    assert settings_path.read_text(encoding="utf-8") == original


def test_claude_code_install_writes_settings_permissions(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # No claude CLI on PATH: registration is skipped but the settings.json
    # permission policy is still written.
    monkeypatch.setattr(install_mcp_server.shutil, "which", lambda _name: None)
    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    claude_dir = tmp_path / ".claude"  # absent -> skills copy skipped, settings created

    install_mcp_server.install_claude_code(target_dir, client_path, claude_dir)

    settings = json.loads((claude_dir / "settings.json").read_text(encoding="utf-8"))
    assert settings["permissions"]["deny"] == list(install_mcp_server.CLAUDE_CODE_DENY_RULES)
    assert settings["permissions"]["allow"] == list(install_mcp_server.CLAUDE_CODE_ALLOW_RULES)


def test_install_local_mcp_server_threads_claude_config_dir(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The claude-code dispatch must forward claude_config_dir to install_claude_code.
    monkeypatch.delenv(_install_shared.BROWSER_SELECTION_ENV, raising=False)
    client_path = tmp_path / "bin" / "sky-cua-client"
    monkeypatch.setattr(install_mcp_server, "install_binaries", lambda _target: client_path)
    received: dict[str, object] = {}

    def spy(
        target_dir: Path,
        client: Path,
        claude_config_dir: Path | None = None,
        resource_root: Path | None = None,
        launch_policy: install_mcp_server.McpLaunchPolicy | None = None,
    ) -> Path:
        received["claude_config_dir"] = claude_config_dir
        received["resource_root"] = resource_root
        received["launch_policy"] = launch_policy
        return target_dir / "claude_code_mcp.json"

    monkeypatch.setattr(install_mcp_server, "install_claude_code", spy)
    config_dir = tmp_path / "custom-claude"

    install_mcp_server.install_local_mcp_server(
        tmp_path / "target", "claude-code", claude_config_dir=config_dir
    )

    assert received["claude_config_dir"] == config_dir
    assert received["resource_root"] == _install_shared.REPO_ROOT.resolve()
    assert isinstance(received["launch_policy"], install_mcp_server.McpLaunchPolicy)


def test_machine_config_seeding_writes_and_updates_browser(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import tomllib

    config_path = tmp_path / "sky-cua.toml"
    monkeypatch.setenv(_install_shared.MACHINE_CONFIG_PATH_ENV, str(config_path))
    monkeypatch.setenv(_install_shared.BROWSER_SELECTION_ENV, "brave")

    assert _install_shared.seed_machine_config_from_environment() == config_path
    assert tomllib.loads(config_path.read_text(encoding="utf-8"))["browser"] == "brave"

    # Existing unrelated keys survive a selection change.
    config_path.write_text('future_knob = 3\nbrowser = "chrome"\n', encoding="utf-8")
    monkeypatch.setenv(_install_shared.BROWSER_SELECTION_ENV, "brave")
    assert _install_shared.seed_machine_config_from_environment() == config_path
    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed == {"future_knob": 3, "browser": "brave"}

    # Same value is a no-op; unset env never writes.
    before = config_path.read_text(encoding="utf-8")
    assert _install_shared.seed_machine_config_from_environment() == config_path
    assert config_path.read_text(encoding="utf-8") == before
    monkeypatch.delenv(_install_shared.BROWSER_SELECTION_ENV)
    assert _install_shared.seed_machine_config_from_environment() is None


def test_machine_config_seeding_refuses_unparseable_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    config_path = tmp_path / "sky-cua.toml"
    config_path.write_text("browser = ", encoding="utf-8")
    monkeypatch.setenv(_install_shared.MACHINE_CONFIG_PATH_ENV, str(config_path))
    monkeypatch.setenv(_install_shared.BROWSER_SELECTION_ENV, "brave")

    assert _install_shared.seed_machine_config_from_environment() is None

    assert "fails TOML validation" in capsys.readouterr().err
    assert config_path.read_text(encoding="utf-8") == "browser = "


def test_pi_install_merges_mcp_config_and_copies_sky_cua_skills(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv(_install_shared.BROWSER_SELECTION_ENV, "brave")
    repo_root = tmp_path / "repo"
    for skill_name in _install_shared.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)

    target_dir = tmp_path / "installed"
    target_dir.mkdir()
    client_path = target_dir / "bin" / "sky-cua-client"
    agent_dir = tmp_path / "pi-agent"
    agent_dir.mkdir()
    (agent_dir / "mcp.json").write_text(
        json.dumps({"mcpServers": {"context7": {"command": "context7"}}}),
        encoding="utf-8",
    )
    stale_skill = agent_dir / "skills" / _install_shared.SKY_CUA_SKILLS[0]
    stale_skill.mkdir(parents=True)
    (stale_skill / "obsolete.md").write_text("old", encoding="utf-8")
    unrelated_skill = agent_dir / "skills" / "other-skill"
    unrelated_skill.mkdir(parents=True)
    (unrelated_skill / "SKILL.md").write_text("# other\n", encoding="utf-8")

    snippet_path = install_mcp_server.install_pi(target_dir, client_path, agent_dir)

    wrapper = target_dir / "pi_mcp_wrapper.sh"
    wrapper_text = wrapper.read_text(encoding="utf-8")
    assert _install_shared.BROWSER_SELECTION_ENV not in wrapper_text
    assert snippet_path == target_dir / "pi_mcp.json"
    snippet = json.loads(snippet_path.read_text(encoding="utf-8"))
    assert snippet["mcpServers"]["sky_cua"]["command"] == str(wrapper)

    merged = json.loads((agent_dir / "mcp.json").read_text(encoding="utf-8"))
    assert merged["mcpServers"]["context7"] == {"command": "context7"}
    assert merged["mcpServers"]["sky_cua"]["command"] == str(wrapper)
    for skill_name in _install_shared.SKY_CUA_SKILLS:
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
    for skill_name in _install_shared.SKY_CUA_SKILLS:
        skill_dir = repo_root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text(f"# {skill_name}\n", encoding="utf-8")
    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)

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

    def fake_install_openclaw(
        target: Path,
        client: Path,
        openclaw_dir: Path,
        openclaw_bin: str = "openclaw",
        resource_root: Path | None = None,
        launch_env: dict[str, str] | None = None,
    ) -> Path:
        _ = openclaw_bin, resource_root, launch_env
        installed.append((target, client, openclaw_dir))
        return target / "openclaw_mcp.json"

    monkeypatch.setattr(install_mcp_server, "install_openclaw", fake_install_openclaw)
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

    monkeypatch.setattr(_install_shared.os, "replace", fail_replace)

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
    monkeypatch.setattr(_install_shared, "SKY_CUA_SKILLS", (skill_name,))
    repo_root = tmp_path / "repo"
    source = repo_root / "skills" / skill_name
    source.mkdir(parents=True)
    (source / "SKILL.md").write_text("# new\n", encoding="utf-8")
    monkeypatch.setattr(_install_shared, "REPO_ROOT", repo_root)

    skills_dir = tmp_path / "skills"
    destination = skills_dir / skill_name
    destination.mkdir(parents=True)
    (destination / "SKILL.md").write_text("# old\n", encoding="utf-8")

    def fail_copytree(_source: Path, destination: Path) -> None:
        destination.mkdir(parents=True)
        (destination / "partial.md").write_text("partial\n", encoding="utf-8")
        raise OSError("copy failed")

    monkeypatch.setattr(_install_shared.shutil, "copytree", fail_copytree)

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
    real_replace = _install_shared.os.replace

    def fail_new_tree_replace(source_path: Path, destination_path: Path) -> None:
        if (
            Path(source_path).name.startswith(".skill.tmp-")
            and Path(destination_path) == destination
        ):
            raise OSError("replace failed")
        real_replace(source_path, destination_path)

    monkeypatch.setattr(_install_shared.os, "replace", fail_new_tree_replace)

    with pytest.raises(OSError, match="replace failed"):
        _install_shared.replace_tree_atomically(source, destination)

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


def test_install_mcp_server_kwin_effect_failure_returns_nonzero(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    def fake_deploy(**_kwargs: object) -> kwin_effect.ReloadOutcome:
        return kwin_effect.ReloadOutcome(
            converged=False,
            loaded=False,
            expected_build_id="new",
            running_build_id="old",
            effect_id="sky-cua-agent-cursor-000004",
            rollback_effect_id="sky-cua-agent-cursor-000003",
            live_load_attempted=True,
        )

    monkeypatch.setattr(install_mcp_server, "deploy_kwin_effect", fake_deploy)
    monkeypatch.setattr(
        install_mcp_server, "print_kwin_effect_deploy_outcome", lambda _outcome: None
    )
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

    assert install_mcp_server.main() == 1
    assert "did not converge" in capsys.readouterr().err
