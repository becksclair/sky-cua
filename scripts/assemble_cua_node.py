#!/usr/bin/env python3
"""Assemble the sky-cua-owned Linux x64 glibc cua_node component offline."""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import re
import shutil
import subprocess
import tempfile
import uuid
from pathlib import Path
from typing import Any, cast

from _artifact_helpers import (
    canonical_json_bytes,
    sha256_file,
    write_json_durably,
)
from _cua_node_compliance import (
    _generate_compliance,
)
from _cua_node_inputs import (
    ASSEMBLER_SOURCES,
    BROWSER_VERSION,
    FIRST_PARTY_BUILD_COMMANDS,
    HOST_VERSION,
    MIGRATION_COMMIT,
    NODE_VERSION,
    SEED_MEMBERS,
    SKY_CUA_VERSION,
    TARGET,
)
from _cua_node_locks import (
    _generate_locks,
)
from _cua_node_packages import (
    _import_prepared_packages,
    _install_first_party_packages,
    _install_libvips_compliance_records,
    _remove_wrong_platform_content,
)
from _cua_node_seed import (
    _cache_root,
    _resolve_seed,
)
from _cua_node_tree import (
    AssemblyError,
    _canonical_json,
    _copy_tree,
    _file_manifest,
    _fsync_directory,
    _fsync_tree,
    _normalize_checkout_text_paths,
    _normalize_tree,
    _sha256_bytes,
    _tree_hash,
)
from _plugin_bundle import REPO_ROOT, remove_path


def _rebuild_first_party_outputs() -> dict[str, object]:
    for command in FIRST_PARTY_BUILD_COMMANDS:
        subprocess.run(
            cast(list[str], command["argv"]),
            cwd=REPO_ROOT / cast(str, command["cwd"]),
            check=True,
            capture_output=True,
            text=True,
        )
    bun_version = subprocess.run(
        ["bun", "--version"], check=True, capture_output=True, text=True
    ).stdout.strip()
    compiler_version = subprocess.run(
        ["cc", "--version"], check=True, capture_output=True, text=True
    ).stdout.splitlines()[0]
    runtime_cli = REPO_ROOT / "runtime" / "cua-node" / "dist" / "cli.js"
    browser_build = REPO_ROOT / "packages" / "browser-use" / "build"
    browser_client = browser_build / "browser-client.mjs"
    sky_tarball = REPO_ROOT / "packages" / "sky-cua-js" / "out" / "sky-cua-0.1.0.tgz"
    for path in (runtime_cli, browser_client, sky_tarball):
        if not path.is_file() or path.is_symlink():
            raise AssemblyError(
                f"first-party build output is missing: {path.relative_to(REPO_ROOT)}"
            )
    browser_tree_sha256, browser_tree_size, browser_tree_files = _tree_hash(browser_build)
    return {
        "schema_version": 1,
        "commands": list(FIRST_PARTY_BUILD_COMMANDS),
        "toolchain": {"bun": bun_version, "cc": compiler_version},
        "outputs": {
            "runtime_cli": {
                "path": "runtime/cua-node/dist/cli.js",
                "sha256": sha256_file(runtime_cli),
                "size_bytes": runtime_cli.stat().st_size,
            },
            "browser_client": {
                "path": "packages/browser-use/build/browser-client.mjs",
                "sha256": sha256_file(browser_client),
                "size_bytes": browser_client.stat().st_size,
            },
            "browser_build": {
                "path": "packages/browser-use/build",
                "tree_sha256": browser_tree_sha256,
                "size_bytes": browser_tree_size,
                "file_count": browser_tree_files,
            },
            "sky_cua_tarball": {
                "path": "packages/sky-cua-js/out/sky-cua-0.1.0.tgz",
                "sha256": sha256_file(sky_tarball),
                "size_bytes": sky_tarball.stat().st_size,
            },
        },
    }


def _producer_source_inventory() -> list[dict[str, object]]:
    roots = (
        REPO_ROOT / "runtime" / "cua-node",
        REPO_ROOT / "packages" / "browser-use",
        REPO_ROOT / "packages" / "sky-cua-js",
        REPO_ROOT / "resources" / "release" / "license-texts",
    )
    inventory: list[dict[str, object]] = []
    for root in roots:
        for path in sorted(root.rglob("*"), key=lambda item: item.as_posix()):
            local_parts = path.relative_to(root).parts
            if "node_modules" in local_parts or "out" in local_parts:
                continue
            if path.is_symlink():
                raise AssemblyError(f"producer source tree contains symlink: {path}")
            if not path.is_file():
                continue
            relative = path.relative_to(REPO_ROOT).as_posix()
            inventory.append(
                {
                    "path": relative,
                    "sha256": sha256_file(path),
                    "size_bytes": path.stat().st_size,
                }
            )
    for relative in ASSEMBLER_SOURCES:
        assembler = REPO_ROOT / relative
        inventory.append(
            {
                "path": relative,
                "sha256": sha256_file(assembler),
                "size_bytes": assembler.stat().st_size,
            }
        )
    inventory.sort(key=lambda item: cast(str, item["path"]))
    return inventory


def _compile_launcher(root: Path) -> None:
    destination = root / "bin" / "node_repl"
    subprocess.run(
        [
            "cc",
            "-Os",
            "-nostdlib",
            "-static",
            "-ffreestanding",
            "-fno-builtin",
            "-fno-stack-protector",
            "-fno-pie",
            "-no-pie",
            "-Wl,--build-id=none",
            "-o",
            str(destination),
            str(REPO_ROOT / "runtime" / "cua-node" / "native" / "node_repl.c"),
        ],
        check=True,
    )


def _write_manifest(
    root: Path,
    *,
    producer_commit: str,
    sky_tarball_sha256: str,
    browser_sha256: str,
    runtime_lock: bytes,
    native_lock: bytes,
    migration_input: dict[str, object],
) -> None:
    host_hash = sha256_file(root / "lib" / "node_repl" / "cli.js")
    manifest = {
        "schema_version": 2,
        "manifest_version": 1,
        "runtime_name": "cua_node",
        "platform": "linux",
        "arch": "x64",
        "libc": "glibc",
        "target": TARGET,
        "node_version": NODE_VERSION,
        "node_path": "bin/node",
        "node_sha256": sha256_file(root / "bin" / "node"),
        "node_repl_version": HOST_VERSION,
        "node_repl_path": "bin/node_repl",
        "node_repl_sha256": sha256_file(root / "bin" / "node_repl"),
        "node_modules": "lib/node_modules",
        "data": {
            "playwright": "share/playwright",
            "tessdata": "share/tessdata",
            "pdfjs": "share/pdfjs",
            "licenses": "licenses",
            "sbom": "sbom.cdx.json",
        },
        "components": {
            "host": {
                "name": "@heliasar/cua-node-host",
                "version": HOST_VERSION,
                "build_id": host_hash[:16],
                "sha256": host_hash,
            },
            "kernel": {
                "name": "@heliasar/cua-node-kernel",
                "version": HOST_VERSION,
                "build_id": host_hash[:16],
                "sha256": host_hash,
            },
            "sky_cua": {
                "package_name": "@heliasar/sky-cua",
                "package_version": SKY_CUA_VERSION,
                "entrypoint": "lib/node_modules/@heliasar/sky-cua/dist/index.js",
                "tarball_sha256": sky_tarball_sha256,
            },
            "browser_use": {
                "package_name": "@heliasar/browser-use",
                "package_version": BROWSER_VERSION,
                "entrypoint": "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs",
                "declarations": "lib/node_modules/@heliasar/browser-use/build/index.d.ts",
            },
        },
        "browser": {"revision": f"system-chromium-browser-use-{BROWSER_VERSION}"},
        "source": {
            "producer": "sky-cua",
            "producer_commit": producer_commit,
            "migration_evidence": {"codex_desktop_commit": MIGRATION_COMMIT},
            "migration_input": migration_input,
        },
        "checksums": {
            "algorithm": "sha256",
            "files": _file_manifest(root, excluded={"manifest.json"}),
            "lock_hashes": {
                "runtime_lock_sha256": _sha256_bytes(runtime_lock),
                "native_assets_lock_sha256": _sha256_bytes(native_lock),
            },
        },
    }
    (root / "manifest.json").write_bytes(_canonical_json(manifest))


def _compose(
    seed: Path,
    staging: Path,
    producer_commit: str,
    *,
    release_eligible: bool,
    first_party_build: dict[str, object],
) -> tuple[Path, Path]:
    migration_input_path = seed / "SKY_CUA_MIGRATION_INPUT.json"
    migration_input = cast(
        dict[str, object], json.loads(migration_input_path.read_text(encoding="utf-8"))
    )
    for member in SEED_MEMBERS:
        source = seed / member
        destination = staging / member
        if source.is_dir():
            _copy_tree(source, destination)
        elif source.is_file() and not source.is_symlink():
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        else:
            raise AssemblyError(f"migration seed member is missing: {member}")
    remove_path(staging / "manifest.json")
    host = REPO_ROOT / "runtime" / "cua-node" / "dist" / "cli.js"
    if not host.is_file():
        raise AssemblyError("runtime/cua-node/dist/cli.js is missing; run bun run build")
    (staging / "lib" / "node_repl").mkdir(parents=True, exist_ok=True)
    shutil.copy2(host, staging / "lib" / "node_repl" / "cli.js")
    (staging / "lib" / "node_repl" / "package.json").write_bytes(
        _canonical_json(
            {"name": "@heliasar/cua-node-host-bundle", "private": True, "type": "module"}
        )
    )
    _compile_launcher(staging)
    cast(dict[str, object], first_party_build["outputs"])["node_repl_launcher"] = {
        "path": "bin/node_repl",
        "sha256": sha256_file(staging / "bin" / "node_repl"),
        "size_bytes": (staging / "bin" / "node_repl").stat().st_size,
    }
    sky_hash, browser_hash = _install_first_party_packages(staging)
    _import_prepared_packages(staging)
    _install_libvips_compliance_records(staging)
    _remove_wrong_platform_content(staging / "lib" / "node_modules")
    _normalize_checkout_text_paths(staging)
    _generate_compliance(
        staging,
        producer_commit=producer_commit,
        browser_hash=browser_hash,
        migration_input=migration_input,
    )
    source_inventory = _producer_source_inventory()
    attestation_path = staging / "share" / "provenance" / "SKY_CUA_BUILD_ATTESTATION.json"
    attestation_path.parent.mkdir(parents=True, exist_ok=True)
    attestation_path.write_bytes(
        _canonical_json(
            {
                "schema_version": 1,
                "release_eligible": release_eligible,
                "producer_commit": producer_commit,
                "migration_input": migration_input,
                "first_party_build": first_party_build,
                "source_inventory_sha256": _sha256_bytes(canonical_json_bytes(source_inventory)),
                "source_inventory": source_inventory,
            }
        )
    )
    _normalize_tree(staging)
    seed_sha256 = cast(str, migration_input["source_tree_sha256"])
    runtime_lock, native_lock = _generate_locks(staging, seed_sha256=seed_sha256)
    locks = staging / "share" / "locks"
    locks.mkdir()
    (locks / "runtime-lock.json").write_bytes(runtime_lock)
    (locks / "native-assets.lock.json").write_bytes(native_lock)
    _normalize_tree(staging)
    _write_manifest(
        staging,
        producer_commit=producer_commit,
        sky_tarball_sha256=sky_hash,
        browser_sha256=browser_hash,
        runtime_lock=runtime_lock,
        native_lock=native_lock,
        migration_input=migration_input,
    )
    _normalize_tree(staging)
    return locks / "runtime-lock.json", locks / "native-assets.lock.json"


def _verify(root: Path, runtime_lock: Path, native_lock: Path) -> dict[str, Any]:
    command = [
        "bun",
        str(REPO_ROOT / "runtime" / "cua-node" / "tools" / "verify-cua-node.ts"),
        f"--root={root}",
        f"--target={TARGET}",
        f"--enforce-lock={runtime_lock}",
        f"--enforce-lock={native_lock}",
        "--json",
    ]
    result = subprocess.run(command, cwd=REPO_ROOT, check=False, text=True, capture_output=True)
    if result.returncode != 0:
        try:
            failed_report = json.loads(result.stdout)
            failures = [
                f"{check.get('id')}: {check.get('detail')}"
                for check in failed_report.get("checks", [])
                if check.get("status") != "passed"
            ]
        except json.JSONDecodeError:
            failures = []
        raise AssemblyError(
            "cua_node verifier failed:\n"
            + ("\n".join(failures) if failures else result.stdout[-12000:])
            + result.stderr[-12000:]
        )
    report = json.loads(result.stdout)
    if report.get("status") != "passed":
        raise AssemblyError("cua_node verifier did not report passed")
    expected_identity = f"node_repl/{HOST_VERSION}"
    for identity_command in (
        [str(root / "bin" / "node_repl"), "--version"],
        [str(root / "bin" / "node"), str(root / "lib" / "node_repl" / "cli.js"), "--version"],
    ):
        identity = subprocess.run(
            identity_command,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        if identity != expected_identity:
            raise AssemblyError(
                f"node_repl identity mismatch: expected {expected_identity}, got {identity}"
            )

    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    lock = json.loads(runtime_lock.read_text(encoding="utf-8"))
    versions = {
        manifest["node_repl_version"],
        manifest["components"]["host"]["version"],
        manifest["components"]["kernel"]["version"],
        lock["runtime"]["version"],
        *(item["version"] for item in lock["outputs"]),
    }
    if versions != {HOST_VERSION}:
        raise AssemblyError(f"node_repl version contract diverges: {sorted(versions)}")
    return cast(dict[str, Any], report)


def _verify_installed_transcript(root: Path) -> None:
    transcript_command = [
        "bun",
        str(
            REPO_ROOT / "runtime" / "cua-node" / "production" / "installed-transcript-acceptance.ts"
        ),
        f"--runtime-root={root}",
        "--json",
    ]
    transcript = subprocess.run(
        transcript_command,
        cwd=REPO_ROOT,
        check=False,
        text=True,
        capture_output=True,
    )
    try:
        transcript_report = json.loads(transcript.stdout)
    except json.JSONDecodeError as error:
        raise AssemblyError(
            "installed node_repl transcript acceptance returned invalid JSON:\n"
            + transcript.stdout[-12000:]
            + transcript.stderr[-12000:]
        ) from error
    if transcript.returncode != 0 or transcript_report.get("status") != "passed":
        raise AssemblyError(
            "installed node_repl transcript acceptance failed:\n"
            + json.dumps(transcript_report, indent=2)[-12000:]
            + transcript.stderr[-12000:]
        )


def _recover_publication(output: Path, journal: Path) -> None:
    if not journal.exists():
        return
    value = json.loads(journal.read_text(encoding="utf-8"))
    if value.get("schema_version") != 1 or value.get("output") != output.name:
        raise AssemblyError("cua_node publication journal is invalid")
    staging_name = value.get("staging")
    backup_name = value.get("backup")
    staging_pattern = re.compile(rf"^\.{re.escape(output.name)}\.staging-[0-9a-f]{{32}}$")
    backup_pattern = re.compile(rf"^\.{re.escape(output.name)}\.backup-[0-9a-f]{{32}}$")
    if (
        not isinstance(staging_name, str)
        or not isinstance(backup_name, str)
        or Path(staging_name).name != staging_name
        or Path(backup_name).name != backup_name
        or staging_pattern.fullmatch(staging_name) is None
        or backup_pattern.fullmatch(backup_name) is None
    ):
        raise AssemblyError("cua_node publication journal paths are invalid")
    staging = output.parent / staging_name
    backup = output.parent / backup_name
    phase = value.get("phase")
    if phase == "promoting":
        if backup.exists():
            remove_path(output)
            os.replace(backup, output)
            remove_path(staging)
        elif staging.exists():
            # The old output has not moved yet (or there was no prior output).
            # Keeping an existing output and discarding the staged candidate is rollback.
            remove_path(staging)
        else:
            # A first install moved staging into output before the commit point.
            remove_path(output)
    elif phase == "committed":
        remove_path(backup)
        remove_path(staging)
    else:
        raise AssemblyError("cua_node publication journal phase is invalid")
    journal.unlink()
    _fsync_directory(output.parent)


def assemble(
    *,
    cache: Path,
    seed_argument: Path | None,
    output: Path,
    producer_commit: str,
    check: bool,
    allow_development_dirty: bool = False,
) -> dict[str, Any]:
    release_eligible = _assert_producer_sources(
        producer_commit, allow_development_dirty=allow_development_dirty
    )
    output = output.expanduser()
    if output.is_symlink():
        raise AssemblyError(f"output root must not be a symlink: {output}")
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    lock_path = output.parent / ".cua-node-assembly.lock"
    journal = output.parent / ".cua-node-assembly-journal.json"
    with lock_path.open("a+b") as lock:
        lock_path.chmod(0o600)
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        _recover_publication(output, journal)
        first_party_build = _rebuild_first_party_outputs()
        seed = _resolve_seed(cache, seed_argument)
        workspace = Path(tempfile.mkdtemp(prefix="cua-node-assemble-", dir=output.parent))
        staging = workspace / "cua-node-linux-x64-glibc"
        staging.mkdir()
        try:
            runtime_lock, native_lock = _compose(
                seed,
                staging,
                producer_commit,
                release_eligible=release_eligible,
                first_party_build=first_party_build,
            )
            report = _verify(staging, runtime_lock, native_lock)
            _verify_installed_transcript(staging)
            tree_sha256, size_bytes, file_count = _tree_hash(staging)
            if check:
                if not output.is_dir() or output.is_symlink():
                    raise AssemblyError(f"assembled output is missing: {output}")
                _verify(
                    output,
                    output / "share" / "locks" / "runtime-lock.json",
                    output / "share" / "locks" / "native-assets.lock.json",
                )
                current_sha256, _, _ = _tree_hash(output)
                if current_sha256 != tree_sha256:
                    raise AssemblyError(
                        f"assembled output drift: expected {tree_sha256}, got {current_sha256}"
                    )
                return {
                    "status": "checked",
                    "output": str(output),
                    "tree_sha256": tree_sha256,
                    "file_count": file_count,
                    "size_bytes": size_bytes,
                    "verification": report["status"],
                    "release_eligible": release_eligible,
                }
            _fsync_tree(staging)
            backup = output.parent / f".{output.name}.backup-{uuid.uuid4().hex}"
            publication_staging = output.parent / f".{output.name}.staging-{uuid.uuid4().hex}"
            os.replace(staging, publication_staging)
            write_json_durably(
                journal,
                {
                    "schema_version": 1,
                    "phase": "promoting",
                    "output": output.name,
                    "staging": publication_staging.name,
                    "backup": backup.name,
                },
            )
            if output.exists():
                os.replace(output, backup)
            os.replace(publication_staging, output)
            _fsync_directory(output.parent)
            _verify(
                output,
                output / "share" / "locks" / "runtime-lock.json",
                output / "share" / "locks" / "native-assets.lock.json",
            )
            write_json_durably(
                journal,
                {
                    "schema_version": 1,
                    "phase": "committed",
                    "output": output.name,
                    "staging": publication_staging.name,
                    "backup": backup.name,
                },
            )
            remove_path(backup)
            journal.unlink()
            _fsync_directory(output.parent)
            return {
                "status": "prepared" if release_eligible else "development-prepared",
                "output": str(output),
                "tree_sha256": tree_sha256,
                "file_count": file_count,
                "size_bytes": size_bytes,
                "verification": report["status"],
                "release_eligible": release_eligible,
            }
        finally:
            remove_path(workspace)


def _producer_commit(argument: str | None) -> str:
    if argument is not None:
        value = argument
    else:
        value = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    if re.fullmatch(r"[0-9a-f]{40}", value) is None:
        raise AssemblyError("producer commit must be a full 40-character Git commit")
    return value


def _assert_producer_sources(commit: str, *, allow_development_dirty: bool) -> bool:
    required_paths = (
        *ASSEMBLER_SOURCES,
        "runtime/cua-node/src/cli.ts",
        "runtime/cua-node/src/host/runtime-manager.ts",
        "runtime/cua-node/src/kernel/kernel.ts",
        "runtime/cua-node/native/node_repl.c",
        "packages/browser-use/src/index.ts",
        "packages/sky-cua-js/src/index.ts",
    )
    commit_check = subprocess.run(
        ["git", "cat-file", "-e", f"{commit}^{{commit}}"],
        cwd=REPO_ROOT,
        check=False,
        capture_output=True,
    )
    if commit_check.returncode != 0:
        raise AssemblyError(f"producer commit does not exist: {commit}")
    for path in required_paths:
        exists = subprocess.run(
            ["git", "cat-file", "-e", f"{commit}:{path}"],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
        )
        if exists.returncode != 0:
            if allow_development_dirty:
                return False
            raise AssemblyError(f"producer commit does not contain required source: {path}")
    clean = subprocess.run(
        [
            "git",
            "diff",
            "--quiet",
            commit,
            "--",
            *ASSEMBLER_SOURCES,
            "runtime/cua-node",
            "packages/browser-use",
            "packages/sky-cua-js",
        ],
        cwd=REPO_ROOT,
        check=False,
    )
    if clean.returncode != 0:
        if allow_development_dirty:
            return False
        raise AssemblyError("producer source paths differ from the bound producer commit")
    status = subprocess.run(
        [
            "git",
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            *ASSEMBLER_SOURCES,
            "runtime/cua-node",
            "packages/browser-use",
            "packages/sky-cua-js",
        ],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if status:
        if allow_development_dirty:
            return False
        raise AssemblyError("producer source paths contain uncommitted or untracked files")
    return True


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-root", type=Path)
    parser.add_argument("--seed-runtime", type=Path)
    parser.add_argument(
        "--output-root",
        type=Path,
        default=REPO_ROOT / "out" / "components" / "cua-node-linux-x64-glibc",
    )
    parser.add_argument("--producer-commit")
    parser.add_argument("--target", default=TARGET)
    parser.add_argument("--check", action="store_true")
    parser.add_argument(
        "--allow-development-dirty",
        action="store_true",
        help="Build a non-release-eligible validation candidate before the producer commit exists.",
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    if args.target != TARGET:
        raise SystemExit(f"unsupported target: {args.target}; only {TARGET} is complete")
    try:
        result = assemble(
            cache=_cache_root(args.cache_root),
            seed_argument=args.seed_runtime,
            output=args.output_root,
            producer_commit=_producer_commit(args.producer_commit),
            check=args.check,
            allow_development_dirty=args.allow_development_dirty,
        )
    except (AssemblyError, OSError, subprocess.SubprocessError, json.JSONDecodeError) as error:
        if args.json:
            print(json.dumps({"status": "error", "error": str(error)}, sort_keys=True))
        else:
            print(f"error: {error}")
        return 1
    if args.json:
        print(json.dumps(result, sort_keys=True))
    else:
        for key, value in result.items():
            print(f"{key}={value}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
