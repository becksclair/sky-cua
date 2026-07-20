"""Build the immutable componentized Linux x64 glibc sky-cua release set."""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

from _plugin_bundle import remove_path
from release_generation import (
    CHECKSUMS_FILE,
    COMPAT_VERSION,
    FULL_PROFILE,
    LOCKED_TARGET,
    RELEASE_MANIFEST,
    SCHEMA_VERSION,
    VerifiedRelease,
    canonical_json_bytes,
    component_record,
    content_addressed_release_id,
    sha256_file,
    verify_release_root,
    write_deterministic_tar_gz,
)


@dataclass(frozen=True)
class ComponentSource:
    name: str
    source: Path
    dependencies: tuple[str, ...] = ()
    required: bool = True
    profiles: tuple[str, ...] = (FULL_PROFILE,)


@dataclass(frozen=True)
class FileSource:
    name: str
    source: Path
    destination: str


@dataclass(frozen=True)
class ReleaseBuild:
    release: VerifiedRelease
    fat_archive: Path | None


def _verify_cua_node_release_eligibility(
    components: Sequence[ComponentSource], *, producer_commit: str
) -> None:
    component = next((item for item in components if item.name == "cua-node-linux-x64-glibc"), None)
    if component is None:
        return
    attestation_path = component.source / "share" / "provenance" / "SKY_CUA_BUILD_ATTESTATION.json"
    manifest_path = component.source / "manifest.json"
    if not attestation_path.is_file() or attestation_path.is_symlink():
        raise ValueError("cua_node component is missing its build attestation")
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise ValueError("cua_node component is missing its manifest")
    attestation = json.loads(attestation_path.read_text(encoding="utf-8"))
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if attestation.get("schema_version") != 1 or attestation.get("release_eligible") is not True:
        raise ValueError("cua_node component is not release eligible")
    if attestation.get("producer_commit") != producer_commit:
        raise ValueError("cua_node component producer commit does not match the release producer")
    inventory = attestation.get("source_inventory")
    if not isinstance(inventory, list) or not inventory:
        raise ValueError("cua_node build attestation source inventory is invalid")
    inventory_sha256 = hashlib.sha256(canonical_json_bytes(inventory)).hexdigest()
    if attestation.get("source_inventory_sha256") != inventory_sha256:
        raise ValueError("cua_node build attestation source inventory hash is invalid")
    migration_input = attestation.get("migration_input")
    if not isinstance(migration_input, dict):
        raise ValueError("cua_node build attestation migration input is missing")
    migration_sha256 = migration_input.get("source_tree_sha256")
    if (
        not isinstance(migration_sha256, str)
        or re.fullmatch(r"[0-9a-f]{64}", migration_sha256) is None
    ):
        raise ValueError("cua_node build attestation migration input hash is invalid")
    if not isinstance(migration_input.get("source_size_bytes"), int) or not isinstance(
        migration_input.get("source_file_count"), int
    ):
        raise ValueError("cua_node build attestation migration input inventory is invalid")
    if manifest.get("source", {}).get("producer_commit") != producer_commit:
        raise ValueError("cua_node manifest producer commit does not match the release producer")
    if manifest.get("source", {}).get("migration_input") != migration_input:
        raise ValueError("cua_node manifest migration input does not match its build attestation")
    first_party_build = attestation.get("first_party_build")
    if not isinstance(first_party_build, dict) or first_party_build.get("schema_version") != 1:
        raise ValueError("cua_node build attestation has no first-party build evidence")
    if not isinstance(first_party_build.get("commands"), list) or not isinstance(
        first_party_build.get("toolchain"), dict
    ):
        raise ValueError("cua_node first-party build commands or toolchain are invalid")
    outputs = first_party_build.get("outputs")
    if not isinstance(outputs, dict):
        raise ValueError("cua_node first-party build outputs are invalid")
    expected_files = {
        "runtime_cli": component.source / "lib/node_repl/cli.js",
        "browser_client": component.source
        / "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs",
        "node_repl_launcher": component.source / "bin/node_repl",
    }
    for name, path in expected_files.items():
        output = outputs.get(name)
        if not isinstance(output, dict) or output.get("sha256") != sha256_file(path):
            raise ValueError(f"cua_node first-party build output hash is invalid: {name}")
    sky_output = outputs.get("sky_cua_tarball")
    sky_manifest = manifest.get("components", {}).get("sky_cua", {})
    if not isinstance(sky_output, dict) or sky_output.get("sha256") != sky_manifest.get(
        "tarball_sha256"
    ):
        raise ValueError("cua_node sky-cua tarball build hash is invalid")
    if manifest.get("target") != "linux-x64-glibc" or manifest.get("node_version") != "24.14.0":
        raise ValueError("cua_node manifest target or Node version is invalid")
    browser_manifest = manifest.get("components", {}).get("browser_use", {})
    browser_path = expected_files["browser_client"]
    browser_hash = sha256_file(browser_path)
    if browser_manifest.get("entrypoint_sha256") != browser_hash or manifest.get(
        "trusted_browser_client_sha256s"
    ) != [browser_hash]:
        raise ValueError("cua_node embedded Browser hash or trust set is invalid")
    checksums = manifest.get("checksums")
    if not isinstance(checksums, dict) or checksums.get("algorithm") != "sha256":
        raise ValueError("cua_node manifest checksums are invalid")
    checksum_records = checksums.get("files")
    if not isinstance(checksum_records, list):
        raise ValueError("cua_node manifest file checksums are invalid")
    declared: dict[str, tuple[str, int]] = {}
    for record in checksum_records:
        if not isinstance(record, dict):
            raise ValueError("cua_node manifest file checksum record is invalid")
        relative = record.get("path")
        digest = record.get("sha256")
        size = record.get("size_bytes")
        if (
            not isinstance(relative, str)
            or _component_relative_path(relative, field="cua_node checksum path").as_posix()
            != relative
            or not isinstance(digest, str)
            or re.fullmatch(r"[0-9a-f]{64}", digest) is None
            or not isinstance(size, int)
            or size < 0
            or relative in declared
        ):
            raise ValueError("cua_node manifest file checksum record is invalid")
        declared[relative] = (digest, size)
    actual: dict[str, tuple[str, int]] = {}
    for path in sorted(component.source.rglob("*"), key=lambda item: item.as_posix()):
        relative = path.relative_to(component.source).as_posix()
        if path.is_symlink():
            raise ValueError(f"cua_node component contains a symlink: {relative}")
        if path.is_file() and relative != "manifest.json":
            actual[relative] = (sha256_file(path), path.stat().st_size)
        elif not path.is_file() and not path.is_dir():
            raise ValueError(f"cua_node component contains a special entry: {relative}")
    if declared != actual:
        raise ValueError("cua_node manifest file inventory does not match the component tree")
    lock_hashes = checksums.get("lock_hashes")
    runtime_lock_path = component.source / "share/locks/runtime-lock.json"
    native_lock_path = component.source / "share/locks/native-assets.lock.json"
    if not isinstance(lock_hashes, dict) or lock_hashes != {
        "runtime_lock_sha256": sha256_file(runtime_lock_path),
        "native_assets_lock_sha256": sha256_file(native_lock_path),
    }:
        raise ValueError("cua_node manifest lock hashes are invalid")
    for name, path in (("runtime", runtime_lock_path), ("native", native_lock_path)):
        lock = json.loads(path.read_text(encoding="utf-8"))
        if lock.get("release_ready") is not True or lock.get("release_blockers") != []:
            raise ValueError(f"cua_node {name} lock is not release ready")


def _copy_file_source(staging: Path, source: FileSource) -> dict[str, str]:
    destination = staging / source.destination
    if not source.source.is_file() or source.source.is_symlink():
        raise FileNotFoundError(f"release input is not a regular file: {source.source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.parent.chmod(0o755)
    shutil.copy2(source.source, destination)
    destination.chmod(0o644)
    return {"path": source.destination, "sha256": sha256_file(destination)}


def _write_checksums(release_root: Path) -> None:
    paths = sorted(
        (
            path
            for path in release_root.rglob("*")
            if path.is_file() and not path.is_symlink() and path.name != CHECKSUMS_FILE
        ),
        key=lambda path: path.relative_to(release_root).as_posix(),
    )
    lines = [f"{sha256_file(path)}  {path.relative_to(release_root).as_posix()}" for path in paths]
    destination = release_root / CHECKSUMS_FILE
    destination.write_text("\n".join(lines) + "\n", encoding="utf-8")
    destination.chmod(0o644)


def _component_relative_path(value: str, *, field: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if not value or path.is_absolute() or ".." in path.parts or value != path.as_posix():
        raise ValueError(f"{field} must be a normalized relative POSIX path")
    return path


def build_release_set(
    output_root: Path,
    *,
    producer_commit: str,
    runtime: Mapping[str, object],
    trusted_browser_client_sha256s: Sequence[str],
    capabilities_supported: Sequence[str],
    capabilities_unsupported: Sequence[str],
    browser_api_schema_version: int,
    browser_command_schema_version: int,
    canonical_browser_entrypoint: str,
    compatibility_browser_projections: Sequence[str],
    components: Sequence[ComponentSource],
    locks: Sequence[FileSource],
    artifacts: Sequence[FileSource],
    documentation: Mapping[str, str] | None = None,
    source_date_epoch: int | None = None,
    include_fat_archive: bool = True,
) -> ReleaseBuild:
    """Build and verify a content-addressed release under ``output_root``."""
    component_names: set[str] = set()
    for component in components:
        normalized_name = _component_relative_path(component.name, field="component name")
        if len(normalized_name.parts) != 1 or component.name in component_names:
            raise ValueError(f"component name must be a unique path segment: {component.name}")
        component_names.add(component.name)
    required_components = {
        "core-linux-x64",
        "browser-js",
        "cua-node-linux-x64-glibc",
        "codex-compat",
        "compliance",
    }
    if not required_components.issubset(component_names):
        missing = sorted(required_components - component_names)
        raise ValueError(f"release is missing required components: {missing}")
    _verify_cua_node_release_eligibility(components, producer_commit=producer_commit)
    destinations: set[str] = set()
    for source in (*locks, *artifacts):
        destination = _component_relative_path(source.destination, field="file destination")
        if destination.as_posix() in destinations:
            raise ValueError(f"duplicate release file destination: {destination}")
        destinations.add(destination.as_posix())

    output_root.mkdir(parents=True, exist_ok=True)
    staging = output_root / f".release-staging-{os.getpid()}"
    remove_path(staging)
    staging.mkdir()
    staging.chmod(0o755)
    try:
        component_records: list[dict[str, object]] = []
        for component in sorted(components, key=lambda item: item.name):
            if not component.source.is_dir() or component.source.is_symlink():
                raise FileNotFoundError(
                    f"component source is not a real directory: {component.source}"
                )
            destination = staging / "components" / component.name
            shutil.copytree(component.source, destination, symlinks=True)
            archive_relative = f"archives/{component.name}.tar.gz"
            write_deterministic_tar_gz(
                destination,
                staging / archive_relative,
                arcname=component.name,
            )
            (staging / archive_relative).chmod(0o644)
            (staging / "components").chmod(0o755)
            (staging / "archives").chmod(0o755)
            component_records.append(
                component_record(
                    staging,
                    name=component.name,
                    path=f"components/{component.name}",
                    archive=archive_relative,
                    dependencies=component.dependencies,
                    required=component.required,
                    profiles=component.profiles,
                )
            )

        canonical_relative = _component_relative_path(
            canonical_browser_entrypoint, field="canonical_browser_entrypoint"
        )
        canonical_path = staging / "components" / "browser-js" / canonical_relative
        if not canonical_path.is_file() or canonical_path.is_symlink():
            raise FileNotFoundError(f"canonical Browser entrypoint is missing: {canonical_path}")
        canonical_browser_sha256 = sha256_file(canonical_path)
        if sorted(set(trusted_browser_client_sha256s)) != [canonical_browser_sha256]:
            raise ValueError(
                "trusted Browser SHA list must contain exactly the canonical Browser entrypoint hash"
            )
        projection_records: list[dict[str, str]] = []
        canonical_bytes = canonical_path.read_bytes()
        for projection in sorted(set(compatibility_browser_projections)):
            projection_relative = _component_relative_path(
                projection, field="compatibility_browser_projections"
            )
            projection_path = staging / "components" / "codex-compat" / projection_relative
            if not projection_path.is_file() or projection_path.is_symlink():
                raise FileNotFoundError(
                    f"Codex Browser compatibility projection is missing: {projection_path}"
                )
            if projection_path.read_bytes() != canonical_bytes:
                raise ValueError(
                    f"Codex Browser compatibility projection differs from canonical bytes: {projection}"
                )
            projection_records.append(
                {
                    "component": "codex-compat",
                    "path": f"components/codex-compat/{projection_relative.as_posix()}",
                    "sha256": canonical_browser_sha256,
                }
            )
        if not projection_records:
            raise ValueError("at least one Codex Browser compatibility projection is required")

        lock_records = {
            source.name: _copy_file_source(staging, source)
            for source in sorted(locks, key=lambda item: item.name)
        }
        artifact_records = {
            source.name: _copy_file_source(staging, source)
            for source in sorted(artifacts, key=lambda item: item.name)
        }

        producer: dict[str, object] = {"commit": producer_commit}
        if source_date_epoch is not None:
            producer["source_date_epoch"] = source_date_epoch
        manifest_without_id: dict[str, object] = {
            "schema_version": SCHEMA_VERSION,
            "compat_version": COMPAT_VERSION,
            "producer": producer,
            "target": dict(LOCKED_TARGET),
            "components": component_records,
            "runtime": dict(runtime),
            "trusted_browser_client_sha256s": sorted(set(trusted_browser_client_sha256s)),
            "locks": lock_records,
            "artifacts": artifact_records,
            "capabilities": {
                "supported": sorted(set(capabilities_supported)),
                "unsupported": sorted(set(capabilities_unsupported)),
            },
            "browser_contract": {
                "api_schema_version": browser_api_schema_version,
                "command_schema_version": browser_command_schema_version,
                "caller_provenance": [
                    "codex_desktop",
                    "direct_mcp",
                    "openclaw",
                    "opencode",
                ],
                "transport_identities": ["extension_native_host", "host_provided_iab"],
                "no_ambiguous_mutation_retry": True,
                "canonical_browser": {
                    "component": "browser-js",
                    "path": f"components/browser-js/{canonical_relative.as_posix()}",
                    "sha256": canonical_browser_sha256,
                },
                "compatibility_projections": projection_records,
            },
        }
        if documentation is not None:
            manifest_without_id["documentation"] = {
                "component": "documentation",
                **{
                    name: {
                        "path": relative,
                        "sha256": sha256_file(staging / relative),
                    }
                    for name, relative in sorted(documentation.items())
                },
            }
        release_id = content_addressed_release_id(manifest_without_id)
        manifest = {**manifest_without_id, "release_id": release_id}
        manifest_path = staging / RELEASE_MANIFEST
        manifest_path.write_bytes(canonical_json_bytes(manifest) + b"\n")
        manifest_path.chmod(0o644)
        _write_checksums(staging)
        verified_staging = verify_release_root(staging)

        final = output_root / release_id
        if final.exists():
            existing = verify_release_root(
                final, expected_manifest_sha256=verified_staging.manifest_sha256
            )
            remove_path(staging)
            verified = existing
        else:
            os.replace(staging, final)
            verified = verify_release_root(
                final, expected_manifest_sha256=verified_staging.manifest_sha256
            )

        fat_archive: Path | None = None
        if include_fat_archive:
            fat_archive = output_root / f"sky-cua-{release_id}-linux-x64-glibc.tar.gz"
            write_deterministic_tar_gz(final, fat_archive, arcname=f"sky-cua-{release_id}")
        return ReleaseBuild(verified, fat_archive)
    finally:
        remove_path(staging)
