"""Physical install and stable public-root topology for standalone sky-cua."""

from __future__ import annotations

import json
import os
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from _skill_projection import (
    PUBLIC_SKILLS_ROOT_MARKER,
    PUBLIC_SKILLS_ROOT_MARKER_CONTENT,
    replace_relative_symlink,
)


@dataclass(frozen=True)
class InstallRootPlan:
    """Validated physical roots involved in one install transition."""

    stop_roots: tuple[Path, ...]
    obsolete_payload_roots: tuple[Path, ...]


def public_root(home: Path) -> Path:
    """Return the stable agent-facing install ABI, independent of XDG storage."""
    return home / ".local/share/sky-cua"


def _absolute_link_target(link: Path) -> Path:
    raw = link.readlink()
    target = raw if raw.is_absolute() else link.parent / raw
    return Path(os.path.abspath(target))


def _roots_overlap(first: Path, second: Path) -> bool:
    first = Path(os.path.abspath(first))
    second = Path(os.path.abspath(second))
    return first.is_relative_to(second) or second.is_relative_to(first)


def _validated_installed_root(
    path: Path,
    *,
    target: str,
    skill_names: Sequence[str],
) -> bool:
    if path.is_symlink() or not path.is_dir():
        return False
    try:
        manifest = json.loads((path / "RELEASE.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return False
    if manifest.get("schema_version") != 1 or manifest.get("product") != "sky-cua":
        return False
    if manifest.get("target") != target:
        return False
    required = (
        "bin/sky-cua-client",
        "codex/openai-bundled/.agents/plugins/marketplace.json",
        *(f"skills/{name}/SKILL.md" for name in skill_names),
    )
    return all((path / relative).is_file() for relative in required)


def _managed_public_skills_root(path: Path, *, skill_names: Sequence[str]) -> bool:
    marker = path / PUBLIC_SKILLS_ROOT_MARKER
    try:
        if marker.is_symlink() or marker.read_text(encoding="utf-8") != (
            PUBLIC_SKILLS_ROOT_MARKER_CONTENT
        ):
            return False
    except (OSError, UnicodeDecodeError):
        return False
    return all((path / "skills" / name / "SKILL.md").is_file() for name in skill_names)


def _managed_public_target(
    public: Path,
    *,
    target: str,
    skill_names: Sequence[str],
) -> tuple[Path, bool] | None:
    """Classify an existing rendezvous without granting authority over arbitrary paths."""
    if public.is_symlink():
        candidate = _absolute_link_target(public)
        if _validated_installed_root(candidate, target=target, skill_names=skill_names):
            return candidate, True
        if _managed_public_skills_root(candidate, skill_names=skill_names):
            return candidate, False
        raise ValueError(f"refusing to replace an unmanaged sky-cua rendezvous: {public}")
    if not public.exists():
        return None
    if _validated_installed_root(public, target=target, skill_names=skill_names):
        return public, True
    if _managed_public_skills_root(public, skill_names=skill_names):
        return public, False
    raise ValueError(f"refusing to replace an unmanaged sky-cua rendezvous: {public}")


def prepare_install_roots(
    install: Path,
    public: Path,
    *,
    target: str,
    skill_names: Sequence[str],
) -> InstallRootPlan:
    """Validate physical/public topology and return roots whose runtimes must stop."""
    install = Path(os.path.abspath(install))
    public = Path(os.path.abspath(public))
    if install != public and _roots_overlap(install, public):
        raise ValueError(
            f"physical sky-cua root overlaps its public rendezvous: {install} and {public}"
        )

    old_public: tuple[Path, bool] | None = None
    if install != public or public.is_symlink():
        old_public = _managed_public_target(
            public,
            target=target,
            skill_names=skill_names,
        )
    roots = [install]
    obsolete: list[Path] = []
    if old_public is not None:
        old_public_target, installed_payload = old_public
        if old_public_target != install:
            if _roots_overlap(old_public_target, install):
                raise ValueError(
                    "new physical sky-cua root overlaps the prior managed root: "
                    f"{install} and {old_public_target}"
                )
            roots.append(old_public_target)
            if installed_payload:
                obsolete.append(old_public_target)
    return InstallRootPlan(tuple(roots), tuple(obsolete))


def converge_public_root(install: Path, public: Path) -> None:
    if install != public:
        replace_relative_symlink(install, public)
