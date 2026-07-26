#!/usr/bin/env python3
"""Validate, smoke, and publish standalone Sky CUA release archives."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import posixpath
import re
import subprocess
import sys
import tarfile
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid
from collections.abc import Mapping, Sequence
from pathlib import Path, PurePosixPath
from typing import Any, cast

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


class GiteaClient:
    """Small Gitea API client for one repository release workflow."""

    def __init__(self, base_url: str, repository: str, token: str) -> None:
        owner, separator, repo = repository.partition("/")
        if not separator or not owner or not repo or "/" in repo:
            raise ValueError(f"invalid Gitea repository: {repository!r}")
        self.base_url = base_url.rstrip("/")
        self.repository = f"{owner}/{repo}"
        self.token = token
        self.api_root = f"/api/v1/repos/{urllib.parse.quote(owner)}/{urllib.parse.quote(repo)}"

    def _request(
        self,
        method: str,
        path_or_url: str,
        *,
        body: bytes | None = None,
        content_type: str | None = "application/json",
        expected: Sequence[int] = (200,),
    ) -> bytes:
        url = (
            path_or_url
            if path_or_url.startswith(("http://", "https://"))
            else f"{self.base_url}{path_or_url}"
        )
        headers = {"Authorization": f"token {self.token}", "Accept": "application/json"}
        if body is not None and content_type is not None:
            headers["Content-Type"] = content_type
        request = urllib.request.Request(url, data=body, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                status = response.status
                value = response.read()
        except urllib.error.HTTPError as error:
            value = error.read()
            if error.code not in expected:
                detail = value.decode("utf-8", errors="replace")
                raise RuntimeError(
                    f"Gitea {method} {url} failed ({error.code}): {detail}"
                ) from error
            return value
        if status not in expected:
            raise RuntimeError(f"Gitea {method} {url} returned unexpected status {status}")
        return value

    def _json_request(
        self,
        method: str,
        path: str,
        *,
        value: Mapping[str, object] | None = None,
        expected: Sequence[int] = (200,),
    ) -> dict[str, Any]:
        body = None if value is None else json.dumps(value).encode("utf-8")
        response = self._request(method, path, body=body, expected=expected)
        return cast(dict[str, Any], json.loads(response)) if response else {}

    def release_by_tag(self, tag: str) -> dict[str, Any] | None:
        encoded_tag = urllib.parse.quote(tag, safe="")
        try:
            return self._json_request(
                "GET", f"{self.api_root}/releases/tags/{encoded_tag}", expected=(200,)
            )
        except RuntimeError as error:
            if " failed (404):" in str(error):
                return None
            raise

    def create_release(self, *, tag: str, commit: str) -> dict[str, Any]:
        return self._json_request(
            "POST",
            f"{self.api_root}/releases",
            value={
                "tag_name": tag,
                "target_commitish": commit,
                "name": tag,
                "body": f"Standalone Sky CUA release built from `{commit}`.",
                "draft": True,
                "prerelease": False,
            },
            expected=(201,),
        )

    def publish_draft(self, release_id: int) -> dict[str, Any]:
        return self._json_request(
            "PATCH",
            f"{self.api_root}/releases/{release_id}",
            value={"draft": False},
            expected=(200,),
        )

    def upload_asset(self, release_id: int, path: Path) -> dict[str, Any]:
        """Stream one multipart attachment without buffering the archive in memory."""
        boundary = f"sky-cua-{uuid.uuid4().hex}"
        prefix = b"".join(
            (
                f"--{boundary}\r\n".encode(),
                (
                    f'Content-Disposition: form-data; name="attachment"; filename="{path.name}"\r\n'
                ).encode(),
                b"Content-Type: application/octet-stream\r\n\r\n",
            )
        )
        suffix = f"\r\n--{boundary}--\r\n".encode()
        query = urllib.parse.urlencode({"name": path.name})
        url = urllib.parse.urlsplit(
            f"{self.base_url}{self.api_root}/releases/{release_id}/assets?{query}"
        )
        connection_type = (
            http.client.HTTPSConnection if url.scheme == "https" else http.client.HTTPConnection
        )
        if url.hostname is None:
            raise ValueError(f"invalid Gitea server URL: {self.base_url!r}")
        connection = connection_type(url.hostname, url.port, timeout=120)
        request_path = urllib.parse.urlunsplit(("", "", url.path, url.query, ""))
        try:
            connection.putrequest("POST", request_path)
            connection.putheader("Authorization", f"token {self.token}")
            connection.putheader("Accept", "application/json")
            connection.putheader("Content-Type", f"multipart/form-data; boundary={boundary}")
            connection.putheader(
                "Content-Length", str(len(prefix) + path.stat().st_size + len(suffix))
            )
            connection.endheaders()
            connection.send(prefix)
            with path.open("rb") as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                    connection.send(chunk)
            connection.send(suffix)
            response = connection.getresponse()
            body = response.read()
        finally:
            connection.close()
        if response.status != 201:
            detail = body.decode("utf-8", errors="replace")
            raise RuntimeError(f"Gitea asset upload failed ({response.status}): {detail}")
        return cast(dict[str, Any], json.loads(body))

    def download_asset_to(self, url: str, destination: Path, *, authenticated: bool = True) -> None:
        headers = {"Authorization": f"token {self.token}"} if authenticated else {}
        request = urllib.request.Request(url, headers=headers, method="GET")
        try:
            with (
                urllib.request.urlopen(request, timeout=120) as response,
                destination.open("wb") as output,
            ):
                while chunk := response.read(1024 * 1024):
                    output.write(chunk)
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"Gitea asset download failed ({error.code}): {detail}") from error

    def dispatch_deploy(self, *, tag: str, digest: str) -> None:
        workflow = urllib.parse.quote(DEPLOY_WORKFLOW, safe="")
        self._request(
            "POST",
            f"{self.api_root}/actions/workflows/{workflow}/dispatches",
            body=json.dumps(
                {
                    "ref": "main",
                    "inputs": {"tag": tag, "archive_sha256": digest},
                }
            ).encode("utf-8"),
            expected=(204,),
        )


def _release_id(release: Mapping[str, Any]) -> int:
    value = release.get("id")
    if not isinstance(value, int):
        raise ValueError("Gitea release response has no numeric id")
    return value


def _release_assets(release: Mapping[str, Any]) -> dict[str, dict[str, Any]]:
    raw_assets = release.get("assets")
    if not isinstance(raw_assets, list):
        raise ValueError("Gitea release response has no asset list")
    assets: dict[str, dict[str, Any]] = {}
    for raw_asset in raw_assets:
        if not isinstance(raw_asset, dict) or not isinstance(raw_asset.get("name"), str):
            raise ValueError("Gitea release response contains an invalid asset")
        name = cast(str, raw_asset["name"])
        if name in assets:
            raise ValueError(f"Gitea release has duplicate asset name {name!r}")
        assets[name] = cast(dict[str, Any], raw_asset)
    return assets


def _asset_url(asset: Mapping[str, Any]) -> str:
    for key in ("browser_download_url", "url"):
        value = asset.get(key)
        if isinstance(value, str) and value:
            return value
    raise ValueError("Gitea release asset has no download URL")


def _verify_asset_readback(
    *,
    client: GiteaClient,
    assets: Mapping[str, Mapping[str, Any]],
    digest: str,
    checksum_bytes: bytes,
    authenticated: bool,
) -> None:
    with tempfile.TemporaryDirectory(prefix="sky-cua-release-readback-") as temporary:
        readback_root = Path(temporary)
        archive_asset = assets.get(ARCHIVE_NAME)
        if archive_asset is not None:
            published_archive = readback_root / ARCHIVE_NAME
            client.download_asset_to(
                _asset_url(archive_asset), published_archive, authenticated=authenticated
            )
            if sha256_file(published_archive) != digest:
                raise ValueError("published archive readback does not match the producer SHA-256")
        checksum_asset = assets.get(CHECKSUM_NAME)
        if checksum_asset is not None:
            published_checksum = readback_root / CHECKSUM_NAME
            client.download_asset_to(
                _asset_url(checksum_asset), published_checksum, authenticated=authenticated
            )
            if published_checksum.read_bytes() != checksum_bytes:
                raise ValueError("published checksum sidecar does not match the producer digest")


def publish_release(
    *,
    client: GiteaClient,
    archive_path: Path,
    tag: str,
    commit: str,
) -> str:
    """Create/reuse a release, prove readback identity, and dispatch Saga."""
    validate_archive(archive_path, tag=tag)
    if archive_path.name != ARCHIVE_NAME:
        raise ValueError(f"standalone archive must be named {ARCHIVE_NAME!r}")
    if not re.fullmatch(r"[0-9a-f]{40,64}", commit):
        raise ValueError("source commit must be a full lowercase hexadecimal object id")
    digest = sha256_file(archive_path)
    validate_digest(digest)
    checksum_path = archive_path.with_name(CHECKSUM_NAME)
    checksum_bytes = f"{digest}  {ARCHIVE_NAME}\n".encode()
    checksum_path.write_bytes(checksum_bytes)

    release = client.release_by_tag(tag)
    if release is None:
        release = client.create_release(tag=tag, commit=commit)
    elif release.get("target_commitish") != commit:
        raise ValueError("existing Gitea release points at a different source commit")

    assets = _release_assets(release)
    unexpected = sorted(set(assets) - {ARCHIVE_NAME, CHECKSUM_NAME})
    if unexpected:
        raise ValueError(f"existing Gitea release has unexpected assets: {unexpected}")
    draft = release.get("draft")
    if not isinstance(draft, bool):
        raise ValueError("Gitea release response has no draft state")

    release_id = _release_id(release)
    missing_assets = [path for path in (archive_path, checksum_path) if path.name not in assets]
    if missing_assets and not draft:
        raise ValueError("published Gitea release is missing required assets and cannot be mutated")
    if missing_assets and assets:
        _verify_asset_readback(
            client=client,
            assets=assets,
            digest=digest,
            checksum_bytes=checksum_bytes,
            authenticated=True,
        )
    for path in missing_assets:
        client.upload_asset(release_id, path)

    refreshed = client.release_by_tag(tag)
    if refreshed is None:
        raise RuntimeError("Gitea release disappeared after asset upload")
    assets = _release_assets(refreshed)
    if set(assets) != {ARCHIVE_NAME, CHECKSUM_NAME}:
        raise ValueError(f"Gitea release assets are incomplete: {sorted(assets)}")

    _verify_asset_readback(
        client=client,
        assets=assets,
        digest=digest,
        checksum_bytes=checksum_bytes,
        authenticated=True,
    )

    if draft:
        published = client.publish_draft(release_id)
        if published.get("draft") is not False:
            raise RuntimeError("Gitea release remained a draft after publication")

    _verify_asset_readback(
        client=client,
        assets=assets,
        digest=digest,
        checksum_bytes=checksum_bytes,
        authenticated=False,
    )
    client.dispatch_deploy(tag=tag, digest=digest)
    return digest


def _command_validate(args: argparse.Namespace) -> int:
    manifest = validate_archive(args.archive, tag=args.tag)
    print(json.dumps(manifest, sort_keys=True))
    return 0


def _command_smoke(args: argparse.Namespace) -> int:
    manifest = smoke_install(args.archive, tag=args.tag)
    print(json.dumps(manifest, sort_keys=True))
    return 0


def _command_publish(args: argparse.Namespace) -> int:
    token = os.environ.get("SKY_CUA_GITEA_TOKEN", "").strip()
    if not token:
        raise ValueError("SKY_CUA_GITEA_TOKEN is required")
    client = GiteaClient(args.server_url, args.repository, token)
    digest = publish_release(
        client=client,
        archive_path=args.archive,
        tag=args.tag,
        commit=args.commit,
    )
    print(json.dumps({"tag": args.tag, "archive_sha256": digest}, sort_keys=True))
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

    publish = subparsers.add_parser("publish", help="publish, read back, and dispatch deployment")
    publish.add_argument("--server-url", required=True)
    publish.add_argument("--repository", required=True)
    publish.add_argument("--archive", type=Path, required=True)
    publish.add_argument("--tag", required=True)
    publish.add_argument("--commit", required=True)
    publish.set_defaults(handler=_command_publish)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    handler = cast(Any, args.handler)
    return cast(int, handler(args))


if __name__ == "__main__":
    raise SystemExit(main())
