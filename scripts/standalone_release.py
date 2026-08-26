#!/usr/bin/env python3
"""Build, install, or release the standalone fixed-root sky-cua distribution."""

from __future__ import annotations

import argparse
import json
import os
import shlex
import shutil
import subprocess
import sys
import tarfile
import tomllib
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Any

import _hermes_config
import _opencode_config
import _plugin_bundle
import _standalone_release_command
from _skill_projection import apply_skill_link_plan, plan_skill_links
from _standalone_release_command import (
    ReleaseError,
    parse_stable_version,
)
from _standalone_topology import (
    converge_public_root,
    prepare_install_roots,
)
from _standalone_topology import (
    public_root as stable_public_root,
)

REPO_ROOT = Path(__file__).resolve().parents[1]
TARGET = "linux-x64-glibc"
PRODUCT_VERSION = "0.12.0"
ARCHIVE_NAME = f"sky-cua-{TARGET}.tar.gz"
PAYLOAD_DIR_NAME = f"sky-cua-{TARGET}"
SURFACE_SKILL_MAP = {
    "desktop": "computer-use",
    "browser": "browser-use",
    "phone": "phone-use",
}
SKILL_NAMES = tuple(SURFACE_SKILL_MAP.values())
PLUGIN_NAMES = ("computer-use", "browser")
LAUNCHER_NAMES = (
    "sky-cua-client",
    "sky-cua-service",
    "sky-cua-overlay-host",
    "sky-cua-cosmic-helper",
    "sky-cua-input-helper",
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
PI_MCP_RUNTIME_ENV = (
    "PATH",
    "SKY_CUA_BROWSER_EVAL",
    "SKY_CUA_MODEL_SUPPORTS_IMAGES",
    "SKY_CUA_PRESENCE_ENABLED",
    "SKY_CUA_BROWSER_CONTROL_MODE",
    "SKY_CUA_CODEX_BROWSER_SOCKET_PATH",
    "SKY_CUA_PHONE_DIRECT",
    "SKY_CUA_PHONE_DIRECT_ADVERTISED_ENDPOINT",
    "SKY_CUA_PHONE_DIRECT_ENROLLMENT_TTL_MS",
    "SKY_CUA_PHONE_DIRECT_LISTEN_ADDR",
    "SKY_CUA_PHONE_DIRECT_STATE_PATH",
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XAUTHORITY",
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
        REPO_ROOT / "scripts/_standalone_release_command.py",
        artifact_scripts / "_standalone_release_command.py",
    )
    _copy_file(
        REPO_ROOT / "scripts/_codex_app_server.py", artifact_scripts / "_codex_app_server.py"
    )
    _copy_file(REPO_ROOT / "scripts/_opencode_config.py", artifact_scripts / "_opencode_config.py")
    _copy_file(REPO_ROOT / "scripts/_hermes_config.py", artifact_scripts / "_hermes_config.py")
    _copy_file(REPO_ROOT / "scripts/_plugin_bundle.py", artifact_scripts / "_plugin_bundle.py")
    _copy_file(
        REPO_ROOT / "scripts/_skill_projection.py", artifact_scripts / "_skill_projection.py"
    )
    _copy_file(
        REPO_ROOT / "scripts/_standalone_topology.py", artifact_scripts / "_standalone_topology.py"
    )
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
        "bin/runtimes/linux-x64/sky-cua-cosmic-helper",
        "bin/runtimes/linux-x64/sky-cua-input-helper",
        "bin/node",
        "bin/node_repl",
        "browser/browser-client.mjs",
        "browser/extension/manifest.json",
        "browser/native-host/sky-cua-chrome-host",
        "codex/openai-bundled/.agents/plugins/marketplace.json",
        "codex/openai-bundled/plugins/computer-use/.codex-plugin/plugin.json",
        "codex/openai-bundled/plugins/computer-use/assets/app-icon.png",
        "codex/openai-bundled/plugins/computer-use/.mcp.json",
        "codex/openai-bundled/plugins/browser/.codex-plugin/plugin.json",
        "codex/openai-bundled/plugins/browser/assets/browser.png",
        "codex/openai-bundled/plugins/browser/assets/composer-icon.png",
        "codex/openai-bundled/plugins/browser/.mcp.json",
        "codex/openai-bundled/plugins/browser/scripts/browser-client.mjs",
        "codex/openai-bundled/plugins/browser/skills/control-in-app-browser/SKILL.md",
        "skills/computer-use/SKILL.md",
        "skills/browser-use/SKILL.md",
        "skills/phone-use/SKILL.md",
        "resources/pi/sky-cua-image-capability.ts",
        "docs/README.md",
        "docs/inventories/api-inventory.json",
        "docs/inventories/capability-inventory.json",
        "docs/inventories/example-inventory.json",
        "docs/inventories/routing-inventory.json",
        "install.py",
        "scripts/standalone_release.py",
        "scripts/_standalone_release_command.py",
        "scripts/_opencode_config.py",
        "scripts/_hermes_config.py",
        "scripts/_plugin_bundle.py",
        "scripts/_skill_projection.py",
        "scripts/_standalone_topology.py",
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
        if not isinstance(plugin, dict):
            raise ValueError(f"standalone Codex plugin {plugin_name} metadata must be an object")
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
        if plugin_name == "computer-use":
            interface = plugin_manifest.get("interface")
            if not isinstance(interface, dict) or interface.get("logo") != "./assets/app-icon.png":
                raise ValueError("standalone Computer Use plugin must reference its packaged icon")
        if plugin_name == "browser":
            interface = plugin_manifest.get("interface")
            if not isinstance(interface, dict) or (
                interface.get("composerIcon") != "./assets/composer-icon.png"
                or interface.get("logo") != "./assets/browser.png"
            ):
                raise ValueError("standalone Browser plugin must reference its packaged icons")
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
    portable_x86_64_v3: bool,
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
    build_plugin_command = [
        sys.executable,
        str(REPO_ROOT / "scripts/build_plugin.py"),
        "--dist-root",
        str(core),
    ]
    if portable_x86_64_v3:
        build_plugin_command.append("--portable-x86-64-v3")
    runner(tuple(build_plugin_command))
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


def _install_launchers(
    install_root: Path,
    home: Path,
    *,
    managed_install_roots: Sequence[Path] = (),
) -> tuple[Path, ...]:
    bin_root = home / ".local/bin"
    legacy_node = bin_root / "node"
    if legacy_node.is_symlink():
        try:
            legacy_target = legacy_node.resolve(strict=False)
            managed_roots = tuple(
                root.resolve(strict=False) for root in (install_root, *managed_install_roots)
            )
        except RuntimeError:
            legacy_target = None
            managed_roots = ()
        if legacy_target is not None and any(
            legacy_target.is_relative_to(root) for root in managed_roots
        ):
            legacy_node.unlink()
    targets = {
        "sky-cua-client": install_root / "bin/sky-cua-client",
        "sky-cua-service": install_root / "bin/sky-cua-service",
        "sky-cua-overlay-host": install_root / "bin/sky-cua-overlay-host",
        "sky-cua-cosmic-helper": install_root / "bin/runtimes/linux-x64/sky-cua-cosmic-helper",
        "sky-cua-input-helper": install_root / "bin/runtimes/linux-x64/sky-cua-input-helper",
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


def _durable_skill_names(home: Path, env: Mapping[str, str]) -> tuple[str, ...]:
    explicit = env.get("SKY_CUA_CONFIG_PATH")
    if explicit is not None:
        config_path = Path(explicit) if explicit else None
    else:
        xdg = env.get("XDG_CONFIG_HOME")
        config_path = (Path(xdg) if xdg else home / ".config") / "sky-cua/sky-cua.toml"
    if config_path is None or not config_path.exists():
        return SKILL_NAMES
    try:
        parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise ValueError(
            f"cannot read durable sky-cua surface policy from {config_path}: {error}"
        ) from error
    surfaces = parsed.get("surfaces", {})
    if not isinstance(surfaces, dict):
        raise ValueError(f"[surfaces] must be a TOML table in {config_path}")
    unknown_surfaces = set(surfaces) - set(SURFACE_SKILL_MAP)
    if unknown_surfaces:
        raise ValueError(
            f"unknown [surfaces] key(s) in {config_path}: {', '.join(sorted(unknown_surfaces))}"
        )
    enabled = set(SURFACE_SKILL_MAP)
    for surface in SURFACE_SKILL_MAP:
        value = surfaces.get(surface, True)
        if not isinstance(value, bool):
            raise ValueError(f"[surfaces].{surface} must be boolean in {config_path}")
        if not value:
            enabled.discard(surface)
    phone = parsed.get("phone", {})
    if phone is not None and not isinstance(phone, dict):
        raise ValueError(f"[phone] must be a TOML table in {config_path}")
    if isinstance(phone, dict) and "enabled" in phone:
        value = phone["enabled"]
        if not isinstance(value, bool):
            raise ValueError(f"[phone].enabled must be boolean in {config_path}")
        if not value:
            enabled.discard("phone")
    return tuple(skill for surface, skill in SURFACE_SKILL_MAP.items() if surface in enabled)


def _skill_projection_roots(
    home: Path,
    *,
    env: Mapping[str, str],
    configure_hosts: bool,
) -> tuple[Path, ...]:
    roots = [home / ".agents/skills"]
    if (home / ".codex").exists():
        roots.append(home / ".codex/skills")
    if (home / ".openclaw").exists():
        roots.append(home / ".openclaw/skills")
    hermes_config = _hermes_config.hermes_home(home=home, env=env) / "config.yaml"
    if configure_hosts and hermes_config.exists():
        roots.append(hermes_config.parent / "skills")
    return tuple(roots)


def _link_or_copy_directory(target: Path, link_path: Path) -> None:
    """Symlink ``link_path`` -> ``target``, falling back to a directory copy."""
    if link_path.is_symlink():
        try:
            if link_path.readlink() == target:
                return
        except OSError:
            pass
        link_path.unlink()
    elif link_path.exists():
        _remove_path(link_path)
    link_path.parent.mkdir(parents=True, exist_ok=True)
    try:
        link_path.symlink_to(target, target_is_directory=target.is_dir())
    except (OSError, NotImplementedError, FileExistsError):
        _remove_path(link_path)
        if not target.is_absolute():
            target = (link_path.parent / target).resolve()
        shutil.copytree(target, link_path, symlinks=False)


def _toml_string(value: Path | str) -> str:
    return json.dumps(str(value))


def _upsert_toml_key(config_text: str, header: str, key: str, rendered_value: str) -> str:
    import re

    section_re = re.compile(
        rf"(^\[{re.escape(header)}\]\r?\n)(.*?)(?=^\[|\Z)", re.MULTILINE | re.DOTALL
    )
    match = section_re.search(config_text)
    line = f"{key} = {rendered_value}"
    if match is None:
        separator = "" if not config_text or config_text.endswith("\n") else "\n"
        return f"{config_text}{separator}\n[{header}]\n{line}\n"
    body = match.group(2)
    key_re = re.compile(rf"^{re.escape(key)}\s*=.*$", re.MULTILINE)
    if key_re.search(body):
        new_body = key_re.sub(line, body, count=1)
    else:
        suffix = "" if body.endswith(("\n", "\r\n")) or body == "" else "\n"
        new_body = f"{body}{suffix}{line}\n"
    return config_text[: match.start(2)] + new_body + config_text[match.end(2) :]


def _sync_codex_bundled_marketplace(install_root: Path, codex_home: Path) -> None:
    """Project the standalone ``openai-bundled`` marketplace via the bundled-marketplace cache.

    ``codex app-server`` now rejects ``plugin/install`` for the reserved
    ``openai-bundled`` marketplace (0.149.0: ``reserved and cannot be loaded
    from this source``). The supported override is the same filesystem
    projection ``deploy_plugin.py``/``resources/chrome_preflight.py`` use:
    copy the marketplace into
    ``codex_home/.tmp/bundled-marketplaces/openai-bundled`` and link each
    plugin directory there, then point ``config.toml`` at it. See
    ``resources/chrome_preflight.py:sync_marketplace``.
    """
    source_root = install_root / "codex/openai-bundled"
    source_marketplace = source_root / ".agents/plugins/marketplace.json"
    if not source_marketplace.is_file():
        return
    dest_marketplace = (
        codex_home / ".tmp/bundled-marketplaces/openai-bundled/.agents/plugins/marketplace.json"
    )
    dest_marketplace.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_marketplace, dest_marketplace)
    try:
        manifest = json.loads(dest_marketplace.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        manifest = None
    if isinstance(manifest, dict):
        plugins = manifest.get("plugins")
        if isinstance(plugins, list):
            existing = {p.get("name") for p in plugins if isinstance(p, dict)}
            # Preserve chrome if the standalone marketplace does not ship it;
            # Codex Desktop still expects chrome alongside our two plugins.
            changed = False
            for extra in ("chrome",):
                if extra not in existing:
                    plugins.append(
                        {
                            "name": extra,
                            "source": {"source": "local", "path": f"./plugins/{extra}"},
                            "policy": {
                                "installation": "AVAILABLE",
                                "authentication": "ON_INSTALL",
                            },
                            "category": "Engineering",
                        }
                    )
                    changed = True
            if changed:
                dest_marketplace.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    for plugin_name in PLUGIN_NAMES:
        source_plugin = source_root / "plugins" / plugin_name
        if not source_plugin.is_dir():
            continue
        dest_link = codex_home / ".tmp/bundled-marketplaces/openai-bundled/plugins" / plugin_name
        _link_or_copy_directory(source_plugin.resolve(), dest_link)


def _update_codex_config_for_bundled(codex_home: Path) -> None:
    from datetime import UTC, datetime

    config_path = codex_home / "config.toml"
    try:
        config_text = config_path.read_text(encoding="utf-8")
    except FileNotFoundError:
        config_text = ""
    marketplace_root = codex_home / ".tmp/bundled-marketplaces/openai-bundled"
    config_text = _upsert_toml_key(config_text, "features", "plugins", "true")
    config_text = _upsert_toml_key(
        config_text,
        "marketplaces.openai-bundled",
        "last_updated",
        _toml_string(datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")),
    )
    config_text = _upsert_toml_key(
        config_text,
        "marketplaces.openai-bundled",
        "source_type",
        '"local"',
    )
    config_text = _upsert_toml_key(
        config_text,
        "marketplaces.openai-bundled",
        "source",
        _toml_string(marketplace_root),
    )
    for plugin_id in (f"{name}@openai-bundled" for name in PLUGIN_NAMES):
        config_text = _upsert_toml_key(
            config_text,
            f'plugins."{plugin_id}"',
            "enabled",
            "true",
        )
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_path.write_text(config_text, encoding="utf-8")


def _install_codex_plugins(
    install_root: Path,
    *,
    home: Path | None = None,
    env: Mapping[str, str] | None = None,
    which: Callable[[str], str | None],
) -> tuple[str, ...]:
    if env is None:
        env = {}
    codex = which("codex")
    if codex is None:
        return ()
    # ``install_root`` is the physical fixed root (``XDG_DATA_HOME/sky-cua``);
    # ``home`` is the user home that owns ``~/.codex``. Derive from ``env`` when
    # not explicitly passed so tests that monkeypatch ``which`` keep working.
    home_path = Path(env.get("HOME", str(Path.home()))).expanduser() if home is None else home
    codex_home = home_path / ".codex"
    # Filesystem projection replaces the 0.149.0-blocked ``plugin/install`` RPC.
    _sync_codex_bundled_marketplace(install_root, codex_home)
    _update_codex_config_for_bundled(codex_home)
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
        # node_repl derives its runtime paths (node binary, module root,
        # Playwright browsers, launcher path) from its own binary location at
        # startup, so the host config must not pin them. Keep an empty env so
        # the process inherits the parent environment and auto-detects.
        "env": {},
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


def _set_top_level_toml_string(text: str, key: str, value: str) -> str:
    """Set one top-level TOML string while preserving unrelated formatting."""
    lines = text.splitlines(keepends=True)
    first_table = next(
        (index for index, line in enumerate(lines) if line.lstrip().startswith("[")),
        len(lines),
    )
    replacement = f'{key} = "{value}"\n'
    matches = [
        index
        for index, line in enumerate(lines[:first_table])
        if line.split("=", 1)[0].strip() == key and "=" in line
    ]
    if len(matches) > 1:
        raise ValueError(f"OpenClaw Codex config has duplicate top-level {key!r} entries")
    if matches:
        lines[matches[0]] = replacement
    else:
        lines.insert(first_table, replacement)
    return "".join(lines)


def _write_text_atomically(path: Path, text: str) -> None:
    write_path = path.resolve(strict=False) if path.is_symlink() else path
    write_path.parent.mkdir(parents=True, exist_ok=True)
    mode = write_path.stat().st_mode if write_path.exists() else 0o600
    temporary = write_path.with_name(f".{write_path.name}.tmp-{os.getpid()}")
    _remove_path(temporary)
    try:
        temporary.write_text(text, encoding="utf-8")
        temporary.chmod(mode)
        os.replace(temporary, write_path)
    finally:
        _remove_path(temporary)


def _install_pi_mcp(
    install_root: Path,
    *,
    home: Path,
    env: Mapping[str, str],
) -> Path | None:
    """Project the fixed-root MCP launcher into an existing Pi installation."""
    wrapper = install_root / "pi_mcp_wrapper.sh"
    forwarded = {
        name: value.strip() for name in PI_MCP_RUNTIME_ENV if (value := env.get(name, "")).strip()
    }
    forwarded.setdefault("SKY_CUA_PRESENCE_ENABLED", "1")
    exports = "".join(f"export {name}={shlex.quote(value)}\n" for name, value in forwarded.items())
    wrapper.write_text(
        "".join(
            (
                "#!/bin/bash\n",
                f"export SKY_CUA_REPO_ROOT={shlex.quote(str(install_root))}\n",
                exports,
                "export SKY_CUA_MCP_CALLER_PROVENANCE=pi\n",
                f'exec {shlex.quote(str(install_root / "bin/sky-cua-client"))} mcp "$@"\n',
            )
        ),
        encoding="utf-8",
    )
    wrapper.chmod(0o755)

    agent_dir = home / ".pi/agent"
    if not agent_dir.is_dir():
        return None
    extension_source = install_root / "resources/pi/sky-cua-image-capability.ts"
    extension_dir = agent_dir / "extensions"
    extension_dir.mkdir(parents=True, exist_ok=True)
    extension_path = extension_dir / "sky-cua-image-capability.ts"
    if extension_path.exists() or extension_path.is_symlink():
        if extension_path.is_symlink():
            expected = extension_source.resolve(strict=False)
            actual = extension_path.resolve(strict=False)
            if actual != expected:
                # Legacy installs under /home/bex and other prior fixed roots left
                # dangling or absolute symlinks that still point at a sky-cua
                # managed copy of this exact extension. Treat those as managed
                # so host migrations (bex -> ubuntu) and reinstalls converge
                # instead of failing the entire deploy.
                try:
                    raw_target = str(extension_path.readlink())
                except OSError:
                    raw_target = ""
                is_legacy_managed = (
                    raw_target.endswith("sky-cua-image-capability.ts") and "sky-cua" in raw_target
                )
                if not is_legacy_managed:
                    raise ValueError(
                        f"refusing to replace an unmanaged Pi extension: {extension_path}"
                    )
        else:
            # Regular file at the extension path: only replace if it is
            # byte-identical to the shipped source (e.g. a prior copy
            # instead of a symlink). Otherwise preserve the user's file.
            try:
                if extension_path.read_bytes() != extension_source.read_bytes():
                    raise ValueError(
                        f"refusing to replace an unmanaged Pi extension: {extension_path}"
                    )
            except OSError as exc:
                raise ValueError(
                    f"refusing to replace an unmanaged Pi extension: {extension_path}"
                ) from exc
        extension_path.unlink()
    extension_path.symlink_to(extension_source)
    config_path = agent_dir / "mcp.json"
    if config_path.exists():
        config = json.loads(config_path.read_text(encoding="utf-8"))
        if not isinstance(config, dict):
            raise ValueError(f"Pi MCP config must be a JSON object: {config_path}")
    else:
        config = {}
    servers = config.setdefault("mcpServers", {})
    if not isinstance(servers, dict):
        raise ValueError(f"Pi MCP config mcpServers must be a JSON object: {config_path}")
    servers["sky_cua"] = {
        "command": str(wrapper),
        "args": [],
        "lifecycle": "lazy",
        "directTools": True,
    }
    _write_text_atomically(config_path, json.dumps(config, indent=2) + "\n")
    return config_path


def _configure_openclaw_no_prompt_permissions(
    *,
    home: Path,
    env: Mapping[str, str],
    openclaw: str,
    runner: Runner,
) -> tuple[Path, ...]:
    """Converge OpenClaw and every existing Codex home to full-auto operation."""
    config_paths = sorted((home / ".openclaw/agents").glob("*/agent/codex-home/config.toml"))
    planned: list[tuple[Path, str]] = []
    for config_path in config_paths:
        original = config_path.read_text(encoding="utf-8")
        updated = _set_top_level_toml_string(original, "approval_policy", "never")
        updated = _set_top_level_toml_string(updated, "sandbox_mode", "danger-full-access")
        tomllib.loads(updated)
        if updated != original:
            planned.append((config_path, updated))

    batch = [
        {
            "path": "plugins.entries.codex.config.appServer.mode",
            "value": "yolo",
        },
        {
            "path": "plugins.entries.codex.config.appServer.approvalPolicy",
            "value": "never",
        },
        {
            "path": "plugins.entries.codex.config.appServer.sandbox",
            "value": "danger-full-access",
        },
        {
            "path": "plugins.entries.codex.config.codexPlugins.enabled",
            "value": True,
        },
        {
            "path": "plugins.entries.codex.config.codexPlugins.allow_all_plugins",
            "value": True,
        },
        {
            "path": "plugins.entries.codex.config.codexPlugins.allow_destructive_actions",
            "value": "approve",
        },
    ]
    command_env = os.environ.copy()
    command_env.update(env)
    command_env["HOME"] = str(home)
    result = runner(
        [openclaw, "config", "set", "--batch-json", json.dumps(batch, separators=(",", ":"))],
        check=False,
        capture_output=True,
        text=True,
        env=command_env,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout or "").strip()
        raise RuntimeError(f"OpenClaw no-prompt permission configuration failed: {detail}")
    for config_path, updated in planned:
        _write_text_atomically(config_path, updated)
    return tuple(config_paths)


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
    install_root = (_data_root(active_env, install_home).absolute() / "sky-cua").absolute()
    public_root = stable_public_root(install_home).absolute()
    topology = prepare_install_roots(
        install_root,
        public_root,
        target=TARGET,
        skill_names=SKILL_NAMES,
    )
    durable_skill_names = _durable_skill_names(install_home, active_env)
    managed_skill_roots = (
        *(root / "skills" for root in topology.stop_roots),
        REPO_ROOT / "skills",
    )
    skill_plan = plan_skill_links(
        public_root / "skills",
        _skill_projection_roots(
            install_home,
            env=active_env,
            configure_hosts=configure_hosts,
        ),
        SKILL_NAMES,
        durable_skill_names,
        managed_source_roots=managed_skill_roots,
        validation_source_root=payload_root / "skills",
    )
    install_root.parent.mkdir(parents=True, exist_ok=True)
    # Retire processes executing from the fixed root before replacing it. MCP
    # hosts respawn their stdio clients lazily, and the next client starts the
    # shared daemon with the newly projected desktop environment.
    _plugin_bundle.stop_unix_runtime_processes(list(topology.stop_roots))
    _remove_path(install_root)
    shutil.copytree(payload_root, install_root, symlinks=False)
    validate_payload(install_root)
    for obsolete_root in topology.obsolete_payload_roots:
        _remove_path(obsolete_root)
    converge_public_root(install_root, public_root)
    skills = apply_skill_link_plan(skill_plan)

    launchers = _install_launchers(
        public_root,
        install_home,
        managed_install_roots=topology.stop_roots,
    )
    manifests = _install_native_manifests(
        install_home, install_home / ".local/bin/sky-cua-chrome-host"
    )
    pi_config = _install_pi_mcp(public_root, home=install_home, env=active_env)
    opencode_report = _opencode_config.install_opencode_config(
        public_root, home=install_home, env=active_env
    )
    hermes_report: dict[str, object] = {
        "status": "skipped",
        "config_path": None,
        "backup_path": None,
        "servers": [],
    }
    plugins: tuple[str, ...] = ()
    openclaw = False
    openclaw_permission_configs: tuple[Path, ...] = ()
    if configure_hosts:
        hermes_config = _hermes_config.install_hermes_config(
            public_root,
            home=install_home,
            env=active_env,
        )
        hermes_report = hermes_config.report()
        if hermes_config.config_path is not None:
            hermes_report["agents"] = _hermes_config.install_hermes_agents(
                home=install_home,
                env=active_env,
            ).report()
        plugins = _install_codex_plugins(
            public_root, home=install_home, env=active_env, which=which
        )
        openclaw_bin = which("openclaw")
        if openclaw_bin is not None:
            openclaw_permission_configs = _configure_openclaw_no_prompt_permissions(
                home=install_home,
                env=active_env,
                openclaw=openclaw_bin,
                runner=runner,
            )
            openclaw = _install_openclaw_node_repl(
                public_root,
                home=install_home,
                env=active_env,
                which=lambda name: openclaw_bin if name == "openclaw" else which(name),
                runner=runner,
            )
    return {
        "install_root": str(install_root),
        "public_root": str(public_root),
        "launchers": [str(path) for path in launchers],
        "native_manifests": [str(path) for path in manifests],
        "skills": [str(path) for path in skills],
        "pi_config": str(pi_config) if pi_config is not None else None,
        "codex_plugins": list(plugins),
        "openclaw_node_repl": openclaw,
        "openclaw_permission_configs": [str(path) for path in openclaw_permission_configs],
        "opencode_config": opencode_report,
        "hermes_config": hermes_report,
    }


def _is_checkout(root: Path) -> bool:
    return (root / ".git").exists() and (root / "scripts/build_plugin.py").is_file()


def _argparse_stable_version(value: str) -> str:
    try:
        parse_stable_version(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError(str(exc)) from exc
    return value


def release_command(
    *,
    bump: str = "minor",
    explicit_version: str | None = None,
    runner: Runner = subprocess.run,
    repo_root: Path | None = None,
) -> int:
    """Create and atomically push a guarded standalone release commit and tag."""
    return _standalone_release_command.release_command(
        product_version=PRODUCT_VERSION,
        repo_root=REPO_ROOT if repo_root is None else repo_root,
        bump=bump,
        explicit_version=explicit_version,
        runner=runner,
    )


def build_command() -> int:
    payload, archive = build_payload(
        REPO_ROOT / "dist", create_archive=True, portable_x86_64_v3=True
    )
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
        payload, _archive = build_payload(
            REPO_ROOT / "dist", create_archive=False, portable_x86_64_v3=False
        )
        report = install_payload(payload)
    else:
        report = install_payload(REPO_ROOT)
    print(json.dumps(report, sort_keys=True))
    return 0


def _argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("build", help="build the standalone archive")
    subparsers.add_parser("install", help="build or install the standalone payload")
    release = subparsers.add_parser(
        "release",
        help="commit, tag, and push a guarded release",
        description=(
            "Verify a clean synchronized main checkout, bump the standalone version, "
            "commit it, and atomically push main plus its annotated release tag."
        ),
    )
    versions = release.add_mutually_exclusive_group()
    versions.add_argument(
        "--version",
        type=_argparse_stable_version,
        help="use an explicit increasing stable X.Y.Z version",
    )
    versions.add_argument(
        "--patch", action="store_const", const="patch", dest="bump", help="increment patch"
    )
    versions.add_argument(
        "--minor",
        action="store_const",
        const="minor",
        dest="bump",
        help="increment minor and reset patch (default)",
    )
    versions.add_argument(
        "--major",
        action="store_const",
        const="major",
        dest="bump",
        help="increment major and reset minor and patch",
    )
    release.set_defaults(bump="minor", version=None)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _argument_parser()
    args = parser.parse_args(argv)
    if args.command == "build":
        return build_command()
    if args.command == "install":
        return install_command()
    try:
        return release_command(bump=args.bump, explicit_version=args.version)
    except ReleaseError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
