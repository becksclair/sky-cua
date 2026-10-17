"""Package staging for the assembled cua_node module tree."""

from __future__ import annotations

import json
import re
import shutil
import tarfile
from pathlib import Path, PurePosixPath
from typing import BinaryIO, cast

from _artifact_helpers import (
    sha256_file,
)
from _cua_node_inputs import (
    ALLOWED_NATIVE_PACKAGES,
    BROWSER_VERSION,
    NODE_VERSION,
    PREPARED_PACKAGE_IMPORTS,
    SEED_SHARP_LIBVIPS_RECORDS,
    SHARP_LIBVIPS_RECORDS,
    WRONG_PLATFORM_PACKAGE_TOKENS,
    PreparedPackageImport,
)
from _cua_node_tree import (
    AssemblyError,
    _canonical_json,
    _copy_tree,
    _files,
    _tree_hash,
)
from _plugin_bundle import REPO_ROOT, remove_path


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


def _install_libvips_compliance_records(root: Path) -> None:
    licenses = root / "licenses"
    for stale in SEED_SHARP_LIBVIPS_RECORDS:
        remove_path(licenses / stale)
    compliance = REPO_ROOT / "runtime" / "cua-node" / "compliance"
    for relative in SHARP_LIBVIPS_RECORDS:
        source = compliance / relative
        if source.is_symlink() or not source.is_file():
            raise AssemblyError(f"sharp-libvips compliance record is missing: {relative}")
        destination = licenses / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


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
