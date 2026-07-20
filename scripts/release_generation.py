"""Immutable component release verification and transactional generation install.

The producer manifest is the only authority for component membership and hashes.
Installed generations are never repaired by borrowing files from another release.
"""

from __future__ import annotations

import argparse
import base64
import fcntl
import gzip
import hashlib
import json
import os
import re
import shutil
import stat
import tarfile
from collections.abc import Callable, Iterable, Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path, PurePosixPath
from typing import Any, BinaryIO, cast

from _plugin_bundle import remove_path

RELEASE_MANIFEST = "RELEASE.json"
CHECKSUMS_FILE = "SHA256SUMS"
INSTALLATION_STATE = "INSTALLATION.json"
JOURNAL_FILE = "install-journal.json"
SCHEMA_VERSION = 1
COMPAT_VERSION = 1
FULL_PROFILE = "full"
CORE_ONLY_PROFILE = "core-only"
INSTALL_PROFILES = frozenset({FULL_PROFILE, CORE_ONLY_PROFILE})
CORE_COMPONENT = "core-linux-x64"
LOCKED_TARGET = {
    "os": "linux",
    "arch": "x86_64",
    "libc": "glibc",
    "triple": "x86_64-unknown-linux-gnu",
}
LOCKED_RUNTIME_VERSIONS = {
    "node": "24.14.0",
    "playwright": "1.57.0",
    "pdfjs": "5.4.624",
    "tesseract_js": "7.0.0",
    "sharp": "0.34.5",
    "sharp_linux_x64": "0.34.5",
    "sharp_libvips_linux_x64": "1.2.4",
    "canvas_linux_x64_gnu": "0.1.91",
}
CALLER_PROVENANCE_VOCABULARY = (
    "codex_desktop",
    "direct_mcp",
    "openclaw",
    "opencode",
)
BRIDGE_TRANSPORT_IDENTITIES = ("extension_native_host", "host_provided_iab")
CHECKOUT_SHAPED_PATH_PATTERNS = (
    re.compile(rb"/(?:home|Users)/[^/\x00\s]+/(?:projects?|src|source|code|workspace|repos?)/"),
    re.compile(
        rb"[A-Za-z]:\\Users\\[^\\\x00\s]+\\(?:projects?|src|source|code|workspace|repos?)\\",
        re.IGNORECASE,
    ),
)
FORBIDDEN_CHECKOUT_PATH_PATTERNS = (
    re.compile(
        rb"/(?:home|Users)/[^/\x00\s]+/(?:projects?|src|source|code|workspace|repos?)/sky-cua(?:/|\x00|\s|$)"
    ),
    re.compile(
        rb"[A-Za-z]:\\Users\\[^\\\x00\s]+\\(?:projects?|src|source|code|workspace|repos?)\\sky-cua(?:\\|\x00|\s|$)",
        re.IGNORECASE,
    ),
)
RELEASE_ID_PATTERN = re.compile(r"^[0-9a-f]{64}$")
SHA256_PATTERN = re.compile(r"^[0-9a-f]{64}$")
Failpoint = Callable[[str], None]


class ReleaseValidationError(ValueError):
    """A release or installed generation violates the producer contract."""


class InstallTransactionError(RuntimeError):
    """A generation install or recovery could not converge safely."""


def _chrome_extension_id(public_key: str) -> str:
    try:
        decoded = base64.b64decode(public_key, validate=True)
    except (ValueError, TypeError) as error:
        raise ReleaseValidationError("Browser extension manifest key is invalid") from error
    hexadecimal = hashlib.sha256(decoded).hexdigest()[:32]
    return "".join(chr(ord("a") + int(nibble, 16)) for nibble in hexadecimal)


@dataclass(frozen=True)
class TreeDigest:
    sha256: str
    size: int
    entries: tuple[dict[str, object], ...]


@dataclass(frozen=True)
class VerifiedRelease:
    root: Path
    release_id: str
    manifest_sha256: str
    profile: str
    component_names: tuple[str, ...]


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_json_bytes(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def content_addressed_release_id(manifest: Mapping[str, object]) -> str:
    """Derive the release id from the canonical manifest with no embedded id."""
    unsigned = dict(manifest)
    unsigned.pop("release_id", None)
    return hashlib.sha256(canonical_json_bytes(unsigned)).hexdigest()


def _relative_contract_path(value: object, *, field: str) -> PurePosixPath:
    if not isinstance(value, str) or not value:
        raise ReleaseValidationError(f"{field} must be a non-empty relative path")
    path = PurePosixPath(value)
    if path.is_absolute() or ".." in path.parts or value != path.as_posix():
        raise ReleaseValidationError(f"{field} must be a normalized relative POSIX path")
    return path


def _safe_join(root: Path, relative: PurePosixPath) -> Path:
    candidate = root.joinpath(*relative.parts)
    resolved_root = root.resolve()
    resolved_candidate = candidate.resolve(strict=False)
    if not resolved_candidate.is_relative_to(resolved_root):
        raise ReleaseValidationError(f"path escapes release root: {relative}")
    return candidate


def _mode(path: Path, *, follow_symlinks: bool = True) -> int:
    return stat.S_IMODE(path.stat(follow_symlinks=follow_symlinks).st_mode)


def canonical_tree_digest(root: Path) -> TreeDigest:
    """Hash a directory without depending on mtimes, owners, or traversal order."""
    if not root.is_dir() or root.is_symlink():
        raise ReleaseValidationError(f"component path is not a real directory: {root}")

    entries: list[dict[str, object]] = []
    total_size = 0
    for path in sorted(root.rglob("*"), key=lambda entry: entry.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise ReleaseValidationError(f"component symlink is unsupported: {relative}")
        elif path.is_dir():
            entries.append({"path": relative, "type": "directory", "mode": _mode(path)})
        elif path.is_file():
            size = path.stat().st_size
            total_size += size
            entries.append(
                {
                    "path": relative,
                    "type": "file",
                    "mode": _mode(path),
                    "size": size,
                    "sha256": sha256_file(path),
                }
            )
        else:
            raise ReleaseValidationError(f"unsupported special file in component: {path}")
    encoded = canonical_json_bytes(entries)
    return TreeDigest(hashlib.sha256(encoded).hexdigest(), total_size, tuple(entries))


def _normalized_tar_info(info: tarfile.TarInfo) -> tarfile.TarInfo:
    info.uid = 0
    info.gid = 0
    info.uname = "root"
    info.gname = "root"
    info.mtime = 0
    info.pax_headers = {}
    return info


def _write_component_archive_stream(source: Path, raw: BinaryIO, *, arcname: str) -> None:
    with (
        gzip.GzipFile(filename="", mode="wb", fileobj=raw, compresslevel=9, mtime=0) as zipped,
        tarfile.open(fileobj=zipped, mode="w", format=tarfile.PAX_FORMAT) as tar,
    ):
        tar.add(source, arcname=arcname, recursive=False, filter=_normalized_tar_info)
        for path in sorted(
            source.rglob("*"), key=lambda entry: entry.relative_to(source).as_posix()
        ):
            relative = path.relative_to(source).as_posix()
            tar.add(
                path,
                arcname=f"{arcname}/{relative}",
                recursive=False,
                filter=_normalized_tar_info,
            )


def write_deterministic_tar_gz(source: Path, archive: Path, *, arcname: str) -> None:
    """Archive a component with stable ordering and normalized ownership/times."""
    if not source.is_dir() or source.is_symlink():
        raise ReleaseValidationError(f"archive source is not a real directory: {source}")
    if not arcname or PurePosixPath(arcname).name != arcname:
        raise ReleaseValidationError("archive root name must be one path segment")
    archive.parent.mkdir(parents=True, exist_ok=True)
    temp = archive.with_name(f".{archive.name}.tmp-{os.getpid()}")
    remove_path(temp)
    try:
        with temp.open("wb") as raw:
            _write_component_archive_stream(source, raw, arcname=arcname)
            raw.flush()
            os.fsync(raw.fileno())
        os.replace(temp, archive)
        _fsync_directory(archive.parent)
    finally:
        remove_path(temp)


def archive_tree_digest(archive: Path, *, root_name: str, maximum_tree_size: int) -> TreeDigest:
    """Hash a component archive's expanded regular-file tree without extracting it."""
    entries: list[dict[str, object]] = []
    seen: set[str] = set()
    root_seen = False
    total_size = 0
    try:
        with tarfile.open(archive, mode="r:gz") as tar:
            for member in tar:
                raw = member.name.rstrip("/")
                path = PurePosixPath(raw)
                if path.is_absolute() or ".." in path.parts or not path.parts:
                    raise ReleaseValidationError(
                        f"component archive contains unsafe path: {member.name}"
                    )
                if path.parts[0] != root_name:
                    raise ReleaseValidationError(
                        f"component archive root must be {root_name}: {member.name}"
                    )
                if len(path.parts) == 1:
                    if root_seen or not member.isdir():
                        raise ReleaseValidationError("component archive root must be a directory")
                    root_seen = True
                    continue
                relative = PurePosixPath(*path.parts[1:]).as_posix()
                if relative in seen:
                    raise ReleaseValidationError(
                        f"component archive contains duplicate path: {relative}"
                    )
                seen.add(relative)
                mode = member.mode & 0o7777
                if member.isdir():
                    entries.append({"path": relative, "type": "directory", "mode": mode})
                    continue
                if not member.isfile():
                    raise ReleaseValidationError(
                        f"component archive contains unsupported entry: {relative}"
                    )
                total_size += member.size
                if total_size > maximum_tree_size:
                    raise ReleaseValidationError(
                        "component archive expands beyond declared tree size"
                    )
                extracted = tar.extractfile(member)
                if extracted is None:
                    raise ReleaseValidationError(
                        f"component archive file cannot be read: {relative}"
                    )
                digest = hashlib.sha256()
                actual_size = 0
                while chunk := extracted.read(1024 * 1024):
                    actual_size += len(chunk)
                    digest.update(chunk)
                if actual_size != member.size:
                    raise ReleaseValidationError(
                        f"component archive file size mismatch: {relative}"
                    )
                entries.append(
                    {
                        "path": relative,
                        "type": "file",
                        "mode": mode,
                        "size": actual_size,
                        "sha256": digest.hexdigest(),
                    }
                )
    except (tarfile.TarError, OSError) as error:
        raise ReleaseValidationError(f"invalid component archive {archive}: {error}") from error
    if not root_seen:
        raise ReleaseValidationError(f"component archive is missing root directory {root_name}")
    entries.sort(key=lambda entry: cast(str, entry["path"]))
    encoded = canonical_json_bytes(entries)
    return TreeDigest(hashlib.sha256(encoded).hexdigest(), total_size, tuple(entries))


def component_record(
    release_root: Path,
    *,
    name: str,
    path: str,
    archive: str,
    dependencies: Sequence[str] = (),
    required: bool = True,
    profiles: Sequence[str] = (FULL_PROFILE,),
) -> dict[str, object]:
    relative = _relative_contract_path(path, field=f"component {name!r} path")
    digest = canonical_tree_digest(_safe_join(release_root, relative))
    archive_relative = _relative_contract_path(archive, field=f"component {name!r} archive")
    archive_path = _safe_join(release_root, archive_relative)
    if not archive_path.is_file() or archive_path.is_symlink():
        raise ReleaseValidationError(f"component {name!r} archive is missing: {archive_relative}")
    return {
        "name": name,
        "path": relative.as_posix(),
        "archive": archive_relative.as_posix(),
        "dependencies": sorted(set(dependencies)),
        "required": required,
        "profiles": sorted(set(profiles)),
        "sha256": sha256_file(archive_path),
        "size": archive_path.stat().st_size,
        "tree_size": digest.size,
        "tree_sha256": digest.sha256,
    }


def _load_object(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReleaseValidationError(f"cannot read {path}: {error}") from error
    if not isinstance(value, dict):
        raise ReleaseValidationError(f"{path} must contain a JSON object")
    return cast(dict[str, Any], value)


def _string(value: object, *, field: str) -> str:
    if not isinstance(value, str) or not value:
        raise ReleaseValidationError(f"{field} must be a non-empty string")
    return value


def _sha256(value: object, *, field: str) -> str:
    result = _string(value, field=field)
    if SHA256_PATTERN.fullmatch(result) is None:
        raise ReleaseValidationError(f"{field} must be a lowercase SHA-256")
    return result


def _string_list(value: object, *, field: str) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) or not item for item in value):
        raise ReleaseValidationError(f"{field} must be an array of non-empty strings")
    return cast(list[str], value)


def _component_map(manifest: Mapping[str, Any]) -> dict[str, dict[str, Any]]:
    components = manifest.get("components")
    if not isinstance(components, list) or not components:
        raise ReleaseValidationError("components must be a non-empty array")
    result: dict[str, dict[str, Any]] = {}
    for index, raw in enumerate(components):
        if not isinstance(raw, dict):
            raise ReleaseValidationError(f"components[{index}] must be an object")
        component = cast(dict[str, Any], raw)
        name = _string(component.get("name"), field=f"components[{index}].name")
        if name in result:
            raise ReleaseValidationError(f"duplicate component name: {name}")
        result[name] = component
    return result


def _selected_components(
    components: Mapping[str, Mapping[str, Any]], profile: str
) -> tuple[str, ...]:
    if profile not in INSTALL_PROFILES:
        raise ReleaseValidationError(f"unsupported install profile: {profile}")
    selected: set[str] = set()
    if profile == CORE_ONLY_PROFILE:
        if CORE_COMPONENT not in components:
            raise ReleaseValidationError(f"{CORE_ONLY_PROFILE} requires {CORE_COMPONENT}")
        selected.add(CORE_COMPONENT)
        for name, component in components.items():
            profiles = _string_list(
                component.get("profiles", []), field=f"component {name} profiles"
            )
            if CORE_ONLY_PROFILE in profiles:
                selected.add(name)
    else:
        for name, component in components.items():
            profiles = _string_list(
                component.get("profiles", []), field=f"component {name} profiles"
            )
            if bool(component.get("required")) or profile in profiles:
                selected.add(name)

    pending = list(selected)
    while pending:
        name = pending.pop()
        component = components.get(name)
        if component is None:
            raise ReleaseValidationError(f"selected component does not exist: {name}")
        dependencies = _string_list(
            component.get("dependencies", []), field=f"component {name} dependencies"
        )
        for dependency in dependencies:
            if dependency not in components:
                raise ReleaseValidationError(f"component {name} depends on missing {dependency}")
            if dependency not in selected:
                selected.add(dependency)
                pending.append(dependency)
    return tuple(sorted(selected))


def _verify_hashed_artifact(root: Path, raw: object, *, field: str) -> None:
    if not isinstance(raw, dict):
        raise ReleaseValidationError(f"{field} must be an object")
    artifact = cast(dict[str, Any], raw)
    relative = _relative_contract_path(artifact.get("path"), field=f"{field}.path")
    expected = _sha256(artifact.get("sha256"), field=f"{field}.sha256")
    path = _safe_join(root, relative)
    if not path.is_file() or path.is_symlink():
        raise ReleaseValidationError(f"{field} is missing: {relative}")
    actual = sha256_file(path)
    if actual != expected:
        raise ReleaseValidationError(f"{field} hash mismatch: expected {expected}, got {actual}")


def _browser_binding(
    raw: object,
    *,
    field: str,
    expected_component: str,
    components: Mapping[str, Mapping[str, Any]],
) -> dict[str, str]:
    if not isinstance(raw, dict):
        raise ReleaseValidationError(f"{field} must be an object")
    component = _string(raw.get("component"), field=f"{field}.component")
    if component != expected_component or component not in components:
        raise ReleaseValidationError(f"{field}.component must be {expected_component}")
    path = _relative_contract_path(raw.get("path"), field=f"{field}.path")
    component_root = _relative_contract_path(
        components[component].get("path"), field=f"component {component} path"
    )
    if path.parts[: len(component_root.parts)] != component_root.parts:
        raise ReleaseValidationError(f"{field}.path must be inside component {component}")
    digest = _sha256(raw.get("sha256"), field=f"{field}.sha256")
    return {"component": component, "path": path.as_posix(), "sha256": digest}


def _verify_checksums(
    root: Path,
    *,
    omitted_component_paths: Sequence[PurePosixPath] = (),
    omitted_archives: Sequence[PurePosixPath] = (),
) -> None:
    checksums_path = root / CHECKSUMS_FILE
    if not checksums_path.is_file() or checksums_path.is_symlink():
        raise ReleaseValidationError(f"checksums file missing: {checksums_path}")
    declared: dict[str, str] = {}
    for line_number, line in enumerate(
        checksums_path.read_text(encoding="utf-8").splitlines(), start=1
    ):
        digest, separator, relative_raw = line.partition("  ")
        if not separator:
            raise ReleaseValidationError(f"invalid SHA256SUMS line {line_number}")
        relative = _relative_contract_path(relative_raw, field=f"SHA256SUMS line {line_number}")
        relative_text = relative.as_posix()
        if relative_text in declared:
            raise ReleaseValidationError(f"duplicate SHA256SUMS path: {relative_text}")
        declared[relative_text] = _sha256(digest, field=f"SHA256SUMS line {line_number}")

    expected_paths: set[str] = set()
    for path in root.rglob("*"):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise ReleaseValidationError(f"release tree symlink is unsupported: {relative}")
        if path.is_file() and relative not in {CHECKSUMS_FILE, INSTALLATION_STATE}:
            expected_paths.add(relative)
    allowed_missing = {
        relative
        for relative in declared
        if not _safe_join(root, PurePosixPath(relative)).exists()
        and (
            any(
                relative == prefix.as_posix() or relative.startswith(prefix.as_posix() + "/")
                for prefix in omitted_component_paths
            )
            or any(relative == archive.as_posix() for archive in omitted_archives)
        )
    }
    if set(declared) - allowed_missing != expected_paths:
        missing = sorted(expected_paths - set(declared))
        extra = sorted(set(declared) - expected_paths - allowed_missing)
        raise ReleaseValidationError(
            f"SHA256SUMS path set mismatch: missing={missing}, extra={extra}"
        )
    for relative, expected in declared.items():
        if relative in allowed_missing:
            continue
        actual = sha256_file(_safe_join(root, PurePosixPath(relative)))
        if actual != expected:
            raise ReleaseValidationError(
                f"SHA256SUMS hash mismatch for {relative}: expected {expected}, got {actual}"
            )


def validate_manifest_shape(manifest: Mapping[str, Any]) -> dict[str, dict[str, Any]]:
    if manifest.get("schema_version") != SCHEMA_VERSION:
        raise ReleaseValidationError(f"schema_version must be {SCHEMA_VERSION}")
    if manifest.get("compat_version") != COMPAT_VERSION:
        raise ReleaseValidationError(f"compat_version must be {COMPAT_VERSION}")
    release_id = _string(manifest.get("release_id"), field="release_id")
    if RELEASE_ID_PATTERN.fullmatch(release_id) is None:
        raise ReleaseValidationError("release_id contains unsupported characters")

    producer = manifest.get("producer")
    if not isinstance(producer, dict):
        raise ReleaseValidationError("producer must be an object")
    _string(producer.get("commit"), field="producer.commit")

    target = manifest.get("target")
    if not isinstance(target, dict):
        raise ReleaseValidationError("target must be an object")
    for field, expected in LOCKED_TARGET.items():
        if target.get(field) != expected:
            raise ReleaseValidationError(f"target.{field} must be {expected}")

    runtime = manifest.get("runtime")
    if not isinstance(runtime, dict):
        raise ReleaseValidationError("runtime must be an object")
    for field in ("node", "node_repl", "browser_use", "sky_cua_js"):
        _string(runtime.get(field), field=f"runtime.{field}")
    for field, expected in LOCKED_RUNTIME_VERSIONS.items():
        if runtime.get(field) != expected:
            raise ReleaseValidationError(f"runtime.{field} must be {expected}")
    _string(runtime.get("pixelmatch"), field="runtime.pixelmatch")
    codecs = _string_list(runtime.get("codecs"), field="runtime.codecs")
    if not codecs:
        raise ReleaseValidationError("runtime.codecs must not be empty")

    hashes = _string_list(
        manifest.get("trusted_browser_client_sha256s"),
        field="trusted_browser_client_sha256s",
    )
    if not hashes:
        raise ReleaseValidationError("trusted_browser_client_sha256s must not be empty")
    for index, value in enumerate(hashes):
        _sha256(value, field=f"trusted_browser_client_sha256s[{index}]")

    capabilities = manifest.get("capabilities")
    if not isinstance(capabilities, dict):
        raise ReleaseValidationError("capabilities must be an object")
    _string_list(capabilities.get("supported"), field="capabilities.supported")
    _string_list(capabilities.get("unsupported"), field="capabilities.unsupported")

    browser_contract = manifest.get("browser_contract")
    if not isinstance(browser_contract, dict):
        raise ReleaseValidationError("browser_contract must be an object")
    for field in ("api_schema_version", "command_schema_version"):
        value = browser_contract.get(field)
        if not isinstance(value, int) or isinstance(value, bool) or value < 1:
            raise ReleaseValidationError(f"browser_contract.{field} must be a positive integer")
    provenance = _string_list(
        browser_contract.get("caller_provenance"),
        field="browser_contract.caller_provenance",
    )
    if tuple(sorted(provenance)) != CALLER_PROVENANCE_VOCABULARY:
        raise ReleaseValidationError("browser_contract.caller_provenance vocabulary mismatch")
    transports = _string_list(
        browser_contract.get("transport_identities"),
        field="browser_contract.transport_identities",
    )
    if tuple(sorted(transports)) != BRIDGE_TRANSPORT_IDENTITIES:
        raise ReleaseValidationError("browser_contract.transport_identities vocabulary mismatch")
    if browser_contract.get("no_ambiguous_mutation_retry") is not True:
        raise ReleaseValidationError("browser_contract.no_ambiguous_mutation_retry must be true")

    components = _component_map(manifest)
    required_components = {
        "core-linux-x64",
        "browser-js",
        "cua-node-linux-x64-glibc",
        "codex-compat",
        "compliance",
    }
    allowed_components = required_components | {"documentation", "installer"}
    if not required_components.issubset(components):
        raise ReleaseValidationError(
            f"required release components are missing: {sorted(required_components - components.keys())}"
        )
    if not set(components).issubset(allowed_components):
        raise ReleaseValidationError(
            f"unsupported release components: {sorted(set(components) - allowed_components)}"
        )
    documentation = manifest.get("documentation")
    if documentation is not None:
        if not isinstance(documentation, dict):
            raise ReleaseValidationError("documentation must be an object when present")
        documentation_component = _string(
            documentation.get("component"), field="documentation.component"
        )
        if documentation_component != "documentation" or documentation_component not in components:
            raise ReleaseValidationError("documentation.component must be documentation")
        documentation_root = _relative_contract_path(
            components[documentation_component].get("path"),
            field="component documentation path",
        )
        for field in (
            "api_inventory",
            "capability_inventory",
            "example_inventory",
            "routing_inventory",
        ):
            if not isinstance(documentation.get(field), dict):
                raise ReleaseValidationError(f"documentation.{field} must be an object")
            artifact = cast(dict[str, Any], documentation[field])
            artifact_path = _relative_contract_path(
                artifact.get("path"), field=f"documentation.{field}.path"
            )
            _sha256(artifact.get("sha256"), field=f"documentation.{field}.sha256")
            if artifact_path.parts[: len(documentation_root.parts)] != documentation_root.parts:
                raise ReleaseValidationError(
                    f"documentation.{field}.path must be inside the documentation component"
                )
    elif "model-facing-documentation" in capabilities.get("supported", []):
        raise ReleaseValidationError(
            "model-facing-documentation support requires the documentation manifest section"
        )
    for name, component in components.items():
        _relative_contract_path(component.get("path"), field=f"component {name} path")
        _relative_contract_path(component.get("archive"), field=f"component {name} archive")
        dependencies = _string_list(
            component.get("dependencies"), field=f"component {name} dependencies"
        )
        if name in dependencies:
            raise ReleaseValidationError(f"component {name} cannot depend on itself")
        if not isinstance(component.get("required"), bool):
            raise ReleaseValidationError(f"component {name} required must be boolean")
        profiles = _string_list(component.get("profiles"), field=f"component {name} profiles")
        if any(profile not in INSTALL_PROFILES for profile in profiles):
            raise ReleaseValidationError(f"component {name} has unsupported install profile")
        size = component.get("size")
        if not isinstance(size, int) or isinstance(size, bool) or size < 0:
            raise ReleaseValidationError(f"component {name} size must be a non-negative integer")
        tree_size = component.get("tree_size")
        if not isinstance(tree_size, int) or isinstance(tree_size, bool) or tree_size < 0:
            raise ReleaseValidationError(
                f"component {name} tree_size must be a non-negative integer"
            )
        _sha256(component.get("tree_sha256"), field=f"component {name} tree_sha256")
        _sha256(component.get("sha256"), field=f"component {name} sha256")

    canonical_browser = _browser_binding(
        browser_contract.get("canonical_browser"),
        field="browser_contract.canonical_browser",
        expected_component="browser-js",
        components=components,
    )
    projections_raw = browser_contract.get("compatibility_projections")
    if not isinstance(projections_raw, list) or not projections_raw:
        raise ReleaseValidationError(
            "browser_contract.compatibility_projections must be a non-empty array"
        )
    projection_paths: set[str] = set()
    for index, raw in enumerate(projections_raw):
        projection = _browser_binding(
            raw,
            field=f"browser_contract.compatibility_projections[{index}]",
            expected_component="codex-compat",
            components=components,
        )
        if projection["path"] in projection_paths:
            raise ReleaseValidationError("duplicate Browser compatibility projection path")
        projection_paths.add(projection["path"])
        if projection["sha256"] != canonical_browser["sha256"]:
            raise ReleaseValidationError(
                "Browser compatibility projection hash must equal the canonical Browser hash"
            )
    if hashes != [canonical_browser["sha256"]]:
        raise ReleaseValidationError(
            "trusted_browser_client_sha256s must contain exactly the canonical Browser hash"
        )

    extension = browser_contract.get("extension_bridge")
    extension_capability = "browser-extension-bridge" in capabilities.get("supported", [])
    if extension_capability:
        if not isinstance(extension, dict) or set(extension) != {
            "component",
            "extension_id",
            "manifest_sha256",
            "path",
            "tree_sha256",
            "version",
        }:
            raise ReleaseValidationError(
                "browser-extension-bridge support requires one exact extension binding"
            )
        if extension.get("component") != "core-linux-x64":
            raise ReleaseValidationError("Browser extension must be owned by the core component")
        extension_path = _relative_contract_path(
            extension.get("path"), field="browser_contract.extension_bridge.path"
        )
        core_path = _relative_contract_path(
            components["core-linux-x64"].get("path"), field="component core-linux-x64 path"
        )
        if extension_path.parts[: len(core_path.parts)] != core_path.parts:
            raise ReleaseValidationError("Browser extension path must be inside the core component")
        extension_id = _string(
            extension.get("extension_id"),
            field="browser_contract.extension_bridge.extension_id",
        )
        if re.fullmatch(r"[a-p]{32}", extension_id) is None:
            raise ReleaseValidationError("Browser extension id is invalid")
        _string(extension.get("version"), field="browser_contract.extension_bridge.version")
        _sha256(
            extension.get("manifest_sha256"),
            field="browser_contract.extension_bridge.manifest_sha256",
        )
        _sha256(
            extension.get("tree_sha256"),
            field="browser_contract.extension_bridge.tree_sha256",
        )
    elif extension is not None:
        raise ReleaseValidationError(
            "Browser extension binding requires browser-extension-bridge capability"
        )

    artifacts = manifest.get("artifacts")
    if not isinstance(artifacts, dict):
        raise ReleaseValidationError("artifacts must be an object")
    required_artifacts = {"sbom", "provenance", "licenses"}
    if not required_artifacts.issubset(artifacts):
        raise ReleaseValidationError(
            f"required release artifacts are missing: {sorted(required_artifacts - artifacts.keys())}"
        )

    full_components = _selected_components(components, FULL_PROFILE)
    if (
        "model-facing-documentation" in capabilities.get("supported", [])
        and "documentation" not in full_components
    ):
        raise ReleaseValidationError(
            "model-facing-documentation support requires documentation in the full profile"
        )
    return components


def _release_scope_compliance_enabled(manifest: Mapping[str, Any]) -> bool:
    capabilities = manifest.get("capabilities")
    return isinstance(capabilities, dict) and "release-wide-compliance" in capabilities.get(
        "supported", []
    )


def _verify_release_scope_compliance(
    root: Path,
    manifest: Mapping[str, Any],
    *,
    selected_components: Sequence[str],
) -> None:
    """Verify the expanded compliance claim against exact selected release bytes."""
    artifacts = cast(dict[str, Any], manifest["artifacts"])
    loaded: dict[str, dict[str, Any]] = {}
    for name in ("sbom", "provenance", "licenses"):
        artifact = cast(dict[str, Any], artifacts[name])
        path = _safe_join(
            root,
            _relative_contract_path(artifact.get("path"), field=f"artifacts.{name}.path"),
        )
        loaded[name] = _load_object(path)

    provenance = loaded["provenance"]
    licenses = loaded["licenses"]
    sbom = loaded["sbom"]
    for name, document in loaded.items():
        if document.get("scope") != "complete-sky-cua-release" and not (
            name == "sbom"
            and isinstance(document.get("metadata"), dict)
            and any(
                isinstance(item, dict)
                and item.get("name") == "sky-cua:scope"
                and item.get("value") == "complete-sky-cua-release"
                for item in cast(dict[str, Any], document["metadata"]).get("properties", [])
            )
        ):
            raise ReleaseValidationError(f"{name} does not declare complete-release scope")
    if provenance.get("schema_version") != 2 or provenance.get("absolute_checkout_paths") != {
        "embedded_native_build_debug_metadata": True,
        "runtime_path_dependencies": False,
    }:
        raise ReleaseValidationError("release provenance path/scope claim is invalid")
    if provenance.get("producer_commit") != cast(dict[str, Any], manifest["producer"]).get(
        "commit"
    ):
        raise ReleaseValidationError("release provenance producer commit mismatch")
    if licenses.get("schema_version") != 2:
        raise ReleaseValidationError("release license inventory schema must be 2")

    inventory = provenance.get("release_inventory")
    if not isinstance(inventory, list):
        raise ReleaseValidationError("release provenance inventory is missing")
    required_inventory = {
        "sky-cua-core",
        "sky-cua-client",
        "sky-cua-service",
        "sky-cua-chrome-host",
        "browser-extension-assets",
        "browser-js",
        "cua-node",
        "codex-compat",
        "installer",
        "installer-entrypoint",
    }
    expected_paths = {
        "sky-cua-core": "components/core-linux-x64",
        "sky-cua-client": "components/core-linux-x64/bin/runtimes/linux-x64/sky-cua-client",
        "sky-cua-service": "components/core-linux-x64/bin/runtimes/linux-x64/sky-cua-service",
        "sky-cua-chrome-host": (
            "components/core-linux-x64/bin/runtimes/linux-x64/sky-cua-chrome-host"
        ),
        "browser-extension-assets": "components/core-linux-x64/resources/chrome-extension",
        "browser-js": "components/browser-js",
        "cua-node": "components/cua-node-linux-x64-glibc",
        "codex-compat": "components/codex-compat",
        "installer": "components/installer",
        "installer-entrypoint": "install.py",
    }
    records: dict[str, dict[str, Any]] = {}
    for raw in inventory:
        if not isinstance(raw, dict):
            raise ReleaseValidationError("release provenance inventory record is invalid")
        name = _string(raw.get("name"), field="release provenance inventory name")
        if name in records:
            raise ReleaseValidationError(f"duplicate release provenance inventory name: {name}")
        records[name] = raw
    if set(records) != required_inventory:
        raise ReleaseValidationError(
            "release provenance inventory coverage mismatch: "
            f"missing={sorted(required_inventory - records.keys())}, "
            f"extra={sorted(records.keys() - required_inventory)}"
        )
    if any(records[name].get("path") != path for name, path in expected_paths.items()):
        raise ReleaseValidationError("release provenance inventory paths are not canonical")

    selected_prefixes = {f"components/{name}" for name in selected_components}
    for name, record in records.items():
        relative = _relative_contract_path(
            record.get("path"), field=f"release provenance inventory {name} path"
        )
        relative_text = relative.as_posix()
        owning_prefix = next(
            (
                prefix
                for prefix in selected_prefixes
                if relative_text == prefix or relative_text.startswith(prefix + "/")
            ),
            None,
        )
        if owning_prefix is None and relative_text != "install.py":
            continue
        path = _safe_join(root, relative)
        kind = record.get("kind")
        if kind == "file":
            if not path.is_file() or path.is_symlink():
                raise ReleaseValidationError(f"release provenance file is missing: {name}")
            if (
                record.get("sha256") != sha256_file(path)
                or record.get("size_bytes") != path.stat().st_size
            ):
                raise ReleaseValidationError(f"release provenance file binding mismatch: {name}")
        elif kind == "tree":
            digest = canonical_tree_digest(path)
            file_count = sum(1 for entry in digest.entries if entry.get("type") == "file")
            if (
                record.get("tree_sha256") != digest.sha256
                or record.get("size_bytes") != digest.size
                or record.get("file_count") != file_count
            ):
                raise ReleaseValidationError(f"release provenance tree binding mismatch: {name}")
        else:
            raise ReleaseValidationError(f"release provenance inventory kind is invalid: {name}")

    release_licenses = licenses.get("release_artifacts")
    if not isinstance(release_licenses, list):
        raise ReleaseValidationError("release artifact license inventory is missing")
    license_records = {
        item.get("name"): item for item in release_licenses if isinstance(item, dict)
    }
    if set(license_records) != required_inventory - {"browser-extension-assets"}:
        raise ReleaseValidationError("release artifact license coverage mismatch")
    mit_names = {
        "sky-cua-core",
        "sky-cua-client",
        "sky-cua-service",
        "sky-cua-chrome-host",
        "installer",
        "installer-entrypoint",
    }
    for name in mit_names:
        record = license_records[name]
        if (
            record.get("license") != "MIT"
            or record.get("path") != expected_paths[name]
            or not isinstance(record.get("license_files"), list)
            or not record["license_files"]
        ):
            raise ReleaseValidationError(f"MIT release license binding is invalid: {name}")
    for name in ("browser-js", "codex-compat"):
        record = license_records[name]
        if (
            record.get("license") != "LicenseRef-Heliasar-Proprietary"
            or record.get("path") != expected_paths[name]
            or not isinstance(record.get("license_files"), list)
            or not record["license_files"]
        ):
            raise ReleaseValidationError(f"Browser release license binding is invalid: {name}")
    if (
        license_records["cua-node"].get("path") != expected_paths["cua-node"]
        or license_records["cua-node"].get("license_inventory") != "packages"
    ):
        raise ReleaseValidationError("cua-node aggregate license inventory binding is invalid")
    for name in (*sorted(mit_names), "browser-js", "codex-compat"):
        record = license_records[name]
        license_files = cast(list[object], record["license_files"])
        license_hashes = record.get("license_file_sha256s")
        if (
            not all(isinstance(value, str) for value in license_files)
            or not isinstance(license_hashes, dict)
            or set(license_hashes) != set(cast(list[str], license_files))
        ):
            raise ReleaseValidationError(f"release license file hash set is invalid: {name}")
        for index, relative_raw in enumerate(license_files):
            relative = _relative_contract_path(
                relative_raw, field=f"release license {name} file {index}"
            )
            path = root / "components/compliance" / relative
            if (
                not path.is_file()
                or path.is_symlink()
                or license_hashes[relative.as_posix()] != sha256_file(path)
            ):
                raise ReleaseValidationError(f"release license file binding mismatch: {name}")
    bundled_assets = licenses.get("bundled_assets")
    if not isinstance(bundled_assets, list) or not any(
        isinstance(item, dict)
        and item.get("name") == "browser-extension-assets"
        and item.get("license") == "NOASSERTION"
        and isinstance(item.get("license_status"), str)
        for item in bundled_assets
    ):
        raise ReleaseValidationError("browser extension asset license status is missing")

    sbom_components = sbom.get("components")
    if not isinstance(sbom_components, list):
        raise ReleaseValidationError("release SBOM components are missing")
    sbom_pairs = {
        (item.get("name"), item.get("version"))
        for item in sbom_components
        if isinstance(item, dict)
    }
    if not all(any(pair[0] == name for pair in sbom_pairs) for name in required_inventory):
        raise ReleaseValidationError("release SBOM first-party/asset coverage mismatch")
    sbom_records = {
        item.get("name"): item
        for item in sbom_components
        if isinstance(item, dict) and item.get("name") in required_inventory
    }
    for name, provenance_record in records.items():
        sbom_record = sbom_records[name]
        digest = provenance_record.get("sha256", provenance_record.get("tree_sha256"))
        if {"alg": "SHA-256", "content": digest} not in sbom_record.get("hashes", []):
            raise ReleaseValidationError(f"release SBOM digest binding mismatch: {name}")
    packages = licenses.get("packages")
    if not isinstance(packages, list) or not all(
        isinstance(package, dict) and (package.get("name"), package.get("version")) in sbom_pairs
        for package in packages
    ):
        raise ReleaseValidationError("release SBOM does not cover the cua_node package inventory")
    package_inventory = provenance.get("node_package_inventory")
    licenses_artifact_path = _safe_join(
        root,
        _relative_contract_path(artifacts["licenses"].get("path"), field="artifacts.licenses.path"),
    )
    if not isinstance(package_inventory, dict) or (
        package_inventory.get("path") != artifacts["licenses"].get("path")
        or package_inventory.get("package_count") != len(packages)
        or package_inventory.get("sha256") != sha256_file(licenses_artifact_path)
    ):
        raise ReleaseValidationError("release provenance package inventory binding mismatch")

    if "core-linux-x64" in selected_components:
        producer_commit = cast(dict[str, Any], manifest["producer"])["commit"]
        core = root / "components/core-linux-x64"
        buildstamps = sorted(core.rglob("*.buildstamp.json"), key=lambda path: path.as_posix())
        if not buildstamps:
            raise ReleaseValidationError("release core contains no buildstamp")
        for path in buildstamps:
            stamp = _load_object(path)
            if (
                "repo_root" in stamp
                or "deployed_at_ms" in stamp
                or stamp.get("git_sha") != producer_commit
                or stamp.get("git_dirty") is not False
                or stamp.get("source") != {"kind": "git-archive", "commit": producer_commit}
            ):
                raise ReleaseValidationError(
                    f"release core buildstamp leaks producer state or is unbound: {path}"
                )

    scan_roots = [root / "components" / name for name in selected_components]
    scan_roots.extend((root / "compliance", root / "locks", root / "install.py"))
    for scan_root in scan_roots:
        if not scan_root.exists():
            continue
        paths: Iterable[Path] = [scan_root] if scan_root.is_file() else scan_root.rglob("*")
        for path in paths:
            if not path.is_file() or path.is_symlink():
                continue
            blob = path.read_bytes()
            forbidden = any(pattern.search(blob) for pattern in FORBIDDEN_CHECKOUT_PATH_PATTERNS)
            checkout_shaped_text = b"\x00" not in blob and any(
                pattern.search(blob) for pattern in CHECKOUT_SHAPED_PATH_PATTERNS
            )
            if forbidden or checkout_shaped_text:
                raise ReleaseValidationError(
                    "release contains an absolute checkout-shaped path: "
                    f"{path.relative_to(root).as_posix()}"
                )


def verify_release_root(
    root: Path,
    *,
    profile: str = FULL_PROFILE,
    expected_manifest_sha256: str | None = None,
    enforce_profile_shape: bool = False,
) -> VerifiedRelease:
    if root.is_symlink() or not root.is_dir():
        raise ReleaseValidationError(f"release root must be a real directory: {root}")
    manifest_path = root / RELEASE_MANIFEST
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise ReleaseValidationError(f"release manifest missing: {manifest_path}")
    manifest_hash = sha256_file(manifest_path)
    if expected_manifest_sha256 is not None and manifest_hash != expected_manifest_sha256:
        raise ReleaseValidationError(
            f"release manifest hash mismatch: expected {expected_manifest_sha256}, got {manifest_hash}"
        )
    manifest = _load_object(manifest_path)
    components = validate_manifest_shape(manifest)
    release_id = cast(str, manifest["release_id"])
    derived_release_id = content_addressed_release_id(manifest)
    if release_id != derived_release_id:
        raise ReleaseValidationError(
            f"release_id mismatch: expected {derived_release_id}, got {release_id}"
        )
    selected = _selected_components(components, profile)

    seen_paths: set[str] = set()
    seen_archives: set[str] = set()
    for name in selected:
        component = components[name]
        relative = _relative_contract_path(component.get("path"), field=f"component {name} path")
        if relative.as_posix() in seen_paths:
            raise ReleaseValidationError(f"multiple selected components use path {relative}")
        seen_paths.add(relative.as_posix())
        archive_relative = _relative_contract_path(
            component.get("archive"), field=f"component {name} archive"
        )
        if archive_relative.as_posix() in seen_archives:
            raise ReleaseValidationError(
                f"multiple selected components use archive {archive_relative}"
            )
        seen_archives.add(archive_relative.as_posix())
        archive_path = _safe_join(root, archive_relative)
        if not archive_path.is_file() or archive_path.is_symlink():
            raise ReleaseValidationError(f"component {name} archive is missing: {archive_relative}")
        archive_hash = sha256_file(archive_path)
        expected_archive_hash = _sha256(component.get("sha256"), field=f"component {name} sha256")
        if archive_hash != expected_archive_hash:
            raise ReleaseValidationError(
                f"component {name} archive hash mismatch: "
                f"expected {expected_archive_hash}, got {archive_hash}"
            )
        if archive_path.stat().st_size != component.get("size"):
            raise ReleaseValidationError(f"component {name} archive size mismatch")
        digest = canonical_tree_digest(_safe_join(root, relative))
        expected_tree = _sha256(component.get("tree_sha256"), field=f"component {name} tree_sha256")
        if digest.sha256 != expected_tree:
            raise ReleaseValidationError(
                f"component {name} tree hash mismatch: expected {expected_tree}, got {digest.sha256}"
            )
        if digest.size != component.get("tree_size"):
            raise ReleaseValidationError(f"component {name} tree size mismatch")
        archived_digest = archive_tree_digest(
            archive_path,
            root_name=name,
            maximum_tree_size=digest.size,
        )
        if archived_digest.sha256 != digest.sha256 or archived_digest.size != digest.size:
            raise ReleaseValidationError(
                f"component {name} archive contents do not match the expanded tree"
            )

    for section_name in ("locks", "artifacts"):
        section = manifest.get(section_name)
        if not isinstance(section, dict) or not section:
            raise ReleaseValidationError(f"{section_name} must be a non-empty object")
        for name, artifact in section.items():
            _verify_hashed_artifact(root, artifact, field=f"{section_name}.{name}")

    if _release_scope_compliance_enabled(manifest):
        _verify_release_scope_compliance(
            root,
            manifest,
            selected_components=selected,
        )

    if manifest.get("documentation") is not None:
        documentation = cast(dict[str, Any], manifest["documentation"])
        if documentation.get("component") in selected:
            for name in (
                "api_inventory",
                "capability_inventory",
                "example_inventory",
                "routing_inventory",
            ):
                _verify_hashed_artifact(root, documentation[name], field=f"documentation.{name}")

    browser_contract = cast(dict[str, Any], manifest["browser_contract"])
    canonical_browser = cast(dict[str, Any], browser_contract["canonical_browser"])
    if canonical_browser.get("component") in selected:
        _verify_hashed_artifact(root, canonical_browser, field="browser_contract.canonical_browser")
    projections = cast(list[object], browser_contract["compatibility_projections"])
    for index, projection in enumerate(projections):
        if isinstance(projection, dict) and projection.get("component") in selected:
            _verify_hashed_artifact(
                root,
                projection,
                field=f"browser_contract.compatibility_projections[{index}]",
            )
    extension = browser_contract.get("extension_bridge")
    if isinstance(extension, dict) and extension.get("component") in selected:
        extension_root = _safe_join(
            root,
            _relative_contract_path(
                extension.get("path"), field="browser_contract.extension_bridge.path"
            ),
        )
        manifest_path = extension_root / "manifest.json"
        if (
            extension_root.is_symlink()
            or not extension_root.is_dir()
            or manifest_path.is_symlink()
            or not manifest_path.is_file()
        ):
            raise ReleaseValidationError("selected Browser extension is missing")
        if sha256_file(manifest_path) != extension.get("manifest_sha256"):
            raise ReleaseValidationError("Browser extension manifest hash mismatch")
        if canonical_tree_digest(extension_root).sha256 != extension.get("tree_sha256"):
            raise ReleaseValidationError("Browser extension tree hash mismatch")
        extension_manifest = _load_object(manifest_path)
        if (
            extension_manifest.get("version") != extension.get("version")
            or not isinstance(extension_manifest.get("key"), str)
            or _chrome_extension_id(extension_manifest["key"]) != extension.get("extension_id")
            or "nativeMessaging" not in extension_manifest.get("permissions", [])
        ):
            raise ReleaseValidationError("Browser extension identity or capability mismatch")

    omitted_names = set(components) - set(selected)
    if enforce_profile_shape:
        for name in sorted(omitted_names):
            component_path = _safe_join(
                root,
                _relative_contract_path(
                    components[name].get("path"), field=f"component {name} path"
                ),
            )
            archive_path = _safe_join(
                root,
                _relative_contract_path(
                    components[name].get("archive"), field=f"component {name} archive"
                ),
            )
            if component_path.exists() or component_path.is_symlink():
                raise ReleaseValidationError(
                    f"omitted component is present in {profile} generation: {name}"
                )
            if archive_path.exists() or archive_path.is_symlink():
                raise ReleaseValidationError(
                    f"omitted component archive is present in {profile} generation: {name}"
                )
    _verify_checksums(
        root,
        omitted_component_paths=tuple(
            _relative_contract_path(components[name].get("path"), field=f"component {name} path")
            for name in sorted(omitted_names)
        ),
        omitted_archives=tuple(
            _relative_contract_path(
                components[name].get("archive"), field=f"component {name} archive"
            )
            for name in sorted(omitted_names)
        ),
    )

    return VerifiedRelease(
        root=root,
        release_id=release_id,
        manifest_sha256=manifest_hash,
        profile=profile,
        component_names=selected,
    )


def write_json_durably(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    remove_path(temp)
    try:
        with temp.open("wb") as handle:
            handle.write(canonical_json_bytes(value) + b"\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp, path)
        _fsync_directory(path.parent)
    finally:
        remove_path(temp)


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _fsync_tree(root: Path) -> None:
    """Durably flush a verified tree before its committed-name journal is written."""
    if root.is_symlink() or not root.is_dir():
        raise ReleaseValidationError(f"release tree must be a real directory: {root}")
    directories: list[Path] = [root]
    for path in sorted(root.rglob("*"), key=lambda item: item.as_posix()):
        if path.is_symlink():
            raise ReleaseValidationError(f"release tree symlink is unsupported: {path}")
        if path.is_dir():
            directories.append(path)
        elif path.is_file():
            descriptor = os.open(path, os.O_RDONLY)
            try:
                os.fsync(descriptor)
            finally:
                os.close(descriptor)
        else:
            raise ReleaseValidationError(f"unsupported release tree entry: {path}")
    for directory in reversed(directories):
        _fsync_directory(directory)


def _atomic_symlink(root: Path, name: str, release_id: str | None) -> None:
    destination = root / name
    if release_id is None:
        if destination.exists() or destination.is_symlink():
            destination.unlink()
            _fsync_directory(root)
        return
    temp = root / f".{name}.tmp-{os.getpid()}"
    remove_path(temp)
    temp.symlink_to(Path("releases") / release_id)
    os.replace(temp, destination)
    _fsync_directory(root)


def _link_release_id(root: Path, name: str) -> str | None:
    link = root / name
    if not link.is_symlink():
        return None
    target = Path(os.readlink(link))
    if target.parent != Path("releases") or RELEASE_ID_PATTERN.fullmatch(target.name) is None:
        raise InstallTransactionError(f"{name} points outside the generation store")
    return target.name


def _validate_journal(journal: Mapping[str, Any]) -> None:
    if journal.get("schema_version") != 1:
        raise InstallTransactionError("journal schema_version must be 1")
    operation = _string(journal.get("operation"), field="journal operation")
    phases = {
        "install": {"staged", "generation_committed", "current_switched", "previous_switched"},
        "rollback": {"prepared", "current_switched"},
    }
    if operation not in phases:
        raise InstallTransactionError(f"unsupported journal operation: {operation}")
    phase = _string(journal.get("phase"), field="journal phase")
    if phase not in phases[operation]:
        raise InstallTransactionError(f"unsupported {operation} journal phase: {phase}")
    target = _string(journal.get("target_release_id"), field="journal target_release_id")
    if RELEASE_ID_PATTERN.fullmatch(target) is None:
        raise InstallTransactionError("journal target_release_id must be a release id")
    _sha256(journal.get("target_manifest_sha256"), field="journal target_manifest_sha256")
    profile = _string(journal.get("profile"), field="journal profile")
    if profile not in INSTALL_PROFILES:
        raise InstallTransactionError(f"unsupported journal profile: {profile}")
    previous = journal.get("previous_release_id")
    if previous is not None and (
        not isinstance(previous, str) or RELEASE_ID_PATTERN.fullmatch(previous) is None
    ):
        raise InstallTransactionError("journal previous_release_id must be null or a release id")
    staging = journal.get("staging_name")
    if operation == "rollback" and staging is not None:
        raise InstallTransactionError("rollback journal staging_name must be null")
    if staging is not None:
        expected_prefix = f".{target}.staging-"
        if (
            not isinstance(staging, str)
            or not staging.startswith(expected_prefix)
            or not staging.removeprefix(expected_prefix).isdigit()
            or Path(staging).name != staging
        ):
            raise InstallTransactionError("journal staging_name is invalid")


def _copy_release_for_profile(candidate: Path, staging: Path, verified: VerifiedRelease) -> None:
    if verified.profile == FULL_PROFILE:
        shutil.copytree(candidate, staging, symlinks=True)
        return

    manifest = _load_object(candidate / RELEASE_MANIFEST)
    components = _component_map(manifest)
    staging.mkdir(parents=True)
    for source in sorted(candidate.iterdir(), key=lambda path: path.name):
        if source.name in {"components", "archives", INSTALLATION_STATE}:
            continue
        if source.is_symlink():
            raise ReleaseValidationError(f"release-root symlink is unsupported: {source.name}")
        destination = staging / source.name
        if source.is_dir():
            shutil.copytree(source, destination, symlinks=True)
        elif source.is_file():
            shutil.copy2(source, destination)
        else:
            raise ReleaseValidationError(f"unsupported release-root entry: {source.name}")

    for name in verified.component_names:
        component = components[name]
        component_relative = _relative_contract_path(
            component.get("path"), field=f"component {name} path"
        )
        archive_relative = _relative_contract_path(
            component.get("archive"), field=f"component {name} archive"
        )
        component_source = _safe_join(candidate, component_relative)
        component_destination = staging.joinpath(*component_relative.parts)
        component_destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(component_source, component_destination, symlinks=True)
        archive_source = _safe_join(candidate, archive_relative)
        archive_destination = staging.joinpath(*archive_relative.parts)
        archive_destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(archive_source, archive_destination)


class GenerationStore:
    """Install complete releases and atomically switch the active generation."""

    def __init__(self, root: Path):
        self.root = root
        self.releases = root / "releases"
        self.journal = root / JOURNAL_FILE

    def initialize(self) -> None:
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.releases.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.root.chmod(0o700)
        self.releases.chmod(0o700)

    def current_release_id(self) -> str | None:
        return _link_release_id(self.root, "current")

    def previous_release_id(self) -> str | None:
        return _link_release_id(self.root, "previous")

    @contextmanager
    def _transaction_lock(self) -> Iterator[None]:
        self.initialize()
        lock_path = self.root / ".generation.lock"
        with lock_path.open("a+b") as handle:
            lock_path.chmod(0o600)
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)

    def install(
        self,
        candidate: Path,
        *,
        expected_manifest_sha256: str | None = None,
        profile: str = FULL_PROFILE,
        failpoint: Failpoint | None = None,
    ) -> VerifiedRelease:
        with self._transaction_lock():
            return self._install(
                candidate,
                expected_manifest_sha256=expected_manifest_sha256,
                profile=profile,
                failpoint=failpoint,
            )

    def _install(
        self,
        candidate: Path,
        *,
        expected_manifest_sha256: str | None,
        profile: str,
        failpoint: Failpoint | None,
    ) -> VerifiedRelease:
        self.initialize()
        if self.journal.exists():
            self._recover(failpoint=failpoint)
        verified = verify_release_root(
            candidate,
            profile=profile,
            expected_manifest_sha256=expected_manifest_sha256,
        )
        current = self.current_release_id()
        if current is not None:
            self._verify_installed_generation(current)
        if current != verified.release_id:
            prior = current
        else:
            prior = self.previous_release_id()
            if prior is not None:
                self._verify_installed_generation(prior)
        final = self.releases / verified.release_id
        staging = self.releases / f".{verified.release_id}.staging-{os.getpid()}"

        if final.is_symlink():
            raise InstallTransactionError(
                f"generation path must not be a symlink: {verified.release_id}"
            )
        if final.exists():
            if not final.is_dir():
                raise InstallTransactionError(
                    f"generation path must be a directory: {verified.release_id}"
                )
            installed = self._verify_installed_generation(verified.release_id)
            if installed.release_id != verified.release_id:
                raise InstallTransactionError("existing generation has a different release id")
            if installed.manifest_sha256 != verified.manifest_sha256:
                raise InstallTransactionError("existing generation has a different manifest")
            if installed.profile != profile:
                raise InstallTransactionError(
                    "an installed release id cannot change profile in place: "
                    f"installed={installed.profile}, requested={profile}"
                )
        else:
            remove_path(staging)
            _copy_release_for_profile(candidate, staging, verified)
            (staging / INSTALLATION_STATE).unlink(missing_ok=True)
            self._write_installation_state(staging, verified, profile)
            verify_release_root(
                staging,
                profile=profile,
                expected_manifest_sha256=verified.manifest_sha256,
                enforce_profile_shape=True,
            )
            _fsync_tree(staging)

        journal = {
            "schema_version": 1,
            "operation": "install",
            "phase": "staged",
            "target_release_id": verified.release_id,
            "target_manifest_sha256": verified.manifest_sha256,
            "profile": profile,
            "previous_release_id": prior,
            "staging_name": staging.name if staging.exists() else None,
        }
        write_json_durably(self.journal, journal)
        self._hit(failpoint, "after_staged_journal")

        if not final.exists():
            os.replace(staging, final)
            _fsync_directory(self.releases)
        journal["phase"] = "generation_committed"
        journal["staging_name"] = None
        write_json_durably(self.journal, journal)
        self._hit(failpoint, "after_generation_commit")

        _atomic_symlink(self.root, "current", verified.release_id)
        journal["phase"] = "current_switched"
        write_json_durably(self.journal, journal)
        self._hit(failpoint, "after_current_switch")

        _atomic_symlink(self.root, "previous", prior)
        journal["phase"] = "previous_switched"
        write_json_durably(self.journal, journal)
        self._hit(failpoint, "after_previous_switch")

        self._prune_generations({verified.release_id, prior} - {None})
        self.journal.unlink(missing_ok=True)
        _fsync_directory(self.root)
        return verify_release_root(
            final,
            profile=profile,
            expected_manifest_sha256=verified.manifest_sha256,
            enforce_profile_shape=True,
        )

    def recover(self, *, failpoint: Failpoint | None = None) -> VerifiedRelease | None:
        with self._transaction_lock():
            return self._recover(failpoint=failpoint)

    def _recover(self, *, failpoint: Failpoint | None = None) -> VerifiedRelease | None:
        self.initialize()
        if not self.journal.exists():
            return None
        journal = _load_object(self.journal)
        _validate_journal(journal)
        operation = _string(journal.get("operation"), field="journal operation")
        if operation == "rollback":
            return self._recover_rollback(journal, failpoint=failpoint)
        if operation != "install":
            raise InstallTransactionError(f"unsupported journal operation: {operation}")
        target = _string(journal.get("target_release_id"), field="journal target_release_id")
        expected = _sha256(
            journal.get("target_manifest_sha256"), field="journal target_manifest_sha256"
        )
        profile = _string(journal.get("profile"), field="journal profile")
        previous_raw = journal.get("previous_release_id")
        if previous_raw is not None and not isinstance(previous_raw, str):
            raise InstallTransactionError("journal previous_release_id must be string or null")
        previous = cast(str | None, previous_raw)
        staging_raw = journal.get("staging_name")
        if staging_raw is not None and not isinstance(staging_raw, str):
            raise InstallTransactionError("journal staging_name must be string or null")
        staging = self.releases / cast(str, staging_raw) if staging_raw else None
        final = self.releases / target

        if final.is_symlink():
            raise InstallTransactionError(f"generation path must not be a symlink: {target}")
        if not final.exists():
            if staging is None or not staging.exists():
                raise InstallTransactionError("journal target has neither staging nor generation")
            verify_release_root(
                staging,
                profile=profile,
                expected_manifest_sha256=expected,
                enforce_profile_shape=True,
            )
            _fsync_tree(staging)
            os.replace(staging, final)
            _fsync_directory(self.releases)
        elif not final.is_dir():
            raise InstallTransactionError(f"generation path must be a directory: {target}")
        verified = verify_release_root(
            final,
            profile=profile,
            expected_manifest_sha256=expected,
            enforce_profile_shape=True,
        )
        self._hit(failpoint, "recovery_before_current_switch")
        _atomic_symlink(self.root, "current", target)
        _atomic_symlink(self.root, "previous", previous if previous != target else None)
        self._prune_generations({target, previous} - {None})
        self.journal.unlink(missing_ok=True)
        _fsync_directory(self.root)
        return verified

    def rollback(self, *, failpoint: Failpoint | None = None) -> VerifiedRelease:
        with self._transaction_lock():
            return self._rollback(failpoint=failpoint)

    def deactivate_initial_activation(self, release_id: str) -> VerifiedRelease:
        """Remove ``current`` after a failed first-install consumer cutover.

        The verified generation is retained for diagnosis/retry; only the
        activation pointer is removed. This operation is valid solely when no
        prior generation exists, so it cannot discard a rollback target.
        """
        with self._transaction_lock():
            if self.journal.exists():
                self._recover(failpoint=None)
            if self.current_release_id() != release_id:
                raise InstallTransactionError(
                    "initial deactivation target is not the current generation"
                )
            if self.previous_release_id() is not None:
                raise InstallTransactionError(
                    "initial deactivation requires no previous generation"
                )
            verified = self._verify_installed_generation(release_id)
            _atomic_symlink(self.root, "current", None)
            return verified

    def _rollback(self, *, failpoint: Failpoint | None = None) -> VerifiedRelease:
        self.initialize()
        if self.journal.exists():
            pending = _load_object(self.journal)
            if pending.get("operation") == "rollback":
                recovered = self._recover(failpoint=failpoint)
                if recovered is None:  # pragma: no cover - journal existence is checked above
                    raise InstallTransactionError("rollback journal disappeared during recovery")
                return recovered
            self._recover(failpoint=failpoint)
        current = self.current_release_id()
        previous = self.previous_release_id()
        if current is None or previous is None:
            raise InstallTransactionError("rollback requires current and previous generations")
        self._verify_installed_generation(current)
        verified = self._verify_installed_generation(previous)
        journal = {
            "schema_version": 1,
            "operation": "rollback",
            "phase": "prepared",
            "target_release_id": previous,
            "target_manifest_sha256": verified.manifest_sha256,
            "profile": verified.profile,
            "previous_release_id": current,
            "staging_name": None,
        }
        write_json_durably(self.journal, journal)
        self._hit(failpoint, "rollback_after_prepared_journal")
        _atomic_symlink(self.root, "current", previous)
        journal["phase"] = "current_switched"
        write_json_durably(self.journal, journal)
        self._hit(failpoint, "rollback_after_current_switch")
        _atomic_symlink(self.root, "previous", current)
        self.journal.unlink(missing_ok=True)
        _fsync_directory(self.root)
        return verified

    def _recover_rollback(
        self, journal: Mapping[str, Any], *, failpoint: Failpoint | None
    ) -> VerifiedRelease:
        target = _string(
            journal.get("target_release_id"), field="rollback journal target_release_id"
        )
        prior = _string(
            journal.get("previous_release_id"), field="rollback journal previous_release_id"
        )
        verified = self._verify_installed_generation(target)
        self._verify_installed_generation(prior)
        expected = _sha256(
            journal.get("target_manifest_sha256"),
            field="rollback journal target_manifest_sha256",
        )
        if verified.manifest_sha256 != expected:
            raise InstallTransactionError("rollback journal target manifest hash mismatch")
        self._hit(failpoint, "rollback_recovery_before_current_switch")
        _atomic_symlink(self.root, "current", target)
        _atomic_symlink(self.root, "previous", prior)
        self.journal.unlink(missing_ok=True)
        _fsync_directory(self.root)
        return verified

    def _verify_installed_generation(self, release_id: str) -> VerifiedRelease:
        generation = self.releases / release_id
        if generation.is_symlink() or not generation.is_dir():
            raise InstallTransactionError(
                f"installed generation is missing or invalid: {release_id}"
            )
        state_path = generation / INSTALLATION_STATE
        if state_path.is_symlink() or not state_path.is_file():
            raise InstallTransactionError(
                f"installed generation is missing installation state: {release_id}"
            )
        installation = _load_object(state_path)
        if installation.get("schema_version") != 1:
            raise InstallTransactionError(f"installation state schema mismatch for {release_id}")
        if installation.get("release_id") != release_id:
            raise InstallTransactionError(
                f"installation state release id mismatch for {release_id}"
            )
        profile = _string(installation.get("profile"), field="installation profile")
        expected = _sha256(
            installation.get("manifest_sha256"), field="installation manifest_sha256"
        )
        verified = verify_release_root(
            generation,
            profile=profile,
            expected_manifest_sha256=expected,
            enforce_profile_shape=True,
        )
        components = installation.get("components")
        if components != list(verified.component_names):
            raise InstallTransactionError(
                f"installation state component set mismatch for {release_id}"
            )
        return verified

    def _write_installation_state(
        self, generation: Path, verified: VerifiedRelease, profile: str
    ) -> None:
        write_json_durably(
            generation / INSTALLATION_STATE,
            {
                "schema_version": 1,
                "release_id": verified.release_id,
                "manifest_sha256": verified.manifest_sha256,
                "profile": profile,
                "components": list(verified.component_names),
            },
        )

    def _prune_generations(self, keep: Iterable[str]) -> None:
        keep_set = set(keep)
        for path in self.releases.iterdir():
            if path.name.startswith(".") or (path.is_dir() and path.name not in keep_set):
                remove_path(path)

    @staticmethod
    def _hit(failpoint: Failpoint | None, name: str) -> None:
        if failpoint is not None:
            failpoint(name)


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Verify or transactionally install an immutable sky-cua release."
    )
    subcommands = parser.add_subparsers(dest="command", required=True)

    verify = subcommands.add_parser("verify", help="Verify a release tree without mutation.")
    verify.add_argument("release_root", type=Path)
    verify.add_argument("--profile", choices=sorted(INSTALL_PROFILES), default=FULL_PROFILE)
    verify.add_argument("--manifest-sha256")

    install = subcommands.add_parser("install", help="Install and promote a complete generation.")
    install.add_argument("release_root", type=Path)
    install.add_argument("--store-root", type=Path, default=Path.home() / ".local/share/sky-cua")
    install.add_argument("--profile", choices=sorted(INSTALL_PROFILES), default=FULL_PROFILE)
    install.add_argument("--manifest-sha256")

    recover = subcommands.add_parser("recover", help="Finish an interrupted promotion journal.")
    recover.add_argument("--store-root", type=Path, default=Path.home() / ".local/share/sky-cua")

    rollback = subcommands.add_parser(
        "rollback", help="Atomically activate the retained prior generation."
    )
    rollback.add_argument("--store-root", type=Path, default=Path.home() / ".local/share/sky-cua")
    return parser


def _verified_payload(verified: VerifiedRelease) -> dict[str, object]:
    return {
        "release_id": verified.release_id,
        "manifest_sha256": verified.manifest_sha256,
        "profile": verified.profile,
        "components": list(verified.component_names),
        "root": str(verified.root.resolve()),
    }


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    try:
        if args.command == "verify":
            verified = verify_release_root(
                args.release_root.expanduser().resolve(),
                profile=args.profile,
                expected_manifest_sha256=args.manifest_sha256,
            )
        else:
            store = GenerationStore(args.store_root.expanduser().resolve())
            if args.command == "install":
                verified = store.install(
                    args.release_root.expanduser().resolve(),
                    profile=args.profile,
                    expected_manifest_sha256=args.manifest_sha256,
                )
            elif args.command == "recover":
                recovered = store.recover()
                if recovered is None:
                    print(json.dumps({"status": "clean", "journal": None}, sort_keys=True))
                    return 0
                verified = recovered
            elif args.command == "rollback":
                verified = store.rollback()
            else:  # pragma: no cover - argparse owns command exhaustiveness
                raise AssertionError(args.command)
    except (ReleaseValidationError, InstallTransactionError, OSError) as error:
        print(json.dumps({"status": "error", "error": str(error)}, sort_keys=True))
        return 1
    print(json.dumps({"status": "ok", **_verified_payload(verified)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
