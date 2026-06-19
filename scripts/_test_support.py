"""Shared fixtures-on-disk helpers for bundle-shaped test trees."""

from __future__ import annotations

import json
from pathlib import Path

from _plugin_bundle import (
    current_runtime_platform,
    runtime_binary_names,
    runtime_binary_path,
)


def write_minimal_bundle(root: Path, *, binaries: list[str]) -> None:
    write_minimal_bundle_sources(root)
    (root / "bin").mkdir(parents=True, exist_ok=True)
    for binary_name in binaries:
        relative_name = binary_name
        if binary_name in runtime_binary_names():
            relative_name = (
                runtime_binary_path(current_runtime_platform(), binary_name.removesuffix(".exe"))
                .as_posix()
                .removeprefix("bin/")
            )
        binary_path = root / "bin" / relative_name
        binary_path.parent.mkdir(parents=True, exist_ok=True)
        binary_path.write_text(binary_name, encoding="utf-8")


def write_minimal_bundle_sources(root: Path) -> None:
    (root / ".codex-plugin").mkdir(parents=True)
    (root / ".codex-plugin" / "plugin.json").write_text(
        json.dumps({"version": "0.1.0"}),
        encoding="utf-8",
    )
    (root / ".claude-plugin").mkdir(parents=True)
    (root / ".claude-plugin" / "plugin.json").write_text(
        json.dumps({"name": "sky-cua", "version": "0.1.0"}),
        encoding="utf-8",
    )
    (root / ".claude-plugin" / "marketplace.json").write_text(
        json.dumps({"name": "sky-cua", "plugins": []}),
        encoding="utf-8",
    )
    (root / ".mcp.json").write_text("{}", encoding="utf-8")
    (root / "bin").mkdir(parents=True, exist_ok=True)
    (root / "bin" / "sky-cua-client").write_text("#!/bin/sh\n", encoding="utf-8")
    (root / "bin" / "sky-cua-service").write_text("#!/bin/sh\n", encoding="utf-8")
    (root / "bin" / "sky-cua-overlay-host").write_text("#!/bin/sh\n", encoding="utf-8")
    (root / "bin" / "sky-cua-browser-preflight").write_text("#!/bin/sh\n", encoding="utf-8")
    (root / "skills" / "computer-use").mkdir(parents=True)
    (root / "skills" / "computer-use" / "SKILL.md").write_text(
        "skill",
        encoding="utf-8",
    )
    (root / "skills" / "browser-use").mkdir(parents=True)
    (root / "skills" / "browser-use" / "SKILL.md").write_text(
        "skill",
        encoding="utf-8",
    )
    (root / "skills" / "phone-use").mkdir(parents=True)
    (root / "skills" / "phone-use" / "SKILL.md").write_text(
        "skill",
        encoding="utf-8",
    )
    (root / "docs" / "operations").mkdir(parents=True)
    (root / "docs" / "operations" / "testing-vm-desktop-smokes.md").write_text(
        "testing vm desktop smoke notes\n",
        encoding="utf-8",
    )
    (root / "resources" / "app-instructions").mkdir(parents=True)
    (root / "resources" / "app-instructions" / "index.json").write_text(
        "{}",
        encoding="utf-8",
    )


def tracked_minimal_bundle_files() -> list[Path]:
    return [
        Path(".claude-plugin/plugin.json"),
        Path(".claude-plugin/marketplace.json"),
        Path(".codex-plugin/plugin.json"),
        Path("bin/sky-cua-client"),
        Path("bin/sky-cua-service"),
        Path("bin/sky-cua-overlay-host"),
        Path("bin/sky-cua-browser-preflight"),
        Path("skills/computer-use/SKILL.md"),
        Path("skills/browser-use/SKILL.md"),
        Path("skills/phone-use/SKILL.md"),
        Path("docs/operations/testing-vm-desktop-smokes.md"),
        Path("resources/app-instructions/index.json"),
    ]
