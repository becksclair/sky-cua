"""Content-addressed migration seed cache for cua_node assembly."""

from __future__ import annotations

import hashlib
import json
import os
import re
import uuid
from pathlib import Path
from typing import cast

from _artifact_helpers import (
    write_json_durably,
)
from _cua_node_inputs import (
    MIGRATION_COMMIT,
    MIGRATION_SEED_FILE_COUNT,
    MIGRATION_SEED_SHA256,
    MIGRATION_SEED_SIZE_BYTES,
    NODE_VERSION,
    TARGET,
)
from _cua_node_tree import (
    AssemblyError,
    _canonical_json,
    _copy_tree,
    _file_manifest,
    _fsync_tree,
    _normalize_tree,
    _tree_hash,
)
from _plugin_bundle import remove_path


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
