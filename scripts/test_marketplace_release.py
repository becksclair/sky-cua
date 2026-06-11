"""Tests for marketplace publishing and setup helpers."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

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
