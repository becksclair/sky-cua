"""Tests for durable global skill projection and its checkout migration rules."""

import os
from pathlib import Path

import pytest

import sync_agent_skills as sync_skills
from _skill_projection import (
    PUBLIC_SKILLS_ROOT_MARKER,
    PUBLIC_SKILLS_ROOT_MARKER_CONTENT,
    project_skill_links,
    relative_symlink_target,
)


def _make_source(root: Path, names: list[str], *, label: str = "") -> Path:
    source = root / "skills"
    for name in names:
        (source / name).mkdir(parents=True)
        (source / name / "SKILL.md").write_text(f"# {label}{name}\n", encoding="utf-8")
    return source


def _make_checkout_source(root: Path, names: list[str], *, label: str = "") -> Path:
    source = _make_source(root, names, label=label)
    (root / "scripts").mkdir()
    (root / "scripts/sync_agent_skills.py").write_text("# fixture\n", encoding="utf-8")
    (root / "crates/sky-cua-client").mkdir(parents=True)
    (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
    (root / "crates/sky-cua-client/Cargo.toml").write_text(
        '[package]\nname = "sky-cua-client"\n', encoding="utf-8"
    )
    return source


def _raw_target(path: Path) -> str:
    return os.readlink(path)


def test_default_sync_copies_payload_and_projects_relative_links(tmp_path: Path) -> None:
    names = ["computer-use", "browser-use"]
    source = _make_source(tmp_path, names)
    public = tmp_path / "public" / "skills"
    dest = tmp_path / "home" / ".agents" / "skills"

    sync_skills.sync_agents_skill_symlinks(
        source,
        dest,
        names,
        enabled_surfaces=frozenset({"desktop", "browser"}),
        public_root=public,
    )

    for name in names:
        link = dest / name
        assert link.is_symlink()
        assert _raw_target(link) == os.fspath(relative_symlink_target(public / name, link))
        assert link.resolve() == (public / name).resolve()
        assert (public / name / "SKILL.md").read_text(encoding="utf-8") == (
            source / name / "SKILL.md"
        ).read_text(encoding="utf-8")
    assert (public.parent / PUBLIC_SKILLS_ROOT_MARKER).is_file()
    assert (public.parent / PUBLIC_SKILLS_ROOT_MARKER).read_text(encoding="utf-8") == (
        PUBLIC_SKILLS_ROOT_MARKER_CONTENT
    )


def test_checkout_links_require_explicit_opt_in(tmp_path: Path) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path, names)
    public = tmp_path / "public" / "skills"
    dest = tmp_path / "agents-skills"

    sync_skills.sync_agents_skill_symlinks(
        source,
        dest,
        names,
        enabled_surfaces=frozenset({"desktop"}),
        public_root=public,
        checkout_links=True,
    )
    link = dest / names[0]
    assert _raw_target(link) == os.fspath(relative_symlink_target(source / names[0], link))
    assert not public.exists()
    assert not (public.parent / PUBLIC_SKILLS_ROOT_MARKER).exists()

    sync_skills.sync_agents_skill_symlinks(
        source,
        dest,
        names,
        enabled_surfaces=frozenset({"desktop"}),
        public_root=public,
    )
    assert _raw_target(link) == os.fspath(relative_symlink_target(public / names[0], link))


def test_default_sync_migrates_link_from_a_prior_live_checkout(tmp_path: Path) -> None:
    names = ["computer-use", "browser-use"]
    source = _make_checkout_source(tmp_path / "current-checkout", names, label="current ")
    stale_source = _make_checkout_source(tmp_path / "stale-worktree", names, label="stale ")
    public = tmp_path / "home/.local/share/sky-cua/skills"
    dest = tmp_path / "home/.agents/skills"
    dest.mkdir(parents=True)
    (dest / "computer-use").symlink_to(stale_source / "computer-use")

    sync_skills.sync_agents_skill_symlinks(
        source,
        dest,
        names,
        enabled_surfaces=frozenset({"desktop", "browser"}),
        public_root=public,
    )

    link = dest / "computer-use"
    assert link.readlink() == relative_symlink_target(public / "computer-use", link)
    assert (link / "SKILL.md").read_text(encoding="utf-8") == "# current computer-use\n"


def test_exact_canonical_relative_link_is_a_raw_readlink_noop(tmp_path: Path) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path, names)
    dest = tmp_path / "agents-skills"
    project_skill_links(source, (dest,), names, names)
    link = dest / names[0]
    before = link.lstat()
    raw_before = _raw_target(link)

    project_skill_links(source, (dest,), names, names)

    after = link.lstat()
    assert _raw_target(link) == raw_before
    assert after.st_ino == before.st_ino
    assert after.st_mtime_ns == before.st_mtime_ns


def test_relative_target_uses_lexical_canonical_paths(tmp_path: Path) -> None:
    source = tmp_path / "checkout" / "skills"
    destination = tmp_path / "home" / ".agents" / "skills" / "computer-use"
    expected = Path(os.path.relpath(source / "computer-use", destination.parent))
    assert relative_symlink_target(source / "computer-use", destination) == expected


def test_absolute_legacy_link_migrates_only_when_root_is_managed(tmp_path: Path) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path / "new", names, label="new ")
    old_source = _make_source(tmp_path / "old", names, label="old ")
    dest = tmp_path / "agents-skills"
    dest.mkdir()
    link = dest / names[0]
    link.symlink_to(old_source / names[0])

    project_skill_links(
        source,
        (dest,),
        names,
        names,
        managed_source_roots=(old_source,),
    )

    assert _raw_target(link) == os.fspath(relative_symlink_target(source / names[0], link))
    assert link.resolve() == (source / names[0]).resolve()


def test_relative_legacy_link_migrates_only_when_root_is_managed(tmp_path: Path) -> None:
    names = ["browser-use"]
    source = _make_source(tmp_path / "new", names)
    old_source = _make_source(tmp_path / "old", names)
    dest = tmp_path / "agents-skills"
    dest.mkdir()
    link = dest / names[0]
    link.symlink_to(os.path.relpath(old_source / names[0], dest))

    project_skill_links(
        source,
        (dest,),
        names,
        names,
        managed_source_roots=(old_source,),
    )

    assert _raw_target(link) == os.fspath(relative_symlink_target(source / names[0], link))


@pytest.mark.parametrize("kind", ["directory", "file"])
def test_enabled_unmanaged_directory_or_file_raises_without_mutation(
    tmp_path: Path, kind: str
) -> None:
    names = ["phone-use"]
    source = _make_source(tmp_path / "source", names)
    dest = tmp_path / "agents-skills"
    destination = dest / names[0]
    if kind == "directory":
        destination.mkdir(parents=True)
        (destination / "keep").write_text("user data\n", encoding="utf-8")
        before = (destination / "keep").read_text(encoding="utf-8")
    else:
        destination.parent.mkdir(parents=True)
        destination.write_text("user file\n", encoding="utf-8")
        before = destination.read_text(encoding="utf-8")

    with pytest.raises(ValueError, match="unmanaged"):
        project_skill_links(source, (dest,), names, names)

    if kind == "directory":
        assert destination.is_dir()
        assert (destination / "keep").read_text(encoding="utf-8") == before
    else:
        assert destination.read_text(encoding="utf-8") == before


def test_enabled_arbitrary_symlink_raises_without_mutation(tmp_path: Path) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path / "source", names)
    unrelated = tmp_path / "unrelated"
    unrelated.mkdir()
    dest = tmp_path / "agents-skills"
    dest.mkdir()
    link = dest / names[0]
    link.symlink_to(unrelated)
    before = _raw_target(link)

    with pytest.raises(ValueError, match="unmanaged"):
        project_skill_links(source, (dest,), names, names)

    assert link.is_symlink()
    assert _raw_target(link) == before


def test_disabled_managed_links_are_removed_and_unmanaged_entries_survive(
    tmp_path: Path,
) -> None:
    names = ["computer-use", "browser-use", "phone-use"]
    source = _make_source(tmp_path / "source", names)
    old_source = _make_source(tmp_path / "old", ["browser-use"])
    dest = tmp_path / "agents-skills"
    project_skill_links(source, (dest,), names, names)
    (dest / "browser-use").unlink()
    (dest / "browser-use").symlink_to(old_source / "browser-use")
    unrelated = tmp_path / "unrelated"
    unrelated.mkdir()
    (dest / "unrelated").symlink_to(unrelated)

    project_skill_links(
        source,
        (dest,),
        names,
        ("computer-use", "phone-use"),
        managed_source_roots=(old_source,),
    )

    assert (dest / "computer-use").is_symlink()
    assert not (dest / "browser-use").exists()
    assert (dest / "unrelated").is_symlink()


def test_missing_source_prevalidation_leaves_existing_projection_untouched(tmp_path: Path) -> None:
    source = _make_source(tmp_path / "source", ["computer-use"])
    dest = tmp_path / "agents-skills"
    project_skill_links(source, (dest,), ["computer-use"], ["computer-use"])
    link = dest / "computer-use"
    before = _raw_target(link)

    with pytest.raises(FileNotFoundError):
        project_skill_links(
            source,
            (dest,),
            ["computer-use", "browser-use"],
            ["computer-use", "browser-use"],
        )

    assert _raw_target(link) == before


def test_sync_uses_custom_home_for_default_public_root(tmp_path: Path, monkeypatch) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path / "source", names)
    custom_home = tmp_path / "custom-home"
    monkeypatch.setenv("HOME", str(custom_home))
    dest = custom_home / ".agents" / "skills"

    sync_skills.sync_agents_skill_symlinks(
        source,
        dest,
        names,
        enabled_surfaces=frozenset({"desktop"}),
    )

    public = custom_home / ".local" / "share" / "sky-cua" / "skills"
    assert (public / names[0] / "SKILL.md").is_file()
    assert (public.parent / PUBLIC_SKILLS_ROOT_MARKER).is_file()
    assert _raw_target(dest / names[0]) == os.fspath(
        relative_symlink_target(public / names[0], dest / names[0])
    )


def test_sync_refuses_foreign_public_root_marker_before_copying(tmp_path: Path) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path / "source", names)
    public = tmp_path / "public" / "skills"
    marker = public.parent / PUBLIC_SKILLS_ROOT_MARKER
    marker.parent.mkdir(parents=True)
    marker.write_text("user-owned\n", encoding="utf-8")

    with pytest.raises(ValueError, match="unmanaged public skills marker"):
        sync_skills.sync_agents_skill_symlinks(
            source,
            tmp_path / "agents-skills",
            names,
            enabled_surfaces=frozenset({"desktop"}),
            public_root=public,
        )

    assert marker.read_text(encoding="utf-8") == "user-owned\n"
    assert not public.exists()


def test_sync_refuses_unmarked_existing_public_root_without_mutation(tmp_path: Path) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path / "source", names, label="new ")
    public = tmp_path / "public/skills"
    keep = public / "computer-use/keep.txt"
    keep.parent.mkdir(parents=True)
    keep.write_text("user-owned\n", encoding="utf-8")

    with pytest.raises(ValueError, match="unmanaged public skills root"):
        sync_skills.sync_agents_skill_symlinks(
            source,
            tmp_path / "agents-skills",
            names,
            enabled_surfaces=frozenset({"desktop"}),
            public_root=public,
        )

    assert keep.read_text(encoding="utf-8") == "user-owned\n"
    assert not (public.parent / PUBLIC_SKILLS_ROOT_MARKER).exists()


def test_sync_projection_conflict_preflight_leaves_owned_payload_untouched(
    tmp_path: Path,
) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path / "source", names, label="new ")
    public = _make_source(tmp_path / "public", names, label="old ")
    marker = public.parent / PUBLIC_SKILLS_ROOT_MARKER
    marker.write_text(PUBLIC_SKILLS_ROOT_MARKER_CONTENT, encoding="utf-8")
    dest = tmp_path / "agents-skills"
    user_skill = dest / "computer-use"
    user_skill.mkdir(parents=True)
    (user_skill / "keep.txt").write_text("user-owned\n", encoding="utf-8")

    with pytest.raises(ValueError, match="unmanaged skill projection"):
        sync_skills.sync_agents_skill_symlinks(
            source,
            dest,
            names,
            enabled_surfaces=frozenset({"desktop"}),
            public_root=public,
        )

    assert (public / "computer-use/SKILL.md").read_text(encoding="utf-8") == (
        "# old computer-use\n"
    )
    assert marker.read_text(encoding="utf-8") == PUBLIC_SKILLS_ROOT_MARKER_CONTENT
    assert (user_skill / "keep.txt").read_text(encoding="utf-8") == "user-owned\n"


def test_sync_accepts_an_existing_standalone_public_payload(tmp_path: Path) -> None:
    names = ["computer-use"]
    source = _make_source(tmp_path / "source", names, label="new ")
    public = _make_source(tmp_path / "public", names, label="installed ")
    (public.parent / "RELEASE.json").write_text(
        '{"schema_version":1,"product":"sky-cua"}\n', encoding="utf-8"
    )
    dest = tmp_path / "agents-skills"

    sync_skills.sync_agents_skill_symlinks(
        source,
        dest,
        names,
        enabled_surfaces=frozenset({"desktop"}),
        public_root=public,
    )

    assert (public / "computer-use/SKILL.md").read_text(encoding="utf-8") == (
        "# new computer-use\n"
    )
    assert (dest / "computer-use").resolve() == (public / "computer-use").resolve()
