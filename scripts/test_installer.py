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


def test_run_agent_phase_threads_claude_config_dir(tmp_path: Path) -> None:
    captured: list[list[str]] = []

    def fake_runner(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        captured.append(command)
        return completed(0)

    claude_dir = tmp_path / ".claude"
    installer.run_agent_phase(
        "claude-code",
        target_dir=tmp_path / "t",
        claude_config_dir=claude_dir,
        runner=fake_runner,
    )
    assert captured[0][-2:] == ["--claude-config-dir", str(claude_dir)]

    # Non-claude hosts never receive the flag.
    captured.clear()
    installer.run_agent_phase(
        "opencode",
        target_dir=tmp_path / "t",
        claude_config_dir=claude_dir,
        runner=fake_runner,
    )
    assert "--claude-config-dir" not in captured[0]


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


def test_build_phase_reports_failure() -> None:
    calls: list[list[str]] = []

    def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return completed(returncode=1)

    result = installer.run_build_phase(skip=False, runner=runner)
    assert result.failed
    assert calls and calls[0][1].endswith("build_plugin.py")


def test_build_phase_skip() -> None:
    result = installer.run_build_phase(skip=True)
    assert result.status == "skipped"


def test_codex_phase_skipped_when_not_selected() -> None:
    result = installer.run_codex_phase(
        enabled=False,
        codex_home=Path("/tmp/codex"),
        marketplace_root=None,
        marketplace_source=None,
    )
    assert result.status == "skipped"


def test_codex_phase_passes_marketplace_arguments(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(installer.shutil, "which", lambda name: "/usr/bin/codex")
    calls: list[list[str]] = []

    def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return completed()

    result = installer.run_codex_phase(
        enabled=True,
        codex_home=Path("/tmp/codex-home"),
        marketplace_root=Path("/tmp/marketplace"),
        marketplace_source="example/source",
        runner=runner,
    )
    assert result.status == "ok"
    command = calls[0]
    assert command[1].endswith("setup_heliasar_marketplace.py")
    assert "--codex-home" in command and "/tmp/codex-home" in command
    assert "--marketplace-root" in command and "/tmp/marketplace" in command
    assert "--marketplace-source" in command and "example/source" in command


def test_agent_phase_invokes_install_mcp_server() -> None:
    calls: list[list[str]] = []

    def runner(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return completed()

    result = installer.run_agent_phase("pi", target_dir=Path("/tmp/target"), runner=runner)
    assert result.status == "ok"
    command = calls[0]
    assert command[1].endswith("install_mcp_server.py")
    assert command[command.index("--host") : command.index("--host") + 2] == ["--host", "pi"]
    assert "--restart-runtime" in command


def test_kwin_phase_skipped_by_default(tmp_path: Path) -> None:
    result = installer.run_kwin_phase(enabled=False, target_dir=tmp_path)
    assert result.status == "skipped"


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


def test_main_rejects_unknown_agent(capsys: pytest.CaptureFixture[str]) -> None:
    exit_code = installer.main(["--dry-run", "--agents", "nope"])
    assert exit_code == 2
    assert "unknown agent" in capsys.readouterr().err
