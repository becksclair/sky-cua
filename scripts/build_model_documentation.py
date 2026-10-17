#!/usr/bin/env python3
"""Build and validate the immutable model-facing documentation component."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
from pathlib import Path, PurePosixPath

from _plugin_bundle import REPO_ROOT, remove_path

SOURCE = REPO_ROOT / "resources/model-documentation"
INVENTORY_NAMES = ("api", "capability", "example", "routing")
PRODUCTION_PACKAGE = REPO_ROOT / "runtime/cua-node/production/package.json"
LOCKED_NPM_PACKAGES = {
    "playwright": "playwright",
    "pdfjs": "pdfjs-dist",
    "tesseract_js": "tesseract.js",
    "sharp": "sharp",
    "sharp_linux_x64": "@img/sharp-linux-x64",
    "sharp_libvips_linux_x64": "@img/sharp-libvips-linux-x64",
    "canvas_linux_x64_gnu": "@napi-rs/canvas-linux-x64-gnu",
    "pixelmatch": "pixelmatch",
}


def production_pins() -> dict[str, str]:
    package = json.loads(PRODUCTION_PACKAGE.read_text(encoding="utf-8"))
    return package["dependencies"]


TOOLBOX = SOURCE / "references/toolbox.md"
TOOLBOX_VERSION_PATTERNS = {
    r"Acorn (\S+) and": "acorn",
    r"Acorn Walk (\S+) for": "acorn-walk",
    r"Sharp (\S+) with": "sharp",
    r"libvips (\S+) for": "@img/sharp-libvips-linux-x64",
    r"`@napi-rs/canvas` (\S+) for": "@napi-rs/canvas",
    r"pixelmatch (\S+) for": "pixelmatch",
    r"PDF\.js (\S+) with": "pdfjs-dist",
    r"Tesseract\.js (\S+) with": "tesseract.js",
    r"Playwright (\S+) using": "playwright",
}


def toolbox_version_drift(pins: dict[str, str]) -> list[str]:
    """Hand-written toolbox prose versions that disagree with the production pins."""
    prose = TOOLBOX.read_text(encoding="utf-8")
    drift: list[str] = []
    for pattern, package in TOOLBOX_VERSION_PATTERNS.items():
        match = re.search(pattern, prose)
        stated = match.group(1) if match is not None else None
        if stated != pins[package]:
            drift.append(
                f"{TOOLBOX.relative_to(REPO_ROOT)} states {package} {stated}, pinned {pins[package]}"
            )
    return drift


def locked_versions() -> dict[str, object]:
    pins = production_pins()
    return {
        "node": "24.14.0",
        **{key: pins[name] for key, name in LOCKED_NPM_PACKAGES.items()},
        "codecs": ["bmp", "jpeg", "png", "webp", "zlib"],
    }


UNSUPPORTED = (
    "@heliasar/sky-cua/advanced",
    "linux-arm64-node-repl",
    "linux-musl-node-repl",
    "macos-node-repl",
    "npm-publication",
    "windows-node-repl",
)
LINK = re.compile(r"(?<![A-Za-z0-9_./-])((?:skills|references|recipes|examples)/[A-Za-z0-9_./-]+)")


def _sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _record(root: Path, path: Path) -> dict[str, object]:
    return {
        "path": path.relative_to(root).as_posix(),
        "sha256": _sha(path),
        "size_bytes": path.stat().st_size,
    }


def _example_record(root: Path, path: Path) -> dict[str, object]:
    record = _record(root, path)
    category = path.relative_to(root / "examples").parts[0]
    return {
        **record,
        "capability": category,
        "expected": "completes without error and reports or emits the documented artifact",
    }


def _files(root: Path, prefix: str) -> list[Path]:
    return sorted((root / prefix).rglob("*"), key=lambda item: item.as_posix())


def _validate_source(root: Path) -> None:
    for path in sorted(root.rglob("*"), key=lambda item: item.as_posix()):
        if path.is_symlink() or (not path.is_dir() and not path.is_file()):
            raise ValueError(f"documentation contains unsupported entry: {path}")
        if not path.is_file():
            continue
        data = path.read_bytes()
        if b"/home/" in data or b"/projects/sky-cua" in data:
            raise ValueError(f"documentation contains checkout path: {path}")
        if path.suffix == ".md":
            text = data.decode("utf-8")
            for relative in LINK.findall(text):
                normalized = PurePosixPath(relative)
                if ".." in normalized.parts or not (root / normalized).is_file():
                    raise ValueError(f"broken documentation route {relative!r} in {path}")


def _write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def build(output: Path, *, phone_declaration_root: Path | None = None) -> None:
    _validate_source(SOURCE)
    remove_path(output)
    shutil.copytree(SOURCE, output)
    docs = [
        _record(output, path)
        for section in ("skills", "references", "recipes")
        for path in _files(output, section)
        if path.is_file()
    ]
    examples = [
        _example_record(output, path) for path in _files(output, "examples") if path.is_file()
    ]
    browser_api = json.loads(
        (REPO_ROOT / "packages/browser-use/fixtures/api-surface.json").read_text(encoding="utf-8")
    )
    browser_commands = json.loads(
        (REPO_ROOT / "packages/browser-use/fixtures/commands.json").read_text(encoding="utf-8")
    )
    phone_source = (REPO_ROOT / "packages/sky-cua-js/src/phone/protocol.ts").read_text(
        encoding="utf-8"
    )
    phone_operations = sorted(
        set(re.findall(r'export type Phone\w+Request = \{ type: "([a-z_]+)"', phone_source))
        - {"phone"}
    )
    if len(phone_operations) != 27:
        raise ValueError(f"expected 27 Phone request operations, found {len(phone_operations)}")
    phone_protocol_path = REPO_ROOT / "packages/sky-cua-js/src/phone/protocol.ts"
    declaration_root = phone_declaration_root or REPO_ROOT / "packages/sky-cua-js/src/phone"
    declaration_paths = sorted(
        declaration_root.glob("*.d.ts" if phone_declaration_root is not None else "*.ts"),
        key=lambda path: path.name,
    )
    if not declaration_paths:
        raise ValueError(f"Phone declaration inventory is empty: {declaration_root}")
    computer_protocol_path = REPO_ROOT / "packages/sky-cua-js/src/protocol/generated.ts"
    node_repl_tools_path = (
        REPO_ROOT / "runtime/cua-node/test/fixtures/upstream-5307/tools-list.json"
    )
    node_repl_tools = json.loads(node_repl_tools_path.read_text(encoding="utf-8"))
    api_inventory = {
        "schema_version": 1,
        "browser": {"api_surface": browser_api, "commands": browser_commands},
        "computer": {
            "package": "@heliasar/sky-cua",
            "operations": [
                "activate_window",
                "click",
                "drag",
                "get_screenshot",
                "move",
                "press_key",
                "scroll",
                "type_text",
            ],
            "protocol_sha256": _sha(computer_protocol_path),
            "protocol_size_bytes": computer_protocol_path.stat().st_size,
        },
        "phone": {
            "package": "@heliasar/sky-cua/phone",
            "wire_operations": phone_operations,
            "client_members": [*phone_operations, "bind", "close", "disconnected", "request"],
            "session_members": [
                *sorted(
                    set(phone_operations) - {"connect", "list_devices", "pair_wireless", "status"}
                ),
                "disconnected",
                "info",
                "selector",
                "serial",
                "session_id",
            ],
            "screenshot_members": [
                "bytes",
                "dataUrl",
                "emitImage",
                "inlineBytes",
                "inlineDataUrl",
                "mimeType",
                "path",
                "response",
            ],
            "declarations": [
                {
                    "name": path.name,
                    "sha256": _sha(path),
                    "size_bytes": path.stat().st_size,
                }
                for path in declaration_paths
            ],
            "protocol_sha256": _sha(phone_protocol_path),
            "protocol_size_bytes": phone_protocol_path.stat().st_size,
        },
        "node_repl": {
            "tools": node_repl_tools["tools"],
            "tool_schema_sha256": _sha(node_repl_tools_path),
            "tool_schema_size_bytes": node_repl_tools_path.stat().st_size,
        },
    }
    capability_inventory = {
        "schema_version": 1,
        "target": "linux-x64-glibc",
        "versions": locked_versions(),
        "supported": [
            "browser-persistent-js",
            "computer-use-persistent-js",
            "phone-use-persistent-js",
            "node-repl-mcp",
            "ocr-pdf-image-file-toolbox",
            "system-chrome-family-playwright",
        ],
        "unsupported": list(UNSUPPORTED),
    }
    example_inventory = {"schema_version": 1, "runtime": "bundled-node-24", "entries": examples}
    routing_inventory = {
        "schema_version": 1,
        "entries": docs,
        "skills": {
            "browser-use": {
                "path": "skills/browser-use/SKILL.md",
                "direct": "sky_cua",
                "persistent": "node_repl",
                "references": ["references/node-repl.md", "references/browser.md"],
            },
            "computer-use": {
                "path": "skills/computer-use/SKILL.md",
                "direct": "sky_cua",
                "persistent": "node_repl",
                "references": ["references/node-repl.md", "references/computer.md"],
            },
            "phone-use": {
                "path": "skills/phone-use/SKILL.md",
                "direct": "sky_cua",
                "persistent": "node_repl",
                "references": ["references/node-repl.md", "references/phone.md"],
            },
        },
    }
    inventories = output / "inventories"
    _write_json(inventories / "api-inventory.json", api_inventory)
    _write_json(inventories / "capability-inventory.json", capability_inventory)
    _write_json(inventories / "example-inventory.json", example_inventory)
    _write_json(inventories / "routing-inventory.json", routing_inventory)
    _validate_source(output)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    build(args.output.expanduser().resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
