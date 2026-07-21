"""Producer-owned activation state for one immutable sky-cua release."""

from __future__ import annotations

import json
import os
import tempfile
from collections.abc import Mapping
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from _native_messaging_install import (
    HOST_NAME,
    HOST_RELATIVE_PATH,
    MANIFEST_RELATIVE_DIRS,
    native_messaging_manifest_bytes,
)
from _plugin_bundle import (
    current_runtime_platform,
    find_unix_runtime_processes,
    platform_runtime_binary_base_names,
    runtime_binary_path,
    stop_unix_runtime_processes,
)
from release_generation import GenerationStore, VerifiedRelease, canonical_json_bytes

ACTIVATION_RECEIPT = "activation-receipt.json"
ACTIVATION_SCHEMA_VERSION = 1
DEFAULT_BIN_DIRECTORY = Path.home() / ".local/bin"


class ActivationVerificationError(RuntimeError):
    """The producer-owned active state does not match the selected release."""


@dataclass(frozen=True)
class ActivationReport:
    release_id: str
    manifest_sha256: str
    release_root: str
    profile: str
    platform: str
    receipt_path: str
    native_manifest_paths: tuple[str, ...]
    stable_links: Mapping[str, str]
    stale_processes_drained: bool

    def as_dict(self) -> dict[str, object]:
        return {
            "schema_version": ACTIVATION_SCHEMA_VERSION,
            "release_id": self.release_id,
            "manifest_sha256": self.manifest_sha256,
            "release_root": self.release_root,
            "profile": self.profile,
            "platform": self.platform,
            "receipt_path": self.receipt_path,
            "native_manifest_paths": list(self.native_manifest_paths),
            "stable_links": dict(self.stable_links),
            "stale_processes_drained": self.stale_processes_drained,
        }


@dataclass(frozen=True)
class PathSnapshot:
    path: Path
    kind: str
    value: bytes | str | None
    mode: int | None


def _atomic_write(path: Path, content: bytes, *, mode: int = 0o600) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor, name = tempfile.mkstemp(prefix=f".{path.name}.tmp-", dir=path.parent)
    temporary = Path(name)
    try:
        os.fchmod(descriptor, mode)
        with os.fdopen(descriptor, "wb", closefd=True) as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        with suppress(OSError):
            os.close(descriptor)
        temporary.unlink(missing_ok=True)


def snapshot_path(path: Path) -> PathSnapshot:
    try:
        if path.is_symlink():
            return PathSnapshot(path, "symlink", os.readlink(path), None)
        if path.is_file():
            info = path.stat()
            return PathSnapshot(path, "file", path.read_bytes(), info.st_mode & 0o777)
        if path.exists():
            raise ActivationVerificationError(f"activation path has unsupported type: {path}")
    except FileNotFoundError:
        pass
    return PathSnapshot(path, "missing", None, None)


def restore_path(snapshot: PathSnapshot) -> None:
    snapshot.path.unlink(missing_ok=True)
    if snapshot.kind == "missing":
        return
    snapshot.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if snapshot.kind == "symlink":
        assert isinstance(snapshot.value, str)
        snapshot.path.symlink_to(snapshot.value)
        return
    if snapshot.kind == "file":
        assert isinstance(snapshot.value, bytes) and snapshot.mode is not None
        _atomic_write(snapshot.path, snapshot.value, mode=snapshot.mode)
        return
    raise AssertionError(snapshot.kind)


def stable_link_targets(
    store_root: Path,
    release: VerifiedRelease,
    *,
    bin_dir: Path | None = None,
) -> dict[Path, str]:
    platform_id = current_runtime_platform()
    selected_bin_dir = (bin_dir or DEFAULT_BIN_DIRECTORY).expanduser().resolve()
    result: dict[Path, str] = {}
    for target_dir in dict.fromkeys((selected_bin_dir, store_root / "bin")):
        for name in platform_runtime_binary_base_names(platform_id):
            relative = Path("components/core-linux-x64") / runtime_binary_path(platform_id, name)
            if (release.root / relative).is_file():
                result[target_dir / name] = os.path.relpath(
                    store_root / "current" / relative,
                    target_dir,
                )
    return result


def install_stable_links(
    store_root: Path,
    release: VerifiedRelease,
    *,
    bin_dir: Path | None = None,
) -> tuple[dict[str, str], tuple[PathSnapshot, ...]]:
    targets = stable_link_targets(store_root, release, bin_dir=bin_dir)
    snapshots = tuple(snapshot_path(path) for path in targets)
    try:
        for path, target in targets.items():
            if path.is_symlink() and os.readlink(path) == target:
                continue
            if path.exists() and path.is_dir() and not path.is_symlink():
                raise ActivationVerificationError(f"stable command path is a directory: {path}")
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            temporary = path.parent / f".{path.name}.tmp-{os.getpid()}"
            temporary.unlink(missing_ok=True)
            temporary.symlink_to(target)
            os.replace(temporary, path)
    except BaseException as error:
        failures: list[str] = []
        for snapshot in reversed(snapshots):
            try:
                restore_path(snapshot)
            except BaseException as rollback_error:
                failures.append(f"{snapshot.path}: {rollback_error}")
        detail = f"; rollback failure(s): {failures}" if failures else ""
        raise ActivationVerificationError(
            f"stable command link installation failed: {error}{detail}"
        ) from error
    return ({str(path): target for path, target in targets.items()}, snapshots)


def receipt_path(store_root: Path) -> Path:
    return store_root / ACTIVATION_RECEIPT


def write_receipt(
    store_root: Path,
    release: VerifiedRelease,
    *,
    native_manifest_paths: tuple[Path, ...],
    stable_links: Mapping[str, str],
) -> PathSnapshot:
    path = receipt_path(store_root)
    snapshot = snapshot_path(path)
    payload = {
        "schema_version": ACTIVATION_SCHEMA_VERSION,
        "release_id": release.release_id,
        "manifest_sha256": release.manifest_sha256,
        "release_root": str(release.root.resolve()),
        "profile": release.profile,
        "platform": current_runtime_platform(),
        "native_manifest_paths": [str(item) for item in native_manifest_paths],
        "stable_links": dict(stable_links),
    }
    _atomic_write(path, canonical_json_bytes(payload) + b"\n")
    return snapshot


def _load_receipt(path: Path) -> Mapping[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ActivationVerificationError(
            f"activation receipt is missing or invalid: {path}"
        ) from error
    if not isinstance(value, dict) or value.get("schema_version") != ACTIVATION_SCHEMA_VERSION:
        raise ActivationVerificationError("activation receipt schema is unsupported")
    return value


def verify_activation(
    candidate: Path,
    *,
    store_root: Path,
    profile: str,
    expected_manifest_sha256: str | None,
    native_messaging_home: Path | None,
    bin_dir: Path | None = None,
    proc_root: Path = Path("/proc"),
) -> ActivationReport:
    from release_generation import verify_release_root

    selected = verify_release_root(
        candidate.expanduser().resolve(),
        profile=profile,
        expected_manifest_sha256=expected_manifest_sha256,
    )
    store_root = store_root.expanduser().resolve()
    store = GenerationStore(store_root)
    current = store.current_release_id()
    if current != selected.release_id:
        raise ActivationVerificationError(
            f"active release mismatch: expected {selected.release_id}, got {current or 'none'}"
        )
    installed = store.verify_installed_generation(selected.release_id)
    if installed.manifest_sha256 != selected.manifest_sha256 or installed.profile != profile:
        raise ActivationVerificationError("active installed generation identity is inconsistent")

    home = (native_messaging_home or Path.home()).expanduser().resolve()
    host_path = installed.root / HOST_RELATIVE_PATH
    manifest_paths = tuple(
        home / relative / f"{HOST_NAME}.json" for relative in MANIFEST_RELATIVE_DIRS
    )
    expected_manifest = native_messaging_manifest_bytes(host_path)
    for path in manifest_paths:
        try:
            actual = path.read_bytes()
            mode = path.stat().st_mode & 0o777
        except OSError as error:
            raise ActivationVerificationError(
                f"native manifest is missing or invalid: {path}"
            ) from error
        if actual != expected_manifest or mode != 0o600:
            raise ActivationVerificationError(f"native manifest content or mode is stale: {path}")

    expected_links = stable_link_targets(store_root, installed, bin_dir=bin_dir)
    for path, target in expected_links.items():
        if not path.is_symlink() or os.readlink(path) != target:
            raise ActivationVerificationError(f"stable command link is stale or missing: {path}")
        relative = path.resolve().relative_to(installed.root.resolve())
        if relative.parts[:1] != ("components",):
            raise ActivationVerificationError(
                f"stable command link does not resolve through current: {path}"
            )

    receipt = _load_receipt(receipt_path(store_root))
    expected_receipt = {
        "schema_version": ACTIVATION_SCHEMA_VERSION,
        "release_id": installed.release_id,
        "manifest_sha256": installed.manifest_sha256,
        "release_root": str(installed.root.resolve()),
        "profile": installed.profile,
        "platform": current_runtime_platform(),
        "native_manifest_paths": [str(path) for path in manifest_paths],
        "stable_links": {str(path): target for path, target in expected_links.items()},
    }
    if dict(receipt) != expected_receipt:
        raise ActivationVerificationError(
            "activation receipt does not match artifact-derived state"
        )
    active_root = str(installed.root.resolve()) + os.sep
    stale = [
        (pid, executable)
        for pid, executable in find_unix_runtime_processes(
            [store_root / "releases"],
            proc_root=proc_root,
            match_all_paths=True,
        )
        if executable is None or not executable.startswith(active_root)
    ]
    if stale:
        raise ActivationVerificationError(
            f"obsolete sky-cua runtime process(es) are still active: {stale}"
        )
    return ActivationReport(
        release_id=installed.release_id,
        manifest_sha256=installed.manifest_sha256,
        release_root=str(installed.root),
        profile=installed.profile,
        platform=current_runtime_platform(),
        receipt_path=str(receipt_path(store_root)),
        native_manifest_paths=manifest_paths_as_strings(manifest_paths),
        stable_links=expected_receipt["stable_links"],
        stale_processes_drained=False,
    )


def manifest_paths_as_strings(paths: tuple[Path, ...]) -> tuple[str, ...]:
    return tuple(str(path) for path in paths)


def drain_stale_processes(store_root: Path, *, proc_root: Path = Path("/proc")) -> None:
    # The shared helper limits matches to current-user, known sky-cua runtime
    # names and handles deleted executable paths. Draining the whole stack is
    # intentional: hosts respawn through `current` after activation commits.
    stop_unix_runtime_processes(
        [store_root / "releases"],
        proc_root=proc_root,
        match_all_paths=True,
    )
