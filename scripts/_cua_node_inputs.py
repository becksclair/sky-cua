"""Locked offline inputs for the Linux x64 glibc cua_node assembly."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import cast

from _plugin_bundle import REPO_ROOT

TARGET = "linux-x64-glibc"
NODE_VERSION = "24.14.0"
HOST_VERSION = "0.1.0"
BROWSER_VERSION = "1.0.0"
SKY_CUA_VERSION = "0.1.0"
MIGRATION_COMMIT = "65c69a3f1afc9f81274189901bc72e80682ea03a"
MIGRATION_SEED_SHA256 = "b6677535a94b231fcf681ee30d2d1d42ef8028095d45e4040c7eefacda6c32d2"
MIGRATION_SEED_SIZE_BYTES = 323_846_851
MIGRATION_SEED_FILE_COUNT = 3_579
SEED_MEMBERS = ("bin", "lib", "share", "licenses", "sbom.cdx.json")
ASSEMBLER_SOURCES = (
    "scripts/assemble_cua_node.py",
    "scripts/_cua_node_compliance.py",
    "scripts/_cua_node_inputs.py",
    "scripts/_cua_node_locks.py",
    "scripts/_cua_node_packages.py",
    "scripts/_cua_node_seed.py",
    "scripts/_cua_node_tree.py",
)
WRONG_PLATFORM_PACKAGE_TOKENS = re.compile(
    r"(?:darwin|win32|windows|android|musl|arm64|aarch64|armv7|riscv|s390x|ppc64|(?:^|[-_])arm(?:[-_]|$))",
    re.IGNORECASE,
)
FIRST_PARTY_BUILD_COMMANDS = (
    {"cwd": "runtime/cua-node", "argv": ["bun", "run", "build"]},
    {"cwd": "packages/browser-use", "argv": ["bun", "run", "build"]},
    {"cwd": "packages/sky-cua-js", "argv": ["bun", "run", "pack:deterministic"]},
)
ALLOWED_NATIVE_PACKAGES = {
    "@img/colour",
    "@img/sharp-linux-x64",
    "@img/sharp-libvips-linux-x64",
    "@napi-rs/canvas",
    "@napi-rs/canvas-linux-x64-gnu",
}


@dataclass(frozen=True)
class PreparedPackageImport:
    name: str
    version: str
    license_expression: str
    tree_sha256: str
    size_bytes: int
    integrity: str = ""
    native_asset: bool = False


PREPARED_IMPORTS_LOCK = REPO_ROOT / "runtime" / "cua-node" / "prepared-imports.lock.json"


def _tree_pin(value: object, label: str) -> tuple[str, int]:
    if not isinstance(value, dict):
        raise ValueError(f"{label} must be an object")
    tree = cast(dict[str, object], value)
    digest, size = tree.get("sha256"), tree.get("size_bytes")
    if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        raise ValueError(f"{label}.sha256 must be a SHA-256 hex digest")
    if not isinstance(size, int) or isinstance(size, bool) or size < 0:
        raise ValueError(f"{label}.size_bytes must be a non-negative integer")
    return digest, size


def load_prepared_imports(path: Path = PREPARED_IMPORTS_LOCK) -> tuple[PreparedPackageImport, ...]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, dict) or document.get("schema_version") != 1:
        raise ValueError(f"{path.name} must be a schema_version 1 object")
    entries = cast(dict[str, object], document).get("packages")
    if not isinstance(entries, list) or not entries:
        raise ValueError(f"{path.name} packages must be a non-empty array")
    imports: list[PreparedPackageImport] = []
    for index, raw in enumerate(cast(list[object], entries)):
        label = f"{path.name} packages[{index}]"
        if not isinstance(raw, dict):
            raise ValueError(f"{label} must be an object")
        entry = cast(dict[str, object], raw)
        name, version, license_expression, integrity, native_asset = (
            entry.get("name"),
            entry.get("version"),
            entry.get("license"),
            entry.get("integrity"),
            entry.get("native_asset"),
        )
        if not all(
            isinstance(value, str) and value for value in (name, version, license_expression)
        ):
            raise ValueError(f"{label} needs non-empty name, version, and license")
        if not isinstance(integrity, str) or not integrity.startswith("sha512-"):
            raise ValueError(f"{label}.integrity must be an npm sha512 integrity")
        if not isinstance(native_asset, bool):
            raise ValueError(f"{label}.native_asset must be a boolean")
        tree_sha256, size_bytes = _tree_pin(entry.get("source_tree"), f"{label}.source_tree")
        _tree_pin(entry.get("assembled_tree"), f"{label}.assembled_tree")
        imports.append(
            PreparedPackageImport(
                cast(str, name),
                cast(str, version),
                cast(str, license_expression),
                tree_sha256,
                size_bytes,
                integrity,
                native_asset,
            )
        )
    if len({item.name for item in imports}) != len(imports):
        raise ValueError(f"{path.name} lists a package more than once")
    return tuple(imports)


PREPARED_PACKAGE_IMPORTS = load_prepared_imports()


SEED_SHARP_LIBVIPS_RECORDS = (
    "sharp-libvips-1.2.4-source-offer.json",
    "notices/sharp-libvips-1.2.4.NOTICE.md",
)
SHARP_LIBVIPS_VERSION = next(
    package.version
    for package in PREPARED_PACKAGE_IMPORTS
    if package.name == "@img/sharp-libvips-linux-x64"
)
SHARP_LIBVIPS_RECORDS = (
    f"sharp-libvips-{SHARP_LIBVIPS_VERSION}-source-offer.json",
    f"notices/sharp-libvips-{SHARP_LIBVIPS_VERSION}.NOTICE.md",
)
