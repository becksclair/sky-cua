from __future__ import annotations

import io
import json
import tarfile
import urllib.request
from collections.abc import Sequence
from pathlib import Path
from typing import Any

import pytest

import release_pipeline

REPO_ROOT = Path(__file__).resolve().parents[1]


def _archive(
    path: Path,
    *,
    version: str = "0.1.0",
    extra_members: tuple[tarfile.TarInfo, ...] = (),
) -> Path:
    manifest = {
        "schema_version": 1,
        "product": "sky-cua",
        "version": version,
        "target": "linux-x64-glibc",
    }
    contents = dict.fromkeys(release_pipeline.REQUIRED_MEMBERS, b"payload\n")
    contents["RELEASE.json"] = json.dumps(manifest).encode()
    contents["install.py"] = (
        b"import json, os, pathlib\n"
        b"source = pathlib.Path('RELEASE.json')\n"
        b"target = pathlib.Path(os.environ['XDG_DATA_HOME']) / 'sky-cua' / 'RELEASE.json'\n"
        b"target.parent.mkdir(parents=True, exist_ok=True)\n"
        b"target.write_text(source.read_text())\n"
    )
    with tarfile.open(path, "w:gz") as archive:
        for relative, content in contents.items():
            info = tarfile.TarInfo(f"{release_pipeline.PAYLOAD_ROOT}/{relative}")
            info.size = len(content)
            info.mode = 0o755 if relative == "install.py" else 0o644
            archive.addfile(info, io.BytesIO(content))
        for member in extra_members:
            archive.addfile(member, io.BytesIO(b"bad") if member.isfile() else None)
    return path


class FakeGiteaClient(release_pipeline.GiteaClient):
    def __init__(
        self,
        *,
        release: dict[str, Any] | None = None,
        corrupt_archive_readback: bool = False,
        corrupt_public_archive_readback: bool = False,
    ) -> None:
        self.release = release
        self.corrupt_archive_readback = corrupt_archive_readback
        self.corrupt_public_archive_readback = corrupt_public_archive_readback
        self.uploads: list[str] = []
        self.asset_bytes: dict[str, bytes] = {}
        self.publish_calls = 0
        self.dispatches: list[tuple[str, str]] = []
        self.events: list[str] = []

    def release_by_tag(self, tag: str) -> dict[str, Any] | None:
        _ = tag
        return self.release

    def create_release(self, *, tag: str, commit: str) -> dict[str, Any]:
        self.release = {
            "id": 7,
            "tag_name": tag,
            "target_commitish": commit,
            "draft": True,
            "assets": [],
        }
        return self.release

    def publish_draft(self, release_id: int) -> dict[str, Any]:
        assert release_id == 7
        assert self.release is not None
        self.publish_calls += 1
        self.events.append("publish")
        self.release["draft"] = False
        return self.release

    def upload_asset(self, release_id: int, path: Path) -> dict[str, Any]:
        assert release_id == 7
        assert self.release is not None
        self.uploads.append(path.name)
        self.events.append(f"upload:{path.name}")
        self.asset_bytes[path.name] = path.read_bytes()
        asset = {"name": path.name, "browser_download_url": f"memory://{path.name}"}
        assets = self.release["assets"]
        assert isinstance(assets, list)
        assets.append(asset)
        return asset

    def download_asset_to(self, url: str, destination: Path, *, authenticated: bool = True) -> None:
        name = url.removeprefix("memory://")
        access = "auth" if authenticated else "public"
        self.events.append(f"read:{access}:{name}")
        corrupt_archive = self.corrupt_archive_readback or (
            self.corrupt_public_archive_readback and not authenticated
        )
        if corrupt_archive and name == release_pipeline.ARCHIVE_NAME:
            destination.write_bytes(b"corrupt")
        else:
            destination.write_bytes(self.asset_bytes[name])

    def dispatch_deploy(self, *, tag: str, digest: str) -> None:
        self.events.append("dispatch")
        self.dispatches.append((tag, digest))


def test_validate_archive_accepts_canonical_payload(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)

    manifest = release_pipeline.validate_archive(archive, tag="standalone-v0.1.0")

    assert manifest["product"] == "sky-cua"
    assert manifest["target"] == "linux-x64-glibc"
    assert manifest["version"] == "0.1.0"


def test_validate_archive_rejects_traversal(tmp_path: Path) -> None:
    malicious = tarfile.TarInfo(f"{release_pipeline.PAYLOAD_ROOT}/../../escaped")
    malicious.size = 3
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME, extra_members=(malicious,))

    with pytest.raises(ValueError, match="unsafe path"):
        release_pipeline.validate_archive(archive, tag="standalone-v0.1.0")


def test_validate_archive_rejects_escaping_symlink(tmp_path: Path) -> None:
    malicious = tarfile.TarInfo(f"{release_pipeline.PAYLOAD_ROOT}/browser/escape")
    malicious.type = tarfile.SYMTYPE
    malicious.linkname = "../../../outside"
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME, extra_members=(malicious,))

    with pytest.raises(ValueError, match="escapes its payload"):
        release_pipeline.validate_archive(archive, tag="standalone-v0.1.0")


def test_validate_archive_rejects_tag_version_mismatch(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME, version="0.2.0")

    with pytest.raises(ValueError, match="does not match archive version"):
        release_pipeline.validate_archive(archive, tag="standalone-v0.1.0")


def test_smoke_install_uses_isolated_home(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)

    manifest = release_pipeline.smoke_install(archive, tag="standalone-v0.1.0")

    assert manifest["version"] == "0.1.0"


def test_publish_creates_assets_reads_them_back_and_dispatches(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)
    client = FakeGiteaClient()
    commit = "a" * 40

    digest = release_pipeline.publish_release(
        client=client,
        archive_path=archive,
        tag="standalone-v0.1.0",
        commit=commit,
    )

    assert digest == release_pipeline.sha256_file(archive)
    assert client.uploads == [release_pipeline.ARCHIVE_NAME, release_pipeline.CHECKSUM_NAME]
    assert client.publish_calls == 1
    assert client.dispatches == [("standalone-v0.1.0", digest)]
    assert client.events == [
        f"upload:{release_pipeline.ARCHIVE_NAME}",
        f"upload:{release_pipeline.CHECKSUM_NAME}",
        f"read:auth:{release_pipeline.ARCHIVE_NAME}",
        f"read:auth:{release_pipeline.CHECKSUM_NAME}",
        "publish",
        f"read:public:{release_pipeline.ARCHIVE_NAME}",
        f"read:public:{release_pipeline.CHECKSUM_NAME}",
        "dispatch",
    ]
    assert client.asset_bytes[release_pipeline.CHECKSUM_NAME] == (
        f"{digest}  {release_pipeline.ARCHIVE_NAME}\n".encode()
    )


def test_publish_reuses_matching_assets_without_overwrite(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)
    digest = release_pipeline.sha256_file(archive)
    checksum = f"{digest}  {release_pipeline.ARCHIVE_NAME}\n".encode()
    release = {
        "id": 7,
        "tag_name": "standalone-v0.1.0",
        "target_commitish": "b" * 40,
        "draft": False,
        "assets": [
            {
                "name": release_pipeline.ARCHIVE_NAME,
                "browser_download_url": f"memory://{release_pipeline.ARCHIVE_NAME}",
            },
            {
                "name": release_pipeline.CHECKSUM_NAME,
                "browser_download_url": f"memory://{release_pipeline.CHECKSUM_NAME}",
            },
        ],
    }
    client = FakeGiteaClient(release=release)
    client.asset_bytes = {
        release_pipeline.ARCHIVE_NAME: archive.read_bytes(),
        release_pipeline.CHECKSUM_NAME: checksum,
    }

    result = release_pipeline.publish_release(
        client=client,
        archive_path=archive,
        tag="standalone-v0.1.0",
        commit="b" * 40,
    )

    assert result == digest
    assert client.uploads == []
    assert client.publish_calls == 0
    assert client.dispatches == [("standalone-v0.1.0", digest)]
    assert client.events == [
        f"read:auth:{release_pipeline.ARCHIVE_NAME}",
        f"read:auth:{release_pipeline.CHECKSUM_NAME}",
        f"read:public:{release_pipeline.ARCHIVE_NAME}",
        f"read:public:{release_pipeline.CHECKSUM_NAME}",
        "dispatch",
    ]


def test_publish_checks_partial_draft_before_uploading_missing_asset(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)
    release = {
        "id": 7,
        "tag_name": "standalone-v0.1.0",
        "target_commitish": "1" * 40,
        "draft": True,
        "assets": [
            {
                "name": release_pipeline.ARCHIVE_NAME,
                "browser_download_url": f"memory://{release_pipeline.ARCHIVE_NAME}",
            }
        ],
    }
    client = FakeGiteaClient(release=release)
    client.asset_bytes = {release_pipeline.ARCHIVE_NAME: b"wrong archive bytes"}

    with pytest.raises(ValueError, match="published archive readback"):
        release_pipeline.publish_release(
            client=client,
            archive_path=archive,
            tag="standalone-v0.1.0",
            commit="1" * 40,
        )

    assert client.uploads == []
    assert client.publish_calls == 0
    assert client.dispatches == []
    assert client.events == [f"read:auth:{release_pipeline.ARCHIVE_NAME}"]


def test_publish_refuses_incomplete_published_release(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)
    client = FakeGiteaClient(
        release={
            "id": 7,
            "tag_name": "standalone-v0.1.0",
            "target_commitish": "f" * 40,
            "draft": False,
            "assets": [],
        }
    )

    with pytest.raises(ValueError, match="missing required assets"):
        release_pipeline.publish_release(
            client=client,
            archive_path=archive,
            tag="standalone-v0.1.0",
            commit="f" * 40,
        )

    assert client.uploads == []
    assert client.publish_calls == 0
    assert client.dispatches == []


def test_publish_refuses_mismatched_readback_without_dispatch(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)
    client = FakeGiteaClient(corrupt_archive_readback=True)

    with pytest.raises(ValueError, match="published archive readback"):
        release_pipeline.publish_release(
            client=client,
            archive_path=archive,
            tag="standalone-v0.1.0",
            commit="c" * 40,
        )

    assert client.dispatches == []


def test_publish_requires_public_readback_after_publishing_draft(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)
    client = FakeGiteaClient(corrupt_public_archive_readback=True)

    with pytest.raises(ValueError, match="published archive readback"):
        release_pipeline.publish_release(
            client=client,
            archive_path=archive,
            tag="standalone-v0.1.0",
            commit="2" * 40,
        )

    assert client.uploads == [release_pipeline.ARCHIVE_NAME, release_pipeline.CHECKSUM_NAME]
    assert client.publish_calls == 1
    assert client.dispatches == []
    assert client.events == [
        f"upload:{release_pipeline.ARCHIVE_NAME}",
        f"upload:{release_pipeline.CHECKSUM_NAME}",
        f"read:auth:{release_pipeline.ARCHIVE_NAME}",
        f"read:auth:{release_pipeline.CHECKSUM_NAME}",
        "publish",
        f"read:public:{release_pipeline.ARCHIVE_NAME}",
    ]


def test_asset_download_can_switch_from_authenticated_to_public(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    authorizations: list[str | None] = []

    class Response:
        def __init__(self) -> None:
            self.stream = io.BytesIO(b"asset bytes")

        def __enter__(self) -> Response:
            return self

        def __exit__(self, *_args: object) -> None:
            return None

        def read(self, size: int) -> bytes:
            return self.stream.read(size)

    def fake_urlopen(request: urllib.request.Request, *, timeout: int) -> Response:
        assert timeout == 120
        authorizations.append(request.get_header("Authorization"))
        return Response()

    monkeypatch.setattr(release_pipeline.urllib.request, "urlopen", fake_urlopen)
    client = release_pipeline.GiteaClient("https://git.example.test", "bex/sky-cua", "secret")

    client.download_asset_to("https://git.example.test/auth", tmp_path / "auth")
    client.download_asset_to(
        "https://git.example.test/public", tmp_path / "public", authenticated=False
    )

    assert authorizations == ["token secret", None]
    assert (tmp_path / "auth").read_bytes() == b"asset bytes"
    assert (tmp_path / "public").read_bytes() == b"asset bytes"


def test_dispatch_uses_reviewed_main_workflow_definition() -> None:
    requests: list[tuple[str, str, bytes | None, tuple[int, ...]]] = []

    class RecordingClient(release_pipeline.GiteaClient):
        def _request(
            self,
            method: str,
            path_or_url: str,
            *,
            body: bytes | None = None,
            content_type: str | None = "application/json",
            expected: Sequence[int] = (200,),
        ) -> bytes:
            _ = content_type
            requests.append((method, path_or_url, body, tuple(expected)))
            return b""

    client = RecordingClient("https://git.example.test", "bex/sky-cua", "token")
    client.dispatch_deploy(tag="standalone-v0.1.0", digest="a" * 64)

    assert len(requests) == 1
    method, path, body, expected = requests[0]
    assert method == "POST"
    assert path.endswith("/actions/workflows/deploy-saga.yml/dispatches")
    assert body is not None
    assert json.loads(body) == {
        "ref": "main",
        "inputs": {"tag": "standalone-v0.1.0", "archive_sha256": "a" * 64},
    }
    assert expected == (204,)


def test_release_workflow_remains_manual_until_cutover() -> None:
    release_workflow = (REPO_ROOT / ".gitea/workflows/release-standalone.yml").read_text()
    deploy_workflow = (REPO_ROOT / ".gitea/workflows/deploy-saga.yml").read_text()

    assert "workflow_dispatch:" in release_workflow
    assert "push:" not in release_workflow
    assert "tags:" not in release_workflow
    assert "ssh -o BatchMode=yes saga" in deploy_workflow
    assert not (REPO_ROOT / ".github/workflows/verify.yml").exists()


def test_publish_refuses_existing_release_for_different_commit(tmp_path: Path) -> None:
    archive = _archive(tmp_path / release_pipeline.ARCHIVE_NAME)
    client = FakeGiteaClient(
        release={
            "id": 7,
            "tag_name": "standalone-v0.1.0",
            "target_commitish": "d" * 40,
            "draft": False,
            "assets": [],
        }
    )

    with pytest.raises(ValueError, match="different source commit"):
        release_pipeline.publish_release(
            client=client,
            archive_path=archive,
            tag="standalone-v0.1.0",
            commit="e" * 40,
        )

    assert client.uploads == []
    assert client.dispatches == []
