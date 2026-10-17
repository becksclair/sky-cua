#!/usr/bin/env python3
"""Refresh the compliance-pinned cua_node packages after a dependency bump.

Run by a person on a dependency-bump branch, never by CI: CI only checks the
pins this writes against independently produced artifacts.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, cast

from _artifact_helpers import sha256_file
from _cua_node_inputs import PREPARED_IMPORTS_LOCK
from _cua_node_tree import _tree_hash
from _plugin_bundle import REPO_ROOT
from build_model_documentation import toolbox_version_drift

RUNTIME = REPO_ROOT / "runtime" / "cua-node"
PRODUCTION_PACKAGE = RUNTIME / "production" / "package.json"
PRODUCTION_LOCK = RUNTIME / "production" / "package-lock.json"
RUNTIME_LOCK = RUNTIME / "runtime-lock.json"
NATIVE_LOCK = RUNTIME / "native-assets.lock.json"
COMPLIANCE = RUNTIME / "compliance"
REGISTRY = "https://registry.npmjs.org"
HOST_BUNDLE = "runtime/cua-node/dist/cli.js"
_TRAILING_COMMA = re.compile(r",(\s*[}\]])")
_VERSIONED_NOTICE = re.compile(r"-(\d+\.\d+\.\d+)(?:\.|-)")

JsonObject = dict[str, Any]


class RefreshError(RuntimeError):
    """The bump branch cannot be refreshed mechanically."""


def _run(argv: list[str], cwd: Path) -> None:
    result = subprocess.run(argv, cwd=cwd, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        tail = "\n".join((result.stderr or result.stdout).strip().splitlines()[-20:])
        raise RefreshError(f"{' '.join(argv[:3])} failed ({result.returncode}):\n{tail}")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: Path, value: object) -> bool:
    raw = path.read_text(encoding="utf-8")
    first = next(line for line in raw.splitlines() if line[:1] in "\t ")
    indent: str | int = "\t" if first[0] == "\t" else len(first) - len(first.lstrip(" "))
    rendered = json.dumps(value, indent=indent, ensure_ascii=False) + "\n"
    if rendered == raw:
        return False
    path.write_text(rendered, encoding="utf-8")
    return True


def tarball_uri(name: str, version: str) -> str:
    return f"{REGISTRY}/{name}/-/{name.rsplit('/', 1)[-1]}-{version}.tgz"


def _read_bun_lock(path: Path) -> JsonObject:
    return json.loads(_TRAILING_COMMA.sub(r"\1", path.read_text(encoding="utf-8")))


def bun_lock_packages(path: Path) -> dict[str, JsonObject]:
    """Top-level bun.lock package entries: version, integrity, and metadata."""
    document = _read_bun_lock(path)
    result: dict[str, JsonObject] = {}
    for name, entry in cast(dict[str, list[Any]], document["packages"]).items():
        spec, integrity = entry[0], entry[-1]
        metadata = next((item for item in entry[1:-1] if isinstance(item, dict)), {})
        if not isinstance(spec, str) or "@" not in spec[1:]:
            raise RefreshError(f"bun.lock entry has no exact spec: {name}")
        result[name] = {
            "version": spec.rsplit("@", 1)[1],
            "integrity": integrity,
            "metadata": metadata,
        }
    return result


def bun_workspace_pins(path: Path) -> dict[str, str]:
    return cast(dict[str, str], _read_bun_lock(path)["workspaces"][""]["dependencies"])


def refresh_sources(
    lock: JsonObject, bun_packages: dict[str, JsonObject], pins: dict[str, str], modules: Path
) -> None:
    for entry in cast(list[JsonObject], lock["packages"]):
        name = cast(str, entry["name"])
        locked = bun_packages.get(name)
        if locked is None or pins.get(name) != locked["version"]:
            raise RefreshError(f"bun.lock does not pin prepared package exactly: {name}")
        package_root = modules / name
        identity = cast(JsonObject, read_json(package_root / "package.json"))
        if (identity.get("name"), identity.get("version")) != (name, locked["version"]):
            raise RefreshError(f"installed {name} does not match bun.lock; run a frozen install")
        license_expression = identity.get("license")
        if not isinstance(license_expression, str) or not license_expression:
            raise RefreshError(f"installed {name} declares no license")
        digest, size_bytes, _ = _tree_hash(package_root)
        entry.update(
            {
                "version": locked["version"],
                "license": license_expression,
                "integrity": locked["integrity"],
                "source_tree": {"sha256": digest, "size_bytes": size_bytes},
            }
        )


def sync_production(
    lock: JsonObject,
    bun_packages: dict[str, JsonObject],
    production_package: JsonObject,
    production_lock: JsonObject,
) -> None:
    pins = cast(dict[str, str], production_package["dependencies"])
    nodes = cast(dict[str, JsonObject], production_lock["packages"])
    root_pins = cast(dict[str, str], nodes[""]["dependencies"])
    for entry in cast(list[JsonObject], lock["packages"]):
        name, version = cast(str, entry["name"]), cast(str, entry["version"])
        if name not in pins:
            continue
        pins[name] = version
        root_pins[name] = version
        node = nodes.get(f"node_modules/{name}")
        if node is None:
            continue
        node.update(
            {
                "version": version,
                "resolved": tarball_uri(name, version),
                "integrity": entry["integrity"],
                "license": entry["license"],
            }
        )
        optional = node.get("optionalDependencies")
        upstream = cast(JsonObject, bun_packages[name]["metadata"]).get("optionalDependencies", {})
        if isinstance(optional, dict):
            for dependency in cast(dict[str, str], optional):
                if dependency not in upstream:
                    raise RefreshError(f"{name} no longer depends on {dependency}")
                optional[dependency] = upstream[dependency]


def libvips_records(lock: JsonObject) -> tuple[str, ...]:
    version = next(
        (
            cast(str, entry["version"])
            for entry in cast(list[JsonObject], lock["packages"])
            if entry["name"] == "@img/sharp-libvips-linux-x64"
        ),
        None,
    )
    if version is None:
        return ()
    return (
        f"sharp-libvips-{version}-source-offer.json",
        f"notices/sharp-libvips-{version}.NOTICE.md",
    )


def compliance_gaps(
    lock: JsonObject, policy: JsonObject, provenance: JsonObject, notice_inventory: JsonObject
) -> list[str]:
    """Human-owned evidence that still names an older version."""
    gaps: list[str] = []
    components = cast(list[JsonObject], policy["components"])
    notices = cast(list[JsonObject], notice_inventory["entries"])
    records = {
        cast(str, record["component_id"]): record
        for record in cast(list[JsonObject], provenance["records"])
    }
    for entry in cast(list[JsonObject], lock["packages"]):
        name, version = cast(str, entry["name"]), cast(str, entry["version"])
        component = next(
            (
                item
                for item in components
                if str(item.get("purl", "")).startswith(f"pkg:npm/{name}@")
            ),
            None,
        )
        if component is None:
            continue
        if (
            component.get("purl") != f"pkg:npm/{name}@{version}"
            or component.get("version") != version
        ):
            gaps.append(
                f"compliance/policy.json component {component['id']} must name {name}@{version}"
            )
        record = records.get(cast(str, component["id"]))
        if record is None or not str(record.get("source_uri", "")).endswith(f"-{version}.tgz"):
            gaps.append(
                f"compliance/provenance.json {component['id']} must cite the {version} tarball"
            )
        for notice in notices:
            planned = str(notice.get("planned_path", ""))
            pinned = _VERSIONED_NOTICE.search(planned)
            if (
                component["id"] in cast(list[str], notice.get("component_ids", []))
                and pinned is not None
                and pinned.group(1) != version
            ):
                gaps.append(f"compliance/{planned} must be re-collected for {name}@{version}")
    gaps.extend(
        f"compliance/{relative} is required corresponding-source evidence"
        for relative in libvips_records(lock)
        if not (COMPLIANCE / relative).is_file()
    )
    return list(dict.fromkeys(gaps))


def _runtime_record(runtime_lock: JsonObject, name: str) -> JsonObject:
    for record in cast(list[JsonObject], runtime_lock["packages"]):
        if record["name"] == name:
            return record
    raise RefreshError(f"runtime-lock.json has no package record for {name}")


def _native_record(native_lock: JsonObject, destination: str) -> JsonObject:
    for record in cast(list[JsonObject], native_lock["assets"]):
        if record["destination"] == destination:
            return record
    raise RefreshError(f"native-assets.lock.json has no asset for {destination}")


def refresh_host_bundle(runtime_lock: JsonObject) -> None:
    _run(["bun", "run", "build"], RUNTIME)
    bundle = REPO_ROOT / HOST_BUNDLE
    new_digest = sha256_file(bundle)
    new_size = bundle.stat().st_size
    old_digest = cast(str, runtime_lock["runtime"]["sha256"])
    for record in [runtime_lock["runtime"], *cast(list[JsonObject], runtime_lock["outputs"])]:
        source = cast(JsonObject, record["source"])
        if source.get("uri") != HOST_BUNDLE:
            continue
        record["sha256"] = new_digest
        record["size_bytes"] = new_size
        source["provenance"] = cast(str, source["provenance"]).replace(old_digest, new_digest)


def refresh_assembled_trees(
    lock: JsonObject, runtime_lock: JsonObject, native_lock: JsonObject
) -> None:
    with tempfile.TemporaryDirectory(prefix="cua-node-refresh-") as scratch:
        output = Path(scratch) / "assembled"
        _run(
            [
                sys.executable,
                str(REPO_ROOT / "scripts" / "assemble_cua_node.py"),
                "--allow-development-dirty",
                "--output-root",
                str(output),
                "--json",
            ],
            REPO_ROOT,
        )
        generated_runtime = cast(
            JsonObject, read_json(output / "share" / "locks" / "runtime-lock.json")
        )
        generated_native = cast(
            JsonObject, read_json(output / "share" / "locks" / "native-assets.lock.json")
        )
    for entry in cast(list[JsonObject], lock["packages"]):
        name, version = cast(str, entry["name"]), cast(str, entry["version"])
        destination = f"lib/node_modules/{name}"
        if entry["native_asset"]:
            generated = _native_record(generated_native, destination)
            template = _native_record(native_lock, destination)
            cast(JsonObject, template["source"])["uri"] = tarball_uri(name, version)
        else:
            generated = _runtime_record(generated_runtime, name)
            template = _runtime_record(runtime_lock, name)
            template["integrity"] = generated["integrity"]
            template["license"] = generated["license"]
            cast(JsonObject, template["source"])["uri"] = generated["source"]["uri"]
        template["version"] = version
        template["sha256"] = generated["sha256"]
        template["size_bytes"] = generated["size_bytes"]
        entry["assembled_tree"] = {
            "sha256": generated["sha256"],
            "size_bytes": generated["size_bytes"],
        }


def refresh(*, assemble: bool) -> list[str]:
    _run(["bun", "install", "--frozen-lockfile"], RUNTIME)
    lock = cast(JsonObject, read_json(PREPARED_IMPORTS_LOCK))
    bun_packages = bun_lock_packages(RUNTIME / "bun.lock")
    refresh_sources(
        lock, bun_packages, bun_workspace_pins(RUNTIME / "bun.lock"), RUNTIME / "node_modules"
    )
    production_package = cast(JsonObject, read_json(PRODUCTION_PACKAGE))
    production_lock = cast(JsonObject, read_json(PRODUCTION_LOCK))
    sync_production(lock, bun_packages, production_package, production_lock)
    changed = [
        path
        for path, value in (
            (PREPARED_IMPORTS_LOCK, lock),
            (PRODUCTION_PACKAGE, production_package),
            (PRODUCTION_LOCK, production_lock),
        )
        if write_json(path, value)
    ]
    gaps = compliance_gaps(
        lock,
        read_json(COMPLIANCE / "policy.json"),
        read_json(COMPLIANCE / "provenance.json"),
        read_json(COMPLIANCE / "notice-inventory.json"),
    )
    gaps += toolbox_version_drift(cast(dict[str, str], production_package["dependencies"]))
    evidence_blocks_assembly = not all(
        (COMPLIANCE / relative).is_file() for relative in libvips_records(lock)
    )
    if assemble and not evidence_blocks_assembly:
        runtime_lock = cast(JsonObject, read_json(RUNTIME_LOCK))
        native_lock = cast(JsonObject, read_json(NATIVE_LOCK))
        refresh_host_bundle(runtime_lock)
        refresh_assembled_trees(lock, runtime_lock, native_lock)
        changed += [
            path
            for path, value in (
                (PREPARED_IMPORTS_LOCK, lock),
                (RUNTIME_LOCK, runtime_lock),
                (NATIVE_LOCK, native_lock),
            )
            if write_json(path, value)
        ]
    elif assemble:
        gaps.append("assembled trees not refreshed: add the sharp-libvips evidence, then rerun")
    if not gaps:
        _run(["bun", "runtime/cua-node/compliance/generate.ts"], REPO_ROOT)
    for path in dict.fromkeys(changed):
        print(f"updated {path.relative_to(REPO_ROOT)}")
    return gaps


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Refresh the compliance-pinned cua_node packages after a dependency bump."
    )
    parser.add_argument(
        "--skip-assembly",
        action="store_true",
        help="refresh source pins and production manifests only",
    )
    arguments = parser.parse_args(argv)
    try:
        gaps = refresh(assemble=not arguments.skip_assembly)
    except RefreshError as error:
        print(f"refresh failed: {error}", file=sys.stderr)
        return 2
    for gap in gaps:
        print(f"needs human update: {gap}", file=sys.stderr)
    return 1 if gaps else 0


if __name__ == "__main__":
    raise SystemExit(main())
