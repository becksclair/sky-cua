#!/usr/bin/env python3
"""Build and install the standalone fixed-root sky-cua distribution."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tarfile
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Any

import _opencode_config

REPO_ROOT = Path(__file__).resolve().parents[1]
TARGET = "linux-x64-glibc"
PRODUCT_VERSION = "0.1.1"
ARCHIVE_NAME = f"sky-cua-{TARGET}.tar.gz"
PAYLOAD_DIR_NAME = f"sky-cua-{TARGET}"
SKILL_NAMES = ("computer-use", "browser-use", "phone-use")
PLUGIN_NAMES = ("computer-use", "browser")
LAUNCHER_NAMES = (
    "sky-cua-client",
    "sky-cua-service",
    "sky-cua-overlay-host",
    "node_repl",
    "sky-cua-chrome-host",
)
NATIVE_HOST_NAME = "com.openai.codexextension"
EXTENSION_ID = "hehggadaopoacecdllhhajmbjkdcmajg"
NATIVE_MANIFEST_DIRS = (
    Path(".config/google-chrome/NativeMessagingHosts"),
    Path(".config/chromium/NativeMessagingHosts"),
    Path(".config/BraveSoftware/Brave-Browser/NativeMessagingHosts"),
    Path(".config/BraveSoftware/Brave-Origin/NativeMessagingHosts"),
)

Runner = Callable[..., subprocess.CompletedProcess[str]]


def _run(command: Sequence[str], *, cwd: Path = REPO_ROOT) -> None:
    print(f"+ {' '.join(command)}")
    subprocess.run(command, cwd=cwd, check=True, text=True)


def _remove_path(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink(missing_ok=True)
    elif path.is_dir():
        shutil.rmtree(path)


def _copy_tree(source: Path, destination: Path) -> None:
    if source.is_symlink() or not source.is_dir():
        raise FileNotFoundError(f"required directory is missing: {source}")
    shutil.copytree(source, destination, dirs_exist_ok=True, symlinks=False)


def _copy_file(source: Path, destination: Path, *, executable: bool = False) -> None:
    if source.is_symlink() or not source.is_file():
        raise FileNotFoundError(f"required file is missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    if executable:
        destination.chmod(destination.stat().st_mode | 0o111)


def _single_extension_root(core_root: Path) -> Path:
    parent = core_root / "resources/chrome-extension/codex"
    candidates = sorted(
        path for path in parent.iterdir() if path.is_dir() and not path.is_symlink()
    )
    if len(candidates) != 1:
        raise ValueError(f"expected one Chrome extension tree under {parent}, found {candidates}")
    return candidates[0]


def _write_release_manifest(payload_root: Path) -> None:
    value = {
        "schema_version": 1,
        "product": "sky-cua",
        "version": PRODUCT_VERSION,
        "target": TARGET,
        "paths": {
            "computer_use": "bin/sky-cua-client",
            "service": "bin/sky-cua-service",
            "node": "bin/node",
            "node_repl": "bin/node_repl",
            "browser_client": "browser/browser-client.mjs",
            "browser_extension": "browser/extension",
            "browser_native_host": "browser/native-host/sky-cua-chrome-host",
            "codex_marketplace": "codex/openai-bundled/.agents/plugins/marketplace.json",
            "skills": "skills",
            "documentation": "docs",
        },
    }
    (payload_root / "RELEASE.json").write_text(
        json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )


def assemble_payload(payload_root: Path, *, core_root: Path, cua_node_root: Path) -> None:
    """Flatten the already-built core and Node runtime into one install tree."""
    _remove_path(payload_root)
    _copy_tree(core_root, payload_root)
    _remove_path(payload_root / "docs")
    _remove_path(payload_root / "resources/release")
    _remove_path(payload_root / "resources/model-documentation")
    for name in ("bin", "lib", "share", "licenses"):
        source = cua_node_root / name
        if source.exists():
            _copy_tree(source, payload_root / name)
    for name in ("manifest.json", "sbom.cdx.json"):
        source = cua_node_root / name
        if source.exists():
            _copy_file(source, payload_root / name)

    browser_build = REPO_ROOT / "packages/browser-use/build"
    _copy_file(browser_build / "browser-client.mjs", payload_root / "browser/browser-client.mjs")
    _copy_tree(_single_extension_root(core_root), payload_root / "browser/extension")
    _remove_path(payload_root / "resources/chrome-extension")
    native_host = core_root / "bin/runtimes/linux-x64/sky-cua-chrome-host"
    _copy_file(
        native_host,
        payload_root / "browser/native-host/sky-cua-chrome-host",
        executable=True,
    )

    compat = REPO_ROOT / "resources/codex-compat/openai-bundled"
    _copy_tree(compat, payload_root / "codex/openai-bundled")
    _copy_tree(REPO_ROOT / "skills", payload_root / "skills")
    _copy_tree(REPO_ROOT / "out/components/model-documentation", payload_root / "docs")

    artifact_scripts = payload_root / "scripts"
    artifact_scripts.mkdir(parents=True, exist_ok=True)
    _copy_file(REPO_ROOT / "install.py", payload_root / "install.py", executable=True)
    _copy_file(Path(__file__), artifact_scripts / "standalone_release.py", executable=True)
    _copy_file(
        REPO_ROOT / "scripts/_codex_app_server.py", artifact_scripts / "_codex_app_server.py"
    )
    _copy_file(REPO_ROOT / "scripts/_opencode_config.py", artifact_scripts / "_opencode_config.py")
    _write_release_manifest(payload_root)
    validate_payload(payload_root)


def validate_payload(payload_root: Path) -> None:
    manifest = json.loads((payload_root / "RELEASE.json").read_text(encoding="utf-8"))
    if manifest.get("schema_version") != 1 or manifest.get("target") != TARGET:
        raise ValueError("standalone RELEASE.json is incompatible")
    required = (
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
        "codex/openai-bundled/plugins/computer-use/.mcp.json",
        "codex/openai-bundled/plugins/browser/.codex-plugin/plugin.json",
        "codex/openai-bundled/plugins/browser/.mcp.json",
        "codex/openai-bundled/plugins/browser/scripts/browser-client.mjs",
        "codex/openai-bundled/plugins/browser/skills/control-in-app-browser/SKILL.md",
        "skills/computer-use/SKILL.md",
        "skills/browser-use/SKILL.md",
        "skills/phone-use/SKILL.md",
        "docs/README.md",
        "docs/inventories/api-inventory.json",
        "docs/inventories/capability-inventory.json",
        "docs/inventories/example-inventory.json",
        "docs/inventories/routing-inventory.json",
        "install.py",
        "scripts/standalone_release.py",
        "scripts/_opencode_config.py",
    )
    missing = [relative for relative in required if not (payload_root / relative).is_file()]
    if missing:
        raise FileNotFoundError(f"standalone payload is incomplete: {missing}")
    marketplace_root = payload_root / "codex/openai-bundled"
    marketplace = json.loads(
        (marketplace_root / ".agents/plugins/marketplace.json").read_text(encoding="utf-8")
    )
    plugins = marketplace.get("plugins")
    if not isinstance(plugins, list) or [
        plugin.get("name") for plugin in plugins if isinstance(plugin, dict)
    ] != list(PLUGIN_NAMES):
        raise ValueError(f"standalone Codex marketplace must expose exactly {list(PLUGIN_NAMES)}")
    for plugin_name, plugin in zip(PLUGIN_NAMES, plugins, strict=True):
        expected_path = f"./plugins/{plugin_name}"
        source = plugin.get("source")
        if not isinstance(source, dict) or source != {
            "source": "local",
            "path": expected_path,
        }:
            raise ValueError(f"standalone Codex plugin {plugin_name} must use {expected_path}")
        plugin_root = marketplace_root / "plugins" / plugin_name
        plugin_manifest = json.loads(
            (plugin_root / ".codex-plugin/plugin.json").read_text(encoding="utf-8")
        )
        if plugin_manifest.get("name") != plugin_name:
            raise ValueError(f"standalone Codex plugin manifest name does not match {plugin_name}")
    plugin_dirs = sorted(
        path.name
        for path in (marketplace_root / "plugins").iterdir()
        if path.is_dir() and not path.is_symlink()
    )
    if plugin_dirs != sorted(PLUGIN_NAMES):
        raise ValueError(
            f"standalone Codex plugin tree must contain exactly {sorted(PLUGIN_NAMES)}"
        )
    forbidden = ("releases", "current", "activation-receipt.json", "promotion-journal.json")
    present = [name for name in forbidden if (payload_root / name).exists()]
    if present:
        raise ValueError(f"standalone payload contains generation state: {present}")
    retired_paths = (
        "resources/release",
        "resources/model-documentation",
        "resources/chrome-extension",
    )
    retained = [relative for relative in retired_paths if (payload_root / relative).exists()]
    if retained:
        raise ValueError(f"standalone payload contains retired producer contracts: {retained}")
    forbidden_contract_terms = (
        "".join(("NODE_REPL_TRUSTED_", "BROWSER_CLIENT_SHA256S")),
        "".join(("trusted_browser_", "client_sha256s")),
        "".join(("resolve", "-active")),
    )
    contract_roots = [payload_root / "docs", payload_root / "codex", payload_root / "scripts"]
    contract_files = [payload_root / "RELEASE.json"]
    for root in contract_roots:
        contract_files.extend(path for path in root.rglob("*") if path.is_file())
    for path in contract_files:
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
        matches = [term for term in forbidden_contract_terms if term in text]
        if matches:
            raise ValueError(
                f"standalone payload contains retired contract terms in {path}: {matches}"
            )


def _archive_payload(payload_root: Path, archive_path: Path) -> None:
    archive_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = archive_path.with_name(f".{archive_path.name}.tmp-{os.getpid()}")
    _remove_path(temporary)
    with tarfile.open(temporary, "w:gz", format=tarfile.PAX_FORMAT) as archive:
        archive.add(payload_root, arcname=PAYLOAD_DIR_NAME, recursive=True)
    os.replace(temporary, archive_path)


def build_payload(
    output_root: Path,
    *,
    create_archive: bool,
    runner: Callable[[Sequence[str]], None] = _run,
) -> tuple[Path, Path | None]:
    """Own every generated input, assemble one payload, and optionally archive it."""
    output_root = output_root.expanduser().resolve()
    output_root.mkdir(parents=True, exist_ok=True)
    cua_node = REPO_ROOT / "out/components/cua-node-linux-x64-glibc"
    core = output_root / "plugin/sky-cua"
    for workspace in ("runtime/cua-node", "packages/browser-use", "packages/sky-cua-js"):
        runner(
            (
                "bun",
                "install",
                "--frozen-lockfile",
                f"--cwd={REPO_ROOT / workspace}",
            )
        )
    runner(
        (
            sys.executable,
            str(REPO_ROOT / "scripts/build_model_documentation.py"),
            "--output",
            str(REPO_ROOT / "out/components/model-documentation"),
        )
    )
    runner(
        (
            sys.executable,
            str(REPO_ROOT / "scripts/assemble_cua_node.py"),
            "--output-root",
            str(cua_node),
            "--allow-development-dirty",
            "--json",
        )
    )
    runner(
        (
            sys.executable,
            str(REPO_ROOT / "scripts/build_plugin.py"),
            "--dist-root",
            str(core),
        )
    )
    payload = output_root / "standalone" / PAYLOAD_DIR_NAME
    assemble_payload(payload, core_root=core, cua_node_root=cua_node)
    archive = output_root / ARCHIVE_NAME if create_archive else None
    if archive is not None:
        _archive_payload(payload, archive)
    return payload, archive


def _data_root(env: Mapping[str, str], home: Path) -> Path:
    configured = env.get("XDG_DATA_HOME", "").strip()
    return Path(configured).expanduser() if configured else home / ".local/share"


def _replace_symlink(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(f".{destination.name}.tmp-{os.getpid()}")
    _remove_path(temporary)
    temporary.symlink_to(source, target_is_directory=source.is_dir())
    _remove_path(destination)
    os.replace(temporary, destination)


def _install_launchers(install_root: Path, home: Path) -> tuple[Path, ...]:
    bin_root = home / ".local/bin"
    legacy_node = bin_root / "node"
    if legacy_node.is_symlink():
        try:
            legacy_target = legacy_node.resolve(strict=False)
        except RuntimeError:
            legacy_target = None
        if legacy_target is not None and legacy_target.is_relative_to(install_root.resolve()):
            legacy_node.unlink()
    targets = {
        "sky-cua-client": install_root / "bin/sky-cua-client",
        "sky-cua-service": install_root / "bin/sky-cua-service",
        "sky-cua-overlay-host": install_root / "bin/sky-cua-overlay-host",
        "node_repl": install_root / "bin/node_repl",
        "sky-cua-chrome-host": install_root / "browser/native-host/sky-cua-chrome-host",
    }
    for name in LAUNCHER_NAMES:
        target = targets[name]
        if not target.is_file():
            raise FileNotFoundError(f"launcher target is missing: {target}")
        _replace_symlink(target, bin_root / name)
    return tuple(bin_root / name for name in LAUNCHER_NAMES)


def _install_native_manifests(home: Path, launcher: Path) -> tuple[Path, ...]:
    value = {
        "name": NATIVE_HOST_NAME,
        "description": "sky-cua browser automation native host",
        "path": str(launcher),
        "type": "stdio",
        "allowed_origins": [f"chrome-extension://{EXTENSION_ID}/"],
    }
    content = json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n"
    paths: list[Path] = []
    for relative in NATIVE_MANIFEST_DIRS:
        path = home / relative / f"{NATIVE_HOST_NAME}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        path.chmod(0o600)
        paths.append(path)
    return tuple(paths)


def _project_skills(install_root: Path, home: Path) -> tuple[Path, ...]:
    roots = [home / ".agents/skills"]
    if (home / ".codex").exists():
        roots.append(home / ".codex/skills")
    if (home / ".openclaw").exists():
        roots.append(home / ".openclaw/skills")
    projected: list[Path] = []
    for root in roots:
        for name in SKILL_NAMES:
            destination = root / name
            _replace_symlink(install_root / "skills" / name, destination)
            projected.append(destination)
    return tuple(projected)


def _install_codex_plugins(
    install_root: Path,
    *,
    env: Mapping[str, str],
    which: Callable[[str], str | None],
) -> tuple[str, ...]:
    codex = which("codex")
    if codex is None:
        return ()
    from _codex_app_server import CodexAppServerClient

    marketplace = install_root / "codex/openai-bundled/.agents/plugins/marketplace.json"
    client_env = os.environ.copy()
    client_env.update(env)
    client = CodexAppServerClient([codex, "app-server"], env=client_env, cwd=install_root)
    try:
        client.initialize(client_name="sky-cua-installer", client_version=PRODUCT_VERSION)
        for plugin_name in PLUGIN_NAMES:
            client.request(
                "plugin/install",
                {"marketplacePath": str(marketplace), "pluginName": plugin_name},
            )
    finally:
        client.close()
    return PLUGIN_NAMES


def _install_openclaw_node_repl(
    install_root: Path,
    *,
    home: Path,
    env: Mapping[str, str],
    which: Callable[[str], str | None],
    runner: Runner,
) -> bool:
    openclaw = which("openclaw")
    if openclaw is None:
        return False
    definition: dict[str, Any] = {
        "enabled": True,
        "command": str(install_root / "bin/node_repl"),
        "args": [],
        "cwd": str(install_root),
        "env": {
            "CODEX_NODE_REPL_PATH": str(install_root / "bin/node_repl"),
            "NODE_REPL_NODE_PATH": str(install_root / "bin/node"),
            "NODE_REPL_NODE_MODULE_DIRS": str(install_root / "lib/node_modules"),
            "PLAYWRIGHT_BROWSERS_PATH": str(install_root / "share/playwright"),
            "SKY_CUA_DOCUMENTATION_ROOT": str(install_root / "docs"),
            "SKY_CUA_REPO_ROOT": str(install_root),
        },
        "connectionTimeoutMs": 120_000,
        "requestTimeoutMs": 3_600_000,
        "supportsParallelToolCalls": False,
    }
    command_env = os.environ.copy()
    command_env.update(env)
    command_env["HOME"] = str(home)
    projected_bin = (home / ".local/bin").resolve()
    command_env["PATH"] = os.pathsep.join(
        entry
        for entry in command_env.get("PATH", "").split(os.pathsep)
        if entry and Path(entry).expanduser().resolve() != projected_bin
    )
    result = runner(
        [
            openclaw,
            "mcp",
            "set",
            "node_repl",
            json.dumps(definition, sort_keys=True, separators=(",", ":")),
        ],
        check=False,
        capture_output=True,
        text=True,
        env=command_env,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise RuntimeError(f"OpenClaw node_repl registration failed: {detail}")
    return True


def install_payload(
    payload_root: Path,
    *,
    home: Path | None = None,
    env: Mapping[str, str] | None = None,
    which: Callable[[str], str | None] = shutil.which,
    runner: Runner = subprocess.run,
    configure_hosts: bool = True,
) -> dict[str, object]:
    """Replace the fixed install tree and update only stable external projections."""
    payload_root = payload_root.expanduser().resolve()
    validate_payload(payload_root)
    active_env = dict(os.environ if env is None else env)
    install_home = (home or Path(active_env.get("HOME", str(Path.home())))).expanduser().resolve()
    install_root = _data_root(active_env, install_home).absolute() / "sky-cua"
    install_root.parent.mkdir(parents=True, exist_ok=True)
    _remove_path(install_root)
    shutil.copytree(payload_root, install_root, symlinks=False)

    launchers = _install_launchers(install_root, install_home)
    manifests = _install_native_manifests(
        install_home, install_home / ".local/bin/sky-cua-chrome-host"
    )
    skills = _project_skills(install_root, install_home)
    opencode_report = _opencode_config.install_opencode_config(
        install_root, home=install_home, env=active_env
    )
    plugins: tuple[str, ...] = ()
    openclaw = False
    if configure_hosts:
        plugins = _install_codex_plugins(install_root, env=active_env, which=which)
        openclaw = _install_openclaw_node_repl(
            install_root,
            home=install_home,
            env=active_env,
            which=which,
            runner=runner,
        )
    return {
        "install_root": str(install_root),
        "launchers": [str(path) for path in launchers],
        "native_manifests": [str(path) for path in manifests],
        "skills": [str(path) for path in skills],
        "codex_plugins": list(plugins),
        "openclaw_node_repl": openclaw,
        "opencode_config": opencode_report,
    }


def _is_checkout(root: Path) -> bool:
    return (root / ".git").exists() and (root / "scripts/build_plugin.py").is_file()


def build_command() -> int:
    payload, archive = build_payload(REPO_ROOT / "dist", create_archive=True)
    assert archive is not None
    print(
        json.dumps(
            {"archive": str(archive), "payload": str(payload), "target": TARGET},
            sort_keys=True,
        )
    )
    return 0


def install_command() -> int:
    if _is_checkout(REPO_ROOT):
        payload, _archive = build_payload(REPO_ROOT / "dist", create_archive=False)
        report = install_payload(payload)
    else:
        report = install_payload(REPO_ROOT)
    print(json.dumps(report, sort_keys=True))
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "install"))
    args = parser.parse_args(argv)
    return build_command() if args.command == "build" else install_command()


if __name__ == "__main__":
    raise SystemExit(main())
