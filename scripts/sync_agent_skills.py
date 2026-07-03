#!/usr/bin/env python3
"""Link repo-local sky-cua skills into the global agents skill root.

Replaces `~/.agents/skills/{computer-use,browser-use,phone-use}` with symlinks
to this checkout's `skills/*` directories so opencode/oracle/worker-style
agents read the current repo skill text. Run after a deploy; deploying from a
worktree leaves the links pointing at that worktree, so a redeploy from the
main checkout repoints them.

The former OpenClaw workspace-skills copy step was retired on 2026-07-03:
OpenClaw no longer needs bundled skill copies in
`~/.openclaw/workspace/skills`.
"""

from __future__ import annotations

import argparse
import os
import sys
from collections.abc import Sequence
from pathlib import Path

from _install_shared import SKY_CUA_SKILLS
from _plugin_bundle import remove_path

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOURCE_ROOT = REPO_ROOT / "skills"
DEFAULT_AGENTS_DEST_ROOT = Path.home() / ".agents" / "skills"
SKILL_NAMES = SKY_CUA_SKILLS


def _validate_source_skills(source_root: Path, skill_names: Sequence[str]) -> None:
    for skill_name in skill_names:
        source = source_root / skill_name
        skill_file = source / "SKILL.md"
        if not source.is_dir() or not skill_file.is_file():
            raise FileNotFoundError(f"missing repo skill: {skill_file}")


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
    source_root: Path = DEFAULT_SOURCE_ROOT,
    dest_root: Path = DEFAULT_AGENTS_DEST_ROOT,
    skill_names: Sequence[str] = SKILL_NAMES,
) -> None:
    """Link sky-cua-owned skills into the global agents skill root."""
    source_root = source_root.expanduser().resolve()
    dest_root = dest_root.expanduser()
    _validate_source_skills(source_root, skill_names)
    for skill_name in skill_names:
        _replace_symlink(source_root / skill_name, dest_root / skill_name)


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Link repo-local sky-cua skills into the global agents skill root."
    )
    parser.add_argument(
        "--source-root",
        type=Path,
        default=DEFAULT_SOURCE_ROOT,
        help="Repo skills root. Defaults to this checkout's skills/.",
    )
    parser.add_argument(
        "--agents-dest-root",
        type=Path,
        default=DEFAULT_AGENTS_DEST_ROOT,
        help="Global agents skills root. Defaults to ~/.agents/skills.",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    sync_agents_skill_symlinks(args.source_root, args.agents_dest_root)
    for skill_name in SKILL_NAMES:
        print(f"Linked {skill_name} to {args.agents_dest_root.expanduser() / skill_name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
