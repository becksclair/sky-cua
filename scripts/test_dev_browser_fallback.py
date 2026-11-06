"""Development Browser fallback owns its runtime, independent of host installs."""

from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path
from typing import Any

import pytest

import build_plugin


def runtime_fixture(root: Path) -> Path:
    for relative, content in {
        "bin/node": "private node",
        "bin/node_repl": "owned launcher",
        "manifest.json": '{"runtime_name":"cua_node"}',
        "lib/node_repl/cli.js": "owned host",
        "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs": "canonical client",
        "lib/node_modules/@heliasar/browser-use/build/sky-enhancements.mjs": "enhancements",
        "share/tessdata/eng.traineddata": "offline asset",
        "licenses/NOTICE": "notices",
    }.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    return root


def test_dev_fallback_stages_complete_runtime_without_other_browser_sources(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runtime = runtime_fixture(tmp_path / "assembled")
    builds: list[Path] = []

    def build() -> Path:
        builds.append(runtime)
        return runtime

    monkeypatch.setattr(build_plugin, "build_development_browser_runtime", build)
    monkeypatch.setattr(build_plugin, "bundled_resource_root", lambda: tmp_path / "no-legacy")
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "no-standalone"))
    staged = tmp_path / "staged"
    build_plugin.stage_openai_bundled_plugins(staged)
    browser = staged / "resources/plugins/openai-bundled/plugins/browser"
    # Copy the plugin as preflight does: the launcher binding must survive this move.
    projected = tmp_path / "codex-cache/browser"
    shutil.copytree(browser, projected)
    config = json.loads((projected / ".mcp.json").read_text(encoding="utf-8"))
    command = config["mcpServers"]["node_repl"]["command"]
    cwd = projected / config["mcpServers"]["node_repl"]["cwd"]
    assert (cwd / command).is_file()
    assert command == "./cua-node/bin/node_repl"
    assert config["mcpServers"]["node_repl"]["cwd"] == "."
    for source in runtime.rglob("*"):
        if source.is_file():
            assert (projected / "cua-node" / source.relative_to(runtime)).read_bytes() == (
                source.read_bytes()
            )
    adapter = (projected / "scripts/browser-client.mjs").read_text(encoding="utf-8")
    assert 'import("@heliasar/browser-use")' in adapter
    assert "RELEASE.json" not in adapter
    assert builds == [runtime]
    assert (staged / "browser/sky-enhancements.mjs").read_text() == "enhancements"
    assert not (staged / "resources/plugins/openai-bundled/plugins/browser-use").exists()


def test_dev_runtime_refresh_uses_existing_full_assembler(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    commands: list[list[str]] = []

    def run(command: list[str], **_kwargs: Any) -> subprocess.CompletedProcess[str]:
        commands.append(command)
        return subprocess.CompletedProcess(command, 0)

    monkeypatch.setattr(build_plugin, "REPO_ROOT", tmp_path)
    monkeypatch.setattr(subprocess, "run", run)
    runtime = build_plugin.build_development_browser_runtime()
    assert runtime == tmp_path / "out/components/cua-node-linux-x64-glibc"
    assert commands == [
        [
            build_plugin.sys.executable,
            str(tmp_path / "scripts/assemble_cua_node.py"),
            "--output-root",
            str(runtime),
            "--allow-development-dirty",
            "--json",
        ]
    ]


def test_standalone_stage_uses_fixed_root_adapter_without_duplicate_runtime(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runtime = runtime_fixture(tmp_path / "assembled")
    canonical = runtime / "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs"
    monkeypatch.setattr(build_plugin, "build_canonical_browser_client", lambda: canonical)
    monkeypatch.setattr(build_plugin, "bundled_resource_root", lambda: tmp_path / "no-legacy")

    def unexpected_build() -> Path:
        pytest.fail("standalone stage must use its release runtime, not build another")

    monkeypatch.setattr(build_plugin, "build_development_browser_runtime", unexpected_build)
    staged = tmp_path / "standalone-core"
    build_plugin.stage_openai_bundled_plugins(staged, browser_runtime_mode="standalone")
    browser = staged / "resources/plugins/openai-bundled/plugins/browser"
    assert not (browser / "cua-node").exists()
    config = json.loads((browser / ".mcp.json").read_text(encoding="utf-8"))
    assert config["mcpServers"]["node_repl"]["command"] == "node_repl"
    assert "RELEASE.json" in (browser / "scripts/browser-client.mjs").read_text()


def test_development_cache_version_is_stable_and_tracks_runtime_manifest(tmp_path: Path) -> None:
    runtime = runtime_fixture(tmp_path / "runtime")
    first, second = tmp_path / "first", tmp_path / "second"
    build_plugin.stage_development_browser_fallback(first, runtime)
    build_plugin.stage_development_browser_fallback(second, runtime)

    def version(root: Path) -> str:
        return str(json.loads((root / ".codex-plugin/plugin.json").read_text())["version"])

    assert version(first) == version(second)
    assert "-dev." in version(first)
    (runtime / "manifest.json").write_text('{"checksums":{"client":"new"}}')
    build_plugin.stage_development_browser_fallback(second, runtime)
    assert version(first) != version(second)


@pytest.mark.parametrize(
    "relative",
    [
        "cua-node/manifest.json",
        "scripts/browser-client.mjs",
        ".mcp.json",
        "assets/browser.png",
        "skills/control-in-app-browser/SKILL.md",
        ".codex-plugin/plugin.json",
    ],
)
def test_development_cache_digest_tracks_all_small_inputs(tmp_path: Path, relative: str) -> None:
    runtime = runtime_fixture(tmp_path / "runtime")
    browser = tmp_path / "browser"
    build_plugin.stage_development_browser_fallback(browser, runtime)
    before = build_plugin.development_browser_fallback_digest(browser)
    changed = browser / relative
    if relative == ".codex-plugin/plugin.json":
        manifest = json.loads(changed.read_text())
        manifest["description"] += " changed"
        changed.write_text(json.dumps(manifest))
    else:
        changed.write_bytes(changed.read_bytes() + b"\nchanged")
    assert build_plugin.development_browser_fallback_digest(browser) != before


def test_development_cache_digest_ignores_derived_version_and_does_not_read_runtime(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runtime = runtime_fixture(tmp_path / "runtime")
    browser = tmp_path / "browser"
    build_plugin.stage_development_browser_fallback(browser, runtime)
    before = build_plugin.development_browser_fallback_digest(browser)
    manifest_path = browser / ".codex-plugin/plugin.json"
    manifest = json.loads(manifest_path.read_text())
    manifest["version"] = "different-derived-version"
    manifest_path.write_text(json.dumps(manifest))
    read_bytes = Path.read_bytes

    def guarded_read(path: Path) -> bytes:
        if path.is_relative_to(browser / "cua-node"):
            assert path == browser / "cua-node/manifest.json"
        return read_bytes(path)

    monkeypatch.setattr(Path, "read_bytes", guarded_read)
    assert build_plugin.development_browser_fallback_digest(browser) == before
