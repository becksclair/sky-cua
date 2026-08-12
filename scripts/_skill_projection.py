"""Safe relative-symlink projection for installed sky-cua skill roots.

This module intentionally depends only on the Python standard library.  The
standalone installer copies it next to its other small runtime helpers, so it
must remain importable without the source checkout's installer modules.
"""

from __future__ import annotations

import json
import os
import shutil
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

PUBLIC_SKILLS_ROOT_MARKER = "SKY_CUA_SKILLS_ROOT"
PUBLIC_SKILLS_ROOT_MARKER_CONTENT = "sky-cua managed public skills root\n"


@dataclass(frozen=True)
class SkillLinkPlan:
    """A prevalidated set of global skill-link mutations."""

    source_root: Path
    operations: tuple[tuple[Literal["remove", "replace"], Path, str | None], ...]
    projected: tuple[Path, ...]


def _lexical_absolute(path: Path) -> Path:
    """Make an absolute, normalized path without resolving symlinks."""
    return Path(os.path.abspath(os.fspath(path)))


def relative_symlink_target(source: Path, destination: Path) -> Path:
    """Return the relative link text from ``destination`` to ``source``.

    The calculation is lexical: ``source`` is not resolved through symlinks.
    This keeps the public root's spelling stable even when that root is itself
    a rendezvous symlink to a physical install tree.
    """
    source_path = _lexical_absolute(Path(source))
    destination_path = _lexical_absolute(Path(destination))
    return Path(os.path.relpath(source_path, start=destination_path.parent))


def _remove_path(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink(missing_ok=True)
    elif path.is_dir():
        shutil.rmtree(path)


def replace_relative_symlink(source: Path, destination: Path) -> None:
    """Replace ``destination`` with a relative symlink to ``source``.

    This is the low-level replacement primitive used for known installer-owned
    rendezvous paths.  Callers that operate on user-facing skill entries must
    classify the existing destination first; :func:`project_skill_links`
    performs that classification and refuses unmanaged entries.
    """
    source_path = _lexical_absolute(Path(source))
    destination_path = _lexical_absolute(Path(destination))
    destination_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination_path.with_name(f".{destination_path.name}.tmp-{os.getpid()}")
    backup = destination_path.with_name(f".{destination_path.name}.backup-{os.getpid()}")
    _remove_path(temporary)
    _remove_path(backup)
    temporary.symlink_to(
        relative_symlink_target(source_path, destination_path),
        target_is_directory=source_path.is_dir(),
    )
    try:
        if destination_path.exists() or destination_path.is_symlink():
            os.replace(destination_path, backup)
        os.replace(temporary, destination_path)
    except BaseException:
        _remove_path(temporary)
        if backup.exists() or backup.is_symlink():
            _remove_path(destination_path)
            os.replace(backup, destination_path)
        raise
    finally:
        _remove_path(temporary)
        _remove_path(backup)


def _resolved_path(path: Path) -> Path | None:
    try:
        return _lexical_absolute(path.resolve(strict=False))
    except (OSError, RuntimeError):
        return None


def _managed_targets(
    source_root: Path,
    managed_source_roots: Sequence[Path],
    skill_name: str,
) -> tuple[Path, ...]:
    roots = (source_root, *managed_source_roots)
    targets: list[Path] = []
    for root in roots:
        target = _resolved_path(root / skill_name)
        if target is not None and target not in targets:
            targets.append(target)
    return tuple(targets)


def _is_checkout_skill_root(source_root: Path, skill_names: Sequence[str]) -> bool:
    """Recognize a live Sky CUA checkout without trusting path spelling."""
    checkout = source_root.parent
    return (
        source_root.name == "skills"
        and (checkout / "Cargo.toml").is_file()
        and (checkout / "scripts/sync_agent_skills.py").is_file()
        and (checkout / "crates/sky-cua-client/Cargo.toml").is_file()
        and all((source_root / name / "SKILL.md").is_file() for name in skill_names)
    )


def _legacy_checkout_roots(roots: Sequence[Path], skill_names: Sequence[str]) -> tuple[Path, ...]:
    """Find checkout roots referenced by existing named projections.

    Direct checkout links were Sky CUA's former development representation.
    Requiring the complete checkout signature keeps arbitrary symlinks outside
    installer ownership while allowing links to old live worktrees to converge.
    """
    discovered: list[Path] = []
    for root in roots:
        for skill_name in skill_names:
            destination = root / skill_name
            if not destination.is_symlink():
                continue
            target = _resolved_path(destination)
            if target is None or target.name != skill_name:
                continue
            candidate = target.parent
            if _is_checkout_skill_root(candidate, skill_names) and candidate not in discovered:
                discovered.append(candidate)
    return tuple(discovered)


def _is_managed_link(
    destination: Path,
    source_root: Path,
    managed_source_roots: Sequence[Path],
    skill_name: str,
) -> bool:
    if not destination.is_symlink():
        return False
    target = _resolved_path(destination)
    return target in _managed_targets(source_root, managed_source_roots, skill_name)


def _validate_sources(source_root: Path, skill_names: Sequence[str]) -> None:
    for skill_name in skill_names:
        source = source_root / skill_name
        skill_file = source / "SKILL.md"
        if not source.is_dir() or not skill_file.is_file():
            raise FileNotFoundError(f"missing skill source: {skill_file}")


def validate_public_skill_payload(public_skills_root: Path, skill_names: Sequence[str]) -> None:
    """Refuse to adopt an existing public root without Sky CUA ownership evidence."""
    public_skills_root = _lexical_absolute(public_skills_root)
    root = public_skills_root.parent
    if not root.exists() and not root.is_symlink():
        return
    if not root.is_dir():
        raise ValueError(f"refusing to replace unmanaged public skills root: {root}")

    marker = root / PUBLIC_SKILLS_ROOT_MARKER
    try:
        marker_owned = not marker.is_symlink() and marker.read_text(encoding="utf-8") == (
            PUBLIC_SKILLS_ROOT_MARKER_CONTENT
        )
    except (OSError, UnicodeDecodeError):
        marker_owned = False
    if marker_owned:
        return
    if marker.exists() or marker.is_symlink():
        raise ValueError(f"refusing to replace unmanaged public skills marker: {marker}")

    try:
        manifest = json.loads((root / "RELEASE.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        manifest = None
    release_owned = (
        isinstance(manifest, dict)
        and manifest.get("schema_version") == 1
        and manifest.get("product") == "sky-cua"
        and all((public_skills_root / name / "SKILL.md").is_file() for name in skill_names)
    )
    if not release_owned:
        raise ValueError(f"refusing to replace unmanaged public skills root: {root}")


def plan_skill_links(
    source_root: Path,
    roots: Sequence[Path],
    skill_names: Sequence[str],
    enabled_names: Sequence[str],
    *,
    managed_source_roots: Sequence[Path] = (),
    validation_source_root: Path | None = None,
) -> SkillLinkPlan:
    """Classify every source and destination without mutating the filesystem."""
    source_root = _lexical_absolute(Path(source_root))
    validation_root = _lexical_absolute(
        Path(validation_source_root) if validation_source_root is not None else source_root
    )
    destination_roots = tuple(_lexical_absolute(Path(root)) for root in roots)
    names = tuple(dict.fromkeys(skill_names))
    enabled = set(enabled_names)
    selected = tuple(name for name in names if name in enabled)
    managed_roots = tuple(
        dict.fromkeys(
            (
                *(_lexical_absolute(Path(root)) for root in managed_source_roots),
                *_legacy_checkout_roots(destination_roots, names),
            )
        )
    )

    _validate_sources(validation_root, selected)

    operations: list[tuple[Literal["remove", "replace"], Path, str | None]] = []
    projected: list[Path] = []
    for root in destination_roots:
        for skill_name in names:
            destination = root / skill_name
            managed = _is_managed_link(destination, source_root, managed_roots, skill_name)
            if skill_name in enabled:
                projected.append(destination)
                if destination.is_symlink():
                    try:
                        raw_target = os.readlink(destination)
                    except OSError as error:
                        raise ValueError(
                            f"cannot inspect skill projection: {destination}"
                        ) from error
                    expected = os.fspath(
                        relative_symlink_target(source_root / skill_name, destination)
                    )
                    if raw_target == expected:
                        continue
                    if not managed:
                        raise ValueError(
                            f"refusing to replace unmanaged skill projection: {destination}"
                        )
                    operations.append(("replace", destination, skill_name))
                elif destination.exists():
                    raise ValueError(
                        f"refusing to replace unmanaged skill projection: {destination}"
                    )
                else:
                    operations.append(("replace", destination, skill_name))
            elif managed:
                operations.append(("remove", destination, None))
    return SkillLinkPlan(source_root, tuple(operations), tuple(projected))


def apply_skill_link_plan(plan: SkillLinkPlan) -> tuple[Path, ...]:
    """Apply a plan produced by :func:`plan_skill_links`."""
    for operation, destination, skill_name in plan.operations:
        if operation == "remove":
            destination.unlink()
        else:
            assert skill_name is not None
            replace_relative_symlink(plan.source_root / skill_name, destination)
    return plan.projected


def project_skill_links(
    source_root: Path,
    roots: Sequence[Path],
    skill_names: Sequence[str],
    enabled_names: Sequence[str],
    *,
    managed_source_roots: Sequence[Path] = (),
) -> tuple[Path, ...]:
    """Project selected skills into one or more destination roots.

    Enabled entries must be absent or symlinks to the canonical source (or to
    an explicitly managed legacy source root).  Existing directories, files,
    and arbitrary symlinks are never overwritten.  Disabled managed links are
    removed while unrelated user entries remain untouched.  All source and
    destination checks happen before the first mutation.
    """
    return apply_skill_link_plan(
        plan_skill_links(
            source_root,
            roots,
            skill_names,
            enabled_names,
            managed_source_roots=managed_source_roots,
        )
    )
