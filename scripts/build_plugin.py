#!/usr/bin/env python3
from __future__ import annotations

import argparse
import io
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tarfile
from pathlib import Path

from _plugin_bundle import (
    DIST_PLUGIN_ROOT,
    REPO_ROOT,
    all_runtime_binary_paths,
    bundle_entrypoint_paths,
    chrome_extension_host_arch,
    current_runtime_platform,
    ensure_bundle_structure,
    ensure_executable,
    mcp_config_source,
    platform_runtime_binary_base_names,
    remove_path,
    runtime_binary_path,
    runtime_binary_source_name,
)
from _portable_elf import LINUX_X64_RUNTIME_NAMES, validate_x86_64_v3_paths
from deploy_freshness import STAMP_SUFFIX, write_build_stamp

BUNDLE_SOURCE_PATHS = [
    Path(".claude-plugin"),
    Path(".codex-plugin"),
    Path(".app.json"),
    Path("assets"),
    Path("hooks"),
    Path("resources"),
    Path("skills"),
    Path("docs"),
    Path("README.md"),
]

OPENAI_BUNDLED_PLUGIN_NAMES = ("browser-use", "chrome")
OPENAI_BUNDLED_MARKETPLACE_PLUGIN_NAMES = ("browser-use", "chrome", "computer-use")
NODE_REPL_NAME = "node_repl"
CANONICAL_BROWSER_CLIENT = Path("packages/browser-use/build/browser-client.mjs")
CANONICAL_BROWSER_PROJECTION = Path("packages/browser-use/build/projection.mjs")
WORKTREE_BUNDLE_FILES = (
    Path(".claude-plugin") / "plugin.json",
    Path(".claude-plugin") / "marketplace.json",
    Path("resources") / "chrome_preflight.py",
    Path("resources") / "systemd" / "sky-cua-input-helper.service",
    Path("docs") / "operations" / "isolated-daemon-smokes.md",
    Path("docs") / "operations" / "plugin-release.md",
    Path("docs") / "operations" / "testing-vm-desktop-smokes.md",
    Path("docs") / "features" / "phone-use.md",
    Path("docs") / "runtime" / "phone-companion-protocol.md",
)
CHROME_EXTENSION_BUNDLE_DIR = Path("resources") / "chrome-extension"
WORKTREE_BUNDLE_DIRS = (
    CHROME_EXTENSION_BUNDLE_DIR,
    Path("resources") / "cosmic",
    Path("resources") / "kwin",
    Path("skills") / "computer-use",
    Path("skills") / "browser-use",
    Path("skills") / "phone-use",
)
RETIRED_BUNDLE_SOURCE_PREFIXES = (
    Path("skills") / "computer-use-workflows",
    Path("skills") / "sky-cua-isolated-daemon",
    Path("skills") / "sky-cua-plugin-release",
    Path("resources") / "kwin" / "effects" / "sky-cua-agent-cursor" / "metadata.json",
    Path("resources") / "kwin" / "effects" / "sky-cua-agent-cursor" / "qml",
)

# The phone companion APK and its identity metadata are built by a separate
# Android lane and may not exist yet. They are bundled conditionally (present →
# staged, absent → skipped with a logged note) and are never part of the
# hard-required bundle structure, so `phone-use` packaging does not block on the
# Android toolchain. The metadata sidecar carries the package id, version, APK
# SHA-256, and signing certificate fingerprint that `phone_connect` verifies.
COMPANION_APK_DIR = Path("resources") / "android"
COMPANION_APK_FILE = COMPANION_APK_DIR / "phone-companion.apk"
COMPANION_APK_METADATA_FILE = COMPANION_APK_DIR / "phone-companion.json"

CARGO_BUILD_PACKAGES = [
    "sky-cua-client",
    "sky-cua-service",
    "sky-cua-overlay-host",
    "sky-cua-chrome-host",
    *([] if sys.platform == "win32" else ["sky-cua-cosmic-helper", "sky-cua-input-helper"]),
]

CARGO_BUILD_COMMAND = [
    "cargo",
    "build",
    "--release",
    *[item for package in CARGO_BUILD_PACKAGES for item in ("--package", package)],
]

RELEASE_CORE_INPUT_PROVENANCE = Path("resources") / "release" / "CORE_BUILD_INPUTS.json"


def run_cargo_build(env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        CARGO_BUILD_COMMAND,
        cwd=REPO_ROOT,
        check=False,
        env=env,
        text=True,
        capture_output=True,
    )


def emit_cargo_output(result: subprocess.CompletedProcess[str]) -> None:
    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)


def is_windows_sccache_shim_failure(result: subprocess.CompletedProcess[str]) -> bool:
    output = f"{result.stdout}\n{result.stderr}".lower()
    return (
        sys.platform == "win32"
        and result.returncode != 0
        and "sccache" in output
        and "could not create process" in output
    )


def cargo_env_without_rustc_wrappers() -> dict[str, str]:
    env = os.environ.copy()
    env["RUSTC_WRAPPER"] = ""
    env["RUSTC_WORKSPACE_WRAPPER"] = ""
    return env


def cargo_target_root() -> Path:
    configured = os.environ.get("CARGO_TARGET_DIR", "").strip()
    if not configured:
        return REPO_ROOT / "target"
    path = Path(configured).expanduser()
    return path if path.is_absolute() else REPO_ROOT / path


def release_client_binary_path() -> Path:
    return (
        cargo_target_root()
        / "release"
        / runtime_binary_source_name(current_runtime_platform(), "sky-cua-client")
    )


def build_release_binaries() -> None:
    result = run_cargo_build()
    if result.returncode == 0:
        emit_cargo_output(result)
        if current_runtime_platform() == "linux-x64":
            validate_x86_64_v3_paths(
                [cargo_target_root() / "release" / name for name in LINUX_X64_RUNTIME_NAMES]
            )
        write_build_stamp(release_client_binary_path())
        return

    if is_windows_sccache_shim_failure(result):
        print(
            "cargo build failed through the Windows sccache shim; retrying without RUSTC_WRAPPER.",
            file=sys.stderr,
        )
        retry = run_cargo_build(env=cargo_env_without_rustc_wrappers())
        if retry.returncode == 0:
            emit_cargo_output(retry)
            if current_runtime_platform() == "linux-x64":
                validate_x86_64_v3_paths(
                    [cargo_target_root() / "release" / name for name in LINUX_X64_RUNTIME_NAMES]
                )
            write_build_stamp(release_client_binary_path())
            return
        emit_cargo_output(result)
        emit_cargo_output(retry)
        retry.check_returncode()

    emit_cargo_output(result)
    result.check_returncode()


def copy_build_stamp_sidecar(source: Path, destination: Path) -> None:
    source_stamp = source.with_name(source.name + STAMP_SUFFIX)
    destination_stamp = destination.with_name(destination.name + STAMP_SUFFIX)
    if not source_stamp.exists():
        remove_path(destination_stamp)
        return
    destination_stamp.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_stamp, destination_stamp)


def tracked_bundle_files(source_paths: list[Path] | None = None) -> list[Path]:
    paths = source_paths if source_paths is not None else BUNDLE_SOURCE_PATHS
    try:
        result = subprocess.run(
            ["git", "ls-files", "-z", "--", *[str(path) for path in paths]],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
        )
    except (FileNotFoundError, subprocess.CalledProcessError):
        return worktree_bundle_files(paths)
    tracked = [
        Path(part.decode("utf-8", errors="replace")) for part in result.stdout.split(b"\0") if part
    ]
    # ``git ls-files`` includes paths deleted from the worktree. Development
    # builds intentionally stage the current worktree, so an in-scope deletion
    # must not be resurrected as a missing required input. A file disappearing
    # after this snapshot is still caught by ``copy_tracked_bundle_sources``.
    return [path for path in tracked if (REPO_ROOT / path).exists()]


_WORKTREE_BUNDLE_EXCLUDED_SUFFIXES = (".pyc", ".pyo")


def is_bundleable_worktree_file(relative_path: Path) -> bool:
    """Mirror the omissions ``git ls-files`` applies when the index is present.

    The no-git fallback walks the worktree directly, so it must reproduce the
    gitignored exclusions the git path gets for free; otherwise the two builds
    diverge. Python bytecode caches and the phone companion APK directory are
    gitignored. The APK and its metadata sidecar are staged separately and
    conditionally by ``copy_companion_apk_if_present`` and must not also leak
    into the generic worktree bundle (which would double-stage them and bypass
    that dedicated stager's contract).
    """
    if "__pycache__" in relative_path.parts:
        return False
    if relative_path.suffix in _WORKTREE_BUNDLE_EXCLUDED_SUFFIXES:
        return False
    return not relative_path.is_relative_to(COMPANION_APK_DIR)


def worktree_bundle_files(source_paths: list[Path]) -> list[Path]:
    files: list[Path] = []
    for relative_path in source_paths:
        source = REPO_ROOT / relative_path
        if source.is_file():
            if is_bundleable_worktree_file(relative_path):
                files.append(relative_path)
            continue
        if not source.is_dir():
            continue
        for child in source.rglob("*"):
            if child.is_file():
                child_relative = child.relative_to(REPO_ROOT)
                if is_bundleable_worktree_file(child_relative):
                    files.append(child_relative)
    return sorted(files)


def copy_tracked_bundle_sources(temp_root: Path) -> None:
    for relative_path in tracked_bundle_files():
        source = REPO_ROOT / relative_path
        if not source.exists():
            if any(
                relative_path.is_relative_to(prefix) for prefix in RETIRED_BUNDLE_SOURCE_PREFIXES
            ):
                continue
            raise FileNotFoundError(f"tracked bundle source is missing: {source}")
        destination = temp_root / relative_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def copy_commit_bundle_sources(
    temp_root: Path,
    producer_commit: str,
    *,
    source_paths: list[Path] | None = None,
) -> None:
    """Materialize regular bundle sources from an exact Git commit.

    Complete releases must not read ignored worktree entries or follow a
    worktree symlink while claiming that their core came from a clean commit.
    Git archive provides the committed bytes directly. The explicit member
    validation keeps a tracked symlink, hard link, or special entry from being
    dereferenced during extraction.
    """
    requested = source_paths or [*BUNDLE_SOURCE_PATHS, Path(".mcp.json"), Path("bin")]
    existing: list[Path] = []
    for relative_path in requested:
        probe = subprocess.run(
            ["git", "cat-file", "-e", f"{producer_commit}:{relative_path.as_posix()}"],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
        )
        if probe.returncode == 0:
            existing.append(relative_path)
    if not existing:
        raise ValueError("producer commit contains no bundle sources")

    archive = subprocess.run(
        [
            "git",
            "archive",
            "--format=tar",
            producer_commit,
            "--",
            *[path.as_posix() for path in existing],
        ],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
    ).stdout
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as bundle:
        for member in bundle.getmembers():
            relative = Path(member.name)
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError(f"producer archive contains an unsafe path: {member.name}")
            destination = temp_root / relative
            if member.isdir():
                destination.mkdir(parents=True, exist_ok=True)
                continue
            if not member.isfile():
                raise ValueError(f"producer archive contains a non-regular entry: {member.name}")
            source = bundle.extractfile(member)
            if source is None:
                raise ValueError(f"producer archive member has no bytes: {member.name}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.read())
            destination.chmod(member.mode & 0o777)


def write_release_core_input_provenance(temp_root: Path, producer_commit: str) -> None:
    destination = temp_root / RELEASE_CORE_INPUT_PROVENANCE
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "producer_commit": producer_commit,
                "source": {"kind": "git-archive", "commit": producer_commit},
                "build_outputs": {
                    "kind": "cargo-release-from-clean-producer-checkout",
                    "command": CARGO_BUILD_COMMAND,
                },
                "external_inputs": [],
                "excluded_inputs": [
                    "optional-android-companion",
                    "legacy-openai-bundled-resources",
                    "preexisting-plugin-runtime-fallbacks",
                    "ignored-worktree-overrides",
                ],
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )


def copy_worktree_bundle_files(temp_root: Path) -> None:
    for relative_path in WORKTREE_BUNDLE_FILES:
        source = REPO_ROOT / relative_path
        if not source.exists():
            continue
        destination = temp_root / relative_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def chrome_extension_bundle_ignore(_directory: str, names: list[str]) -> set[str]:
    """Keep local extraction metadata and source maps out of staged bundles."""
    return {name for name in names if name == "_metadata" or name.endswith(".map")}


def latest_chrome_extension_root(source: Path) -> Path:
    candidates: list[tuple[tuple[int, ...], Path]] = []
    for path in source.iterdir():
        manifest_path = path / "manifest.json"
        if not path.is_dir() or path.is_symlink() or not manifest_path.is_file():
            continue
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise RuntimeError(f"invalid Chrome extension manifest at {manifest_path}") from error
        version_value = manifest.get("version")
        if (
            manifest.get("name") not in {"Codex", "ChatGPT"}
            or not manifest.get("key")
            or not isinstance(version_value, str)
            or path.name != f"{version_value}_0"
        ):
            continue
        try:
            version = tuple(int(part) for part in version_value.split("."))
        except ValueError as error:
            raise RuntimeError(f"invalid Chrome extension version at {manifest_path}") from error
        candidates.append((version, path))
    if not candidates:
        raise RuntimeError(f"no Chrome extension manifests found under {source}")
    return max(candidates)[1]


def copy_worktree_bundle_dirs(temp_root: Path) -> None:
    for relative_path in WORKTREE_BUNDLE_DIRS:
        source = REPO_ROOT / relative_path
        if not source.exists():
            continue
        destination = temp_root / relative_path
        if destination.exists():
            shutil.rmtree(destination)
        destination.parent.mkdir(parents=True, exist_ok=True)
        if relative_path == CHROME_EXTENSION_BUNDLE_DIR:
            latest = latest_chrome_extension_root(source / "codex")
            shutil.copytree(
                latest,
                destination / "codex" / latest.name,
                ignore=chrome_extension_bundle_ignore,
            )
        else:
            shutil.copytree(source, destination)


def copy_companion_apk_if_present(temp_root: Path) -> None:
    """Stage the phone companion APK and its identity metadata when present.

    The APK and `phone-companion.json` sidecar are produced by the separate
    Android build lane and are optional packaging inputs. When the APK is
    missing the bundle is still valid (ADB baseline phone-use needs no
    companion), so this skips with a logged note instead of failing. The
    metadata sidecar is staged whenever it exists so the host can verify the
    installed package signature/hash against packaged expectations.
    """
    source_apk = REPO_ROOT / COMPANION_APK_FILE
    if not source_apk.exists():
        print(
            f"note: phone companion APK not found at {source_apk}; "
            "skipping companion bundling (ADB baseline phone-use is unaffected)",
            file=sys.stderr,
        )
        return

    destination_dir = temp_root / COMPANION_APK_DIR
    destination_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_apk, temp_root / COMPANION_APK_FILE)
    print(f"Staged phone companion APK from {source_apk}.")

    source_metadata = REPO_ROOT / COMPANION_APK_METADATA_FILE
    if source_metadata.exists():
        shutil.copy2(source_metadata, temp_root / COMPANION_APK_METADATA_FILE)
        print(f"Staged phone companion metadata from {source_metadata}.")
    else:
        print(
            f"note: phone companion metadata not found at {source_metadata}; "
            "staging APK without packaged identity sidecar",
            file=sys.stderr,
        )


def remove_macos_sidecar_files(root: Path) -> None:
    for path in root.rglob("*:com.apple.*"):
        if path.is_file() or path.is_symlink():
            path.unlink()


def read_c_string(blob: bytes | bytearray, offset: int) -> str:
    if offset < 0 or offset >= len(blob):
        return ""
    end = blob.find(b"\0", offset)
    if end < 0:
        end = len(blob)
    return blob[offset:end].decode("utf-8", errors="replace")


def elf_hash(name: str) -> int:
    value = 0
    for byte in name.encode("utf-8"):
        value = (value << 4) + byte
        high = value & 0xF0000000
        if high:
            value ^= high >> 24
            value &= ~high
    return value & 0xFFFFFFFF


def patch_browser_use_node_repl_glibc_pidfd_symbols(path: Path) -> bool:
    data = bytearray(path.read_bytes())
    if len(data) < 64 or data[:4] != b"\x7fELF":
        return False
    if data[4] != 2 or data[5] != 1:
        return False
    if struct.unpack_from("<H", data, 18)[0] != 62:
        return False

    e_shoff = struct.unpack_from("<Q", data, 40)[0]
    e_shentsize = struct.unpack_from("<H", data, 58)[0]
    e_shnum = struct.unpack_from("<H", data, 60)[0]
    e_shstrndx = struct.unpack_from("<H", data, 62)[0]
    if e_shoff == 0 or e_shentsize < 64 or e_shnum == 0 or e_shstrndx >= e_shnum:
        return False
    if e_shoff + (e_shnum * e_shentsize) > len(data):
        raise RuntimeError("ELF section table is outside file bounds")

    sections: list[dict[str, int]] = []
    for index in range(e_shnum):
        offset = e_shoff + (index * e_shentsize)
        fields = struct.unpack_from("<IIQQQQIIQQ", data, offset)
        sections.append(
            {
                "name_offset": fields[0],
                "type": fields[1],
                "offset": fields[4],
                "size": fields[5],
                "link": fields[6],
                "entsize": fields[9],
            }
        )

    shstr = sections[e_shstrndx]
    shstr_data = data[shstr["offset"] : shstr["offset"] + shstr["size"]]
    by_name = {read_c_string(shstr_data, section["name_offset"]): section for section in sections}

    dynsym = by_name.get(".dynsym")
    dynstr = by_name.get(".dynstr")
    versym = by_name.get(".gnu.version")
    verneed = by_name.get(".gnu.version_r")
    if not dynsym or not dynstr or not versym or not verneed:
        return False
    if dynsym["entsize"] < 24:
        raise RuntimeError("ELF dynamic symbol table has an unsupported entry size")

    dynstr_data = data[dynstr["offset"] : dynstr["offset"] + dynstr["size"]]
    glibc_234_name_offset = dynstr_data.find(b"GLIBC_2.34\0")
    if glibc_234_name_offset < 0:
        return False
    glibc_234_hash = elf_hash("GLIBC_2.34")

    version_names: dict[int, str] = {}
    version_aux_offsets: dict[int, int] = {}
    cursor = verneed["offset"]
    end = verneed["offset"] + verneed["size"]
    while cursor and cursor + 16 <= end:
        vn_version, vn_cnt, _vn_file, vn_aux, vn_next = struct.unpack_from("<HHIII", data, cursor)
        if vn_version == 0 or vn_cnt == 0:
            break
        aux_cursor = cursor + vn_aux
        for _ in range(vn_cnt):
            if aux_cursor + 16 > end:
                raise RuntimeError("ELF version need auxiliary record is outside section bounds")
            _hash, _flags, other, name_offset, aux_next = struct.unpack_from(
                "<IHHII", data, aux_cursor
            )
            version_names[other] = read_c_string(dynstr_data, name_offset)
            version_aux_offsets[other] = aux_cursor
            if aux_next == 0:
                break
            aux_cursor += aux_next
        if vn_next == 0:
            break
        cursor += vn_next

    target_names = {"pidfd_spawnp", "pidfd_getpid"}
    target_version_ids: set[int] = set()
    non_target_glibc_239_refs: list[str] = []
    patched_symbols = 0
    symbol_count = dynsym["size"] // dynsym["entsize"]
    for index in range(symbol_count):
        symbol_offset = dynsym["offset"] + (index * dynsym["entsize"])
        if symbol_offset + 24 > len(data):
            raise RuntimeError("ELF dynamic symbol entry is outside file bounds")
        name_offset, info, _other, shndx = struct.unpack_from("<IBBH", data, symbol_offset)
        name = read_c_string(dynstr_data, name_offset)
        if not name:
            continue
        versym_offset = versym["offset"] + (index * 2)
        if versym_offset + 2 > versym["offset"] + versym["size"]:
            raise RuntimeError("ELF version symbol entry is outside section bounds")
        raw_version = struct.unpack_from("<H", data, versym_offset)[0]
        version_id = raw_version & 0x7FFF
        if version_names.get(version_id) != "GLIBC_2.39":
            continue
        bind = info >> 4
        is_weak_undefined = bind == 2 and shndx == 0
        if name in target_names and is_weak_undefined:
            struct.pack_into("<H", data, versym_offset, 1)
            target_version_ids.add(version_id)
            patched_symbols += 1
        else:
            non_target_glibc_239_refs.append(name)

    if non_target_glibc_239_refs:
        refs = ", ".join(sorted(set(non_target_glibc_239_refs)))
        raise RuntimeError(f"non-pidfd GLIBC_2.39 references remain: {refs}")

    if patched_symbols == 0:
        return False

    for version_id in target_version_ids:
        aux_offset = version_aux_offsets.get(version_id)
        if aux_offset is None:
            raise RuntimeError("GLIBC_2.39 version need record was not found")
        struct.pack_into("<I", data, aux_offset, glibc_234_hash)
        struct.pack_into("<I", data, aux_offset + 8, glibc_234_name_offset)

    path.write_bytes(data)
    return True


def node_repl_ldd_compatible(path: Path) -> bool:
    if not shutil.which("ldd"):
        return True
    result = subprocess.run(
        ["ldd", str(path)],
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    return result.returncode == 0 and not re.search(
        r"=> not found|version .* not found", result.stdout
    )


def install_browser_use_node_repl(source: Path, destination: Path) -> bool:
    if not source.is_file():
        return False
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    ensure_executable(destination)
    try:
        patched = patch_browser_use_node_repl_glibc_pidfd_symbols(destination)
    except RuntimeError as error:
        print(
            f"warning: Browser Use node_repl has unsupported runtime references: {error}",
            file=sys.stderr,
        )
        destination.unlink(missing_ok=True)
        return False
    if patched:
        print("Patched Browser Use node_repl for glibc 2.34+ compatibility.")
    if not node_repl_ldd_compatible(destination):
        print(
            "warning: Browser Use node_repl is not compatible with this host runtime; skipping",
            file=sys.stderr,
        )
        destination.unlink(missing_ok=True)
        return False
    return True


def bundled_resource_root() -> Path:
    configured = os.environ.get("SKY_CUA_UPSTREAM_CODEX_RESOURCES") or os.environ.get(
        "SKY_CUA_OPENAI_BUNDLED_RESOURCE_ROOT"
    )
    if configured:
        configured_path = Path(configured).expanduser()
        bundled_root = configured_path / "plugins" / "openai-bundled"
        if bundled_root.exists():
            return bundled_root
        return configured_path
    return (
        REPO_ROOT.parent
        / "codex-desktop-linux"
        / "codex-app"
        / "resources"
        / "plugins"
        / "openai-bundled"
    )


def build_canonical_browser_client() -> Path:
    package_root = REPO_ROOT / "packages" / "browser-use"
    subprocess.run(["bun", "run", "build"], cwd=package_root, check=True)
    client = REPO_ROOT / CANONICAL_BROWSER_CLIENT
    if not client.is_file():
        raise FileNotFoundError(f"canonical Browser client build is missing: {client}")
    return client


def project_canonical_browser_client(client: Path, projection_root: Path) -> None:
    projection = REPO_ROOT / CANONICAL_BROWSER_PROJECTION
    if not projection.is_file():
        raise FileNotFoundError(f"canonical Browser projection build is missing: {projection}")
    subprocess.run(
        ["bun", str(projection), str(client), str(projection_root)],
        check=True,
        capture_output=True,
        text=True,
    )


def stage_openai_bundled_plugins(temp_root: Path) -> None:
    source_root = bundled_resource_root()
    source_marketplace = source_root / ".agents" / "plugins" / "marketplace.json"
    if not source_marketplace.exists():
        print(
            f"warning: OpenAI bundled plugin marketplace not found at {source_marketplace}; "
            "skipping Chrome/browser-use resources",
            file=sys.stderr,
        )
        return

    destination_root = temp_root / "resources" / "plugins" / "openai-bundled"
    marketplace_destination = destination_root / ".agents" / "plugins" / "marketplace.json"
    marketplace_destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_marketplace, marketplace_destination)
    ensure_openai_bundled_marketplace_entries(marketplace_destination)

    plugins_destination = destination_root / "plugins"
    plugins_destination.mkdir(parents=True, exist_ok=True)
    canonical_browser_client = build_canonical_browser_client()
    staged_plugins: set[str] = set()
    for plugin_name in OPENAI_BUNDLED_PLUGIN_NAMES:
        source_plugin = source_root / "plugins" / plugin_name
        if not source_plugin.exists():
            print(
                f"warning: OpenAI bundled plugin {plugin_name!r} not found at {source_plugin}; skipping",
                file=sys.stderr,
            )
            continue
        destination_plugin = plugins_destination / plugin_name
        shutil.copytree(source_plugin, destination_plugin, dirs_exist_ok=True)
        remove_macos_sidecar_files(destination_plugin)
        staged_plugins.add(plugin_name)

    project_canonical_browser_client(canonical_browser_client, destination_root.parent)
    for plugin_name in set(OPENAI_BUNDLED_PLUGIN_NAMES) - staged_plugins:
        remove_path(plugins_destination / plugin_name)
    for plugin_name in OPENAI_BUNDLED_PLUGIN_NAMES:
        scripts_root = plugins_destination / plugin_name / "scripts"
        if not scripts_root.is_dir():
            continue
        # The canonical client is self-contained. Do not retain upstream native
        # dependencies such as classic-level: incomplete prebuild trees fail at
        # module initialization before the Chrome transport can be selected.
        remove_path(scripts_root / "node_modules")

    source_node_repl = source_root.parents[1] / NODE_REPL_NAME
    destination_node_repl = temp_root / "resources" / NODE_REPL_NAME
    if source_node_repl.exists() and not install_browser_use_node_repl(
        source_node_repl, destination_node_repl
    ):
        print(
            f"warning: OpenAI bundled Browser Use node_repl at {source_node_repl} "
            "could not be staged",
            file=sys.stderr,
        )

    install_bundled_chrome_host(destination_root)


def ensure_openai_bundled_marketplace_entries(marketplace_path: Path) -> None:
    try:
        manifest = json.loads(marketplace_path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return
    except (OSError, json.JSONDecodeError) as e:
        print(f"warning: failed to read marketplace at {marketplace_path}: {e}", file=sys.stderr)
        return

    plugins = manifest.setdefault("plugins", [])
    if not isinstance(plugins, list):
        return
    existing_names = {plugin.get("name") for plugin in plugins if isinstance(plugin, dict)}
    changed = False
    for plugin_name in OPENAI_BUNDLED_MARKETPLACE_PLUGIN_NAMES:
        if plugin_name in existing_names:
            continue
        plugins.append(local_marketplace_plugin_entry(plugin_name))
        changed = True

    if changed:
        marketplace_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def local_marketplace_plugin_entry(plugin_name: str) -> dict[str, object]:
    return {
        "name": plugin_name,
        "source": {
            "source": "local",
            "path": f"./plugins/{plugin_name}",
        },
        "policy": {
            "installation": "AVAILABLE",
            "authentication": "ON_INSTALL",
        },
        "category": "Productivity",
    }


def install_bundled_chrome_host(destination_root: Path) -> None:
    platform_id = current_runtime_platform()
    extension_arch = chrome_extension_host_arch(platform_id)
    if extension_arch is None:
        return

    source_host = cargo_target_root() / "release" / "sky-cua-chrome-host"
    if not source_host.exists():
        print(
            f"warning: sky-cua Chrome host binary not found at {source_host}; "
            "leaving upstream extension host in place",
            file=sys.stderr,
        )
        return

    destination_host = (
        destination_root
        / "plugins"
        / "chrome"
        / "extension-host"
        / "linux"
        / extension_arch
        / "extension-host"
    )
    destination_host.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_host, destination_host)
    ensure_executable(destination_host)


def stage_bundle(bundle_root: Path, *, release_core_commit: str | None = None) -> None:
    temp_root = bundle_root.parent / f".{bundle_root.name}.tmp"
    remove_path(temp_root)
    temp_root.mkdir(parents=True, exist_ok=True)

    if release_core_commit is None:
        copy_tracked_bundle_sources(temp_root)
        copy_worktree_bundle_files(temp_root)
        copy_worktree_bundle_dirs(temp_root)
        copy_companion_apk_if_present(temp_root)
        stage_openai_bundled_plugins(temp_root)
        shutil.copy2(mcp_config_source(), temp_root / ".mcp.json")
    else:
        copy_commit_bundle_sources(temp_root, release_core_commit)

    bin_dir = temp_root / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    if release_core_commit is None:
        for entrypoint_path in bundle_entrypoint_paths():
            source = REPO_ROOT / entrypoint_path
            if source.exists():
                destination = temp_root / entrypoint_path
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
                ensure_executable(destination)

    platform_id = current_runtime_platform()
    for binary_name in platform_runtime_binary_base_names(platform_id):
        source = (
            cargo_target_root() / "release" / runtime_binary_source_name(platform_id, binary_name)
        )
        destination = temp_root / runtime_binary_path(platform_id, binary_name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        ensure_executable(destination)
        copy_build_stamp_sidecar(source, destination)
    if release_core_commit is None:
        for relative_path in all_runtime_binary_paths():
            destination = temp_root / relative_path
            if destination.exists():
                continue
            source = next(
                (
                    candidate
                    for candidate in [
                        bundle_root / relative_path,
                        REPO_ROOT / relative_path,
                    ]
                    if candidate.exists()
                ),
                None,
            )
            if source is None:
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            if not relative_path.name.endswith(".exe"):
                ensure_executable(destination)
            copy_build_stamp_sidecar(source, destination)

    if release_core_commit is not None:
        write_release_core_input_provenance(temp_root, release_core_commit)

    ensure_bundle_structure(temp_root)
    remove_path(bundle_root)
    temp_root.replace(bundle_root)


def main() -> int:
    parser = argparse.ArgumentParser(description="Build a distributable sky-cua plugin bundle.")
    parser.add_argument(
        "--dist-root",
        type=Path,
        default=DIST_PLUGIN_ROOT,
        help="Bundle output directory (default: dist/plugin/sky-cua).",
    )
    parser.add_argument(
        "--release-core-commit",
        help=(
            "build an isolated complete-release core from exactly this clean Git commit, "
            "excluding optional and legacy external inputs"
        ),
    )
    args = parser.parse_args()

    if args.release_core_commit:
        head = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        if head != args.release_core_commit:
            raise ValueError(
                "release core commit must equal current HEAD: "
                f"producer={args.release_core_commit}, head={head}"
            )
        status = subprocess.run(
            ["git", "status", "--porcelain=v1", "--untracked-files=all"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        if status:
            raise ValueError("release core build requires a clean producer working tree")
        if args.dist_root.exists() or args.dist_root.is_symlink():
            raise ValueError("release core output must be an isolated nonexistent path")

    build_release_binaries()
    args.dist_root.parent.mkdir(parents=True, exist_ok=True)
    stage_bundle(args.dist_root, release_core_commit=args.release_core_commit)
    print(args.dist_root)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
