#!/usr/bin/env python3
"""Validate and smoke-test standalone Sky CUA release archives."""

from __future__ import annotations

import argparse
import hashlib
import json
import posixpath
import re
import subprocess
import sys
import tarfile
import tempfile
from collections.abc import Sequence
from pathlib import Path, PurePosixPath
from typing import Any, cast

from _portable_elf import LINUX_X64_RUNTIME_MEMBERS, validate_x86_64_v3_elf

ARCHIVE_NAME = "sky-cua-linux-x64-glibc.tar.gz"
CHECKSUM_NAME = f"{ARCHIVE_NAME}.sha256"
PAYLOAD_ROOT = "sky-cua-linux-x64-glibc"
PRODUCT = "sky-cua"
TARGET = "linux-x64-glibc"
DEPLOY_WORKFLOW = "deploy-saga.yml"
TAG_PATTERN = re.compile(r"standalone-v(?P<version>[0-9]+(?:\.[0-9]+){2}(?:[-+][0-9A-Za-z.-]+)?)")
DIGEST_PATTERN = re.compile(r"[0-9a-f]{64}")
REQUIRED_MEMBERS = (
    "RELEASE.json",
    "install.py",
    "scripts/standalone_release.py",
    "scripts/_standalone_release_command.py",
    "bin/sky-cua-client",
    "bin/sky-cua-service",
    "bin/sky-cua-overlay-host",
    "bin/node",
    "bin/node_repl",
    "browser/browser-client.mjs",
    "browser/extension/manifest.json",
    "browser/native-host/sky-cua-chrome-host",
    "codex/openai-bundled/.agents/plugins/marketplace.json",
    "codex/openai-bundled/plugins/computer-use/.codex-plugin/plugin.json",
    "codex/openai-bundled/plugins/computer-use/assets/app-icon.png",
    "codex/openai-bundled/plugins/browser/.codex-plugin/plugin.json",
    "codex/openai-bundled/plugins/browser/assets/browser.png",
    "codex/openai-bundled/plugins/browser/assets/composer-icon.png",
    "skills/computer-use/SKILL.md",
    "skills/browser-use/SKILL.md",
)


def parse_tag(tag: str) -> str:
    """Return the release version encoded by a canonical standalone tag."""
    match = TAG_PATTERN.fullmatch(tag)
    if match is None:
        raise ValueError(f"invalid standalone release tag: {tag!r}")
    return match.group("version")


def validate_digest(digest: str) -> str:
    """Return a canonical SHA-256 digest or reject it."""
    if DIGEST_PATTERN.fullmatch(digest) is None:
        raise ValueError("archive SHA-256 must be 64 lowercase hexadecimal characters")
    return digest


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _safe_member_path(name: str) -> PurePosixPath:
    if not name or name.startswith("/"):
        raise ValueError(f"archive contains an unsafe absolute or empty path: {name!r}")
    path = PurePosixPath(name)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        raise ValueError(f"archive contains an unsafe path: {name!r}")
    if path.parts[0] != PAYLOAD_ROOT:
        raise ValueError(f"archive member is outside {PAYLOAD_ROOT!r}: {name!r}")
    return path


def _validate_link(member: tarfile.TarInfo, member_path: PurePosixPath) -> None:
    link = member.linkname
    if not link or link.startswith("/"):
        raise ValueError(f"archive link has an unsafe target: {member.name!r} -> {link!r}")
    if member.issym():
        resolved = posixpath.normpath(posixpath.join(posixpath.dirname(member.name), link))
    else:
        resolved = posixpath.normpath(link)
    resolved_path = PurePosixPath(resolved)
    if (
        resolved_path.is_absolute()
        or ".." in resolved_path.parts
        or not resolved_path.parts
        or resolved_path.parts[0] != PAYLOAD_ROOT
    ):
        raise ValueError(f"archive link escapes its payload: {member_path} -> {link!r}")


def validate_archive(archive_path: Path, *, tag: str | None = None) -> dict[str, Any]:
    """Validate archive paths, payload shape, and embedded release identity."""
    expected_version = parse_tag(tag) if tag is not None else None
    if not archive_path.is_file():
        raise FileNotFoundError(f"standalone archive is missing: {archive_path}")

    members: dict[str, tarfile.TarInfo] = {}
    with tarfile.open(archive_path, "r:gz") as archive:
        for member in archive.getmembers():
            member_path = _safe_member_path(member.name)
            if member.name in members:
                raise ValueError(f"archive contains a duplicate member: {member.name!r}")
            if not (member.isfile() or member.isdir() or member.issym() or member.islnk()):
                raise ValueError(f"archive contains an unsupported member type: {member.name!r}")
            if member.issym() or member.islnk():
                _validate_link(member, member_path)
            members[member.name] = member

        missing = [
            relative
            for relative in REQUIRED_MEMBERS
            if f"{PAYLOAD_ROOT}/{relative}" not in members
            or not members[f"{PAYLOAD_ROOT}/{relative}"].isfile()
        ]
        if missing:
            raise ValueError(f"standalone archive is incomplete: {missing}")

        manifest_member = members[f"{PAYLOAD_ROOT}/RELEASE.json"]
        manifest_stream = archive.extractfile(manifest_member)
        if manifest_stream is None:
            raise ValueError("standalone RELEASE.json cannot be read")
        manifest = cast(dict[str, Any], json.load(manifest_stream))

    if manifest.get("schema_version") != 1:
        raise ValueError("standalone RELEASE.json has an unsupported schema")
    if manifest.get("product") != PRODUCT:
        raise ValueError(f"standalone product must be {PRODUCT!r}")
    if manifest.get("target") != TARGET:
        raise ValueError(f"standalone target must be {TARGET!r}")
    version = manifest.get("version")
    if not isinstance(version, str) or not version:
        raise ValueError("standalone RELEASE.json has no version")
    if expected_version is not None and version != expected_version:
        raise ValueError(
            f"tag version {expected_version!r} does not match archive version {version!r}"
        )
    return manifest


def validate_portable_runtime_archive(archive_path: Path) -> None:
    """Validate Linux Rust runtime members when present in a standalone archive."""
    with tempfile.TemporaryDirectory(prefix="sky-cua-portable-runtime-") as temporary:
        temporary_root = Path(temporary)
        with tarfile.open(archive_path, "r:gz") as archive:
            members = {member.name: member for member in archive.getmembers()}
            archive_members = {
                relative: members.get(f"{PAYLOAD_ROOT}/{relative.as_posix()}")
                for relative in LINUX_X64_RUNTIME_MEMBERS
            }
            present = {relative: member for relative, member in archive_members.items() if member}
            if not present:
                return
            missing = [
                relative.as_posix() for relative, member in archive_members.items() if not member
            ]
            if missing:
                raise ValueError(
                    f"standalone archive is missing portable runtime members: {missing}"
                )

            for relative, member in present.items():
                stream = archive.extractfile(member)
                if stream is None:
                    raise ValueError(f"standalone runtime member cannot be read: {relative}")
                path = temporary_root / relative.name
                path.write_bytes(stream.read())
                validate_x86_64_v3_elf(path, label=relative.as_posix())


def smoke_install(archive_path: Path, *, tag: str) -> dict[str, Any]:
    """Extract and install an archive into an isolated temporary XDG home."""
    manifest = validate_archive(archive_path, tag=tag)
    with tempfile.TemporaryDirectory(prefix="sky-cua-release-smoke-") as temporary:
        root = Path(temporary)
        extract_root = root / "extracted"
        extract_root.mkdir()
        with tarfile.open(archive_path, "r:gz") as archive:
            archive.extractall(extract_root, filter="data")
        payload = extract_root / PAYLOAD_ROOT
        home = root / "home"
        env = {
            "HOME": str(home),
            "XDG_DATA_HOME": str(home / ".local/share"),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_CACHE_HOME": str(home / ".cache"),
            "PATH": "/usr/bin:/bin",
            "LANG": "C.UTF-8",
        }
        subprocess.run(
            [sys.executable, "install.py", "install"],
            cwd=payload,
            env=env,
            check=True,
            text=True,
        )
        installed_manifest_path = home / ".local/share/sky-cua/RELEASE.json"
        installed_manifest = json.loads(installed_manifest_path.read_text(encoding="utf-8"))
        if installed_manifest != manifest:
            raise ValueError("isolated installation identity does not match the archive")
    return manifest


def _command_validate(args: argparse.Namespace) -> int:
    manifest = validate_archive(args.archive, tag=args.tag)
    validate_portable_runtime_archive(args.archive)
    print(json.dumps(manifest, sort_keys=True))
    return 0


def _command_smoke(args: argparse.Namespace) -> int:
    manifest = smoke_install(args.archive, tag=args.tag)
    print(json.dumps(manifest, sort_keys=True))
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    validate = subparsers.add_parser("validate", help="validate archive safety and identity")
    validate.add_argument("--archive", type=Path, required=True)
    validate.add_argument("--tag", required=True)
    validate.set_defaults(handler=_command_validate)

    smoke = subparsers.add_parser("smoke-install", help="install into an isolated temporary home")
    smoke.add_argument("--archive", type=Path, required=True)
    smoke.add_argument("--tag", required=True)
    smoke.set_defaults(handler=_command_smoke)

    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    handler = cast(Any, args.handler)
    return cast(int, handler(args))


if __name__ == "__main__":
    raise SystemExit(main())
