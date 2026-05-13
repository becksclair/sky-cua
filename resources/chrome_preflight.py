#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import platform
import re
import shutil
import stat
from datetime import UTC, datetime
from pathlib import Path

OPENAI_BUNDLED_MARKETPLACE = "openai-bundled"
CHROME_PLUGIN_NAME = "chrome"
BROWSER_USE_PLUGIN_NAME = "browser-use"
COMPUTER_USE_PLUGIN_NAME = "computer-use"
CHROME_EXTENSION_ID_RE = re.compile(r"^[a-p]{32}$")
CHROME_HOST_NAME_RE = re.compile(r"^[a-z0-9_.]+$")
DEFAULT_COMPUTER_USE_ENV_VARS = [
    "CODEX_COMPUTER_USE_COSMIC_HELPER",
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "SKY_CUA_COSMIC_HELPER",
    "SKY_CUA_MODEL_SCREENSHOT_FORMAT",
    "SKY_CUA_MODEL_SCREENSHOT_JPEG_QUALITY",
    "SKY_CUA_MODEL_SCREENSHOT_MAX_HEIGHT",
    "SKY_CUA_MODEL_SCREENSHOT_MAX_WIDTH",
    "SKY_CUA_MODEL_SCREENSHOT_WEBP_QUALITY",
    "SKY_CUA_REPO_ROOT",
    "SKY_CUA_SERVICE_PATH",
    "SKY_CUA_SERVICE_TCP_ADDR",
    "SKY_CUA_SERVICE_SOCKET_PATH",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
]


def ensure_executable(path: Path) -> None:
    try:
        mode = path.stat().st_mode
        path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    except FileNotFoundError:
        pass


def remove_path(path: Path) -> None:
    if not path.exists() and not path.is_symlink():
        return
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    if path.is_dir():
        shutil.rmtree(path)


def copytree_replace(source: Path, destination: Path) -> None:
    remove_path(destination)
    shutil.copytree(source, destination)


def bundled_plugin_version(plugin_root: Path) -> str | None:
    plugin_json = plugin_root / ".codex-plugin" / "plugin.json"
    try:
        version = json.loads(plugin_json.read_text(encoding="utf-8")).get("version")
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(version, str) or not version.strip():
        return None
    return version.strip()


def current_extension_arch() -> str | None:
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return "x64"
    if machine in {"aarch64", "arm64"}:
        return "arm64"
    return None


def computer_use_env_vars(sky_root: Path) -> list[str]:
    try:
        mcp = json.loads((sky_root / ".mcp.json").read_text(encoding="utf-8"))
        env_vars = mcp["mcpServers"]["computer-use"]["env_vars"]
    except (OSError, KeyError, TypeError, json.JSONDecodeError):
        return DEFAULT_COMPUTER_USE_ENV_VARS
    if not isinstance(env_vars, list) or not all(isinstance(name, str) for name in env_vars):
        return DEFAULT_COMPUTER_USE_ENV_VARS
    return env_vars


def link_or_copy_directory(target: Path, link_path: Path) -> None:
    """Symlink ``link_path`` -> ``target``, falling back to a directory copy.

    Windows refuses symlinks without developer mode or admin, so we fall back
    to a full copy there. The cache layout only needs a navigable directory
    at ``link_path``; readers do not require it to be a symlink.
    """
    if link_path.exists() and not link_path.is_symlink():
        remove_path(link_path)
    link_path.unlink(missing_ok=True)
    try:
        link_path.symlink_to(target)
    except FileExistsError:
        remove_path(link_path)
        link_path.symlink_to(target)
    except (OSError, NotImplementedError):
        if not target.is_absolute():
            target = (link_path.parent / target).resolve()
        remove_path(link_path)
        shutil.copytree(target, link_path)


def sync_marketplace(source_root: Path, codex_home: Path) -> None:
    source_marketplace = source_root / ".agents" / "plugins" / "marketplace.json"
    if not source_marketplace.exists():
        return
    destination = (
        codex_home
        / ".tmp"
        / "bundled-marketplaces"
        / OPENAI_BUNDLED_MARKETPLACE
        / ".agents"
        / "plugins"
        / "marketplace.json"
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source_marketplace, destination)
    ensure_marketplace_entries(codex_home)


def sync_browser_use_plugin(source_root: Path, codex_home: Path) -> None:
    source_plugin = source_root / "plugins" / BROWSER_USE_PLUGIN_NAME
    version = bundled_plugin_version(source_plugin)
    if version is None:
        return
    cache_root = (
        codex_home / "plugins" / "cache" / OPENAI_BUNDLED_MARKETPLACE / BROWSER_USE_PLUGIN_NAME
    )
    cache_plugin = cache_root / version
    source_client = source_plugin / "scripts" / "browser-client.mjs"
    cache_client = cache_plugin / "scripts" / "browser-client.mjs"
    if (
        cache_client.exists()
        and source_client.exists()
        and files_equal(source_client, cache_client)
    ):
        ensure_cached_plugin_link(cache_root, version)
        ensure_marketplace_plugin_link(codex_home, BROWSER_USE_PLUGIN_NAME, cache_root / "latest")
        return
    copytree_replace(source_plugin, cache_plugin)
    ensure_cached_plugin_link(cache_root, version)
    ensure_marketplace_plugin_link(codex_home, BROWSER_USE_PLUGIN_NAME, cache_root / "latest")


def sync_chrome_plugin(source_root: Path, codex_home: Path) -> None:
    source_plugin = source_root / "plugins" / CHROME_PLUGIN_NAME
    version = bundled_plugin_version(source_plugin)
    extension_arch = current_extension_arch()
    if version is None or extension_arch is None:
        return

    source_host = source_plugin / "extension-host" / "linux" / extension_arch / "extension-host"
    source_client = source_plugin / "scripts" / "browser-client.mjs"
    source_install_manifest = source_plugin / "scripts" / "installManifest.mjs"
    if (
        not source_host.is_file()
        or not source_client.is_file()
        or not source_install_manifest.is_file()
    ):
        return

    cache_root = codex_home / "plugins" / "cache" / OPENAI_BUNDLED_MARKETPLACE / CHROME_PLUGIN_NAME
    cache_plugin = cache_root / version
    copytree_replace(source_plugin, cache_plugin)
    ensure_executable(cache_plugin / "extension-host" / "linux" / extension_arch / "extension-host")

    latest = cache_root / "latest"
    ensure_cached_plugin_link(cache_root, version)
    ensure_marketplace_plugin_link(codex_home, CHROME_PLUGIN_NAME, latest)

    host_path = (
        cache_root / "latest" / "extension-host" / "linux" / extension_arch / "extension-host"
    )
    write_chrome_native_host_manifests(host_path, cache_root / "latest")


def sky_cua_plugin_root(source_root: Path) -> Path:
    return source_root.parents[2]


def sync_computer_use_compat_plugin(source_root: Path, codex_home: Path) -> None:
    sky_root = sky_cua_plugin_root(source_root)
    sky_manifest_path = sky_root / ".codex-plugin" / "plugin.json"
    try:
        sky_manifest = json.loads(sky_manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return

    version = sky_manifest.get("version")
    if not isinstance(version, str) or not version.strip():
        version = "0.1.0"
    compat_version = f"{version.strip()}-sky-cua"
    cache_root = (
        codex_home / "plugins" / "cache" / OPENAI_BUNDLED_MARKETPLACE / COMPUTER_USE_PLUGIN_NAME
    )
    cache_plugin = cache_root / compat_version

    remove_path(cache_plugin)
    (cache_plugin / ".codex-plugin").mkdir(parents=True, exist_ok=True)
    (cache_plugin / "assets").mkdir(parents=True, exist_ok=True)
    if (sky_root / "assets" / "app-icon.png").exists():
        shutil.copy2(sky_root / "assets" / "app-icon.png", cache_plugin / "assets" / "app-icon.png")

    compat_manifest = {
        "name": COMPUTER_USE_PLUGIN_NAME,
        "version": compat_version,
        "description": "Control desktop apps on Linux from Codex through sky-cua Computer Use.",
        "author": sky_manifest.get("author", {"name": "Rebecca Clair"}),
        "homepage": sky_manifest.get("homepage", "https://github.com/becksclair/sky-cua"),
        "repository": sky_manifest.get("repository", "https://github.com/becksclair/sky-cua"),
        "license": sky_manifest.get("license", "MIT"),
        "keywords": ["computer-use", "desktop-control", "linux", "wayland", "accessibility"],
        "mcpServers": "./.mcp.json",
        "interface": {
            "displayName": "Computer Use",
            "shortDescription": "Control Linux desktop apps from Codex",
            "longDescription": (
                "Linux Computer Use lets Codex inspect and control desktop apps"
                " through the sky-cua native backend. It may use screenshots,"
                " accessibility metadata, and input events after you allow it."
            ),
            "developerName": "Rebecca Clair",
            "category": "Productivity",
            "websiteURL": "https://github.com/becksclair/sky-cua",
            "privacyPolicyURL": "https://openai.com/policies/row-privacy-policy/",
            "termsOfServiceURL": "https://openai.com/policies/row-terms-of-use/",
            "logo": "./assets/app-icon.png",
            "defaultPrompt": [
                "Check whether Linux Computer Use is ready",
                "List running desktop apps",
            ],
            "brandColor": "#0F172A",
            "screenshots": [],
        },
    }
    (cache_plugin / ".codex-plugin" / "plugin.json").write_text(
        json.dumps(compat_manifest, indent=2) + "\n",
        encoding="utf-8",
    )
    mcp = {
        "mcpServers": {
            "computer-use": {
                "command": str((sky_root / "bin" / "sky-cua-client").resolve()),
                "args": ["mcp"],
                "env_vars": computer_use_env_vars(sky_root),
                "cwd": str(sky_root.resolve()),
            }
        }
    }
    (cache_plugin / ".mcp.json").write_text(json.dumps(mcp, indent=2) + "\n", encoding="utf-8")

    latest = cache_root / "latest"
    ensure_cached_plugin_link(cache_root, compat_version)
    ensure_marketplace_plugin_link(codex_home, COMPUTER_USE_PLUGIN_NAME, latest)


def ensure_cached_plugin_link(cache_root: Path, version: str) -> None:
    link_or_copy_directory(Path(version), cache_root / "latest")


def ensure_marketplace_plugin_link(codex_home: Path, plugin_name: str, target: Path) -> None:
    marketplace_root = codex_home / ".tmp" / "bundled-marketplaces" / OPENAI_BUNDLED_MARKETPLACE
    plugin_link = marketplace_root / "plugins" / plugin_name
    plugin_link.parent.mkdir(parents=True, exist_ok=True)
    link_or_copy_directory(target, plugin_link)


def ensure_marketplace_entries(codex_home: Path) -> None:
    marketplace_path = (
        codex_home
        / ".tmp"
        / "bundled-marketplaces"
        / OPENAI_BUNDLED_MARKETPLACE
        / ".agents"
        / "plugins"
        / "marketplace.json"
    )
    try:
        manifest = json.loads(marketplace_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return

    plugins = manifest.setdefault("plugins", [])
    if not isinstance(plugins, list):
        return
    existing_names = {plugin.get("name") for plugin in plugins if isinstance(plugin, dict)}
    changed = False
    for plugin_name in (CHROME_PLUGIN_NAME, BROWSER_USE_PLUGIN_NAME, COMPUTER_USE_PLUGIN_NAME):
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


def files_equal(left: Path, right: Path) -> bool:
    try:
        return left.read_bytes() == right.read_bytes()
    except OSError:
        return False


def read_chrome_extension_metadata(plugin_root: Path) -> tuple[str, str]:
    scripts_dir = plugin_root / "scripts"
    extension_id_json = scripts_dir / "extension-id.json"
    try:
        data = json.loads(extension_id_json.read_text(encoding="utf-8"))
        extension_id = data.get("extensionId")
        host_name = data.get("extensionHostName")
    except (OSError, json.JSONDecodeError):
        extension_id = None
        host_name = None

    if extension_id is None or host_name is None:
        try:
            install_manifest = (scripts_dir / "installManifest.mjs").read_text(encoding="utf-8")
        except OSError:
            install_manifest = ""
        extension_id_match = re.search(r'extensionId\s*:\s*"([a-p]{32})"', install_manifest)
        host_name_match = re.search(r'extensionHostName\s*:\s*"([A-Za-z0-9_.]+)"', install_manifest)
        if extension_id is None and extension_id_match is not None:
            extension_id = extension_id_match.group(1)
        if host_name is None and host_name_match is not None:
            host_name = host_name_match.group(1)

    if not isinstance(extension_id, str) or CHROME_EXTENSION_ID_RE.fullmatch(extension_id) is None:
        raise RuntimeError("invalid Chrome extension id in bundled plugin metadata")
    if not isinstance(host_name, str) or CHROME_HOST_NAME_RE.fullmatch(host_name) is None:
        raise RuntimeError("invalid Chrome native host name in bundled plugin metadata")
    return extension_id, host_name


def write_chrome_native_host_manifests(host_path: Path, plugin_root: Path) -> None:
    extension_id, host_name = read_chrome_extension_metadata(plugin_root)
    manifest = {
        "name": host_name,
        "description": "Codex chrome native messaging host",
        "type": "stdio",
        "path": str(host_path),
        "allowed_origins": [f"chrome-extension://{extension_id}/"],
    }
    text = json.dumps(manifest, separators=(",", ":"))
    home = Path.home()
    for relative in (
        ".config/google-chrome/NativeMessagingHosts",
        ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts",
        ".config/chromium/NativeMessagingHosts",
    ):
        destination = home / relative / f"{host_name}.json"
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists() and destination.read_text(encoding="utf-8") == text:
            continue
        destination.write_text(text, encoding="utf-8")


def fallback_extension_path(source_root: Path) -> Path | None:
    fallback_root = source_root.parents[1] / "chrome-extension" / "codex"
    candidates = sorted(fallback_root.glob("*_0"), reverse=True)
    for candidate in candidates:
        manifest = candidate / "manifest.json"
        if not manifest.exists():
            continue
        try:
            data = json.loads(manifest.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            continue
        if data.get("name") == "Codex" and data.get("key"):
            return candidate
    return None


def toml_string(value: Path | str) -> str:
    return json.dumps(str(value))


def upsert_toml_key(config_text: str, header: str, key: str, rendered_value: str) -> str:
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


def update_codex_config(codex_home: Path) -> None:
    config_path = codex_home / "config.toml"
    try:
        config_text = config_path.read_text(encoding="utf-8")
    except FileNotFoundError:
        config_text = ""

    marketplace_root = codex_home / ".tmp" / "bundled-marketplaces" / OPENAI_BUNDLED_MARKETPLACE
    config_text = upsert_toml_key(config_text, "features", "plugins", "true")
    config_text = upsert_toml_key(
        config_text,
        f"marketplaces.{OPENAI_BUNDLED_MARKETPLACE}",
        "last_updated",
        toml_string(datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")),
    )
    config_text = upsert_toml_key(
        config_text,
        f"marketplaces.{OPENAI_BUNDLED_MARKETPLACE}",
        "source_type",
        '"local"',
    )
    config_text = upsert_toml_key(
        config_text,
        f"marketplaces.{OPENAI_BUNDLED_MARKETPLACE}",
        "source",
        toml_string(marketplace_root),
    )
    config_text = upsert_toml_key(
        config_text,
        f'plugins."{CHROME_PLUGIN_NAME}@{OPENAI_BUNDLED_MARKETPLACE}"',
        "enabled",
        "true",
    )
    config_text = upsert_toml_key(
        config_text,
        f'plugins."{BROWSER_USE_PLUGIN_NAME}@{OPENAI_BUNDLED_MARKETPLACE}"',
        "enabled",
        "true",
    )
    config_text = upsert_toml_key(
        config_text,
        f'plugins."{COMPUTER_USE_PLUGIN_NAME}@{OPENAI_BUNDLED_MARKETPLACE}"',
        "enabled",
        "false",
    )
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_path.write_text(config_text, encoding="utf-8")


def default_source_root() -> Path:
    return Path(__file__).resolve().parent / "plugins" / "openai-bundled"


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Sync bundled OpenAI Chrome/browser-use plugins for sky-cua preflight."
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        default=Path.home() / ".codex",
        help="Codex home directory to update (default: ~/.codex).",
    )
    parser.add_argument(
        "--source-root",
        type=Path,
        default=default_source_root(),
        help="Bundled openai-bundled resource root to sync from.",
    )
    args = parser.parse_args()

    source_root = args.source_root.resolve()
    if not (source_root / ".agents" / "plugins" / "marketplace.json").exists():
        print(f"browser_preflight=skipped missing_source={source_root}")
        return 0

    sync_marketplace(source_root, args.codex_home)
    sync_browser_use_plugin(source_root, args.codex_home)
    sync_chrome_plugin(source_root, args.codex_home)
    sync_computer_use_compat_plugin(source_root, args.codex_home)
    update_codex_config(args.codex_home)
    if fallback := fallback_extension_path(source_root):
        print(f"browser_extension_fallback={fallback}")
        print(
            "browser_extension_fallback_hint="
            f"launch Chrome/Chromium with --load-extension={fallback}"
            " if the Web Store is unavailable"
        )
    print("browser_preflight=ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
