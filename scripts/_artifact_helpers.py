"""Small deterministic artifact assembly helpers."""

from __future__ import annotations

import hashlib
import json
import os
import re
from pathlib import Path

from _plugin_bundle import remove_path

CHECKOUT_SHAPED_PATH_PATTERNS = (
    re.compile(rb"/(?:home|Users)/[^/\x00\s]+/(?:projects?|src|source|code|workspace|repos?)/"),
    re.compile(
        rb"[A-Za-z]:\\Users\\[^\\\x00\s]+\\(?:projects?|src|source|code|workspace|repos?)\\",
        re.IGNORECASE,
    ),
)


def canonical_json_bytes(value: object) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


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
