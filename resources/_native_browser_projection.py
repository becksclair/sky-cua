"""Copy the app-owned native Browser package and manage Sky's supplemental skill."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import tempfile
from pathlib import Path
from typing import Any

NATIVE_BROWSER_PLUGIN_ROOT = Path(
    "/usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/browser"
)


class ValidatedNativeBrowser:
    """A native package whose required assets and manifest have been checked."""

    def __init__(self, root: Path, manifest: dict[str, Any]) -> None:
        self.root = root
        self.manifest = manifest


def validate_native_browser_plugin(candidate: Path) -> ValidatedNativeBrowser | None:
    """Absent packages permit fallback; present but broken packages fail explicitly."""
    try:
        candidate.lstat()
    except FileNotFoundError:
        return None
    except OSError as error:
        raise ValueError(
            f"cannot inspect native app Browser plugin {candidate}: {error}"
        ) from error
    manifest = _native_manifest(candidate)
    for relative in (
        "scripts/browser-client.mjs",
        "scripts/browser-service.mjs",
        "docs/api.json",
        "skills/control-in-app-browser/SKILL.md",
    ):
        if not (candidate / relative).is_file():
            raise ValueError(
                f"native app Browser plugin is incomplete: missing {candidate / relative}"
            )
    return ValidatedNativeBrowser(candidate.resolve(), manifest)


def native_browser_plugin_root(candidate: Path) -> Path | None:
    """Compatibility helper returning the validated package path."""
    native = validate_native_browser_plugin(candidate)
    return native.root if native is not None else None


def _native_manifest(root: Path) -> dict[str, Any]:
    path = root / ".codex-plugin/plugin.json"
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read native app Browser manifest {path}: {error}") from error
    if not isinstance(manifest, dict) or manifest.get("name") != "browser":
        raise ValueError(f"native app Browser manifest must name browser: {path}")
    version = manifest.get("version")
    if not isinstance(version, str) or not version.strip():
        raise ValueError(f"native app Browser manifest must have a version: {path}")
    return manifest


def _tree_digest(root: Path) -> str:
    """Hash a projection's complete shape: paths, entry kinds, modes and bytes."""
    digest = hashlib.sha256()
    for directory, dirnames, filenames in os.walk(root, followlinks=False):
        dirnames.sort()
        base = Path(directory)
        for name in sorted(dirnames + filenames):
            path = base / name
            relative = path.relative_to(root).as_posix().encode()
            if path.is_symlink():
                digest.update(b"L\0" + relative + b"\0" + os.fsencode(os.readlink(path)))
            elif path.is_dir():
                digest.update(b"D\0" + relative)
            else:
                content = path.read_bytes()
                executable = b"x" if path.stat().st_mode & 0o111 else b"-"
                digest.update(
                    b"F\0" + relative + b"\0" + executable + str(len(content)).encode() + b"\0"
                )
                digest.update(content)
            digest.update(b"\0")
    return digest.hexdigest()


def materialize_native_browser_plugin(
    native_root: Path | ValidatedNativeBrowser,
    destination: Path,
) -> str:
    """Copy the complete native package without changing its identity or bytes.

    Codex's plugin installer omits symlinks; dereference native package links
    into regular files. The native app remains the Browser cache owner and may
    reconcile this reserved identity at startup.
    """
    native = (
        native_root
        if isinstance(native_root, ValidatedNativeBrowser)
        else validate_native_browser_plugin(native_root)
    )
    if native is None:
        raise ValueError(f"native app Browser plugin is missing: {native_root}")
    if destination.absolute() == native.root:
        raise ValueError("native Browser projection destination must differ from the native root")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".sky-browser-", dir=destination.parent) as temporary:
        staged = Path(temporary) / "browser"
        shutil.copytree(native.root, staged, symlinks=False, ignore_dangling_symlinks=True)
        version = str(native.manifest["version"])
        # Avoid churning an unchanged root under an active plugin host.
        if (
            destination.is_dir()
            and not destination.is_symlink()
            and _tree_digest(destination) == _tree_digest(staged)
        ):
            return version
        if destination.is_symlink() or destination.is_file():
            destination.unlink()
        elif destination.exists():
            shutil.rmtree(destination)
        staged.rename(destination)
    return version


SUPPLEMENTAL_SKILL_NAME = "sky-browser-enhancements"
SUPPLEMENTAL_SKILL_MARKER = ".sky-cua-managed"


def sync_native_browser_skill(
    install_root: Path, codex_home: Path, *, enabled: bool, native_browser_root: Path | None = None
) -> None:
    """Keep the Codex-only helper skill separate from the reserved Browser plugin."""
    destination = codex_home / "skills" / SUPPLEMENTAL_SKILL_NAME
    marker = destination / SUPPLEMENTAL_SKILL_MARKER
    exists = destination.exists() or destination.is_symlink()
    managed = exists and not destination.is_symlink() and marker.is_file()
    if not enabled:
        if managed:
            shutil.rmtree(destination)
        return
    if exists and not managed:
        raise ValueError(f"refusing to replace unmanaged Sky Browser skill: {destination}")
    if native_browser_root is None:
        raise ValueError("enabled Sky Browser skill requires a validated native Browser root")
    source = install_root / "resources" / SUPPLEMENTAL_SKILL_NAME / "SKILL.md"
    if not source.is_file():
        raise ValueError(f"Sky Browser supplemental skill is missing: {source}")
    rendered = (
        source.read_text(encoding="utf-8")
        .replace("@SKY_CUA_ROOT_JSON@", json.dumps(str(install_root)))
        .replace("@SKY_CUA_ROOT@", str(install_root))
        .replace(
            "@NATIVE_BROWSER_SKILL@",
            str(native_browser_root / "skills/control-in-app-browser/SKILL.md"),
        )
    )
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "SKILL.md").write_text(rendered, encoding="utf-8")
    marker.write_text("sky-cua\n", encoding="utf-8")
