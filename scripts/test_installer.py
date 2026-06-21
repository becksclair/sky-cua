"""Tests for the one-shot installer orchestration logic."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

import installer


def completed(returncode: int = 0) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess(args=[], returncode=returncode, stdout="", stderr="")


def test_claude_code_permissions_status_reports_states(tmp_path: Path) -> None:
    import json

    settings_path = tmp_path / "settings.json"

    # Absent file -> skipped.
    assert installer.claude_code_permissions_status(settings_path) == (
        "skipped",
        "settings.json not found",
    )

    # Both server-scope rules present -> ok.
    settings_path.write_text(
        json.dumps({"permissions": {"deny": ["mcp__computer-use"], "allow": ["mcp__sky-cua"]}}),
        encoding="utf-8",
    )
    status, detail = installer.claude_code_permissions_status(settings_path)
    assert status == "ok"
    assert "denied" in detail and "auto-approved" in detail

    # Missing deny rule -> failed naming the gap.
    settings_path.write_text(
        json.dumps({"permissions": {"allow": ["mcp__sky-cua"]}}), encoding="utf-8"
    )
    status, detail = installer.claude_code_permissions_status(settings_path)
    assert status == "failed"
    assert "computer-use deny rule" in detail


def test_claude_code_permissions_status_unreadable(tmp_path: Path) -> None:
    settings_path = tmp_path / "settings.json"
    settings_path.write_text("{ not json", encoding="utf-8")
    status, detail = installer.claude_code_permissions_status(settings_path)
    assert status == "failed"
    assert "unreadable" in detail


def test_claude_code_permissions_status_handles_non_utf8(tmp_path: Path) -> None:
    # A non-UTF-8 settings.json must report failed, not crash the health phase
    # with an unhandled UnicodeDecodeError (a ValueError, not an OSError).
    settings_path = tmp_path / "settings.json"
    settings_path.write_bytes(b"\xff\xfe not utf-8")
    status, detail = installer.claude_code_permissions_status(settings_path)
    assert status == "failed"
    assert "unreadable" in detail


def test_claude_code_health_attests_installer_rule_constants(tmp_path: Path) -> None:
    # installer.py's health check re-encodes the server-scope rule literals
    # rather than importing install_mcp_server (lean orchestrator, subprocess
    # boundary). Pin the two sides through BEHAVIOR: a settings file written
    # from the writer's canonical constants must read as healthy, so renaming
    # either side without the other flips this status to "failed".
    import json

    import install_mcp_server

    settings_path = tmp_path / "settings.json"
    settings_path.write_text(
        json.dumps(
            {
                "permissions": {
                    "deny": list(install_mcp_server.CLAUDE_CODE_DENY_RULES),
                    "allow": list(install_mcp_server.CLAUDE_CODE_ALLOW_RULES),
                }
            }
        ),
        encoding="utf-8",
    )

    # Pin the constants contract via status only; the detail wording is not the
    # axis this test guards.
    status, _detail = installer.claude_code_permissions_status(settings_path)
    assert status == "ok"


def test_run_health_phase_claude_cli_present_reports_both(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import json

    # With the claude CLI present, the mcp-list probe runs AND the permissions
    # attestation runs; both PhaseResults must appear (covers the CLI-present arm
    # of the restructured health block).
    monkeypatch.setattr(installer.shutil, "which", lambda _name: "/usr/bin/claude")
    monkeypatch.setattr(
        installer.subprocess,
        "run",
        lambda *_args, **_kwargs: subprocess.CompletedProcess([], 0, stdout="sky-cua\n", stderr=""),
    )
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    (claude_dir / "settings.json").write_text(
        json.dumps({"permissions": {"deny": ["mcp__computer-use"], "allow": ["mcp__sky-cua"]}}),
        encoding="utf-8",
    )

    results = installer.run_health_phase(
        agents=["claude-code"],
        target_dir=tmp_path / "empty",
        claude_dir=claude_dir,
        runner=lambda *_args, **_kwargs: completed(0),
    )

    by_name = {result.name: result.status for result in results}
    assert by_name["health:claude-code"] == "ok"
    assert by_name["health:claude-code-permissions"] == "ok"


def test_main_resolves_relative_claude_config_dir_for_health(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A relative --claude-config-dir must reach run_health_phase as a resolved
    # absolute path, matching how the install_mcp_server.py subprocess resolves
    # it, so the health check attests the file the install actually wrote.
    captured: dict[str, object] = {}
    monkeypatch.setattr(installer, "detect_agents", lambda: {"claude-code": True})
    monkeypatch.setattr(
        installer, "run_system_deps_phase", lambda **_k: installer.PhaseResult("system-deps", "ok")
    )
    monkeypatch.setattr(
        installer, "run_build_phase", lambda **_k: installer.PhaseResult("build", "ok")
    )
    monkeypatch.setattr(
        installer, "run_codex_phase", lambda **_k: installer.PhaseResult("codex", "skipped")
    )
    monkeypatch.setattr(
        installer,
        "run_agent_phase",
        lambda host, **_k: installer.PhaseResult(f"agent:{host}", "ok"),
    )
    monkeypatch.setattr(
        installer, "run_kwin_phase", lambda **_k: installer.PhaseResult("kwin-effect", "skipped")
    )

    def fake_health(**kwargs: object) -> list[installer.PhaseResult]:
        captured["claude_dir"] = kwargs.get("claude_dir")
        return []

    monkeypatch.setattr(installer, "run_health_phase", fake_health)

    installer.main(["--agents", "claude-code", "--claude-config-dir", "./relcfg"])

    assert captured["claude_dir"] == (Path.cwd() / "relcfg").resolve()


def test_claude_code_permissions_status_non_object_top_level(tmp_path: Path) -> None:
    import json

    # Top-level JSON that is not an object (e.g. a list) falls through to
    # "failed, missing both rules" rather than crashing.
    settings_path = tmp_path / "settings.json"
    settings_path.write_text(json.dumps([1, 2, 3]), encoding="utf-8")
    status, detail = installer.claude_code_permissions_status(settings_path)
    assert status == "failed"
    assert "computer-use deny rule" in detail
    assert "sky-cua allow rule" in detail


def test_run_health_phase_attests_permissions_at_claude_dir(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import json

    # No claude CLI -> the mcp-list probe is skipped; fake runner -> no real
    # doctor subprocess. The permissions check must read the supplied claude_dir
    # (not the real ~/.claude) and report ok.
    monkeypatch.setattr(installer.shutil, "which", lambda _name: None)
    claude_dir = tmp_path / ".claude"
    claude_dir.mkdir()
    (claude_dir / "settings.json").write_text(
        json.dumps({"permissions": {"deny": ["mcp__computer-use"], "allow": ["mcp__sky-cua"]}}),
        encoding="utf-8",
    )

    results = installer.run_health_phase(
        agents=["claude-code"],
        target_dir=tmp_path / "empty",
        claude_dir=claude_dir,
        runner=lambda *_args, **_kwargs: completed(0),
    )

    by_name = {result.name: result.status for result in results}
    assert by_name["health:claude-code-permissions"] == "ok"
    assert "health:claude-code" not in by_name  # CLI absent -> mcp-list probe skipped


def test_run_agent_phase_threads_claude_config_dir(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    calls: list[dict[str, object]] = []

    def fake_install(
        target_dir: Path,
        host: str,
        *,
        restart_runtime: bool = False,
        bundle_root: Path | None = None,
        claude_config_dir: Path | None = None,
        **_kwargs: object,
    ) -> tuple[Path, Path]:
        calls.append(
            {
                "host": host,
                "claude_config_dir": claude_config_dir,
                "bundle_root": bundle_root,
                "restart_runtime": restart_runtime,
            }
        )
        return target_dir / "bin" / "sky-cua-client", target_dir / "cfg.json"

    monkeypatch.setattr(installer, "install_local_mcp_server", fake_install)

    claude_dir = tmp_path / ".claude"
    bundle = tmp_path / "bundle"
    installer.run_agent_phase(
        "claude-code", bundle_root=bundle, target_dir=tmp_path / "t", claude_config_dir=claude_dir
    )
    assert calls[0]["host"] == "claude-code"
    assert calls[0]["claude_config_dir"] == claude_dir
    assert calls[0]["bundle_root"] == bundle
    assert calls[0]["restart_runtime"] is True

    # Non-claude hosts never receive the claude config dir.
    calls.clear()
    installer.run_agent_phase(
        "opencode", bundle_root=bundle, target_dir=tmp_path / "t", claude_config_dir=claude_dir
    )
    assert calls[0]["claude_config_dir"] is None


def test_detect_package_manager_prefers_pacman() -> None:
    assert installer.detect_package_manager(lambda name: "/usr/bin/" + name) == "pacman"
    assert (
        installer.detect_package_manager(
            lambda name: "/usr/bin/apt-get" if name == "apt-get" else None
        )
        == "apt"
    )
    assert installer.detect_package_manager(lambda name: None) is None


def test_missing_packages_filters_installed() -> None:
    packages = {"pacman": ("alpha", "beta", "gamma")}
    missing = installer.missing_packages("pacman", lambda package: package == "beta", packages)
    assert missing == ["alpha", "gamma"]
    assert installer.missing_packages("apt", lambda package: False, packages) == []


def test_detect_agents_from_home_layout(tmp_path: Path) -> None:
    (tmp_path / ".claude").mkdir()
    (tmp_path / ".pi" / "agent").mkdir(parents=True)
    (tmp_path / ".config" / "Claude").mkdir(parents=True)

    detected = installer.detect_agents(home=tmp_path, which=lambda name: None)
    assert detected["claude-code"] is True
    assert detected["claude-desktop"] is True
    assert detected["pi"] is True
    assert detected["codex"] is False
    assert detected["opencode"] is False
    assert detected["openclaw"] is False


def test_detect_agents_from_path_commands(tmp_path: Path) -> None:
    available = {"codex", "opencode", "openclaw"}
    detected = installer.detect_agents(
        home=tmp_path,
        which=lambda name: "/usr/bin/" + name if name in available else None,
    )
    assert detected["codex"] is True
    assert detected["opencode"] is True
    assert detected["openclaw"] is True
    assert detected["claude-code"] is False
    assert detected["pi"] is False


def test_select_agents_explicit_request_wins() -> None:
    detected: dict[str, bool] = dict.fromkeys(installer.KNOWN_AGENTS, True)
    assert installer.select_agents("codex, pi", detected) == ["codex", "pi"]
    assert installer.select_agents("codex,codex", detected) == ["codex"]


def test_select_agents_rejects_unknown_names() -> None:
    with pytest.raises(ValueError, match="unknown agent 'cursor'"):
        installer.select_agents("cursor", {})


def test_select_agents_defaults_to_detected_order() -> None:
    detected = {"pi": True, "codex": True, "opencode": False}
    assert installer.select_agents(None, detected) == ["codex", "pi"]


def test_missing_required_commands() -> None:
    assert installer.missing_required_commands(lambda name: None) == ["cargo", "git"]
    assert installer.missing_required_commands(lambda name: "/usr/bin/" + name) == []


def test_resolve_mode_explicit_overrides_detection() -> None:
    assert (
        installer.resolve_mode("repo", repo_root=Path("/no/git"), which=lambda _n: None) == "repo"
    )
    assert installer.resolve_mode("bundle") == "bundle"


def test_resolve_mode_auto_picks_bundle_without_git(tmp_path: Path) -> None:
    # No .git -> bundle (a release package has no checkout), even with cargo.
    assert (
        installer.resolve_mode("auto", repo_root=tmp_path, which=lambda _n: "/usr/bin/cargo")
        == "bundle"
    )
    # .git present but no cargo -> repo, so the required-command check reports
    # missing cargo/git instead of installing a stale dist/plugin bundle.
    (tmp_path / ".git").mkdir()
    assert installer.resolve_mode("auto", repo_root=tmp_path, which=lambda _n: None) == "repo"
    # .git present and cargo present -> repo.
    assert (
        installer.resolve_mode("auto", repo_root=tmp_path, which=lambda _n: "/usr/bin/cargo")
        == "repo"
    )


def test_build_phase_reports_failure() -> None:
    calls: list[list[str]] = []

    def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return completed(returncode=1)

    result = installer.run_build_phase(mode="repo", skip=False, runner=runner)
    assert result.failed
    assert calls and calls[0][1].endswith("build_plugin.py")


def test_build_phase_skip() -> None:
    result = installer.run_build_phase(mode="repo", skip=True)
    assert result.status == "skipped"


def test_build_phase_bundle_mode_skips_without_running() -> None:
    calls: list[list[str]] = []

    def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return completed()

    result = installer.run_build_phase(mode="bundle", skip=False, runner=runner)
    assert result.status == "skipped"
    assert not calls  # bundle mode never shells out to build_plugin.py


def test_codex_phase_skipped_when_not_selected(tmp_path: Path) -> None:
    result = installer.run_codex_phase(
        enabled=False, bundle_root=tmp_path / "bundle", codex_home=tmp_path / "codex"
    )
    assert result.status == "skipped"


def test_codex_phase_fails_when_bundle_missing(tmp_path: Path) -> None:
    result = installer.run_codex_phase(
        enabled=True, bundle_root=tmp_path / "nope", codex_home=tmp_path / "codex"
    )
    assert result.failed
    assert "bundle not found" in result.detail


def test_codex_phase_materializes_compat_plugin_from_bundle(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The Codex phase installs the bundle and runs the bundled browser preflight
    # (which materializes the compat plugin) - no marketplace, no codex CLI.
    bundle_root = tmp_path / "bundle"
    bundle_root.mkdir()
    codex_home = tmp_path / "codex-home"
    calls: dict[str, object] = {}

    monkeypatch.setattr(installer, "stop_unix_runtime_processes", lambda _roots: None)
    monkeypatch.setattr(installer, "stop_windows_cache_processes", lambda _root: None)
    monkeypatch.setattr(
        installer,
        "installed_plugin_root",
        lambda home: home / "plugins" / "cache" / "local" / "sky-cua" / "local",
    )
    monkeypatch.setattr(
        installer,
        "install_bundle",
        lambda src, dest, symlink: calls.update({"install_bundle_src": src, "install_dest": dest}),
    )
    monkeypatch.setattr(
        installer,
        "run_browser_preflight",
        lambda dest, home: calls.update({"preflight": (dest, home)}),
    )
    monkeypatch.setattr(installer, "compat_plugin_targets_payload", lambda _home, _dest: True)
    monkeypatch.setattr(
        installer,
        "update_codex_config",
        lambda path, *, compat_enablement: calls.update(
            {"config_path": path, "compat": compat_enablement}
        ),
    )

    result = installer.run_codex_phase(enabled=True, bundle_root=bundle_root, codex_home=codex_home)
    assert result.status == "ok"
    assert calls["install_bundle_src"] == bundle_root
    assert "preflight" in calls
    assert calls["compat"] is True
    assert calls["config_path"] == codex_home / "config.toml"


def test_codex_phase_channel_id_fallback_when_no_compat_root(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Off-compat (Windows / bundle without openai-bundled resources): no compat
    # root materializes, so the phase enables sky-cua@local directly and says so
    # in its detail. Linux CI never reaches this branch through a real install.
    bundle_root = tmp_path / "bundle"
    bundle_root.mkdir()
    codex_home = tmp_path / "codex-home"
    calls: dict[str, object] = {}

    monkeypatch.setattr(installer, "stop_unix_runtime_processes", lambda _roots: None)
    monkeypatch.setattr(installer, "stop_windows_cache_processes", lambda _root: None)
    monkeypatch.setattr(installer, "installed_plugin_root", lambda home: home / "cache" / "local")
    monkeypatch.setattr(installer, "install_bundle", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(installer, "run_browser_preflight", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(installer, "compat_plugin_targets_payload", lambda _home, _dest: False)
    monkeypatch.setattr(
        installer,
        "update_codex_config",
        lambda _path, *, compat_enablement: calls.update({"compat": compat_enablement}),
    )

    result = installer.run_codex_phase(enabled=True, bundle_root=bundle_root, codex_home=codex_home)
    assert result.status == "ok"
    assert calls["compat"] is False
    assert "fallback" in result.detail


def test_codex_phase_converges_retired_channels_on_in_place_upgrade(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # In-place repo-mode upgrade of a box left in the old Heliasar-enabled state:
    # the real update_codex_config must disable the retired stanza so Codex does
    # not end up with two enabled computer-use plugin ids. update_codex_config is
    # intentionally NOT monkeypatched here - this pins the live config write that
    # routes the installer through the same convergence as deploy_plugin.
    import tomllib

    bundle_root = tmp_path / "bundle"
    bundle_root.mkdir()
    codex_home = tmp_path / "codex-home"
    codex_home.mkdir()
    (codex_home / "config.toml").write_text(
        '[plugins."sky-cua@Heliasar"]\nenabled = true\n', encoding="utf-8"
    )

    monkeypatch.setattr(installer, "stop_unix_runtime_processes", lambda _roots: None)
    monkeypatch.setattr(installer, "stop_windows_cache_processes", lambda _root: None)
    monkeypatch.setattr(installer, "installed_plugin_root", lambda home: home / "cache" / "local")
    monkeypatch.setattr(installer, "install_bundle", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(installer, "run_browser_preflight", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(installer, "compat_plugin_targets_payload", lambda _home, _dest: True)

    result = installer.run_codex_phase(enabled=True, bundle_root=bundle_root, codex_home=codex_home)

    assert result.status == "ok"
    parsed = tomllib.loads((codex_home / "config.toml").read_text(encoding="utf-8"))
    assert parsed["plugins"]["sky-cua@Heliasar"]["enabled"] is False
    assert parsed["plugins"]["computer-use@openai-bundled"]["enabled"] is True


def test_codex_phase_failed_result_when_config_write_raises(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # A config-write failure must surface as a failed PhaseResult - preserving the
    # per-phase summary contract - not an unhandled exception that crashes the
    # installer mid-run. The config write lives inside run_codex_phase's try/except
    # alongside install+preflight, matching the old subprocess lane's behavior.
    bundle_root = tmp_path / "bundle"
    bundle_root.mkdir()
    codex_home = tmp_path / "codex-home"

    monkeypatch.setattr(installer, "stop_unix_runtime_processes", lambda _roots: None)
    monkeypatch.setattr(installer, "stop_windows_cache_processes", lambda _root: None)
    monkeypatch.setattr(installer, "installed_plugin_root", lambda home: home / "cache" / "local")
    monkeypatch.setattr(installer, "install_bundle", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(installer, "run_browser_preflight", lambda *_args, **_kwargs: None)
    monkeypatch.setattr(installer, "compat_plugin_targets_payload", lambda _home, _dest: True)

    def boom(*_args: object, **_kwargs: object) -> None:
        raise OSError("config write failed")

    monkeypatch.setattr(installer, "update_codex_config", boom)

    result = installer.run_codex_phase(enabled=True, bundle_root=bundle_root, codex_home=codex_home)

    assert result.failed
    assert "config write failed" in result.detail


def test_agent_phase_failed_result_when_install_raises(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The agent phase wraps install_local_mcp_server so a registration failure is a
    # failed PhaseResult, not a crash that drops the summary and later phases.
    def boom(*_args: object, **_kwargs: object) -> None:
        raise RuntimeError("registration failed")

    monkeypatch.setattr(installer, "install_local_mcp_server", boom)

    result = installer.run_agent_phase(
        "opencode", bundle_root=tmp_path / "bundle", target_dir=tmp_path / "target"
    )

    assert result.failed
    assert "registration failed" in result.detail


def test_kwin_phase_skipped_by_default(tmp_path: Path) -> None:
    result = installer.run_kwin_phase(enabled=False, target_dir=tmp_path)
    assert result.status == "skipped"


def test_kwin_phase_fails_when_rotating_live_reload_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import _kwin_effect as kwin_effect

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

    monkeypatch.setattr(kwin_effect, "deploy_kwin_effect", fake_deploy)

    result = installer.run_kwin_phase(enabled=True, target_dir=tmp_path)

    assert result.status == "failed"
    assert "did not converge" in result.detail


def test_main_dry_run_lists_phases(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    def no_agents_detected(*args: object, **kwargs: object) -> dict[str, bool]:
        return dict.fromkeys(installer.KNOWN_AGENTS, False)

    monkeypatch.setattr(installer, "detect_agents", no_agents_detected)
    exit_code = installer.main(["--dry-run", "--agents", "codex,pi"])
    output = capsys.readouterr().out
    assert exit_code == 0
    assert "codex" in output
    assert "agent:pi" in output
    assert "health" in output


def test_main_skip_health_does_not_run_health_phase(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    called = False

    monkeypatch.setattr(installer, "detect_agents", lambda: {"opencode": True})
    monkeypatch.setattr(
        installer, "run_system_deps_phase", lambda **_k: installer.PhaseResult("system-deps", "ok")
    )
    monkeypatch.setattr(
        installer, "run_build_phase", lambda **_k: installer.PhaseResult("build", "ok")
    )
    monkeypatch.setattr(
        installer, "run_codex_phase", lambda **_k: installer.PhaseResult("codex", "skipped")
    )
    monkeypatch.setattr(
        installer,
        "run_agent_phase",
        lambda host, **_k: installer.PhaseResult(f"agent:{host}", "ok"),
    )
    monkeypatch.setattr(
        installer, "run_kwin_phase", lambda **_k: installer.PhaseResult("kwin-effect", "skipped")
    )

    def fake_health(**_kwargs: object) -> list[installer.PhaseResult]:
        nonlocal called
        called = True
        return [installer.PhaseResult("health:doctor", "failed")]

    monkeypatch.setattr(installer, "run_health_phase", fake_health)

    exit_code = installer.main(["--agents", "opencode", "--skip-health"])
    output = capsys.readouterr().out

    assert exit_code == 0
    assert called is False
    assert "health" in output
    assert "--skip-health" in output


def test_main_rejects_unknown_agent(capsys: pytest.CaptureFixture[str]) -> None:
    exit_code = installer.main(["--dry-run", "--agents", "nope"])
    assert exit_code == 2
    assert "unknown agent" in capsys.readouterr().err
