#!/usr/bin/env python3
"""Provision the durable global agents skill projection.

The checkout is a development source, not the public skill payload.  The
default workflow copies every named skill into the stable
``~/.local/share/sky-cua/skills`` payload root and projects ``~/.agents/skills``
to that root with relative links.  ``--checkout-links`` is an explicit
development opt-in for links that follow the checkout directly.
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from collections.abc import Sequence
from pathlib import Path

from _install_shared import SKY_CUA_SKILLS, enabled_skill_names
from _skill_projection import (
    PUBLIC_SKILLS_ROOT_MARKER,
    PUBLIC_SKILLS_ROOT_MARKER_CONTENT,
    apply_skill_link_plan,
    plan_skill_links,
    validate_public_skill_payload,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOURCE_ROOT = REPO_ROOT / "skills"
DEFAULT_AGENTS_DEST_ROOT = Path.home() / ".agents" / "skills"
SKILL_NAMES = SKY_CUA_SKILLS


def _home_public_root() -> Path:
    return Path.home() / ".local" / "share" / "sky-cua" / "skills"


def _lexical_absolute(path: Path) -> Path:
    return Path(os.path.abspath(os.fspath(path)))


def _validate_source_skills(source_root: Path, skill_names: Sequence[str]) -> None:
    for skill_name in skill_names:
        source = source_root / skill_name
        skill_file = source / "SKILL.md"
        if source.is_symlink() or not source.is_dir() or not skill_file.is_file():
            raise FileNotFoundError(f"missing repo skill: {skill_file}")


def _remove_path(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink(missing_ok=True)
    elif path.is_dir():
        shutil.rmtree(path)


def _copy_skill_tree(source: Path, destination: Path) -> None:
    """Atomically refresh one owned payload directory from the checkout."""
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    backup = destination.with_name(f".{destination.name}.backup-{os.getpid()}")
    _remove_path(temporary)
    _remove_path(backup)
    try:
        shutil.copytree(source, temporary, symlinks=False)
        if destination.exists() or destination.is_symlink():
            os.replace(destination, backup)
        os.replace(temporary, destination)
    except BaseException:
        _remove_path(temporary)
        if backup.exists() or backup.is_symlink():
            _remove_path(destination)
            os.replace(backup, destination)
        raise
    finally:
        _remove_path(temporary)
        _remove_path(backup)


def _copy_public_skills(source_root: Path, public_root: Path, skill_names: Sequence[str]) -> None:
    for skill_name in skill_names:
        _copy_skill_tree(source_root / skill_name, public_root / skill_name)


def _write_public_root_marker(public_root: Path) -> None:
    marker = public_root.parent / PUBLIC_SKILLS_ROOT_MARKER
    _validate_public_root_marker(marker)
    marker.parent.mkdir(parents=True, exist_ok=True)
    temporary = marker.with_name(f".{marker.name}.tmp-{os.getpid()}")
    _remove_path(temporary)
    try:
        temporary.write_text(PUBLIC_SKILLS_ROOT_MARKER_CONTENT, encoding="utf-8")
        os.replace(temporary, marker)
    finally:
        _remove_path(temporary)


def _validate_public_root_marker(marker: Path) -> None:
    if marker.is_symlink() or (marker.exists() and not marker.is_file()):
        raise ValueError(f"refusing to replace unmanaged public skills marker: {marker}")
    if marker.is_file():
        try:
            content = marker.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            raise ValueError(f"cannot read public skills marker: {marker}") from error
        if content != PUBLIC_SKILLS_ROOT_MARKER_CONTENT:
            raise ValueError(f"refusing to replace unmanaged public skills marker: {marker}")


def sync_agents_skill_symlinks(
    source_root: Path = DEFAULT_SOURCE_ROOT,
    dest_root: Path = DEFAULT_AGENTS_DEST_ROOT,
    skill_names: Sequence[str] = SKILL_NAMES,
    enabled_surfaces: frozenset[str] | None = None,
    *,
    public_root: Path | None = None,
    checkout_links: bool = False,
) -> tuple[Path, ...]:
    """Copy and project durable-enabled named skills.

    Source validation covers every named skill before any copy or projection.
    The durable selection comes from :func:`enabled_skill_names`; a supplied
    ``enabled_surfaces`` is intended for deterministic provisioning/tests and
    still does not read the transient runtime surface override.
    """
    source_root = _lexical_absolute(source_root.expanduser())
    dest_root = _lexical_absolute(dest_root.expanduser())
    public_root = _lexical_absolute(
        (public_root if public_root is not None else _home_public_root()).expanduser()
    )
    names = tuple(dict.fromkeys(skill_names))
    selected = tuple(name for name in enabled_skill_names(enabled_surfaces) if name in names)

    # Validate the complete payload before touching the public root or global
    # projection, including disabled names that remain present in the bundle.
    _validate_source_skills(source_root, names)

    if checkout_links:
        projection_root = source_root
        managed_roots = (public_root,)
    else:
        validate_public_skill_payload(public_root, names)
        _validate_public_root_marker(public_root.parent / PUBLIC_SKILLS_ROOT_MARKER)
        projection_root = public_root
        managed_roots = (source_root,)

    projection_plan = plan_skill_links(
        projection_root,
        (dest_root,),
        names,
        selected,
        managed_source_roots=managed_roots,
        validation_source_root=source_root,
    )

    if not checkout_links:
        if source_root != public_root:
            _copy_public_skills(source_root, public_root, names)
        _write_public_root_marker(public_root)

    return apply_skill_link_plan(projection_plan)


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Project durable sky-cua skills into the global agents skill root."
    )
    parser.add_argument(
        "--source-root",
        type=Path,
        default=DEFAULT_SOURCE_ROOT,
        help="Checkout skills root. Defaults to this checkout's skills/.",
    )
    parser.add_argument(
        "--agents-dest-root",
        type=Path,
        default=DEFAULT_AGENTS_DEST_ROOT,
        help="Global agents skills root. Defaults to ~/.agents/skills.",
    )
    parser.add_argument(
        "--public-root",
        type=Path,
        default=None,
        help="Stable payload skills root. Defaults to ~/.local/share/sky-cua/skills.",
    )
    parser.add_argument(
        "--checkout-links",
        action="store_true",
        help="Explicitly link global skills directly to the checkout.",
    )
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    projected = sync_agents_skill_symlinks(
        args.source_root,
        args.agents_dest_root,
        public_root=args.public_root,
        checkout_links=args.checkout_links,
    )
    selected = set(enabled_skill_names())
    for skill_name in SKILL_NAMES:
        destination = args.agents_dest_root.expanduser() / skill_name
        if skill_name in selected:
            print(f"Linked {skill_name} to {destination}")
        else:
            print(f"Disabled {skill_name}; removed stale managed link at {destination}")
    if not projected:
        print("No enabled sky-cua skills projected")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
