"""Tests for the global agents skill symlink sync."""

from pathlib import Path

import pytest

import sync_agent_skills as sync_skills


def _make_source(root: Path, names: list[str]) -> Path:
    source = root / "skills"
    for name in names:
        (source / name).mkdir(parents=True)
        (source / name / "SKILL.md").write_text(f"# {name}\n", encoding="utf-8")
    return source


def test_sync_creates_symlinks_to_repo_skills(tmp_path: Path) -> None:
    names = ["computer-use", "browser-use"]
    source = _make_source(tmp_path, names)
    dest = tmp_path / "agents-skills"

    sync_skills.sync_agents_skill_symlinks(source, dest, names)

    for name in names:
        link = dest / name
        assert link.is_symlink()
        assert link.resolve() == (source / name).resolve()


def test_sync_repoints_stale_worktree_symlink(tmp_path: Path) -> None:
    # A deploy from a worktree leaves the link at the worktree path; a sync
    # from the main checkout must repoint it, even when the old target is gone.
    names = ["computer-use"]
    source = _make_source(tmp_path, names)
    dest = tmp_path / "agents-skills"
    dest.mkdir()
    (dest / "computer-use").symlink_to(tmp_path / "worktrees" / "gone" / "computer-use")

    sync_skills.sync_agents_skill_symlinks(source, dest, names)

    assert (dest / "computer-use").resolve() == (source / "computer-use").resolve()


def test_sync_replaces_plain_directory_destination(tmp_path: Path) -> None:
    names = ["phone-use"]
    source = _make_source(tmp_path, names)
    dest = tmp_path / "agents-skills"
    (dest / "phone-use").mkdir(parents=True)
    (dest / "phone-use" / "SKILL.md").write_text("stale copy\n", encoding="utf-8")

    sync_skills.sync_agents_skill_symlinks(source, dest, names)

    assert (dest / "phone-use").is_symlink()
    assert (dest / "phone-use").resolve() == (source / "phone-use").resolve()


def test_sync_missing_source_fails_and_leaves_destination_alone(tmp_path: Path) -> None:
    names = ["computer-use"]
    source = tmp_path / "skills"  # never created
    dest = tmp_path / "agents-skills"
    dest.mkdir()
    existing = dest / "computer-use"
    existing.symlink_to(tmp_path / "somewhere")

    with pytest.raises(FileNotFoundError):
        sync_skills.sync_agents_skill_symlinks(source, dest, names)

    assert existing.is_symlink()
    assert Path(str(existing.readlink())) == tmp_path / "somewhere"


def test_sync_removes_only_disabled_managed_surface_links(tmp_path: Path) -> None:
    names = ["computer-use", "browser-use", "phone-use"]
    source = _make_source(tmp_path, names)
    for name in names:
        (source / name / "SKILL.md").write_text(f"# sky-cua {name}\n", encoding="utf-8")
    dest = tmp_path / "agents-skills"

    sync_skills.sync_agents_skill_symlinks(
        source,
        dest,
        names,
        enabled_surfaces=frozenset({"desktop", "browser", "phone"}),
    )
    assert (dest / "browser-use").is_symlink()

    sync_skills.sync_agents_skill_symlinks(
        source,
        dest,
        names,
        enabled_surfaces=frozenset({"desktop", "phone"}),
    )
    assert not (dest / "browser-use").exists()
    assert (dest / "computer-use").is_symlink()
    assert (dest / "phone-use").is_symlink()


def test_sync_does_not_touch_unrelated_destination_entries(tmp_path: Path) -> None:
    names = ["browser-use"]
    source = _make_source(tmp_path, names)
    dest = tmp_path / "agents-skills"
    (dest / "unrelated-skill").mkdir(parents=True)
    (dest / "unrelated-skill" / "SKILL.md").write_text("keep me\n", encoding="utf-8")

    sync_skills.sync_agents_skill_symlinks(source, dest, names)

    assert (dest / "unrelated-skill" / "SKILL.md").read_text(encoding="utf-8") == "keep me\n"
