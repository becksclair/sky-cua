from __future__ import annotations

import json
from pathlib import Path

from _install_shared import project_model_skills
from build_model_documentation import INVENTORY_NAMES, build


def test_build_is_deterministic_and_routes_resolve(tmp_path: Path) -> None:
    first = tmp_path / "first"
    second = tmp_path / "second"
    build(first)
    build(second)
    assert {
        path.relative_to(first).as_posix(): path.read_bytes()
        for path in first.rglob("*")
        if path.is_file()
    } == {
        path.relative_to(second).as_posix(): path.read_bytes()
        for path in second.rglob("*")
        if path.is_file()
    }
    for name in INVENTORY_NAMES:
        inventory = first / "inventories" / f"{name}-inventory.json"
        assert json.loads(inventory.read_text(encoding="utf-8"))["schema_version"] == 1


def test_skills_are_compact_progressive_routes(tmp_path: Path) -> None:
    output = tmp_path / "docs"
    build(output)
    for skill in ("browser-use", "computer-use", "phone-use"):
        text = (output / "skills" / skill / "SKILL.md").read_text(encoding="utf-8")
        assert len(text.splitlines()) < 30
        assert "references/node-repl.md" in text
        assert "direct" in text.lower()
        assert "node_repl" in text


def test_inventory_has_no_checkout_paths_or_stale_locked_versions(tmp_path: Path) -> None:
    output = tmp_path / "docs"
    build(output)
    all_bytes = b"\n".join(path.read_bytes() for path in output.rglob("*") if path.is_file())
    assert b"/home/" not in all_bytes
    assert b"24.14.0" in all_bytes
    assert b"1.57.0" in all_bytes
    assert b"5.4.624" in all_bytes
    assert b"7.0.0" in all_bytes
    assert b"0.34.5" in all_bytes


def test_api_inventory_binds_complete_canonical_contracts(tmp_path: Path) -> None:
    output = tmp_path / "docs"
    build(output)
    inventory = json.loads((output / "inventories/api-inventory.json").read_text(encoding="utf-8"))
    assert len(inventory["phone"]["operations"]) == 27
    assert "phone" not in inventory["phone"]["operations"]
    assert len(inventory["phone"]["protocol_sha256"]) == 64
    assert len(inventory["computer"]["protocol_sha256"]) == 64
    assert [tool["name"] for tool in inventory["node_repl"]["tools"]] == [
        "js",
        "js_reset",
        "js_add_node_module_dir",
    ]


def test_installed_example_runner_is_shipped_and_parseable(tmp_path: Path) -> None:
    output = tmp_path / "docs"
    build(output)
    runner = output / "bin/run-examples.py"
    compile(runner.read_text(encoding="utf-8"), runner.as_posix(), "exec")
    assert sorted(path.suffix for path in (output / "examples").rglob("*.mjs")) == [".mjs"] * 10


def test_host_skill_projection_routes_to_exact_generation_and_rejects_unmanaged(
    tmp_path: Path,
) -> None:
    documentation = tmp_path / "release/components/documentation"
    build(documentation)
    skill_root = tmp_path / "host/skills"
    projected = project_model_skills(documentation, skill_root)
    assert [path.name for path in projected] == ["browser-use", "computer-use", "phone-use"]
    for path in projected:
        text = (path / "SKILL.md").read_text(encoding="utf-8")
        assert str(documentation.resolve()) in text
        assert (path / "SKY_CUA_PROJECTION.json").is_file()
    project_model_skills(documentation, skill_root)
    unmanaged = tmp_path / "unmanaged/skills/browser-use"
    unmanaged.mkdir(parents=True)
    try:
        project_model_skills(documentation, unmanaged.parent)
    except ValueError as error:
        assert "unmanaged" in str(error)
    else:
        raise AssertionError("unmanaged skill replacement must fail")
