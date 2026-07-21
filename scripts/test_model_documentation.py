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


def test_browser_plugin_routes_directly_to_host_provided_iab(tmp_path: Path) -> None:
    output = tmp_path / "docs"
    build(output)
    skill = (output / "skills/browser-use/SKILL.md").read_text(encoding="utf-8")
    reference = (output / "references/browser.md").read_text(encoding="utf-8")
    recipe = (output / "recipes/browser-workflows.md").read_text(encoding="utf-8")
    normalized_skill = " ".join(skill.split())
    normalized_reference = " ".join(reference.split())

    assert "Browser plugin (by name, mention, or plugin reference)" in normalized_skill
    assert "do not probe direct Browser tools or the Chrome extension bridge first" in normalized_skill
    assert 'transport === "host_provided_iab"' in skill
    assert "returns no Agent" in skill
    assert "never assign its result" in skill
    assert "Do not select, open, or test it" in skill
    assert "without probing the Chrome extension bridge first" in normalized_reference
    assert "does not require `markDeliverable()`" in normalized_reference
    assert "image emission does not require marking the tab deliverable" in recipe
    assert "only for an unfamiliar command or after the happy path fails" in normalized_skill
    assert "mcp__node_repl__js" in recipe
    assert 'entry.transport === "host_provided_iab"' in recipe
    assert "await setupBrowserRuntime({ globals: globalThis });" in recipe
    assert "= await setupBrowserRuntime" not in recipe
    assert "await nodeRepl.emitImage(await tab.screenshot())" in recipe
    assert "markDeliverable" not in recipe

    example = (output / "examples/browser/iab-screenshot.mjs").read_text(encoding="utf-8")
    assert 'entry.transport === "host_provided_iab"' in example
    assert "= await setupBrowserRuntime" not in example
    assert "await nodeRepl.emitImage(await tab.screenshot())" in example
    assert "markDeliverable" not in example


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
    assert len(inventory["phone"]["wire_operations"]) == 27
    assert "phone" not in inventory["phone"]["wire_operations"]
    assert {"bind", "close", "disconnected", "request"} <= set(inventory["phone"]["client_members"])
    assert {"disconnected", "info", "selector", "serial", "session_id"} <= set(
        inventory["phone"]["session_members"]
    )
    assert {"bytes", "dataUrl", "emitImage", "path"} <= set(
        inventory["phone"]["screenshot_members"]
    )
    assert {record["name"] for record in inventory["phone"]["declarations"]} == {
        "client.ts",
        "index.ts",
        "protocol.ts",
        "screenshot.ts",
        "transport.ts",
    }
    assert len(inventory["phone"]["protocol_sha256"]) == 64
    assert len(inventory["computer"]["protocol_sha256"]) == 64
    assert [tool["name"] for tool in inventory["node_repl"]["tools"]] == [
        "js",
        "js_reset",
        "js_add_node_module_dir",
    ]


def test_capability_inventory_uses_release_manifest_vocabulary(tmp_path: Path) -> None:
    output = tmp_path / "docs"
    build(output)
    inventory = json.loads(
        (output / "inventories/capability-inventory.json").read_text(encoding="utf-8")
    )
    assert inventory["supported"] == [
        "browser-persistent-js",
        "computer-use-persistent-js",
        "phone-use-persistent-js",
        "node-repl-mcp",
        "ocr-pdf-image-file-toolbox",
        "system-chrome-family-playwright",
    ]


def test_installed_example_runner_is_shipped_and_parseable(tmp_path: Path) -> None:
    output = tmp_path / "docs"
    build(output)
    runner = output / "bin/run-examples.py"
    runner_text = runner.read_text(encoding="utf-8")
    compile(runner_text, runner.as_posix(), "exec")
    assert "IMAGE_EXAMPLES" in runner_text
    assert "image_count == 0" in runner_text
    assert "copied.read_bytes() != binary.read_bytes()" in runner_text
    assert "Sharp example did not write a WebP output" in runner_text
    assert sorted(path.suffix for path in (output / "examples").rglob("*.mjs")) == [".mjs"] * 11
    for relative in (
        "examples/computer/screenshot.mjs",
        "examples/images/canvas-pixelmatch.mjs",
        "examples/images/sharp-transform.mjs",
    ):
        assert "await nodeRepl.emitImage" in (output / relative).read_text(encoding="utf-8")


def test_node_repl_reference_documents_example_environment_and_runtime_keys(
    tmp_path: Path,
) -> None:
    output = tmp_path / "docs"
    build(output)
    reference = (output / "references/node-repl.md").read_text(encoding="utf-8")
    for key in (
        "nodeRepl.env",
        "NODE_REPL_PUBLIC_ENV",
        "SKY_CUA_EXAMPLE_INPUT_FILE",
        "SKY_CUA_EXAMPLE_IMAGE",
        "SKY_CUA_EXAMPLE_PDF",
        "nodeRepl.runtime",
        "pdfjs.{root,cMapUrl,standardFontDataUrl,wasmUrl,workerSrc}",
        "tesseract.{tessdataRoot,languages}",
    ):
        assert key in reference


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
