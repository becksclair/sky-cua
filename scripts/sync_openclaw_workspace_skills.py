#!/usr/bin/env python3
"""Sync bundled sky-cua skills into OpenClaw's workspace skill root."""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from collections.abc import Sequence
from pathlib import Path

from _install_shared import SKY_CUA_SKILLS
from _plugin_bundle import remove_path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOURCE_ROOT = REPO_ROOT / "dist" / "plugin" / "sky-cua" / "skills"
DEFAULT_DEST_ROOT = Path.home() / ".openclaw" / "workspace" / "skills"
DEFAULT_AGENTS_DEST_ROOT = Path.home() / ".agents" / "skills"
SKILL_NAMES = SKY_CUA_SKILLS
STAGE_DIR_NAME = ".sky-cua-openclaw-skills-stage"
COMMITTED_MARKER_NAME = "COMMITTED"
BACKUP_DIR_NAME = "backups"
INSTALLED_DIR_NAME = "installed"


def _move_path(source: Path, destination: Path) -> None:
    source.rename(destination)


def _validate_source_skills(source_root: Path, skill_names: Sequence[str]) -> None:
    for skill_name in skill_names:
        source = source_root / skill_name
        skill_file = source / "SKILL.md"
        if not source.is_dir() or not skill_file.is_file():
            raise FileNotFoundError(f"missing bundled skill: {skill_file}")


def _backup_path(stage_root: Path, skill_name: str) -> Path:
    return stage_root / BACKUP_DIR_NAME / skill_name


def _installed_marker_path(stage_root: Path, skill_name: str) -> Path:
    return stage_root / INSTALLED_DIR_NAME / skill_name


def _rollback_stage(dest_root: Path, stage_root: Path, skill_names: Sequence[str]) -> None:
    for skill_name in skill_names:
        backup = _backup_path(stage_root, skill_name)
        staged_skill = stage_root / skill_name
        installed_marker = _installed_marker_path(stage_root, skill_name)
        destination = dest_root / skill_name
        if backup.exists() or backup.is_symlink():
            remove_path(destination)
            _move_path(backup, destination)
        elif (
            installed_marker.exists()
            and not staged_skill.exists()
            and (destination.exists() or destination.is_symlink())
        ):
            remove_path(destination)


def _recover_incomplete_sync(dest_root: Path, stage_root: Path, skill_names: Sequence[str]) -> None:
    if not stage_root.exists() and not stage_root.is_symlink():
        return
    if (stage_root / COMMITTED_MARKER_NAME).exists():
        remove_path(stage_root)
        return
    _rollback_stage(dest_root, stage_root, skill_names)
    remove_path(stage_root)


def sync_openclaw_workspace_skills(
    source_root: Path = DEFAULT_SOURCE_ROOT,
    dest_root: Path = DEFAULT_DEST_ROOT,
    skill_names: Sequence[str] = SKILL_NAMES,
) -> None:
    """Copy sky-cua-owned skills with rollback if replacement fails."""
    source_root = source_root.resolve()
    dest_root = dest_root.expanduser()

    dest_root.mkdir(parents=True, exist_ok=True)
    stage_root = dest_root / STAGE_DIR_NAME
    _recover_incomplete_sync(dest_root, stage_root, skill_names)
    _validate_source_skills(source_root, skill_names)
    remove_path(stage_root)
    stage_root.mkdir(parents=True)
    backup_root = stage_root / BACKUP_DIR_NAME
    backup_root.mkdir()

    installed: list[str] = []
    try:
        for skill_name in skill_names:
            shutil.copytree(source_root / skill_name, stage_root / skill_name, symlinks=True)
            if not (stage_root / skill_name / "SKILL.md").is_file():
                raise FileNotFoundError(f"staged skill is missing SKILL.md: {skill_name}")

        for skill_name in skill_names:
            destination = dest_root / skill_name
            backup = _backup_path(stage_root, skill_name)
            if destination.exists() or destination.is_symlink():
                _move_path(destination, backup)
            marker = _installed_marker_path(stage_root, skill_name)
            marker.parent.mkdir(exist_ok=True)
            marker.write_text("ok\n", encoding="utf-8")
            _move_path(stage_root / skill_name, destination)
            installed.append(skill_name)
    except Exception:
        for skill_name in reversed(installed):
            remove_path(dest_root / skill_name)
        _rollback_stage(dest_root, stage_root, skill_names)
        raise
    else:
        (stage_root / COMMITTED_MARKER_NAME).write_text("ok\n", encoding="utf-8")
    finally:
        remove_path(stage_root)


def _replace_symlink(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temp_path = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    backup_path = destination.with_name(f".{destination.name}.backup-{os.getpid()}")
    remove_path(temp_path)
    remove_path(backup_path)
    try:
        temp_path.symlink_to(source, target_is_directory=True)
        if destination.exists() or destination.is_symlink():
            os.replace(destination, backup_path)
            try:
                os.replace(temp_path, destination)
            except OSError:
                os.replace(backup_path, destination)
                raise
            remove_path(backup_path)
        else:
            os.replace(temp_path, destination)
    finally:
        remove_path(temp_path)
        remove_path(backup_path)


def sync_agents_skill_symlinks(
    source_root: Path = REPO_ROOT / "skills",
    dest_root: Path = DEFAULT_AGENTS_DEST_ROOT,
    skill_names: Sequence[str] = SKILL_NAMES,
) -> None:
    """Link sky-cua-owned skills into the global agents skill root."""
    source_root = source_root.resolve()
    dest_root = dest_root.expanduser()
    _validate_source_skills(source_root, skill_names)
    for skill_name in skill_names:
        _replace_symlink(source_root / skill_name, dest_root / skill_name)


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Sync bundled sky-cua skills into OpenClaw and link repo skills into "
            "the global agents skill root."
        )
    )
    parser.add_argument(
        "--source-root",
        type=Path,
        default=DEFAULT_SOURCE_ROOT,
        help="Bundled skills root. Defaults to dist/plugin/sky-cua/skills.",
    )
    parser.add_argument(
        "--dest-root",
        type=Path,
        default=DEFAULT_DEST_ROOT,
        help="OpenClaw workspace skills root. Defaults to ~/.openclaw/workspace/skills.",
    )
    parser.add_argument(
        "--agents-dest-root",
        type=Path,
        default=DEFAULT_AGENTS_DEST_ROOT,
        help="Global agents skills root. Defaults to ~/.agents/skills.",
    )
    parser.add_argument(
        "--skip-agents",
        action="store_true",
        help="Do not update ~/.agents/skills symlinks.",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    sync_openclaw_workspace_skills(args.source_root, args.dest_root)
    for skill_name in SKILL_NAMES:
        print(f"Synced {skill_name} to {args.dest_root.expanduser() / skill_name}")
    if not args.skip_agents:
        sync_agents_skill_symlinks(dest_root=args.agents_dest_root)
        for skill_name in SKILL_NAMES:
            print(f"Linked {skill_name} to {args.agents_dest_root.expanduser() / skill_name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
