from __future__ import annotations

import json
import stat
from pathlib import Path

import pytest

import _native_messaging_install as native_install
from _native_messaging_install import (
    EXTENSION_ID,
    HOST_NAME,
    HOST_RELATIVE_PATH,
    MANIFEST_RELATIVE_DIRS,
    NativeMessagingInstallError,
    install_native_messaging_manifests,
    rollback_native_messaging_manifests,
)


def _release(tmp_path: Path, *, executable: bool = True) -> Path:
    release = tmp_path / "store/releases" / ("a" * 64)
    host = release / HOST_RELATIVE_PATH
    host.parent.mkdir(parents=True)
    host.write_bytes(b"native-host")
    host.chmod(0o755 if executable else 0o644)
    return release


def _manifest_paths(home: Path) -> tuple[Path, ...]:
    return tuple(home / relative / f"{HOST_NAME}.json" for relative in MANIFEST_RELATIVE_DIRS)


def test_installs_four_owner_only_manifests_bound_to_exact_generation(tmp_path: Path) -> None:
    release = _release(tmp_path)
    home = tmp_path / "home"

    report = install_native_messaging_manifests(release, home=home)

    assert report.release_root == release
    assert report.host_path == release / HOST_RELATIVE_PATH
    assert report.manifest_paths == _manifest_paths(home)
    assert report.changed_paths == report.manifest_paths
    for path in report.manifest_paths:
        manifest = json.loads(path.read_bytes())
        assert manifest == {
            "allowed_origins": [f"chrome-extension://{EXTENSION_ID}/"],
            "description": "sky-cua browser automation native host",
            "name": "com.openai.codexextension",
            "path": str(release / HOST_RELATIVE_PATH),
            "type": "stdio",
        }
        assert stat.S_IMODE(path.stat().st_mode) == 0o600
        assert "projects/sky-cua" not in manifest["path"]
        assert "/current/" not in manifest["path"]


def test_reinstall_is_idempotent(tmp_path: Path) -> None:
    release = _release(tmp_path)
    home = tmp_path / "home"
    first = install_native_messaging_manifests(release, home=home)
    before = {path: (path.read_bytes(), path.stat().st_mtime_ns) for path in first.manifest_paths}

    second = install_native_messaging_manifests(release, home=home)

    assert second.changed_paths == ()
    assert {
        path: (path.read_bytes(), path.stat().st_mtime_ns) for path in second.manifest_paths
    } == before


@pytest.mark.parametrize("state", ["missing", "non-executable"])
def test_rejects_invalid_host_before_manifest_mutation(tmp_path: Path, state: str) -> None:
    release = tmp_path / "store/releases" / ("a" * 64)
    if state == "non-executable":
        release = _release(tmp_path, executable=False)
    home = tmp_path / "home"
    existing = _manifest_paths(home)[0]
    existing.parent.mkdir(parents=True)
    original = b"do not mutate\n"
    existing.write_bytes(original)

    with pytest.raises(NativeMessagingInstallError, match=r"missing|not an executable"):
        install_native_messaging_manifests(release, home=home)

    assert existing.read_bytes() == original
    assert all(not path.exists() for path in _manifest_paths(home)[1:])


def test_rollback_restores_exact_prior_bytes_modes_and_absence(tmp_path: Path) -> None:
    release = _release(tmp_path)
    home = tmp_path / "home"
    paths = _manifest_paths(home)
    paths[0].parent.mkdir(parents=True)
    previous = b'{"prior":true}\n'
    paths[0].write_bytes(previous)
    paths[0].chmod(0o640)

    report = install_native_messaging_manifests(release, home=home)
    rollback_native_messaging_manifests(report)

    assert paths[0].read_bytes() == previous
    assert stat.S_IMODE(paths[0].stat().st_mode) == 0o640
    assert not paths[1].exists()
    assert not paths[2].exists()
    assert not paths[3].exists()


def test_rollback_rejects_concurrent_manifest_change_without_retry(tmp_path: Path) -> None:
    report = install_native_messaging_manifests(_release(tmp_path), home=tmp_path / "home")
    report.manifest_paths[0].write_bytes(b"external change")

    with pytest.raises(NativeMessagingInstallError, match="changed before rollback"):
        rollback_native_messaging_manifests(report)

    assert report.manifest_paths[0].read_bytes() == b"external change"


def test_partial_install_failure_restores_already_written_manifest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    release = _release(tmp_path)
    home = tmp_path / "home"
    paths = _manifest_paths(home)
    paths[0].parent.mkdir(parents=True)
    original = b"original chrome manifest"
    paths[0].write_bytes(original)
    real_atomic_write = native_install._atomic_write
    calls = 0

    def fail_second_write(path: Path, content: bytes, *, mode: int) -> None:
        nonlocal calls
        calls += 1
        if calls == 2:
            raise OSError("simulated chromium write failure")
        real_atomic_write(path, content, mode=mode)

    monkeypatch.setattr(native_install, "_atomic_write", fail_second_write)
    with pytest.raises(NativeMessagingInstallError, match="simulated chromium write failure"):
        install_native_messaging_manifests(release, home=home)

    assert paths[0].read_bytes() == original
    assert not paths[1].exists()
    assert not paths[2].exists()
