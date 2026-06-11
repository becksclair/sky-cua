"""Tests for marketplace publishing and setup helpers."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

import _install_shared
import _plugin_bundle as plugin_bundle
import publish_marketplace_release
import setup_heliasar_marketplace
from _plugin_bundle import (
    runtime_binary_names,
)
from _test_support import (
    write_minimal_bundle,
)


def test_current_tag_normalizes_full_tag_ref(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GITHUB_REF_NAME", "refs/tags/v1.2.3")

    assert publish_marketplace_release.current_tag() == "v1.2.3"


def test_publish_marketplace_detects_staged_plugin_changes(tmp_path: Path) -> None:
    repo_root = tmp_path / "marketplace"
    plugin_root = repo_root / "plugins" / plugin_bundle.PLUGIN_NAME
    plugin_root.mkdir(parents=True)
    (repo_root / ".agents" / "plugins").mkdir(parents=True)
    (repo_root / ".agents" / "plugins" / "marketplace.json").write_text(
        "{}",
        encoding="utf-8",
    )
    (plugin_root / "README.md").write_text("initial\n", encoding="utf-8")
    subprocess.run(["git", "init", "-b", "main"], cwd=repo_root, check=True)
    subprocess.run(
        ["git", "-c", "user.name=test", "-c", "user.email=test@example.invalid", "add", "."],
        cwd=repo_root,
        check=True,
    )
    subprocess.run(
        [
            "git",
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
        cwd=repo_root,
        check=True,
        stdout=subprocess.DEVNULL,
    )

    (plugin_root / "README.md").write_text("updated\n", encoding="utf-8")

    assert publish_marketplace_release.git_has_head(repo_root)
    assert publish_marketplace_release.git_has_changes(repo_root)


def test_publish_marketplace_preflights_git_repo_before_writing(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    marketplace_root = tmp_path / "marketplace"
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    _pin_repo_root(monkeypatch, tmp_path)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "publish_marketplace_release.py",
            "--no-build",
            "--skip-codex-install",
            "--no-push",
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
        ],
    )

    with pytest.raises(RuntimeError, match="published git repository"):
        publish_marketplace_release.main()

    assert not marketplace_root.exists()


def _init_marketplace_repo(repo_root: Path) -> None:
    repo_root.mkdir(parents=True)
    subprocess.run(
        ["git", "init", "-b", "main"],
        cwd=repo_root,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    subprocess.run(["git", "config", "user.name", "test"], cwd=repo_root, check=True)
    subprocess.run(
        ["git", "config", "user.email", "test@example.invalid"],
        cwd=repo_root,
        check=True,
    )
    (repo_root / "README.md").write_text("marketplace\n", encoding="utf-8")
    subprocess.run(["git", "add", "."], cwd=repo_root, check=True)
    subprocess.run(
        ["git", "commit", "-m", "initial"],
        cwd=repo_root,
        check=True,
        stdout=subprocess.DEVNULL,
    )


def _pin_repo_root(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Point REPO_ROOT at tmp so the stale-bundle guard never sees the real build."""
    monkeypatch.setattr(_install_shared, "REPO_ROOT", tmp_path / "repo")


def _stub_codex_install_steps(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        publish_marketplace_release,
        "resolve_codex_bin",
        lambda _codex_bin: Path("/usr/bin/codex"),
    )
    monkeypatch.setattr(
        publish_marketplace_release,
        "configure_marketplace",
        lambda *args, **kwargs: None,
    )
    monkeypatch.setattr(
        publish_marketplace_release,
        "install_with_codex",
        lambda *args, **kwargs: None,
    )
    monkeypatch.setattr(
        publish_marketplace_release,
        "update_codex_config",
        lambda *args, **kwargs: None,
    )
    monkeypatch.setattr(
        publish_marketplace_release,
        "reload_mcp_servers",
        lambda *args, **kwargs: None,
    )


def test_publish_refreshes_local_install_with_runtime_restart(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    marketplace_root = tmp_path / "marketplace"
    _init_marketplace_repo(marketplace_root)
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())
    local_install_dir = tmp_path / "local-install"

    _pin_repo_root(monkeypatch, tmp_path)
    _stub_codex_install_steps(monkeypatch)
    local_calls: list[tuple[Path, str, bool, Path | None]] = []

    def fake_local_install(
        target_dir: Path,
        host: str,
        *,
        openclaw_dir: Path | None = None,
        restart_runtime: bool = False,
        bundle_root: Path | None = None,
    ) -> tuple[Path, Path]:
        del openclaw_dir
        local_calls.append((target_dir, host, restart_runtime, bundle_root))
        return target_dir / "bin" / "sky-cua-client", target_dir / "claude_code_mcp.json"

    monkeypatch.setattr(
        publish_marketplace_release,
        "install_local_mcp_server",
        fake_local_install,
    )
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "publish_marketplace_release.py",
            "--no-build",
            "--no-push",
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
            "--local-install-dir",
            str(local_install_dir),
        ],
    )

    assert publish_marketplace_release.main() == 0
    assert local_calls == [
        (local_install_dir.resolve(), "claude-code", True, bundle_root.resolve())
    ]


def test_publish_skips_local_install_for_repo_only_runs(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    marketplace_root = tmp_path / "marketplace"
    _init_marketplace_repo(marketplace_root)
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    _pin_repo_root(monkeypatch, tmp_path)

    def fail_local_install(
        target_dir: Path,
        host: str,
        *,
        openclaw_dir: Path | None = None,
        restart_runtime: bool = False,
        bundle_root: Path | None = None,
    ) -> tuple[Path, Path]:
        del target_dir, host, openclaw_dir, restart_runtime, bundle_root
        raise AssertionError("local install must not run for repo-only publishes")

    monkeypatch.setattr(
        publish_marketplace_release,
        "install_local_mcp_server",
        fail_local_install,
    )
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "publish_marketplace_release.py",
            "--no-build",
            "--no-push",
            "--skip-codex-install",
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
        ],
    )

    assert publish_marketplace_release.main() == 0


def test_publish_honors_skip_local_install_flag(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    marketplace_root = tmp_path / "marketplace"
    _init_marketplace_repo(marketplace_root)
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    _pin_repo_root(monkeypatch, tmp_path)
    _stub_codex_install_steps(monkeypatch)

    def fail_local_install(
        target_dir: Path,
        host: str,
        *,
        openclaw_dir: Path | None = None,
        restart_runtime: bool = False,
        bundle_root: Path | None = None,
    ) -> tuple[Path, Path]:
        del target_dir, host, openclaw_dir, restart_runtime, bundle_root
        raise AssertionError("local install must not run when --skip-local-install is set")

    monkeypatch.setattr(
        publish_marketplace_release,
        "install_local_mcp_server",
        fail_local_install,
    )
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "publish_marketplace_release.py",
            "--no-build",
            "--no-push",
            "--skip-local-install",
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
        ],
    )

    assert publish_marketplace_release.main() == 0


def _write_release_binaries(release_dir: Path, *, content_suffix: str = "") -> None:
    platform_id = plugin_bundle.current_runtime_platform()
    release_dir.mkdir(parents=True, exist_ok=True)
    for name in plugin_bundle.platform_runtime_binary_base_names(platform_id):
        source_name = plugin_bundle.runtime_binary_source_name(platform_id, name)
        (release_dir / source_name).write_text(f"{source_name}{content_suffix}", encoding="utf-8")


def test_stale_bundle_binaries_passes_when_content_matches(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())
    release_dir = tmp_path / "release"
    _write_release_binaries(release_dir)

    assert publish_marketplace_release.stale_bundle_binaries(bundle_root, release_dir) == []


def test_stale_bundle_binaries_reports_content_drift(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())
    release_dir = tmp_path / "release"
    _write_release_binaries(release_dir, content_suffix="-rebuilt")

    stale = publish_marketplace_release.stale_bundle_binaries(bundle_root, release_dir)

    assert "sky-cua-client" in stale
    assert "sky-cua-service" in stale


def test_stale_bundle_binaries_skips_missing_release_build(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    assert publish_marketplace_release.stale_bundle_binaries(bundle_root, tmp_path / "absent") == []


def test_publish_no_build_aborts_on_stale_bundle(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    marketplace_root = tmp_path / "marketplace"
    _init_marketplace_repo(marketplace_root)
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    _pin_repo_root(monkeypatch, tmp_path)
    _write_release_binaries(tmp_path / "repo" / "target" / "release", content_suffix="-rebuilt")
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "publish_marketplace_release.py",
            "--no-build",
            "--no-push",
            "--skip-codex-install",
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
        ],
    )

    with pytest.raises(RuntimeError, match="differ from target/release"):
        publish_marketplace_release.main()


def test_publish_allow_stale_bundle_overrides_guard(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    marketplace_root = tmp_path / "marketplace"
    _init_marketplace_repo(marketplace_root)
    bundle_root = tmp_path / "bundle"
    write_minimal_bundle(bundle_root, binaries=runtime_binary_names())

    _pin_repo_root(monkeypatch, tmp_path)
    _write_release_binaries(tmp_path / "repo" / "target" / "release", content_suffix="-rebuilt")
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "publish_marketplace_release.py",
            "--no-build",
            "--allow-stale-bundle",
            "--no-push",
            "--skip-codex-install",
            "--marketplace-root",
            str(marketplace_root),
            "--bundle-root",
            str(bundle_root),
        ],
    )

    assert publish_marketplace_release.main() == 0


def test_setup_marketplace_checkout_expands_github_short_source(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    commands: list[list[str]] = []

    def fake_run(
        command: list[str], *, cwd: Path | None = None, check: bool = True
    ) -> subprocess.CompletedProcess[str]:
        del cwd, check
        commands.append(command)
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

    monkeypatch.setattr(setup_heliasar_marketplace, "run", fake_run)

    setup_heliasar_marketplace.ensure_marketplace_checkout(
        tmp_path / "heliasar-marketplace",
        "becksclair/heliasar-marketplace",
    )

    assert commands == [
        [
            "git",
            "clone",
            "https://github.com/becksclair/heliasar-marketplace.git",
            str(tmp_path / "heliasar-marketplace"),
        ]
    ]
