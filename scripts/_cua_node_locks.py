"""Native dependency audit and runtime/native lock generation for cua_node."""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any, cast

from _artifact_helpers import (
    sha256_file,
)
from _cua_node_inputs import (
    BROWSER_VERSION,
    HOST_VERSION,
    PREPARED_PACKAGE_IMPORTS,
    PreparedPackageImport,
)
from _cua_node_tree import (
    AssemblyError,
    _canonical_json,
    _file_manifest,
    _tree_hash,
)
from _plugin_bundle import REPO_ROOT


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


def _prepared_runtime_lock_records(root: Path) -> list[dict[str, Any]]:
    """Native-asset imports are recorded in the native lock, never as runtime packages."""
    return [
        _prepared_package_lock_record(root, package)
        for package in PREPARED_PACKAGE_IMPORTS
        if not package.native_asset
    ]


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
        "semver",
        "sharp",
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
    packages.extend(_prepared_runtime_lock_records(root))
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
