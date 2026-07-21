#!/usr/bin/env python3
"""Build a self-contained sky-cua release package.

Assembles the built plugin bundle, the pure-Python installer subset, and a
top-level install.py into a versioned tarball that installs sky-cua on a clean
machine (no repo, no toolchain, no marketplace) via `python3 install.py`.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
from pathlib import Path

from _install_shared import atomic_sibling_path
from _plugin_bundle import (
    DIST_PLUGIN_ROOT,
    REPO_ROOT,
    build_bundle,
    current_runtime_platform,
    ensure_bundle_structure,
    platform_runtime_binary_base_names,
    remove_path,
    runtime_binary_path,
    update_plugin_manifest_version,
    version_from_tag,
)

# The pure-Python installer closure shipped inside the package (no cargo, no
# build_plugin). Verified import-closed: installer -> install_mcp_server /
# install_plugin -> _install_shared / _openclaw_install / _kwin_effect /
# _plugin_bundle. Complete-release consumers also use the transactional
# OpenClaw CLI and OpenCode JSONC adapters.
PACKAGE_SCRIPTS = (
    "installer.py",
    "install_mcp_server.py",
    "install_complete_release.py",
    "install_plugin.py",
    "_install_shared.py",
    "_native_messaging_install.py",
    "_release_activation.py",
    "_openclaw_install.py",
    "_openclaw_cli_transaction.py",
    "_opencode_install.py",
    "_kwin_effect.py",
    "_plugin_bundle.py",
    "_mcp_stdio.py",
    "deploy_freshness.py",
    "release_generation.py",
    "release_builder.py",
)

DEFAULT_RELEASE_DIR = REPO_ROOT / "dist" / "release"

# Generated install.py shipped at the package root. It mirrors the repo-root
# shim but pins bundle mode + the package's own payload path; any explicit user
# flags (appended after the defaults) override them.
PACKAGE_INSTALL_PY = '''#!/usr/bin/env python3
"""sky-cua release package installer - run `python3 install.py`."""

from __future__ import annotations

import sys
from pathlib import Path

if sys.version_info < (3, 12):  # noqa: UP036 - guard older interpreters by design
    raise SystemExit("sky-cua install requires Python 3.12 or newer")

PACKAGE_ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(PACKAGE_ROOT / "scripts"))

from installer import main  # noqa: E402

_DEFAULTS = ["--mode", "bundle", "--bundle-root", str(PACKAGE_ROOT / "plugin" / "sky-cua")]

if __name__ == "__main__":
    raise SystemExit(main([*_DEFAULTS, *sys.argv[1:]]))
'''


def plugin_version(bundle_root: Path) -> str:
    metadata_path = bundle_root / ".codex-plugin" / "plugin.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    version = metadata.get("version")
    if not isinstance(version, str) or not version:
        raise ValueError(f"{metadata_path} is missing a string version")
    return version


def current_tag() -> str:
    for name in ("GITHUB_REF_NAME", "GITEA_REF_NAME", "CI_COMMIT_TAG"):
        value = os.environ.get(name, "").strip()
        if value:
            return value.removeprefix("refs/tags/")
    result = subprocess.run(
        ["git", "describe", "--tags", "--exact-match"],
        cwd=REPO_ROOT,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    return result.stdout.strip()


def assert_platform_binaries(bundle_root: Path, platform_id: str) -> None:
    """Fail loudly when the bundle lacks the target platform's runtime binaries.

    The build stages only the current platform plus any pre-staged cross
    binaries; packaging for a platform whose binaries are absent would ship a
    tarball that cannot run.
    """
    missing = [
        str(runtime_binary_path(platform_id, name))
        for name in platform_runtime_binary_base_names(platform_id)
        if not (bundle_root / runtime_binary_path(platform_id, name)).exists()
    ]
    if missing:
        raise SystemExit(
            f"bundle is missing {platform_id} runtime binaries: {', '.join(missing)}. "
            f"Build or pre-stage them before packaging for {platform_id}."
        )


def stage_package(staging_root: Path, bundle_root: Path, version: str) -> Path:
    pkg = staging_root / f"sky-cua-{version}"
    shutil.copytree(bundle_root, pkg / "plugin" / "sky-cua")

    scripts_dest = pkg / "scripts"
    scripts_dest.mkdir(parents=True)
    for name in PACKAGE_SCRIPTS:
        source = REPO_ROOT / "scripts" / name
        if not source.exists():
            raise SystemExit(f"package script missing from repo: {source}")
        shutil.copy2(source, scripts_dest / name)

    # Mirror skills at the package root so _install_shared.install_sky_cua_skills
    # (which reads REPO_ROOT/skills) resolves identically to the in-repo layout.
    skills_source = bundle_root / "skills"
    if skills_source.is_dir():
        shutil.copytree(skills_source, pkg / "skills")

    (pkg / "install.py").write_text(PACKAGE_INSTALL_PY, encoding="utf-8")
    (pkg / "VERSION").write_text(version + "\n", encoding="utf-8")
    return pkg


def write_tarball(pkg: Path, version: str, platform_id: str, release_dir: Path) -> Path:
    release_dir.mkdir(parents=True, exist_ok=True)
    archive = release_dir / f"sky-cua-{version}-{platform_id}.tar.gz"
    # Write to a sibling temp file and atomically rename into place, so a
    # crashed or interrupted run never leaves a partial archive at the
    # published name (and never leaks the temp).
    temp_path = atomic_sibling_path(archive, "tmp")
    remove_path(temp_path)
    try:
        with tarfile.open(temp_path, "w:gz") as tar:
            tar.add(pkg, arcname=pkg.name)
        os.replace(temp_path, archive)
    finally:
        remove_path(temp_path)
    return archive


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Build a self-contained sky-cua release package (tarball + installer)."
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Package the existing dist/plugin/sky-cua bundle without rebuilding.",
    )
    parser.add_argument(
        "--platform",
        default=current_runtime_platform(),
        help="Target platform id (default: current host platform).",
    )
    parser.add_argument(
        "--version-from-tag",
        nargs="?",
        const="",
        default=None,
        metavar="TAG",
        help=(
            "Set the bundle version from a vX.Y.Z git tag before packaging. "
            "When TAG is omitted, use the current CI/git tag."
        ),
    )
    parser.add_argument(
        "--release-dir",
        type=Path,
        default=DEFAULT_RELEASE_DIR,
        help=f"Output directory for the tarball (default: {DEFAULT_RELEASE_DIR}).",
    )
    args = parser.parse_args(argv)

    if not args.no_build:
        build_bundle()

    bundle_root = DIST_PLUGIN_ROOT.resolve()
    ensure_bundle_structure(bundle_root)

    if args.version_from_tag is not None:
        tag = current_tag() if args.version_from_tag == "" else args.version_from_tag
        version = version_from_tag(tag)
        update_plugin_manifest_version(bundle_root, version)
    else:
        version = plugin_version(bundle_root)

    assert_platform_binaries(bundle_root, args.platform)

    with tempfile.TemporaryDirectory() as tmp:
        pkg = stage_package(Path(tmp), bundle_root, version)
        archive = write_tarball(pkg, version, args.platform, args.release_dir)

    print(f"package={archive}")
    print(f"version={version}")
    print(f"platform={args.platform}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
