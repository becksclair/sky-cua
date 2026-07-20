"""Transactional native-messaging manifest projection for complete releases."""

from __future__ import annotations

import json
import os
import stat
import tempfile
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path

HOST_NAME = "com.openai.codexextension"
EXTENSION_ID = "hehggadaopoacecdllhhajmbjkdcmajg"
HOST_RELATIVE_PATH = Path("components/core-linux-x64/bin/runtimes/linux-x64/sky-cua-chrome-host")
MANIFEST_RELATIVE_DIRS = (
    Path(".config/google-chrome/NativeMessagingHosts"),
    Path(".config/chromium/NativeMessagingHosts"),
    Path(".config/BraveSoftware/Brave-Browser/NativeMessagingHosts"),
    Path(".config/BraveSoftware/Brave-Origin/NativeMessagingHosts"),
)


class NativeMessagingInstallError(RuntimeError):
    """Native-messaging manifest installation or rollback was not unambiguous."""


@dataclass(frozen=True)
class _ManifestSnapshot:
    path: Path
    previous_bytes: bytes | None
    previous_mode: int | None


@dataclass(frozen=True)
class NativeMessagingInstallReport:
    release_root: Path
    host_path: Path
    manifest_paths: tuple[Path, ...]
    changed_paths: tuple[Path, ...]
    installed_bytes: bytes
    snapshots: tuple[_ManifestSnapshot, ...]

    def as_dict(self) -> dict[str, object]:
        return {
            "release_root": str(self.release_root),
            "host_path": str(self.host_path),
            "manifest_paths": [str(path) for path in self.manifest_paths],
            "changed_paths": [str(path) for path in self.changed_paths],
        }


def _manifest_bytes(host_path: Path) -> bytes:
    manifest = {
        "name": HOST_NAME,
        "description": "sky-cua browser automation native host",
        "path": str(host_path),
        "type": "stdio",
        "allowed_origins": [f"chrome-extension://{EXTENSION_ID}/"],
    }
    return (json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n").encode()


def _snapshot(path: Path) -> _ManifestSnapshot:
    try:
        info = path.stat()
        return _ManifestSnapshot(path, path.read_bytes(), stat.S_IMODE(info.st_mode))
    except FileNotFoundError:
        return _ManifestSnapshot(path, None, None)


def _snapshot_still_matches(snapshot: _ManifestSnapshot) -> bool:
    if snapshot.previous_bytes is None:
        try:
            snapshot.path.lstat()
        except FileNotFoundError:
            return True
        return False
    try:
        return (
            snapshot.path.read_bytes() == snapshot.previous_bytes
            and stat.S_IMODE(snapshot.path.stat().st_mode) == snapshot.previous_mode
        )
    except FileNotFoundError:
        return False


def _fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _atomic_write(path: Path, content: bytes, *, mode: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.tmp-", dir=path.parent)
    temporary = Path(temporary_name)
    try:
        os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        _fsync_directory(path.parent)
    finally:
        with suppress(OSError):
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def _remove(path: Path) -> None:
    path.unlink()
    _fsync_directory(path.parent)


def _restore_snapshots(
    snapshots: tuple[_ManifestSnapshot, ...],
    *,
    changed_paths: tuple[Path, ...],
    expected_installed_bytes: bytes,
) -> None:
    changed = set(changed_paths)
    failures: list[str] = []
    for snapshot in reversed(snapshots):
        if snapshot.path not in changed:
            continue
        try:
            try:
                current = snapshot.path.read_bytes()
            except FileNotFoundError as error:
                raise NativeMessagingInstallError(
                    f"installed manifest disappeared before rollback: {snapshot.path}"
                ) from error
            if current != expected_installed_bytes:
                raise NativeMessagingInstallError(
                    f"installed manifest changed before rollback: {snapshot.path}"
                )
            if stat.S_IMODE(snapshot.path.stat().st_mode) != 0o600:
                raise NativeMessagingInstallError(
                    f"installed manifest mode changed before rollback: {snapshot.path}"
                )
            if snapshot.previous_bytes is None:
                _remove(snapshot.path)
            else:
                assert snapshot.previous_mode is not None
                _atomic_write(
                    snapshot.path,
                    snapshot.previous_bytes,
                    mode=snapshot.previous_mode,
                )
        except BaseException as error:
            failures.append(f"{snapshot.path}: {error}")
    if failures:
        raise NativeMessagingInstallError(f"native manifest rollback failed: {failures}")


def install_native_messaging_manifests(
    release_root: Path,
    *,
    home: Path | None = None,
) -> NativeMessagingInstallReport:
    """Point all supported user browsers at one exact verified generation."""
    release_root = release_root.expanduser().resolve()
    host_path = release_root / HOST_RELATIVE_PATH
    try:
        info = host_path.stat()
    except FileNotFoundError as error:
        raise NativeMessagingInstallError(
            f"complete release is missing native messaging host: {host_path}"
        ) from error
    if not host_path.is_file() or host_path.is_symlink() or not info.st_mode & 0o111:
        raise NativeMessagingInstallError(
            f"complete release native messaging host is not an executable regular file: {host_path}"
        )

    manifest_home = (home or Path.home()).expanduser().resolve()
    paths = tuple(
        manifest_home / relative / f"{HOST_NAME}.json" for relative in MANIFEST_RELATIVE_DIRS
    )
    desired = _manifest_bytes(host_path)
    snapshots = tuple(_snapshot(path) for path in paths)
    changed: list[Path] = []
    try:
        for snapshot in snapshots:
            if not _snapshot_still_matches(snapshot):
                raise NativeMessagingInstallError(
                    f"native manifest changed before installation: {snapshot.path}"
                )
            if snapshot.previous_bytes == desired and snapshot.previous_mode == 0o600:
                continue
            _atomic_write(snapshot.path, desired, mode=0o600)
            changed.append(snapshot.path)
    except BaseException as error:
        try:
            _restore_snapshots(
                snapshots,
                changed_paths=tuple(changed),
                expected_installed_bytes=desired,
            )
        except BaseException as rollback_error:
            raise NativeMessagingInstallError(
                f"native manifest install failed: {error}; rollback failed: {rollback_error}"
            ) from error
        raise NativeMessagingInstallError(f"native manifest install failed: {error}") from error

    return NativeMessagingInstallReport(
        release_root=release_root,
        host_path=host_path,
        manifest_paths=paths,
        changed_paths=tuple(changed),
        installed_bytes=desired,
        snapshots=snapshots,
    )


def rollback_native_messaging_manifests(report: NativeMessagingInstallReport) -> None:
    """Restore the exact pre-install state when a downstream host cutover fails."""
    _restore_snapshots(
        report.snapshots,
        changed_paths=report.changed_paths,
        expected_installed_bytes=report.installed_bytes,
    )
