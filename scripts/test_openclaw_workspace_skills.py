from __future__ import annotations

from pathlib import Path

import pytest

import sync_openclaw_workspace_skills as sync_skills


def write_skill(root: Path, name: str, marker: str) -> None:
    skill = root / name
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_text(f"---\nname: {name}\n---\n{marker}\n", encoding="utf-8")


def read_marker(root: Path, name: str) -> str:
    return (root / name / "SKILL.md").read_text(encoding="utf-8")


def test_sync_openclaw_workspace_skills_copies_bundled_skills_and_preserves_others(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "bundle" / "skills"
    dest_root = tmp_path / "openclaw" / "skills"
    write_skill(source_root, "browser-use", "new browser")
    write_skill(source_root, "computer-use", "new computer")
    write_skill(dest_root, "browser-use", "old browser")
    write_skill(dest_root, "computer-use", "old computer")
    write_skill(dest_root, "unrelated", "keep me")

    sync_skills.sync_openclaw_workspace_skills(source_root, dest_root)

    assert "new browser" in read_marker(dest_root, "browser-use")
    assert "new computer" in read_marker(dest_root, "computer-use")
    assert "keep me" in read_marker(dest_root, "unrelated")
    assert not (dest_root / sync_skills.STAGE_DIR_NAME).exists()


def test_sync_openclaw_workspace_skills_missing_source_preserves_existing_destinations(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "bundle" / "skills"
    dest_root = tmp_path / "openclaw" / "skills"
    write_skill(source_root, "browser-use", "new browser")
    write_skill(dest_root, "browser-use", "old browser")
    write_skill(dest_root, "computer-use", "old computer")

    with pytest.raises(FileNotFoundError, match="computer-use"):
        sync_skills.sync_openclaw_workspace_skills(source_root, dest_root)

    assert "old browser" in read_marker(dest_root, "browser-use")
    assert "old computer" in read_marker(dest_root, "computer-use")


def test_sync_openclaw_workspace_skills_recovers_stage_before_source_validation(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "bundle" / "skills"
    dest_root = tmp_path / "openclaw" / "skills"
    write_skill(source_root, "browser-use", "new browser")
    write_skill(dest_root, "browser-use", "interrupted browser")
    write_skill(dest_root, "computer-use", "old computer")
    stage_root = dest_root / sync_skills.STAGE_DIR_NAME
    write_skill(stage_root / sync_skills.BACKUP_DIR_NAME, "browser-use", "old browser")

    with pytest.raises(FileNotFoundError, match="computer-use"):
        sync_skills.sync_openclaw_workspace_skills(source_root, dest_root)

    assert "old browser" in read_marker(dest_root, "browser-use")
    assert "old computer" in read_marker(dest_root, "computer-use")
    assert not stage_root.exists()


def test_sync_openclaw_workspace_skills_rolls_back_partial_replacement(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source_root = tmp_path / "bundle" / "skills"
    dest_root = tmp_path / "openclaw" / "skills"
    write_skill(source_root, "browser-use", "new browser")
    write_skill(source_root, "computer-use", "new computer")
    write_skill(dest_root, "browser-use", "old browser")
    write_skill(dest_root, "computer-use", "old computer")
    original_move_path = sync_skills._move_path

    def fail_staged_browser_use(source: Path, destination: Path) -> None:
        if source.name == "browser-use" and source.parent.name == sync_skills.STAGE_DIR_NAME:
            raise OSError("forced move failure")
        original_move_path(source, destination)

    monkeypatch.setattr(sync_skills, "_move_path", fail_staged_browser_use)

    with pytest.raises(OSError, match="forced move failure"):
        sync_skills.sync_openclaw_workspace_skills(source_root, dest_root)

    assert "old browser" in read_marker(dest_root, "browser-use")
    assert "old computer" in read_marker(dest_root, "computer-use")
    assert not (dest_root / sync_skills.STAGE_DIR_NAME).exists()


def test_sync_openclaw_workspace_skills_does_not_clobber_user_backup_paths(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "bundle" / "skills"
    dest_root = tmp_path / "openclaw" / "skills"
    write_skill(source_root, "browser-use", "new browser")
    write_skill(source_root, "computer-use", "new computer")
    write_skill(dest_root, "browser-use", "old browser")
    write_skill(dest_root, "computer-use", "old computer")
    user_backup = dest_root / ".browser-use.backup"
    user_backup.write_text("user-owned backup\n", encoding="utf-8")

    sync_skills.sync_openclaw_workspace_skills(source_root, dest_root)

    assert user_backup.read_text(encoding="utf-8") == "user-owned backup\n"
    assert "new browser" in read_marker(dest_root, "browser-use")
    assert "new computer" in read_marker(dest_root, "computer-use")


def test_sync_openclaw_workspace_skills_recovers_uncommitted_stage(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "bundle" / "skills"
    dest_root = tmp_path / "openclaw" / "skills"
    write_skill(source_root, "browser-use", "fresh browser")
    write_skill(source_root, "computer-use", "fresh computer")
    write_skill(dest_root, "browser-use", "interrupted browser")
    write_skill(dest_root, "computer-use", "old computer")
    stage_root = dest_root / sync_skills.STAGE_DIR_NAME
    write_skill(stage_root / sync_skills.BACKUP_DIR_NAME, "browser-use", "old browser")

    sync_skills.sync_openclaw_workspace_skills(source_root, dest_root)

    assert "fresh browser" in read_marker(dest_root, "browser-use")
    assert "fresh computer" in read_marker(dest_root, "computer-use")
    assert not stage_root.exists()


def test_sync_openclaw_workspace_skills_recovers_new_destination_without_backup(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "bundle" / "skills"
    dest_root = tmp_path / "openclaw" / "skills"
    write_skill(source_root, "browser-use", "fresh browser")
    write_skill(source_root, "computer-use", "fresh computer")
    write_skill(dest_root, "browser-use", "interrupted browser")
    stage_root = dest_root / sync_skills.STAGE_DIR_NAME
    stage_root.mkdir(parents=True)
    installed_marker = stage_root / sync_skills.INSTALLED_DIR_NAME / "browser-use"
    installed_marker.parent.mkdir()
    installed_marker.write_text("ok\n", encoding="utf-8")
    write_skill(stage_root, "computer-use", "staged computer")

    sync_skills.sync_openclaw_workspace_skills(source_root, dest_root)

    assert "fresh browser" in read_marker(dest_root, "browser-use")
    assert "fresh computer" in read_marker(dest_root, "computer-use")
    assert not stage_root.exists()


def test_sync_openclaw_workspace_skills_rolls_back_new_destination_after_move_failure(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    source_root = tmp_path / "bundle" / "skills"
    dest_root = tmp_path / "openclaw" / "skills"
    write_skill(source_root, "browser-use", "new browser")
    write_skill(source_root, "computer-use", "new computer")
    original_move_path = sync_skills._move_path

    def move_browser_then_fail(source: Path, destination: Path) -> None:
        original_move_path(source, destination)
        if source.name == "browser-use" and source.parent.name == sync_skills.STAGE_DIR_NAME:
            raise OSError("forced post-move failure")

    monkeypatch.setattr(sync_skills, "_move_path", move_browser_then_fail)

    with pytest.raises(OSError, match="forced post-move failure"):
        sync_skills.sync_openclaw_workspace_skills(source_root, dest_root)

    assert not (dest_root / "browser-use").exists()
    assert not (dest_root / "computer-use").exists()
    assert not (dest_root / sync_skills.STAGE_DIR_NAME).exists()
