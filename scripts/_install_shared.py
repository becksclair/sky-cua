"""Host-agnostic install helpers shared by the MCP server installers."""

from __future__ import annotations

import os
import re
import shutil
import stat
import subprocess
import sys
import tomllib
from pathlib import Path

from _plugin_bundle import remove_path

REPO_ROOT = Path(__file__).resolve().parents[1]
BROWSER_SELECTION_ENV = "SKY_CUA_BROWSER"
SKY_CUA_SKILLS = ("computer-use", "browser-use")


def subprocess_error_detail(error: Exception) -> str:
    """Render ': <stderr>' for subprocess errors that captured stderr."""
    if isinstance(error, subprocess.CalledProcessError) and error.stderr:
        stderr = error.stderr.decode(errors="replace").strip()
        if stderr:
            return f": {stderr}"
    return ""


def ensure_parent(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def atomic_sibling_path(path: Path, suffix: str) -> Path:
    return path.with_name(f".{path.name}.{suffix}-{os.getpid()}")


def write_text_atomically(path: Path, text: str, mode: int | None = None) -> None:
    ensure_parent(path)
    write_path = atomic_write_target(path)
    if write_path != path:
        ensure_parent(write_path)
    target_mode = mode
    if target_mode is None and write_path.exists():
        target_mode = stat.S_IMODE(write_path.stat().st_mode)
    temp_path = atomic_sibling_path(write_path, "tmp")
    remove_path(temp_path)
    try:
        temp_path.write_text(text, encoding="utf-8")
        if target_mode is not None:
            temp_path.chmod(target_mode)
        os.replace(temp_path, write_path)
    finally:
        remove_path(temp_path)


def atomic_write_target(path: Path) -> Path:
    return path.resolve(strict=False) if path.is_symlink() else path


def replace_tree_atomically(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temp_path = atomic_sibling_path(destination, "tmp")
    backup_path = atomic_sibling_path(destination, "backup")
    backup_in_use = False
    remove_path(temp_path)
    remove_path(backup_path)
    try:
        shutil.copytree(source, temp_path)
        if destination.exists() or destination.is_symlink():
            os.replace(destination, backup_path)
            backup_in_use = True
            try:
                os.replace(temp_path, destination)
            except OSError:
                os.replace(backup_path, destination)
                backup_in_use = False
                raise
            remove_path(backup_path)
            backup_in_use = False
        else:
            os.replace(temp_path, destination)
    finally:
        remove_path(temp_path)
        if not backup_in_use:
            remove_path(backup_path)


def snapshot_text_path(path: Path) -> tuple[Path, str | None, int | None]:
    target_path = atomic_write_target(path)
    return (
        target_path,
        target_path.read_text(encoding="utf-8") if target_path.exists() else None,
        stat.S_IMODE(target_path.stat().st_mode) if target_path.exists() else None,
    )


def restore_text_path_snapshot(_path: Path, snapshot: tuple[Path, str | None, int | None]) -> None:
    target_path, original_text, original_mode = snapshot
    if original_text is None:
        remove_path(target_path)
    else:
        write_text_atomically(target_path, original_text, mode=original_mode)


MACHINE_CONFIG_PATH_ENV = "SKY_CUA_CONFIG_PATH"


def machine_config_path() -> Path | None:
    """Mirror the runtime's machine-config resolution (platform config.rs)."""
    explicit = os.environ.get(MACHINE_CONFIG_PATH_ENV)
    if explicit is not None:
        return Path(explicit) if explicit else None
    if sys.platform == "win32":
        appdata = os.environ.get("APPDATA")
        base = Path(appdata) if appdata else None
    else:
        xdg = os.environ.get("XDG_CONFIG_HOME")
        base = Path(xdg) if xdg else Path.home() / ".config"
    if base is None:
        return None
    return base / "sky-cua" / "sky-cua.toml"


def seed_machine_config_browser(value: str) -> Path | None:
    """Persist the browser selection into the machine config file.

    Machine-level settings belong in one file the runtime reads directly, not
    in every host registration's environment. Returns the written path, or
    None when no config location resolves or the existing file is unsafe to
    edit (unparseable, or `browser` set by something more complex than a
    single assignment line).
    """
    path = machine_config_path()
    if path is None:
        return None
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    try:
        existing = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        print(
            f"warning: not updating machine config {path}: existing file fails TOML "
            f"validation ({error}); fix it by hand.",
            file=sys.stderr,
        )
        return None
    if existing.get("browser") == value:
        return path
    line = f'browser = "{value}"\n'
    if "browser" in existing:
        new_text, replacements = re.subn(r"(?m)^browser\s*=.*\n?", line, text, count=1)
        if replacements != 1:
            print(
                f"warning: not updating machine config {path}: could not locate the "
                "browser assignment to replace; edit it by hand.",
                file=sys.stderr,
            )
            return None
    else:
        separator = "" if not text or text.endswith("\n") else "\n"
        new_text = text + separator + line
    try:
        tomllib.loads(new_text)
    except tomllib.TOMLDecodeError as error:
        print(
            f"warning: not updating machine config {path}: updated file would fail "
            f"TOML validation ({error}).",
            file=sys.stderr,
        )
        return None
    write_text_atomically(path, new_text)
    return path


def seed_machine_config_from_environment() -> Path | None:
    """Seed machine config from SKY_CUA_BROWSER when set at install time."""
    value = os.environ.get(BROWSER_SELECTION_ENV, "").strip()
    if not value:
        return None
    return seed_machine_config_browser(value)


def install_sky_cua_skills(skills_dir: Path) -> None:
    skills_dir.mkdir(parents=True, exist_ok=True)
    for skill_name in SKY_CUA_SKILLS:
        source = REPO_ROOT / "skills" / skill_name
        if not source.exists():
            raise FileNotFoundError(f"sky-cua skill source not found: {source}")
        destination = skills_dir / skill_name
        replace_tree_atomically(source, destination)
