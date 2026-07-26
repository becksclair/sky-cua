#!/usr/bin/env python3
"""Assemble the sky-cua-owned Linux x64 glibc cua_node component offline."""

from __future__ import annotations

import argparse
import base64
import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import tarfile
import tempfile
import uuid
from collections.abc import Iterable
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any, BinaryIO, cast

from _artifact_helpers import (
    CHECKOUT_SHAPED_PATH_PATTERNS,
    canonical_json_bytes,
    sha256_file,
    write_json_durably,
)
from _plugin_bundle import REPO_ROOT, remove_path

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


PREPARED_PACKAGE_IMPORTS = (
    PreparedPackageImport(
        name="acorn",
        version="8.16.0",
        license_expression="MIT",
        tree_sha256="4acbc403f8e4d593b943dd81163a23ec6e4d4c451e706c6922cc097d85db3241",
        size_bytes=558_610,
        integrity="sha512-UVJyE9MttOsBQIDKw1skb9nAwQuR5wuGD3+82K6JgJlm/Y+KI92oNsMNGZCYdDsVtRHSak0pcV5Dno5+4jh9sw==",
    ),
    PreparedPackageImport(
        name="acorn-walk",
        version="8.3.5",
        license_expression="MIT",
        tree_sha256="4b0f0f351e71172da86000269c356022c17b1583f7a7ee0f679a65ceb0c95be0",
        size_bytes=53_765,
        integrity="sha512-HEHNfbars9v4pgpW6SO1KSPkfoS0xVOM/9UzkJltjlsHZmJasxg8aXkuZa7SMf8vKGIBhpUsPluQSqhJFCqebw==",
    ),
)


class AssemblyError(RuntimeError):
    """The offline inputs or assembled runtime violate the locked contract."""


def _canonical_json(value: object) -> bytes:
    return json.dumps(value, ensure_ascii=False, indent=2).encode("utf-8") + b"\n"


def _sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _normalize_checkout_text_paths(root: Path) -> None:
    replacement = b"${SKY_CUA_SOURCE_ROOT}/"
    text_suffixes = {
        ".json",
        ".md",
        ".mjs",
        ".py",
        ".sh",
        ".toml",
        ".ts",
        ".txt",
        ".yaml",
        ".yml",
    }
    for path in _files(root):
        if path.suffix.lower() not in text_suffixes:
            continue
        blob = path.read_bytes()
        if b"\x00" in blob:
            continue
        normalized = blob
        for pattern in CHECKOUT_SHAPED_PATH_PATTERNS:
            normalized = pattern.sub(replacement, normalized)
        if normalized != blob:
            path.write_bytes(normalized)


def _files(root: Path) -> list[Path]:
    result: list[Path] = []
    for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise AssemblyError(f"tree contains symlink: {relative}")
        if path.is_file():
            result.append(path)
        elif not path.is_dir():
            raise AssemblyError(f"tree contains unsupported entry: {relative}")
    return result


def _file_manifest(root: Path, *, excluded: Iterable[str] = ()) -> list[dict[str, object]]:
    excluded_set = set(excluded)
    return [
        {
            "path": path.relative_to(root).as_posix(),
            "sha256": sha256_file(path),
            "size_bytes": path.stat().st_size,
        }
        for path in _files(root)
        if path.relative_to(root).as_posix() not in excluded_set
    ]


def _tree_hash(root: Path) -> tuple[str, int, int]:
    manifest = _file_manifest(root)
    digest = hashlib.sha256(_canonical_json(manifest)).hexdigest()
    return digest, sum(cast(int, item["size_bytes"]) for item in manifest), len(manifest)


def _fsync_tree(root: Path) -> None:
    directories = [root]
    for path in _files(root):
        descriptor = os.open(path, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    directories.extend(
        path for path in sorted(root.rglob("*"), key=lambda item: item.as_posix()) if path.is_dir()
    )
    for directory in reversed(directories):
        _fsync_directory(directory)


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _copy_tree(source: Path, destination: Path) -> None:
    if source.is_symlink() or not source.is_dir():
        raise AssemblyError(f"source is not a real directory: {source}")
    _files(source)
    shutil.copytree(source, destination, copy_function=shutil.copy2)


def _import_prepared_packages(root: Path) -> None:
    prepared_modules = REPO_ROOT / "runtime" / "cua-node" / "node_modules"
    destination_modules = root / "lib" / "node_modules"
    validated: list[tuple[PreparedPackageImport, Path]] = []
    preparation_command = "bun install --frozen-lockfile --cwd=runtime/cua-node"
    for package in PREPARED_PACKAGE_IMPORTS:
        source = prepared_modules / package.name
        if source.is_symlink() or not source.is_dir():
            raise AssemblyError(
                f"prepared package is missing: {source}; run `{preparation_command}` first"
            )
        package_files = _files(source)
        package_json = source / "package.json"
        if package_json.is_symlink() or package_json not in package_files:
            raise AssemblyError(f"prepared package is missing a real package.json: {package.name}")
        try:
            identity = json.loads(package_json.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, OSError) as error:
            raise AssemblyError(
                f"prepared package has invalid package.json: {package.name}"
            ) from error
        if not isinstance(identity, dict) or (
            identity.get("name"),
            identity.get("version"),
            identity.get("license"),
        ) != (package.name, package.version, package.license_expression):
            raise AssemblyError(
                "prepared package identity mismatch: "
                f"expected {package.name}@{package.version} license {package.license_expression}"
            )
        if re.fullmatch(r"[0-9a-f]{64}", package.tree_sha256) is None or package.size_bytes < 0:
            raise AssemblyError(
                f"prepared package lock constants are not filled for {package.name}@{package.version}"
            )
        digest, size_bytes, _ = _tree_hash(source)
        if digest != package.tree_sha256 or size_bytes != package.size_bytes:
            raise AssemblyError(
                f"prepared package tree mismatch for {package.name}@{package.version}: "
                f"sha256={digest}, size_bytes={size_bytes}"
            )
        validated.append((package, source))

    for package, source in validated:
        destination = destination_modules / package.name
        remove_path(destination)
        _copy_tree(source, destination)


def _rebuild_first_party_outputs() -> dict[str, object]:
    for command in FIRST_PARTY_BUILD_COMMANDS:
        subprocess.run(
            cast(list[str], command["argv"]),
            cwd=REPO_ROOT / cast(str, command["cwd"]),
            check=True,
            capture_output=True,
            text=True,
        )
    bun_version = subprocess.run(
        ["bun", "--version"], check=True, capture_output=True, text=True
    ).stdout.strip()
    compiler_version = subprocess.run(
        ["cc", "--version"], check=True, capture_output=True, text=True
    ).stdout.splitlines()[0]
    runtime_cli = REPO_ROOT / "runtime" / "cua-node" / "dist" / "cli.js"
    browser_build = REPO_ROOT / "packages" / "browser-use" / "build"
    browser_client = browser_build / "browser-client.mjs"
    sky_tarball = REPO_ROOT / "packages" / "sky-cua-js" / "out" / "sky-cua-0.1.0.tgz"
    for path in (runtime_cli, browser_client, sky_tarball):
        if not path.is_file() or path.is_symlink():
            raise AssemblyError(
                f"first-party build output is missing: {path.relative_to(REPO_ROOT)}"
            )
    browser_tree_sha256, browser_tree_size, browser_tree_files = _tree_hash(browser_build)
    return {
        "schema_version": 1,
        "commands": list(FIRST_PARTY_BUILD_COMMANDS),
        "toolchain": {"bun": bun_version, "cc": compiler_version},
        "outputs": {
            "runtime_cli": {
                "path": "runtime/cua-node/dist/cli.js",
                "sha256": sha256_file(runtime_cli),
                "size_bytes": runtime_cli.stat().st_size,
            },
            "browser_client": {
                "path": "packages/browser-use/build/browser-client.mjs",
                "sha256": sha256_file(browser_client),
                "size_bytes": browser_client.stat().st_size,
            },
            "browser_build": {
                "path": "packages/browser-use/build",
                "tree_sha256": browser_tree_sha256,
                "size_bytes": browser_tree_size,
                "file_count": browser_tree_files,
            },
            "sky_cua_tarball": {
                "path": "packages/sky-cua-js/out/sky-cua-0.1.0.tgz",
                "sha256": sha256_file(sky_tarball),
                "size_bytes": sky_tarball.stat().st_size,
            },
        },
    }


def _normalize_tree(root: Path) -> None:
    for path in [root, *sorted(root.rglob("*"), key=lambda item: item.as_posix())]:
        if path.is_symlink():
            raise AssemblyError(f"assembled tree contains symlink: {path.relative_to(root)}")
        if path.is_dir():
            path.chmod(0o755)
        elif path.is_file():
            relative = path.relative_to(root).as_posix()
            executable = relative in {"bin/node", "bin/node_repl"} or relative.endswith(".node")
            path.chmod(0o755 if executable else 0o644)
        else:
            raise AssemblyError(f"assembled tree contains special file: {path}")


def _cache_root(argument: Path | None) -> Path:
    if argument is not None:
        return argument.expanduser().resolve()
    explicit = os.environ.get("SKY_CUA_CUA_NODE_CACHE_ROOT", "").strip()
    if explicit:
        return Path(explicit).expanduser().resolve()
    xdg = os.environ.get("XDG_CACHE_HOME", "").strip()
    base = Path(xdg).expanduser() if xdg else Path.home() / ".cache"
    return (base / "sky-cua" / "cua-node" / "v1").resolve()


def _import_seed(cache: Path, source: Path) -> Path:
    source = source.expanduser()
    if source.is_symlink() or not source.is_dir():
        raise AssemblyError(f"migration seed is not a real directory: {source}")
    source = source.resolve()
    manifest = source / "manifest.json"
    if not manifest.is_file() or manifest.is_symlink():
        raise AssemblyError("migration seed is missing its legacy manifest.json")
    value = json.loads(manifest.read_text(encoding="utf-8"))
    if value.get("target") != TARGET or value.get("node_version") != NODE_VERSION:
        raise AssemblyError("migration seed target or Node version is not the locked input")
    tree_sha256, size_bytes, file_count = _tree_hash(source)
    if (
        tree_sha256 != MIGRATION_SEED_SHA256
        or size_bytes != MIGRATION_SEED_SIZE_BYTES
        or file_count != MIGRATION_SEED_FILE_COUNT
    ):
        raise AssemblyError(
            "migration seed does not match the locked Codex Desktop input: "
            f"sha256={tree_sha256}, size_bytes={size_bytes}, file_count={file_count}"
        )
    destination = cache / "seeds" / tree_sha256
    cache.mkdir(parents=True, exist_ok=True, mode=0o700)
    (cache / "seeds").mkdir(parents=True, exist_ok=True, mode=0o700)
    if not destination.exists():
        staging = cache / "seeds" / f".{tree_sha256}.tmp-{os.getpid()}-{uuid.uuid4().hex}"
        remove_path(staging)
        _copy_tree(source, staging)
        inventory = {
            "schema_version": 1,
            "source_tree_sha256": tree_sha256,
            "source_size_bytes": size_bytes,
            "source_file_count": file_count,
            "migration_evidence": {"codex_desktop_commit": MIGRATION_COMMIT},
        }
        (staging / "SKY_CUA_MIGRATION_INPUT.json").write_bytes(_canonical_json(inventory))
        _normalize_tree(staging)
        _fsync_tree(staging)
        os.replace(staging, destination)
    write_json_durably(
        cache / "current-seed.json",
        {"schema_version": 1, "tree_sha256": tree_sha256, "path": f"seeds/{tree_sha256}"},
    )
    return _validate_cached_seed(destination, tree_sha256)


def _validate_cached_seed(seed: Path, digest: str) -> Path:
    if digest != MIGRATION_SEED_SHA256:
        raise AssemblyError("cached seed digest does not match the locked migration input")
    if seed.is_symlink() or not seed.is_dir():
        raise AssemblyError("cached seed tree is missing")
    marker = seed / "SKY_CUA_MIGRATION_INPUT.json"
    if not marker.is_file() or marker.is_symlink():
        raise AssemblyError("cached seed inventory is missing")
    try:
        inventory = json.loads(marker.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as error:
        raise AssemblyError("cached seed inventory is invalid") from error
    if not isinstance(inventory, dict):
        raise AssemblyError("cached seed inventory is invalid")
    source_manifest = _file_manifest(seed, excluded={marker.name})
    actual_digest = hashlib.sha256(_canonical_json(source_manifest)).hexdigest()
    actual_size = sum(cast(int, item["size_bytes"]) for item in source_manifest)
    if (
        inventory.get("schema_version") != 1
        or inventory.get("source_tree_sha256") != MIGRATION_SEED_SHA256
        or inventory.get("source_size_bytes") != MIGRATION_SEED_SIZE_BYTES
        or inventory.get("source_file_count") != MIGRATION_SEED_FILE_COUNT
        or inventory.get("migration_evidence") != {"codex_desktop_commit": MIGRATION_COMMIT}
        or actual_digest != MIGRATION_SEED_SHA256
        or actual_size != MIGRATION_SEED_SIZE_BYTES
        or len(source_manifest) != MIGRATION_SEED_FILE_COUNT
    ):
        raise AssemblyError("cached seed inventory or content hash mismatch")
    return seed


def _resolve_seed(cache: Path, explicit: Path | None) -> Path:
    if explicit is not None:
        return _import_seed(cache, explicit)
    pointer = cache / "current-seed.json"
    if not pointer.is_file() or pointer.is_symlink():
        raise AssemblyError("no sky-cua cua_node seed cache; pass --seed-runtime once")
    try:
        value = json.loads(pointer.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as error:
        raise AssemblyError("cached seed pointer is invalid") from error
    if not isinstance(value, dict):
        raise AssemblyError("cached seed pointer is invalid")
    digest = value.get("tree_sha256")
    if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        raise AssemblyError("cached seed pointer is invalid")
    if (
        value.get("schema_version") != 1
        or digest != MIGRATION_SEED_SHA256
        or value.get("path") != f"seeds/{MIGRATION_SEED_SHA256}"
    ):
        raise AssemblyError("cached seed pointer does not match the locked migration input")
    seed = cache / "seeds" / digest
    return _validate_cached_seed(seed, digest)


def _sanitize_package_json(value: object) -> object:
    if isinstance(value, list):
        return [_sanitize_package_json(item) for item in value if item != "fsevents"]
    if not isinstance(value, dict):
        return value
    result: dict[str, object] = {}
    for key, item in sorted(value.items()):
        if key == "fsevents":
            continue
        if key == "optionalDependencies" and isinstance(item, dict):
            result[key] = {
                name: _sanitize_package_json(dependency)
                for name, dependency in sorted(item.items())
                if name != "fsevents" and WRONG_PLATFORM_PACKAGE_TOKENS.search(name) is None
            }
            continue
        if key == "scripts" and isinstance(item, dict):
            scripts = {
                name: command
                for name, command in sorted(item.items())
                if name not in {"preinstall", "install", "postinstall"}
            }
            result[key] = _sanitize_package_json(scripts)
            continue
        result[key] = _sanitize_package_json(item)
    return result


def _package_constraint_allows(value: object, expected: str) -> bool:
    values = [value] if isinstance(value, str) else value if isinstance(value, list) else []
    normalized = [item.lower() for item in values if isinstance(item, str)]
    positives = [item for item in normalized if not item.startswith("!")]
    if f"!{expected}" in normalized:
        return False
    return not positives or expected in positives or "any" in positives


def _remove_wrong_platform_content(modules: Path) -> None:
    for name in (
        ".bin",
        ".pnpm",
        ".modules.yaml",
        ".pnpm-workspace-state-v1.json",
        "fsevents",
        "@oai",
    ):
        remove_path(modules / name)
    for scope in ("@img", "@napi-rs"):
        scope_root = modules / scope
        if not scope_root.is_dir():
            continue
        for child in scope_root.iterdir():
            package = f"{scope}/{child.name}"
            if package not in ALLOWED_NATIVE_PACKAGES:
                remove_path(child)
    for package_root in reversed(_package_roots(modules)):
        package_path = package_root / "package.json"
        package = json.loads(package_path.read_text(encoding="utf-8"))
        name = package.get("name")
        has_native_addon = any(path.is_file() for path in package_root.rglob("*.node"))
        incompatible = (
            not _package_constraint_allows(package.get("os"), "linux")
            or not _package_constraint_allows(package.get("cpu"), "x64")
            or not _package_constraint_allows(package.get("libc"), "glibc")
            or (
                isinstance(name, str)
                and has_native_addon
                and WRONG_PLATFORM_PACKAGE_TOKENS.search(name) is not None
            )
        )
        if incompatible:
            remove_path(package_root)
    for path in sorted(modules.rglob("*"), key=lambda item: item.as_posix(), reverse=True):
        if path.is_symlink():
            raise AssemblyError(f"module tree contains symlink: {path}")
        if path.is_file() and path.name == "package.json":
            value = json.loads(path.read_text(encoding="utf-8"))
            path.write_bytes(_canonical_json(_sanitize_package_json(value)))


def _extract_package(tarball: Path, destination: Path) -> None:
    if not tarball.is_file() or tarball.is_symlink():
        raise AssemblyError(f"package tarball is missing: {tarball}")
    remove_path(destination)
    destination.mkdir(parents=True)
    with tarfile.open(tarball, "r:gz") as archive:
        members = archive.getmembers()
        for member in members:
            path = PurePosixPath(member.name)
            if (
                path.is_absolute()
                or ".." in path.parts
                or not path.parts
                or path.parts[0] != "package"
                or member.issym()
                or member.islnk()
                or not (member.isdir() or member.isfile())
            ):
                raise AssemblyError(f"unsafe package member: {member.name}")
            relative = PurePosixPath(*path.parts[1:])
            if not relative.parts:
                continue
            target = destination.joinpath(*relative.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                source = archive.extractfile(member)
                if source is None:
                    raise AssemblyError(f"cannot read package member: {member.name}")
                with target.open("wb") as output:
                    shutil.copyfileobj(cast(BinaryIO, source), output)


def _install_first_party_packages(root: Path) -> tuple[str, str]:
    modules = root / "lib" / "node_modules"
    sky_tarball = REPO_ROOT / "packages" / "sky-cua-js" / "out" / "sky-cua-0.1.0.tgz"
    _extract_package(sky_tarball, modules / "@heliasar" / "sky-cua")

    browser_build = REPO_ROOT / "packages" / "browser-use" / "build"
    component = json.loads((browser_build / "BROWSER_COMPONENT.json").read_text(encoding="utf-8"))
    browser_entrypoint = browser_build / "browser-client.mjs"
    browser_hash = sha256_file(browser_entrypoint)
    if (
        component.get("package") != "@heliasar/browser-use"
        or component.get("version") != BROWSER_VERSION
        or component.get("sha256") != browser_hash
    ):
        raise AssemblyError("canonical Browser component metadata does not match its bytes")
    browser_root = modules / "@heliasar" / "browser-use"
    remove_path(browser_root)
    _copy_tree(browser_build, browser_root / "build")
    browser_package = {
        "name": "@heliasar/browser-use",
        "version": BROWSER_VERSION,
        "private": True,
        "type": "module",
        "exports": {
            ".": {
                "types": "./build/index.d.ts",
                "import": "./build/browser-client.mjs",
            },
            "./projection": "./build/projection.mjs",
        },
        "engines": {"node": NODE_VERSION},
    }
    (browser_root / "package.json").write_bytes(_canonical_json(browser_package))
    return sha256_file(sky_tarball), browser_hash


def _license_expression(value: object) -> str:
    if isinstance(value, str) and value:
        return value
    if isinstance(value, dict) and isinstance(value.get("type"), str):
        return cast(str, value["type"])
    return "NOASSERTION"


def _package_roots(modules: Path) -> list[Path]:
    roots: list[Path] = []
    for package_json in sorted(modules.rglob("package.json"), key=lambda item: item.as_posix()):
        parent = package_json.parent
        parts = parent.relative_to(modules).parts
        node_module_indexes = [index for index, part in enumerate(parts) if part == "node_modules"]
        tail = parts[(node_module_indexes[-1] + 1 if node_module_indexes else 0) :]
        is_package_root = (len(tail) == 1 and not tail[0].startswith("@")) or (
            len(tail) == 2 and tail[0].startswith("@")
        )
        if is_package_root:
            roots.append(parent)
    return roots


def _generate_compliance(
    root: Path,
    *,
    producer_commit: str,
    browser_hash: str,
    migration_input: dict[str, object],
) -> None:
    modules = root / "lib" / "node_modules"
    package_roots = _package_roots(modules)
    if not package_roots:
        raise AssemblyError("compliance discovery found no shipped packages")
    notices_root = root / "licenses" / "packages"
    remove_path(notices_root)
    notices_root.mkdir(parents=True)
    inventory: list[dict[str, object]] = []
    accounted_package_paths: set[str] = set()
    for package_root in package_roots:
        package_path = package_root / "package.json"
        package = json.loads(package_path.read_text(encoding="utf-8"))
        name = package.get("name")
        version = package.get("version")
        if not isinstance(name, str) or not name or not isinstance(version, str) or not version:
            raise AssemblyError(f"compliance package identity is missing: {package_path}")
        package_relative = package_root.relative_to(root).as_posix()
        if package_relative in accounted_package_paths:
            raise AssemblyError(f"duplicate compliance package path: {package_relative}")
        accounted_package_paths.add(package_relative)
        license_files = sorted(
            path
            for path in package_root.iterdir()
            if path.is_file()
            and re.fullmatch(
                r"(?i)(?:licen[cs]e|copying|notice)(?:[-._][A-Za-z0-9._-]+)?", path.name
            )
        )
        path_suffix = hashlib.sha256(package_relative.encode()).hexdigest()[:12]
        destination = notices_root / (name.replace("@", "").replace("/", "--") + "--" + path_suffix)
        destination.mkdir(parents=True)
        relative_files: list[str] = []
        for source in license_files:
            target = destination / source.name
            shutil.copy2(source, target)
            relative_files.append(target.relative_to(root).as_posix())
        if not relative_files:
            bound_records = {
                "@img/sharp-libvips-linux-x64": [
                    "licenses/sharp-libvips-1.2.4-source-offer.json",
                    "licenses/notices/sharp-libvips-1.2.4.NOTICE.md",
                ],
                "@napi-rs/canvas-linux-x64-gnu": [
                    "licenses/canvas-0.1.91-source-offer.json",
                    "licenses/canvas-0.1.91-build-record.json",
                    "licenses/notices/canvas-0.1.91-NATIVE-NOTICES.md",
                    "licenses/notices/canvas-0.1.91-RUST-NOTICES.md",
                    "licenses/notices/canvas-0.1.91.NOTICE.md",
                ],
            }.get(name, [])
            for relative in bound_records:
                if not (root / relative).is_file():
                    raise AssemblyError(f"compliance record is missing: {relative}")
            relative_files.extend(bound_records)
        if not relative_files:
            expression = (
                "LicenseRef-Heliasar-Proprietary"
                if name.startswith("@heliasar/")
                else _license_expression(package.get("license"))
            )
            if expression == "NOASSERTION":
                raise AssemblyError(f"compliance package has no declared license: {name}")
            license_text = (
                REPO_ROOT / "resources" / "release" / "license-texts" / f"{expression}.txt"
            )
            if not license_text.is_file() or license_text.is_symlink():
                raise AssemblyError(f"canonical declared license text is missing: {expression}")
            license_target = destination / f"SPDX-{expression}.txt"
            shutil.copy2(license_text, license_target)
            relative_files.append(license_target.relative_to(root).as_posix())
            declared_record = destination / "DECLARED-LICENSE.json"
            declared_record.write_bytes(
                _canonical_json(
                    {
                        "schema_version": 1,
                        "record_kind": "publisher-declared-license",
                        "name": name,
                        "version": version,
                        "license_expression": expression,
                        "package_json_sha256": sha256_file(package_path),
                        "package_path": package_relative,
                    }
                )
            )
            relative_files.append(declared_record.relative_to(root).as_posix())
        inventory.append(
            {
                "name": name,
                "version": version,
                "package_path": package_relative,
                "license": (
                    "LicenseRef-Heliasar-Proprietary"
                    if name.startswith("@heliasar/")
                    else _license_expression(package.get("license"))
                ),
                "license_files": relative_files,
                "license_file_sha256s": {
                    relative: sha256_file(root / relative) for relative in relative_files
                },
                "package_tree_sha256": _tree_hash(package_root)[0],
            }
        )
    inventory.sort(key=lambda item: (cast(str, item["name"]), cast(str, item["package_path"])))
    if accounted_package_paths != {path.relative_to(root).as_posix() for path in package_roots}:
        raise AssemblyError("compliance inventory does not cover every shipped package root")
    (root / "licenses" / "LICENSES.json").write_bytes(
        _canonical_json({"schema_version": 1, "packages": inventory})
    )
    notices = [
        "# cua_node third-party notices",
        "",
        "This inventory is generated from the exact assembled package trees. Exact package license/notice files are copied under `licenses/packages/`; when a publisher omits license text, the package declaration and canonical SPDX text are both bound there.",
        "",
    ]
    notices.extend(
        f"- {item['name']} {item['version']}: {item['license']} ({', '.join(cast(list[str], item['license_files']))})"
        for item in inventory
    )
    notices.extend(
        [
            "",
            "Native Canvas and libvips source-offer/build records are bundled beside this file. Node.js license text is `Node.js-LICENSE.txt`. Playwright uses a system Chrome-family browser; no browser or FFmpeg download is bundled.",
            "",
        ]
    )
    (root / "licenses" / "THIRD_PARTY_NOTICES.md").write_text("\n".join(notices), encoding="utf-8")
    components = [
        {
            "type": "library",
            "name": item["name"],
            "version": item["version"],
            "licenses": [{"license": {"expression": item["license"]}}],
            "hashes": [{"alg": "SHA-256", "content": item["package_tree_sha256"]}],
            "properties": [{"name": "sky-cua:provenance-status", "value": "verified"}],
        }
        for item in inventory
    ]
    for name, version, path, license_path in (
        ("Node.js", NODE_VERSION, "bin/node", "licenses/Node.js-LICENSE.txt"),
        ("cua_node host/kernel", HOST_VERSION, "lib/node_repl/cli.js", None),
        ("Tesseract English traineddata", "4.1.0", "share/tessdata/eng.traineddata", None),
        ("Tesseract OSD traineddata", "4.1.0", "share/tessdata/osd.traineddata", None),
    ):
        component: dict[str, object] = {
            "type": "file",
            "name": name,
            "version": version,
            "hashes": [{"alg": "SHA-256", "content": sha256_file(root / path)}],
            "properties": [{"name": "sky-cua:provenance-status", "value": "verified"}],
        }
        if license_path is not None:
            component["licenses"] = [{"license": {"name": "Node.js bundled license notices"}}]
            cast(list[dict[str, str]], component["properties"]).extend(
                [
                    {"name": "sky-cua:license-path", "value": license_path},
                    {"name": "sky-cua:license-sha256", "value": sha256_file(root / license_path)},
                ]
            )
        components.append(component)
    for name, version, path, license_expression, license_paths in (
        (
            "PDF.js cmaps",
            "5.4.624",
            "share/pdfjs/cmaps",
            "BSD-3-Clause",
            ("share/pdfjs/cmaps/LICENSE",),
        ),
        (
            "PDF.js standard fonts",
            "5.4.624",
            "share/pdfjs/standard_fonts",
            "BSD-3-Clause AND OFL-1.1",
            (
                "share/pdfjs/standard_fonts/LICENSE_FOXIT",
                "share/pdfjs/standard_fonts/LICENSE_LIBERATION",
            ),
        ),
    ):
        tree_sha256, tree_size, tree_files = _tree_hash(root / path)
        properties = [
            {"name": "sky-cua:provenance-status", "value": "verified"},
            {"name": "sky-cua:path", "value": path},
            {"name": "sky-cua:tree-size", "value": str(tree_size)},
            {"name": "sky-cua:file-count", "value": str(tree_files)},
        ]
        for index, license_path in enumerate(license_paths):
            properties.extend(
                [
                    {"name": f"sky-cua:license:{index}:path", "value": license_path},
                    {
                        "name": f"sky-cua:license:{index}:sha256",
                        "value": sha256_file(root / license_path),
                    },
                ]
            )
        components.append(
            {
                "type": "data",
                "name": name,
                "version": version,
                "hashes": [{"alg": "SHA-256", "content": tree_sha256}],
                "licenses": [{"license": {"expression": license_expression}}],
                "properties": properties,
            }
        )
    components.sort(key=lambda item: (cast(str, item["name"]), cast(str, item["version"])))
    serial_digest = hashlib.sha256(_canonical_json(components)).hexdigest()
    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "serialNumber": f"urn:uuid:{serial_digest[:8]}-{serial_digest[8:12]}-{serial_digest[12:16]}-{serial_digest[16:20]}-{serial_digest[20:32]}",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "name": "sky-cua cua_node",
                "version": HOST_VERSION,
            },
            "properties": [
                {"name": "sky-cua:producer-commit", "value": producer_commit},
                {"name": "sky-cua:component-status", "value": "verified-candidate"},
                {"name": "sky-cua:browser-sha256", "value": browser_hash},
            ],
        },
        "components": components,
    }
    (root / "sbom.cdx.json").write_bytes(_canonical_json(sbom))
    provenance = {
        "schema_version": 1,
        "producer": "sky-cua",
        "producer_commit": producer_commit,
        "target": TARGET,
        "migration_evidence": {"codex_desktop_commit": MIGRATION_COMMIT},
        "canonical_browser_sha256": browser_hash,
        "migration_input": migration_input,
        "package_inventory_sha256": _sha256_bytes(_canonical_json(inventory)),
        "absolute_checkout_paths": {
            "embedded_native_build_debug_metadata": True,
            "runtime_path_dependencies": False,
        },
    }
    (root / "licenses" / "PROVENANCE.json").write_bytes(_canonical_json(provenance))


def _producer_source_inventory() -> list[dict[str, object]]:
    roots = (
        REPO_ROOT / "runtime" / "cua-node",
        REPO_ROOT / "packages" / "browser-use",
        REPO_ROOT / "packages" / "sky-cua-js",
        REPO_ROOT / "resources" / "release" / "license-texts",
    )
    inventory: list[dict[str, object]] = []
    for root in roots:
        for path in sorted(root.rglob("*"), key=lambda item: item.as_posix()):
            local_parts = path.relative_to(root).parts
            if "node_modules" in local_parts or "out" in local_parts:
                continue
            if path.is_symlink():
                raise AssemblyError(f"producer source tree contains symlink: {path}")
            if not path.is_file():
                continue
            relative = path.relative_to(REPO_ROOT).as_posix()
            inventory.append(
                {
                    "path": relative,
                    "sha256": sha256_file(path),
                    "size_bytes": path.stat().st_size,
                }
            )
    assembler = REPO_ROOT / "scripts" / "assemble_cua_node.py"
    inventory.append(
        {
            "path": assembler.relative_to(REPO_ROOT).as_posix(),
            "sha256": sha256_file(assembler),
            "size_bytes": assembler.stat().st_size,
        }
    )
    inventory.sort(key=lambda item: cast(str, item["path"]))
    return inventory


def _compile_launcher(root: Path) -> None:
    destination = root / "bin" / "node_repl"
    subprocess.run(
        [
            "cc",
            "-Os",
            "-nostdlib",
            "-static",
            "-ffreestanding",
            "-fno-builtin",
            "-fno-stack-protector",
            "-fno-pie",
            "-no-pie",
            "-Wl,--build-id=none",
            "-o",
            str(destination),
            str(REPO_ROOT / "runtime" / "cua-node" / "native" / "node_repl.c"),
        ],
        check=True,
    )


def _refresh_artifact(record: dict[str, Any], root: Path, relative: str) -> None:
    path = root / relative
    if not path.exists() or path.is_symlink():
        raise AssemblyError(f"locked artifact is missing: {relative}")
    if path.is_dir():
        digest, size, _ = _tree_hash(path)
        record["sha256"] = digest
        record["sha256_scope"] = "tree-manifest"
        record["size_bytes"] = size
    elif path.is_file():
        record["sha256"] = sha256_file(path)
        record["sha256_scope"] = "artifact"
        record["size_bytes"] = path.stat().st_size
    else:
        raise AssemblyError(f"locked artifact has unsupported type: {relative}")


def _dynamic_values(path: Path, label: str) -> list[str]:
    result = subprocess.run(
        ["readelf", "-d", str(path)],
        check=False,
        capture_output=True,
        text=True,
    )
    if (
        result.returncode != 0
        and "There is no dynamic section" not in result.stdout + result.stderr
    ):
        raise AssemblyError(f"readelf dynamic audit failed for {path}: {result.stderr}")
    output = result.stdout
    pattern = re.compile(rf"\({label}\).*\[(.*?)\]")
    return sorted(set(pattern.findall(output)))


def _is_elf(path: Path) -> bool:
    with path.open("rb") as handle:
        return handle.read(4) == b"\x7fELF"


def _native_file_record(path: Path, root: Path) -> dict[str, object]:
    if not _is_elf(path):
        raise AssemblyError(f"native audit input is not ELF: {path}")
    header = subprocess.run(
        ["readelf", "-h", str(path)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    if "Class:                             ELF64" not in header or not re.search(
        r"Machine:\s+Advanced Micro Devices X86-64", header
    ):
        raise AssemblyError(f"native audit target is not ELF64 x86-64: {path}")
    versions = subprocess.run(
        ["readelf", "--version-info", str(path)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    glibc = sorted(
        set(re.findall(r"GLIBC_([0-9]+\.[0-9]+)", versions)),
        key=lambda value: tuple(int(part) for part in value.split(".")),
    )
    glibc_max = glibc[-1] if glibc else "0.0"
    ldd = subprocess.run(["ldd", str(path)], check=False, capture_output=True, text=True)
    ldd_output = ldd.stdout + ldd.stderr
    static = "not a dynamic executable" in ldd_output or "statically linked" in ldd_output
    if (ldd.returncode != 0 and not static) or "not found" in ldd_output:
        raise AssemblyError(
            f"native dependency resolution failed for {path}: {ldd.stdout}{ldd.stderr}"
        )
    return {
        "path": path.relative_to(root).as_posix(),
        "format": "node-addon"
        if path.suffix == ".node"
        else (
            "elf-executable"
            if os.access(path, os.X_OK) and ".so" not in path.name
            else "elf-shared-object"
        ),
        "sha256": sha256_file(path),
        "size_bytes": path.stat().st_size,
        "needed": _dynamic_values(path, "NEEDED"),
        "rpath": _dynamic_values(path, "RPATH"),
        "runpath": _dynamic_values(path, "RUNPATH"),
        "glibc_max_required": glibc_max,
        "ldd": "passed",
    }


def _native_audit(root: Path, relative: str) -> tuple[dict[str, object], list[dict[str, object]]]:
    target = root / relative
    candidates = [target] if target.is_file() else sorted(target.rglob("*"))
    files = [
        _native_file_record(path, root) for path in candidates if path.is_file() and _is_elf(path)
    ]
    if not files:
        raise AssemblyError(f"native audit found no ELF files under {relative}")
    max_glibc = max(
        (cast(str, item["glibc_max_required"]) for item in files),
        key=lambda value: tuple(int(part) for part in value.split(".")),
    )
    audit = {
        "status": "passed",
        "abi": {
            "format": "node-addon"
            if all(item["format"] == "node-addon" for item in files)
            else "elf",
            "class": "ELF64",
            "machine": "x86-64",
        },
        "files": files,
        "needed": sorted({value for item in files for value in cast(list[str], item["needed"])}),
        "rpath": sorted({value for item in files for value in cast(list[str], item["rpath"])}),
        "runpath": sorted({value for item in files for value in cast(list[str], item["runpath"])}),
        "glibc_max_required": max_glibc,
        "ldd": "passed",
        "notes": ["Generated from readelf and ldd against the exact assembled bytes."],
    }
    return audit, files


def _combined_native_audit(files: list[dict[str, object]], *, note: str) -> dict[str, object]:
    if not files:
        raise AssemblyError("combined native audit requires at least one ELF file")
    max_glibc = max(
        (cast(str, item["glibc_max_required"]) for item in files),
        key=lambda value: tuple(int(part) for part in value.split(".")),
    )
    return {
        "status": "passed",
        "abi": {"format": "elf", "class": "ELF64", "machine": "x86-64"},
        "files": files,
        "needed": sorted({value for item in files for value in cast(list[str], item["needed"])}),
        "rpath": sorted({value for item in files for value in cast(list[str], item["rpath"])}),
        "runpath": sorted({value for item in files for value in cast(list[str], item["runpath"])}),
        "glibc_max_required": max_glibc,
        "ldd": "passed",
        "notes": [note],
    }


def _sky_source(record: dict[str, Any], uri: str, provenance: str) -> None:
    record["source"] = {
        "type": "first-party-build"
        if uri.startswith("runtime/") or uri.startswith("packages/")
        else "local-cache",
        "uri": uri,
        "provenance": provenance,
        "resolved": True,
    }


def _browser_lock_record(root: Path) -> dict[str, Any]:
    destination = "lib/node_modules/@heliasar/browser-use"
    digest, size, _ = _tree_hash(root / destination)
    integrity = "sha512-" + base64.b64encode(
        hashlib.sha512(_canonical_json(_file_manifest(root / destination))).digest()
    ).decode("ascii")
    no_native = {
        "status": "not-applicable",
        "abi": {"format": "none", "class": None, "machine": None},
        "files": [],
        "needed": [],
        "rpath": [],
        "runpath": [],
        "glibc_max_required": None,
        "ldd": "not-applicable",
        "notes": [],
    }
    redistribution = {
        "status": "approved",
        "allowed": True,
        "notice_files": ["licenses/THIRD_PARTY_NOTICES.md"],
        "source_offer_required": False,
        "approval": "first-party sky-cua component",
    }
    return {
        "name": "@heliasar/browser-use",
        "version": BROWSER_VERSION,
        "source": {
            "type": "first-party-build",
            "uri": "packages/browser-use/build",
            "provenance": "canonical first-party Browser bytes built by sky-cua",
            "resolved": True,
        },
        "sha256": digest,
        "sha256_scope": "tree-manifest",
        "size_bytes": size,
        "license": {
            "expression": "LicenseRef-Heliasar-Proprietary",
            "notice_files": ["licenses/THIRD_PARTY_NOTICES.md"],
            "redistribution_status": "approved",
        },
        "platform": {"os": "linux", "arch": "x64", "libc": "glibc", "glibc_max_required": "2.28"},
        "native_dependency_audit": no_native,
        "redistribution": redistribution,
        "destination": destination,
        "integrity": integrity,
    }


def _portable_package_lock_record(
    root: Path,
    *,
    name: str,
    version: str,
    license_expression: str,
    source: dict[str, object],
) -> dict[str, Any]:
    destination = f"lib/node_modules/{name}"
    record = _browser_lock_record(root)
    digest, size, _ = _tree_hash(root / destination)
    integrity = "sha512-" + base64.b64encode(
        hashlib.sha512(_canonical_json(_file_manifest(root / destination))).digest()
    ).decode("ascii")
    record.update(
        {
            "name": name,
            "version": version,
            "source": source,
            "sha256": digest,
            "size_bytes": size,
            "license": {
                "expression": license_expression,
                "notice_files": ["licenses/THIRD_PARTY_NOTICES.md"],
                "redistribution_status": "approved",
            },
            "destination": destination,
            "integrity": integrity,
            "redistribution": {
                "status": "approved",
                "allowed": True,
                "notice_files": ["licenses/THIRD_PARTY_NOTICES.md"],
                "source_offer_required": False,
                "approval": "license text copied from the exact verified package tree",
            },
        }
    )
    return record


def _migration_seed_package_lock_record(
    root: Path,
    *,
    name: str,
    version: str,
    license_expression: str,
    seed_sha256: str,
) -> dict[str, Any]:
    return _portable_package_lock_record(
        root,
        name=name,
        version=version,
        license_expression=license_expression,
        source={
            "type": "local-cache",
            "uri": f"sky-cua-cache://cua-node/seeds/{seed_sha256}/packages/{name}",
            "provenance": "exact portable package tree imported from the verified migration seed",
            "resolved": True,
        },
    )


def _prepared_package_lock_record(root: Path, package: PreparedPackageImport) -> dict[str, Any]:
    tarball_name = package.name.rsplit("/", 1)[-1]
    record = _portable_package_lock_record(
        root,
        name=package.name,
        version=package.version,
        license_expression=package.license_expression,
        source={
            "type": "npm",
            "uri": (
                f"https://registry.npmjs.org/{package.name}/-/{tarball_name}-{package.version}.tgz"
            ),
            "provenance": (
                "official npm registry package prepared by the frozen runtime/cua-node "
                "Bun lockfile and copied from runtime/cua-node/node_modules"
            ),
            "resolved": True,
        },
    )
    record["integrity"] = package.integrity
    notice_files = [f"lib/node_modules/{package.name}/LICENSE"]
    cast(dict[str, object], record["license"])["notice_files"] = notice_files
    cast(dict[str, object], record["redistribution"])["notice_files"] = notice_files
    return record


def _generate_locks(root: Path, *, seed_sha256: str) -> tuple[bytes, bytes]:
    runtime_lock = json.loads(
        (REPO_ROOT / "runtime" / "cua-node" / "runtime-lock.json").read_text(encoding="utf-8")
    )
    native_lock = json.loads(
        (REPO_ROOT / "runtime" / "cua-node" / "native-assets.lock.json").read_text(encoding="utf-8")
    )
    for name, lock in (("runtime", runtime_lock), ("native", native_lock)):
        if lock.get("release_ready") is not True or lock.get("release_blockers") != []:
            raise AssemblyError(f"{name} lock template is not release-ready")
    runtime_lock["runtime"]["version"] = HOST_VERSION
    _refresh_artifact(runtime_lock["runtime"], root, "lib/node_repl/cli.js")
    _sky_source(
        runtime_lock["runtime"],
        "runtime/cua-node/dist/cli.js",
        "host/kernel bundle built from sky-cua source",
    )
    for key, relative in (
        ("node", "bin/node"),
        ("npm", "lib/node_modules/npm"),
        ("corepack", "lib/node_modules/corepack"),
    ):
        _refresh_artifact(runtime_lock[key], root, relative)
    node_audit, node_files = _native_audit(root, "bin/node")
    runtime_lock["node"]["native_dependency_audit"] = node_audit
    runtime_lock["node"]["native_files"] = node_files
    packages = cast(list[dict[str, Any]], runtime_lock["packages"])
    generated_package_names = {
        "@heliasar/browser-use",
        "acorn",
        "acorn-walk",
        "pixelmatch",
        "jpeg-js",
        "pngjs",
        "zlibjs",
        "bmp-js",
    }
    packages[:] = [item for item in packages if item.get("name") not in generated_package_names]
    packages.append(_browser_lock_record(root))
    for name, version, license_expression in (
        ("pixelmatch", "7.1.0", "ISC"),
        ("jpeg-js", "0.4.4", "BSD-3-Clause"),
        ("pngjs", "7.0.0", "MIT"),
        ("zlibjs", "0.3.1", "MIT"),
        ("bmp-js", "0.1.0", "MIT"),
    ):
        packages.append(
            _migration_seed_package_lock_record(
                root,
                name=name,
                version=version,
                license_expression=license_expression,
                seed_sha256=seed_sha256,
            )
        )
    packages.extend(
        _prepared_package_lock_record(root, package) for package in PREPARED_PACKAGE_IMPORTS
    )
    packages.sort(key=lambda item: cast(str, item["name"]))
    for item in packages:
        destination = cast(str, item["destination"])
        _refresh_artifact(item, root, destination)
        source = cast(dict[str, Any], item["source"])
        if source.get("type") == "local-cache":
            item["source"] = {
                "type": "local-cache",
                "uri": f"sky-cua-cache://cua-node/seeds/{seed_sha256}/packages/{item['name']}",
                "provenance": "exact package tree imported from the content-addressed verified migration seed",
                "resolved": True,
            }
        elif source.get("type") == "first-party-build":
            _sky_source(
                item,
                destination
                if not cast(str, item["name"]).startswith("@heliasar/")
                else (
                    "packages/browser-use/build"
                    if item["name"] == "@heliasar/browser-use"
                    else "packages/sky-cua-js/out/sky-cua-0.1.0.tgz"
                ),
                "verified sky-cua-owned release input",
            )
    runtime_native_files = list(node_files)
    for item in cast(list[dict[str, Any]], runtime_lock["outputs"]):
        item["version"] = HOST_VERSION
        destination = cast(str, item["destination"])
        _refresh_artifact(item, root, destination)
        _sky_source(
            item,
            "runtime/cua-node/"
            + ("native/node_repl.c" if item["name"] == "node_repl" else "dist/cli.js"),
            "reproducible first-party sky-cua output",
        )
        if item["name"] == "node_repl":
            output_audit, output_files = _native_audit(root, destination)
            item["native_dependency_audit"] = output_audit
            item["native_files"] = output_files
            runtime_native_files.extend(output_files)
    runtime_lock["native_dependency_audit"] = _combined_native_audit(
        runtime_native_files,
        note="Node and node_repl were audited from the exact assembled bytes.",
    )

    native_files: list[dict[str, object]] = []
    for item in cast(list[dict[str, Any]], native_lock["assets"]):
        destination = cast(str, item["destination"])
        _refresh_artifact(item, root, destination)
        source = cast(dict[str, Any], item["source"])
        if source.get("type") == "local-cache":
            item["source"] = {
                "type": "local-cache",
                "uri": f"sky-cua-cache://cua-node/seeds/{seed_sha256}/{item['id']}",
                "provenance": "verified offline input imported into the sky-cua-owned seed cache",
                "resolved": True,
            }
        if item.get("kind") in {"native-addon", "native-library"}:
            audit, files = _native_audit(root, destination)
            item["native_dependency_audit"] = audit
            item["native_files"] = files
            native_files.extend(files)
    native_lock["native_dependency_audit"] = _combined_native_audit(
        native_files,
        note="All native assets were audited from the exact assembled bytes.",
    )
    return _canonical_json(runtime_lock), _canonical_json(native_lock)


def _write_manifest(
    root: Path,
    *,
    producer_commit: str,
    sky_tarball_sha256: str,
    browser_sha256: str,
    runtime_lock: bytes,
    native_lock: bytes,
    migration_input: dict[str, object],
) -> None:
    host_hash = sha256_file(root / "lib" / "node_repl" / "cli.js")
    manifest = {
        "schema_version": 2,
        "manifest_version": 1,
        "runtime_name": "cua_node",
        "platform": "linux",
        "arch": "x64",
        "libc": "glibc",
        "target": TARGET,
        "node_version": NODE_VERSION,
        "node_path": "bin/node",
        "node_sha256": sha256_file(root / "bin" / "node"),
        "node_repl_version": HOST_VERSION,
        "node_repl_path": "bin/node_repl",
        "node_repl_sha256": sha256_file(root / "bin" / "node_repl"),
        "node_modules": "lib/node_modules",
        "data": {
            "playwright": "share/playwright",
            "tessdata": "share/tessdata",
            "pdfjs": "share/pdfjs",
            "licenses": "licenses",
            "sbom": "sbom.cdx.json",
        },
        "components": {
            "host": {
                "name": "@heliasar/cua-node-host",
                "version": HOST_VERSION,
                "build_id": host_hash[:16],
                "sha256": host_hash,
            },
            "kernel": {
                "name": "@heliasar/cua-node-kernel",
                "version": HOST_VERSION,
                "build_id": host_hash[:16],
                "sha256": host_hash,
            },
            "sky_cua": {
                "package_name": "@heliasar/sky-cua",
                "package_version": SKY_CUA_VERSION,
                "entrypoint": "lib/node_modules/@heliasar/sky-cua/dist/index.js",
                "tarball_sha256": sky_tarball_sha256,
            },
            "browser_use": {
                "package_name": "@heliasar/browser-use",
                "package_version": BROWSER_VERSION,
                "entrypoint": "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs",
                "declarations": "lib/node_modules/@heliasar/browser-use/build/index.d.ts",
            },
        },
        "browser": {"revision": f"system-chromium-browser-use-{BROWSER_VERSION}"},
        "source": {
            "producer": "sky-cua",
            "producer_commit": producer_commit,
            "migration_evidence": {"codex_desktop_commit": MIGRATION_COMMIT},
            "migration_input": migration_input,
        },
        "checksums": {
            "algorithm": "sha256",
            "files": _file_manifest(root, excluded={"manifest.json"}),
            "lock_hashes": {
                "runtime_lock_sha256": _sha256_bytes(runtime_lock),
                "native_assets_lock_sha256": _sha256_bytes(native_lock),
            },
        },
    }
    (root / "manifest.json").write_bytes(_canonical_json(manifest))


def _compose(
    seed: Path,
    staging: Path,
    producer_commit: str,
    *,
    release_eligible: bool,
    first_party_build: dict[str, object],
) -> tuple[Path, Path]:
    migration_input_path = seed / "SKY_CUA_MIGRATION_INPUT.json"
    migration_input = cast(
        dict[str, object], json.loads(migration_input_path.read_text(encoding="utf-8"))
    )
    for member in SEED_MEMBERS:
        source = seed / member
        destination = staging / member
        if source.is_dir():
            _copy_tree(source, destination)
        elif source.is_file() and not source.is_symlink():
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        else:
            raise AssemblyError(f"migration seed member is missing: {member}")
    remove_path(staging / "manifest.json")
    host = REPO_ROOT / "runtime" / "cua-node" / "dist" / "cli.js"
    if not host.is_file():
        raise AssemblyError("runtime/cua-node/dist/cli.js is missing; run bun run build")
    (staging / "lib" / "node_repl").mkdir(parents=True, exist_ok=True)
    shutil.copy2(host, staging / "lib" / "node_repl" / "cli.js")
    (staging / "lib" / "node_repl" / "package.json").write_bytes(
        _canonical_json(
            {"name": "@heliasar/cua-node-host-bundle", "private": True, "type": "module"}
        )
    )
    _compile_launcher(staging)
    cast(dict[str, object], first_party_build["outputs"])["node_repl_launcher"] = {
        "path": "bin/node_repl",
        "sha256": sha256_file(staging / "bin" / "node_repl"),
        "size_bytes": (staging / "bin" / "node_repl").stat().st_size,
    }
    sky_hash, browser_hash = _install_first_party_packages(staging)
    _import_prepared_packages(staging)
    _remove_wrong_platform_content(staging / "lib" / "node_modules")
    _normalize_checkout_text_paths(staging)
    _generate_compliance(
        staging,
        producer_commit=producer_commit,
        browser_hash=browser_hash,
        migration_input=migration_input,
    )
    source_inventory = _producer_source_inventory()
    attestation_path = staging / "share" / "provenance" / "SKY_CUA_BUILD_ATTESTATION.json"
    attestation_path.parent.mkdir(parents=True, exist_ok=True)
    attestation_path.write_bytes(
        _canonical_json(
            {
                "schema_version": 1,
                "release_eligible": release_eligible,
                "producer_commit": producer_commit,
                "migration_input": migration_input,
                "first_party_build": first_party_build,
                "source_inventory_sha256": _sha256_bytes(canonical_json_bytes(source_inventory)),
                "source_inventory": source_inventory,
            }
        )
    )
    _normalize_tree(staging)
    seed_sha256 = cast(str, migration_input["source_tree_sha256"])
    runtime_lock, native_lock = _generate_locks(staging, seed_sha256=seed_sha256)
    locks = staging / "share" / "locks"
    locks.mkdir()
    (locks / "runtime-lock.json").write_bytes(runtime_lock)
    (locks / "native-assets.lock.json").write_bytes(native_lock)
    _normalize_tree(staging)
    _write_manifest(
        staging,
        producer_commit=producer_commit,
        sky_tarball_sha256=sky_hash,
        browser_sha256=browser_hash,
        runtime_lock=runtime_lock,
        native_lock=native_lock,
        migration_input=migration_input,
    )
    _normalize_tree(staging)
    return locks / "runtime-lock.json", locks / "native-assets.lock.json"


def _verify(root: Path, runtime_lock: Path, native_lock: Path) -> dict[str, Any]:
    command = [
        "bun",
        str(REPO_ROOT / "runtime" / "cua-node" / "tools" / "verify-cua-node.ts"),
        f"--root={root}",
        f"--target={TARGET}",
        f"--enforce-lock={runtime_lock}",
        f"--enforce-lock={native_lock}",
        "--json",
    ]
    result = subprocess.run(command, cwd=REPO_ROOT, check=False, text=True, capture_output=True)
    if result.returncode != 0:
        try:
            failed_report = json.loads(result.stdout)
            failures = [
                f"{check.get('id')}: {check.get('detail')}"
                for check in failed_report.get("checks", [])
                if check.get("status") != "passed"
            ]
        except json.JSONDecodeError:
            failures = []
        raise AssemblyError(
            "cua_node verifier failed:\n"
            + ("\n".join(failures) if failures else result.stdout[-12000:])
            + result.stderr[-12000:]
        )
    report = json.loads(result.stdout)
    if report.get("status") != "passed":
        raise AssemblyError("cua_node verifier did not report passed")
    expected_identity = f"node_repl/{HOST_VERSION}"
    for identity_command in (
        [str(root / "bin" / "node_repl"), "--version"],
        [str(root / "bin" / "node"), str(root / "lib" / "node_repl" / "cli.js"), "--version"],
    ):
        identity = subprocess.run(
            identity_command,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        if identity != expected_identity:
            raise AssemblyError(
                f"node_repl identity mismatch: expected {expected_identity}, got {identity}"
            )

    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    lock = json.loads(runtime_lock.read_text(encoding="utf-8"))
    versions = {
        manifest["node_repl_version"],
        manifest["components"]["host"]["version"],
        manifest["components"]["kernel"]["version"],
        lock["runtime"]["version"],
        *(item["version"] for item in lock["outputs"]),
    }
    if versions != {HOST_VERSION}:
        raise AssemblyError(f"node_repl version contract diverges: {sorted(versions)}")
    return cast(dict[str, Any], report)


def _verify_installed_transcript(root: Path) -> None:
    transcript_command = [
        "bun",
        str(
            REPO_ROOT / "runtime" / "cua-node" / "production" / "installed-transcript-acceptance.ts"
        ),
        f"--runtime-root={root}",
        "--json",
    ]
    transcript = subprocess.run(
        transcript_command,
        cwd=REPO_ROOT,
        check=False,
        text=True,
        capture_output=True,
    )
    try:
        transcript_report = json.loads(transcript.stdout)
    except json.JSONDecodeError as error:
        raise AssemblyError(
            "installed node_repl transcript acceptance returned invalid JSON:\n"
            + transcript.stdout[-12000:]
            + transcript.stderr[-12000:]
        ) from error
    if transcript.returncode != 0 or transcript_report.get("status") != "passed":
        raise AssemblyError(
            "installed node_repl transcript acceptance failed:\n"
            + json.dumps(transcript_report, indent=2)[-12000:]
            + transcript.stderr[-12000:]
        )


def _recover_publication(output: Path, journal: Path) -> None:
    if not journal.exists():
        return
    value = json.loads(journal.read_text(encoding="utf-8"))
    if value.get("schema_version") != 1 or value.get("output") != output.name:
        raise AssemblyError("cua_node publication journal is invalid")
    staging_name = value.get("staging")
    backup_name = value.get("backup")
    staging_pattern = re.compile(rf"^\.{re.escape(output.name)}\.staging-[0-9a-f]{{32}}$")
    backup_pattern = re.compile(rf"^\.{re.escape(output.name)}\.backup-[0-9a-f]{{32}}$")
    if (
        not isinstance(staging_name, str)
        or not isinstance(backup_name, str)
        or Path(staging_name).name != staging_name
        or Path(backup_name).name != backup_name
        or staging_pattern.fullmatch(staging_name) is None
        or backup_pattern.fullmatch(backup_name) is None
    ):
        raise AssemblyError("cua_node publication journal paths are invalid")
    staging = output.parent / staging_name
    backup = output.parent / backup_name
    phase = value.get("phase")
    if phase == "promoting":
        if backup.exists():
            remove_path(output)
            os.replace(backup, output)
            remove_path(staging)
        elif staging.exists():
            # The old output has not moved yet (or there was no prior output).
            # Keeping an existing output and discarding the staged candidate is rollback.
            remove_path(staging)
        else:
            # A first install moved staging into output before the commit point.
            remove_path(output)
    elif phase == "committed":
        remove_path(backup)
        remove_path(staging)
    else:
        raise AssemblyError("cua_node publication journal phase is invalid")
    journal.unlink()
    _fsync_directory(output.parent)


def assemble(
    *,
    cache: Path,
    seed_argument: Path | None,
    output: Path,
    producer_commit: str,
    check: bool,
    allow_development_dirty: bool = False,
) -> dict[str, Any]:
    release_eligible = _assert_producer_sources(
        producer_commit, allow_development_dirty=allow_development_dirty
    )
    output = output.expanduser()
    if output.is_symlink():
        raise AssemblyError(f"output root must not be a symlink: {output}")
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    lock_path = output.parent / ".cua-node-assembly.lock"
    journal = output.parent / ".cua-node-assembly-journal.json"
    with lock_path.open("a+b") as lock:
        lock_path.chmod(0o600)
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        _recover_publication(output, journal)
        first_party_build = _rebuild_first_party_outputs()
        seed = _resolve_seed(cache, seed_argument)
        workspace = Path(tempfile.mkdtemp(prefix="cua-node-assemble-", dir=output.parent))
        staging = workspace / "cua-node-linux-x64-glibc"
        staging.mkdir()
        try:
            runtime_lock, native_lock = _compose(
                seed,
                staging,
                producer_commit,
                release_eligible=release_eligible,
                first_party_build=first_party_build,
            )
            report = _verify(staging, runtime_lock, native_lock)
            _verify_installed_transcript(staging)
            tree_sha256, size_bytes, file_count = _tree_hash(staging)
            if check:
                if not output.is_dir() or output.is_symlink():
                    raise AssemblyError(f"assembled output is missing: {output}")
                _verify(
                    output,
                    output / "share" / "locks" / "runtime-lock.json",
                    output / "share" / "locks" / "native-assets.lock.json",
                )
                current_sha256, _, _ = _tree_hash(output)
                if current_sha256 != tree_sha256:
                    raise AssemblyError(
                        f"assembled output drift: expected {tree_sha256}, got {current_sha256}"
                    )
                return {
                    "status": "checked",
                    "output": str(output),
                    "tree_sha256": tree_sha256,
                    "file_count": file_count,
                    "size_bytes": size_bytes,
                    "verification": report["status"],
                    "release_eligible": release_eligible,
                }
            _fsync_tree(staging)
            backup = output.parent / f".{output.name}.backup-{uuid.uuid4().hex}"
            publication_staging = output.parent / f".{output.name}.staging-{uuid.uuid4().hex}"
            os.replace(staging, publication_staging)
            write_json_durably(
                journal,
                {
                    "schema_version": 1,
                    "phase": "promoting",
                    "output": output.name,
                    "staging": publication_staging.name,
                    "backup": backup.name,
                },
            )
            if output.exists():
                os.replace(output, backup)
            os.replace(publication_staging, output)
            _fsync_directory(output.parent)
            _verify(
                output,
                output / "share" / "locks" / "runtime-lock.json",
                output / "share" / "locks" / "native-assets.lock.json",
            )
            write_json_durably(
                journal,
                {
                    "schema_version": 1,
                    "phase": "committed",
                    "output": output.name,
                    "staging": publication_staging.name,
                    "backup": backup.name,
                },
            )
            remove_path(backup)
            journal.unlink()
            _fsync_directory(output.parent)
            return {
                "status": "prepared" if release_eligible else "development-prepared",
                "output": str(output),
                "tree_sha256": tree_sha256,
                "file_count": file_count,
                "size_bytes": size_bytes,
                "verification": report["status"],
                "release_eligible": release_eligible,
            }
        finally:
            remove_path(workspace)


def _producer_commit(argument: str | None) -> str:
    if argument is not None:
        value = argument
    else:
        value = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    if re.fullmatch(r"[0-9a-f]{40}", value) is None:
        raise AssemblyError("producer commit must be a full 40-character Git commit")
    return value


def _assert_producer_sources(commit: str, *, allow_development_dirty: bool) -> bool:
    required_paths = (
        "scripts/assemble_cua_node.py",
        "runtime/cua-node/src/cli.ts",
        "runtime/cua-node/src/host/runtime-manager.ts",
        "runtime/cua-node/src/kernel/kernel.ts",
        "runtime/cua-node/native/node_repl.c",
        "packages/browser-use/src/index.ts",
        "packages/sky-cua-js/src/index.ts",
    )
    commit_check = subprocess.run(
        ["git", "cat-file", "-e", f"{commit}^{{commit}}"],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
    )
    if commit_check.returncode != 0:
        raise AssemblyError(f"producer commit does not exist: {commit}")
    for path in required_paths:
        exists = subprocess.run(
            ["git", "cat-file", "-e", f"{commit}:{path}"],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
        )
        if exists.returncode != 0:
            if allow_development_dirty:
                return False
            raise AssemblyError(f"producer commit does not contain required source: {path}")
    clean = subprocess.run(
        [
            "git",
            "diff",
            "--quiet",
            commit,
            "--",
            "scripts/assemble_cua_node.py",
            "runtime/cua-node",
            "packages/browser-use",
            "packages/sky-cua-js",
        ],
        cwd=REPO_ROOT,
        check=False,
    )
    if clean.returncode != 0:
        if allow_development_dirty:
            return False
        raise AssemblyError("producer source paths differ from the bound producer commit")
    status = subprocess.run(
        [
            "git",
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            "scripts/assemble_cua_node.py",
            "runtime/cua-node",
            "packages/browser-use",
            "packages/sky-cua-js",
        ],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if status:
        if allow_development_dirty:
            return False
        raise AssemblyError("producer source paths contain uncommitted or untracked files")
    return True


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-root", type=Path)
    parser.add_argument("--seed-runtime", type=Path)
    parser.add_argument(
        "--output-root",
        type=Path,
        default=REPO_ROOT / "out" / "components" / "cua-node-linux-x64-glibc",
    )
    parser.add_argument("--producer-commit")
    parser.add_argument("--target", default=TARGET)
    parser.add_argument("--check", action="store_true")
    parser.add_argument(
        "--allow-development-dirty",
        action="store_true",
        help="Build a non-release-eligible validation candidate before the producer commit exists.",
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    if args.target != TARGET:
        raise SystemExit(f"unsupported target: {args.target}; only {TARGET} is complete")
    try:
        result = assemble(
            cache=_cache_root(args.cache_root),
            seed_argument=args.seed_runtime,
            output=args.output_root,
            producer_commit=_producer_commit(args.producer_commit),
            check=args.check,
            allow_development_dirty=args.allow_development_dirty,
        )
    except (AssemblyError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        if args.json:
            print(json.dumps({"status": "error", "error": str(error)}, sort_keys=True))
        else:
            print(f"error: {error}")
        return 1
    if args.json:
        print(json.dumps(result, sort_keys=True))
    else:
        for key, value in result.items():
            print(f"{key}={value}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
