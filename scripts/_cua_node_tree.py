"""Deterministic file-tree hashing, copying, and normalization for cua_node assembly."""

from __future__ import annotations

import hashlib
import json
import os
import shutil
from collections.abc import Iterable
from pathlib import Path
from typing import cast

from _artifact_helpers import (
    CHECKOUT_SHAPED_PATH_PATTERNS,
    sha256_file,
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
