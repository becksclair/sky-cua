#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
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

BUNDLE_SOURCE_PATHS = [
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
WORKTREE_BUNDLE_FILES = (Path("resources") / "chrome_preflight.py",)
WORKTREE_BUNDLE_DIRS = (
    Path("resources") / "chrome-extension",
    Path("skills") / "computer-use-workflows" / "references" / "apps",
)

CARGO_BUILD_PACKAGES = [
    "sky-cua-client",
    "sky-cua-service",
    "sky-cua-chrome-host",
    *([] if sys.platform == "win32" else ["sky-cua-cosmic-helper"]),
]

CARGO_BUILD_COMMAND = [
    "cargo",
    "build",
    "--release",
    *[item for package in CARGO_BUILD_PACKAGES for item in ("--package", package)],
]


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


def build_release_binaries() -> None:
    result = run_cargo_build()
    if result.returncode == 0:
        emit_cargo_output(result)
        return

    if is_windows_sccache_shim_failure(result):
        print(
            "cargo build failed through the Windows sccache shim; retrying without RUSTC_WRAPPER.",
            file=sys.stderr,
        )
        retry = run_cargo_build(env=cargo_env_without_rustc_wrappers())
        if retry.returncode == 0:
            emit_cargo_output(retry)
            return
        emit_cargo_output(result)
        emit_cargo_output(retry)
        retry.check_returncode()

    emit_cargo_output(result)
    result.check_returncode()


def tracked_bundle_files(source_paths: list[Path] | None = None) -> list[Path]:
    paths = source_paths if source_paths is not None else BUNDLE_SOURCE_PATHS
    result = subprocess.run(
        ["git", "ls-files", "-z", "--", *[str(path) for path in paths]],
        cwd=REPO_ROOT,
        check=True,
        stdout=subprocess.PIPE,
    )
    return [
        Path(part.decode("utf-8", errors="replace")) for part in result.stdout.split(b"\0") if part
    ]


def copy_tracked_bundle_sources(temp_root: Path) -> None:
    for relative_path in tracked_bundle_files():
        source = REPO_ROOT / relative_path
        destination = temp_root / relative_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def copy_worktree_bundle_files(temp_root: Path) -> None:
    for relative_path in WORKTREE_BUNDLE_FILES:
        source = REPO_ROOT / relative_path
        if not source.exists():
            continue
        destination = temp_root / relative_path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def copy_worktree_bundle_dirs(temp_root: Path) -> None:
    for relative_path in WORKTREE_BUNDLE_DIRS:
        source = REPO_ROOT / relative_path
        if not source.exists():
            continue
        destination = temp_root / relative_path
        if destination.exists():
            shutil.rmtree(destination)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(source, destination)


def remove_macos_sidecar_files(root: Path) -> None:
    for path in root.rglob("*:com.apple.*"):
        if path.is_file() or path.is_symlink():
            path.unlink()


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

    source_host = REPO_ROOT / "target" / "release" / "sky-cua-chrome-host"
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


def stage_bundle(bundle_root: Path) -> None:
    temp_root = bundle_root.parent / f".{bundle_root.name}.tmp"
    remove_path(temp_root)
    temp_root.mkdir(parents=True, exist_ok=True)

    copy_tracked_bundle_sources(temp_root)
    copy_worktree_bundle_files(temp_root)
    copy_worktree_bundle_dirs(temp_root)
    stage_openai_bundled_plugins(temp_root)
    shutil.copy2(mcp_config_source(), temp_root / ".mcp.json")

    bin_dir = temp_root / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
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
            REPO_ROOT / "target" / "release" / runtime_binary_source_name(platform_id, binary_name)
        )
        destination = temp_root / runtime_binary_path(platform_id, binary_name)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        ensure_executable(destination)
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
    args = parser.parse_args()

    build_release_binaries()
    args.dist_root.parent.mkdir(parents=True, exist_ok=True)
    stage_bundle(args.dist_root)
    print(args.dist_root)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
