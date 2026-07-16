from __future__ import annotations

import ast
import tarfile
from pathlib import Path

import pytest

import package
from _plugin_bundle import (
    SKY_CUA_SKILLS,
    current_runtime_platform,
    platform_runtime_binary_base_names,
    runtime_binary_path,
)


def _minimal_bundle(root: Path, version: str = "9.9.9") -> Path:
    (root / ".codex-plugin").mkdir(parents=True)
    (root / ".codex-plugin" / "plugin.json").write_text(
        f'{{"version": "{version}"}}', encoding="utf-8"
    )
    (root / ".claude-plugin").mkdir(parents=True)
    (root / ".claude-plugin" / "plugin.json").write_text(
        f'{{"version": "{version}"}}', encoding="utf-8"
    )
    for skill_name in SKY_CUA_SKILLS:
        skill_dir = root / "skills" / skill_name
        skill_dir.mkdir(parents=True)
        (skill_dir / "SKILL.md").write_text("# skill\n", encoding="utf-8")
    platform_id = current_runtime_platform()
    for name in platform_runtime_binary_base_names(platform_id):
        bin_path = root / runtime_binary_path(platform_id, name)
        bin_path.parent.mkdir(parents=True, exist_ok=True)
        bin_path.write_text("#!/bin/sh\n", encoding="utf-8")
    return root


def test_package_scripts_closure_is_import_complete() -> None:
    # The package ships exactly PACKAGE_SCRIPTS. If any shipped script imports a
    # local scripts/ module that is NOT in that tuple, a clean-machine install
    # would ImportError - this pins the hand-maintained closure against drift.
    scripts_dir = Path(__file__).resolve().parent
    local_modules = {entry.stem for entry in scripts_dir.glob("*.py")}
    closure = set(package.PACKAGE_SCRIPTS)

    referenced: set[str] = set()
    for name in package.PACKAGE_SCRIPTS:
        tree = ast.parse((scripts_dir / name).read_text(encoding="utf-8"))
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                referenced.update(alias.name.split(".")[0] for alias in node.names)
            elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
                referenced.add(node.module.split(".")[0])

    missing = {
        f"{module}.py"
        for module in referenced
        if module in local_modules and f"{module}.py" not in closure
    }
    assert not missing, f"PACKAGE_SCRIPTS is missing local imports: {sorted(missing)}"


def test_plugin_version_reads_manifest(tmp_path: Path) -> None:
    bundle = _minimal_bundle(tmp_path / "b", version="1.2.3")
    assert package.plugin_version(bundle) == "1.2.3"


def test_current_tag_prefers_ci_ref_name(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("GITHUB_REF_NAME", "refs/tags/v1.2.3")

    assert package.current_tag() == "v1.2.3"


def test_assert_platform_binaries_passes_when_present(tmp_path: Path) -> None:
    bundle = _minimal_bundle(tmp_path / "b")
    package.assert_platform_binaries(bundle, current_runtime_platform())  # must not raise


def test_assert_platform_binaries_fails_when_missing(tmp_path: Path) -> None:
    bundle = tmp_path / "empty"
    bundle.mkdir()
    with pytest.raises(SystemExit, match="missing"):
        package.assert_platform_binaries(bundle, current_runtime_platform())


def test_stage_package_layout_and_install_shim(tmp_path: Path) -> None:
    bundle = _minimal_bundle(tmp_path / "b", version="4.5.6")
    staging = tmp_path / "staging"
    staging.mkdir()

    pkg = package.stage_package(staging, bundle, "4.5.6")

    assert pkg.name == "sky-cua-4.5.6"
    assert (pkg / "plugin" / "sky-cua" / ".codex-plugin" / "plugin.json").exists()
    assert (pkg / "VERSION").read_text(encoding="utf-8").strip() == "4.5.6"

    install_py = (pkg / "install.py").read_text(encoding="utf-8")
    assert "--mode" in install_py and "bundle" in install_py
    assert 'PACKAGE_ROOT / "plugin" / "sky-cua"' in install_py

    # Every installer-closure script ships, so parents[1] resolves to the package
    # root and the in-package install behaves like the in-repo one.
    for name in package.PACKAGE_SCRIPTS:
        assert (pkg / "scripts" / name).exists()

    # Skills mirrored at the package root for _install_shared.install_sky_cua_skills.
    for skill_name in SKY_CUA_SKILLS:
        assert (pkg / "skills" / skill_name / "SKILL.md").exists()


def test_write_tarball_is_atomic_on_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # The publish into release_dir must be atomic: a failure after the temp
    # archive is written (here, the rename) must leave no file at the published
    # name and must not leak the temp sibling.
    bundle = _minimal_bundle(tmp_path / "b", version="3.0.0")
    staging = tmp_path / "staging"
    staging.mkdir()
    pkg = package.stage_package(staging, bundle, "3.0.0")
    release = tmp_path / "release"

    def boom_replace(*_args: object) -> None:
        raise OSError("rename failed")

    monkeypatch.setattr(package.os, "replace", boom_replace)

    with pytest.raises(OSError, match="rename failed"):
        package.write_tarball(pkg, "3.0.0", "linux-x64", release)

    assert not (release / "sky-cua-3.0.0-linux-x64.tar.gz").exists()
    assert list(release.glob(".sky-cua-3.0.0-linux-x64.tar.gz.tmp*")) == []


def test_write_tarball_contains_package(tmp_path: Path) -> None:
    bundle = _minimal_bundle(tmp_path / "b", version="7.0.0")
    staging = tmp_path / "staging"
    staging.mkdir()
    pkg = package.stage_package(staging, bundle, "7.0.0")

    archive = package.write_tarball(pkg, "7.0.0", "linux-x64", tmp_path / "release")

    assert archive.name == "sky-cua-7.0.0-linux-x64.tar.gz"
    with tarfile.open(archive) as tar:
        names = tar.getnames()
    assert "sky-cua-7.0.0/install.py" in names
    assert "sky-cua-7.0.0/scripts/installer.py" in names
    assert "sky-cua-7.0.0/plugin/sky-cua/.codex-plugin/plugin.json" in names


def test_main_version_from_tag_without_value_uses_current_tag(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bundle = _minimal_bundle(tmp_path / "b", version="1.0.0")
    release = tmp_path / "release"
    monkeypatch.setattr(package, "DIST_PLUGIN_ROOT", bundle)
    monkeypatch.setattr(package, "ensure_bundle_structure", lambda _root: None)
    monkeypatch.setenv("GITHUB_REF_NAME", "v2.3.4")

    assert package.main(["--no-build", "--version-from-tag", "--release-dir", str(release)]) == 0

    assert (release / f"sky-cua-2.3.4-{current_runtime_platform()}.tar.gz").exists()
    assert package.plugin_version(bundle) == "2.3.4"


def test_main_version_from_tag_accepts_explicit_tag(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bundle = _minimal_bundle(tmp_path / "b", version="1.0.0")
    release = tmp_path / "release"
    monkeypatch.setattr(package, "DIST_PLUGIN_ROOT", bundle)
    monkeypatch.setattr(package, "ensure_bundle_structure", lambda _root: None)

    assert (
        package.main(["--no-build", "--version-from-tag", "v3.4.5", "--release-dir", str(release)])
        == 0
    )

    assert (release / f"sky-cua-3.4.5-{current_runtime_platform()}.tar.gz").exists()
    assert package.plugin_version(bundle) == "3.4.5"
