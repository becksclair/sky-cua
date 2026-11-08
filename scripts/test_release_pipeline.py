from __future__ import annotations

import io
import json
import tarfile
from pathlib import Path

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
