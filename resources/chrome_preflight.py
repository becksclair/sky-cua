#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import platform
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
from contextlib import suppress
from datetime import UTC, datetime
from pathlib import Path

OPENAI_BUNDLED_MARKETPLACE = "openai-bundled"
CHROME_PLUGIN_NAME = "chrome"
BROWSER_USE_PLUGIN_NAME = "browser-use"
COMPUTER_USE_PLUGIN_NAME = "computer-use"
NODE_REPL_NAME = "node_repl"
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


def make_tree_owner_writable(path: Path) -> None:
    if not path.exists() and not path.is_symlink():
        return
    if path.is_symlink():
        return
    if path.is_file():
        with suppress(OSError):
            mode = path.lstat().st_mode
            path.chmod(mode | stat.S_IRUSR | stat.S_IWUSR)
        return
    for child in path.rglob("*"):
        if child.is_symlink():
            continue
        with suppress(OSError):
            mode = child.lstat().st_mode
            if child.is_dir():
                child.chmod(mode | stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR)
            else:
                child.chmod(mode | stat.S_IRUSR | stat.S_IWUSR)
    with suppress(OSError):
        mode = path.stat().st_mode
        path.chmod(mode | stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR)


def remove_path(path: Path) -> None:
    if not path.exists() and not path.is_symlink():
        return
    make_tree_owner_writable(path)
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    if path.is_dir():
        shutil.rmtree(path)


def copytree_replace(source: Path, destination: Path) -> None:
    remove_path(destination)
    shutil.copytree(source, destination)
    make_tree_owner_writable(destination)


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


def validate_browser_use_node_repl(source_root: Path) -> None:
    source_node_repl = source_root.parents[1] / NODE_REPL_NAME
    if not source_node_repl.exists():
        return
    with tempfile.TemporaryDirectory(prefix="sky-cua-node-repl-") as tmp:
        destination = Path(tmp) / NODE_REPL_NAME
        if not install_browser_use_node_repl(source_node_repl, destination):
            print(
                f"warning: Browser Use node_repl at {source_node_repl} is not usable on this host",
                file=sys.stderr,
            )


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
    output = result.stdout
    return result.returncode == 0 and not re.search(r"=> not found|version .* not found", output)


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
        print("browser_preflight=node_repl_patched_glibc_2_34")
    if not node_repl_ldd_compatible(destination):
        print(
            "warning: Browser Use node_repl is not compatible with this host runtime",
            file=sys.stderr,
        )
        destination.unlink(missing_ok=True)
        return False
    return True


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
    validate_browser_use_node_repl(source_root)
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
