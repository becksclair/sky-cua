from __future__ import annotations

import json
import re
import shutil
import stat
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path

PLUGIN_NAME = "sky-cua"
PLUGIN_CHANNEL = "debug"
RELEASE_MARKETPLACE_NAME = "sky-cua-local"
RELEASE_PLUGIN_ID = f"{PLUGIN_NAME}@{RELEASE_MARKETPLACE_NAME}"
PLUGIN_ID = f"{PLUGIN_NAME}@{PLUGIN_CHANNEL}"
REPO_ROOT = Path(__file__).resolve().parents[1]
DIST_PLUGIN_ROOT = REPO_ROOT / "dist" / "plugin" / PLUGIN_NAME
DEFAULT_CODEX_HOME = Path.home() / ".codex"
DEFAULT_MARKETPLACE_ROOT = Path.home() / ".agents" / "sky-cua-marketplace"
INSTALLED_PLUGIN_ROOT = (
    DEFAULT_CODEX_HOME / "plugins" / "cache" / PLUGIN_CHANNEL / PLUGIN_NAME / "local"
)


def executable_name(name: str) -> str:
    if sys.platform == "win32":
        return f"{name}.exe"
    return name


def runtime_binary_names() -> list[str]:
    return [executable_name("sky-cua-client"), executable_name("sky-cua-service")]


def build_bundle() -> None:
    subprocess.run([sys.executable, str(REPO_ROOT / "scripts" / "build_plugin.py")], check=True)


def mcp_config_source() -> Path:
    if sys.platform == "win32":
        return REPO_ROOT / ".mcp.windows.json"
    return REPO_ROOT / ".mcp.json"


def ensure_executable(path: Path) -> None:
    if sys.platform == "win32":
        return
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def remove_path(path: Path) -> None:
    if not path.exists() and not path.is_symlink():
        return
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    shutil.rmtree(path)


def copytree_replace(src: Path, dst: Path) -> None:
    remove_path(dst)
    shutil.copytree(src, dst)


def stop_windows_cache_processes(cache_root: Path) -> None:
    if sys.platform != "win32":
        return
    resolved_cache_root = str(cache_root.resolve())
    escaped_cache_root = resolved_cache_root.replace("'", "''")
    script = (
        "$cacheRoot = '" + escaped_cache_root + "'; "
        "$cacheRoot = [System.IO.Path]::GetFullPath($cacheRoot).TrimEnd('\\'); "
        "$cachePrefix = ($cacheRoot + '\\').ToLowerInvariant(); "
        "$matches = Get-CimInstance Win32_Process | Where-Object { "
        "$path = $_.ExecutablePath; "
        "if ($path) { "
        "$full = [System.IO.Path]::GetFullPath($path).ToLowerInvariant(); "
        "$full.StartsWith($cachePrefix) "
        "} elseif ($_.CommandLine) { "
        "$commandLine = $_.CommandLine.TrimStart('\"').ToLowerInvariant(); "
        "$commandLine.StartsWith($cachePrefix) "
        "} else { $false } "
        "}; "
        "$matches | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True)


def ensure_bundle_structure(root: Path) -> None:
    required = [
        root / ".codex-plugin" / "plugin.json",
        root / ".mcp.json",
        root / "skills" / "computer-use-workflows" / "SKILL.md",
        root / "resources" / "app-instructions" / "index.json",
    ]
    required.extend(root / "bin" / binary_name for binary_name in runtime_binary_names())
    missing = [str(path) for path in required if not path.exists()]
    if missing:
        raise FileNotFoundError(f"plugin bundle is missing required paths: {missing}")
    for binary_name in runtime_binary_names():
        ensure_executable(root / "bin" / binary_name)


def installed_plugin_root(codex_home: Path) -> Path:
    return codex_home / "plugins" / "cache" / PLUGIN_CHANNEL / PLUGIN_NAME / "local"


def release_plugin_root(marketplace_root: Path) -> Path:
    return marketplace_root / "plugins" / PLUGIN_NAME


def marketplace_manifest_path(marketplace_root: Path) -> Path:
    return marketplace_root / ".agents" / "plugins" / "marketplace.json"


def write_release_marketplace(marketplace_root: Path) -> Path:
    manifest_path = marketplace_manifest_path(marketplace_root)
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest = {
        "name": RELEASE_MARKETPLACE_NAME,
        "interface": {
            "displayName": "Sky CUA Local",
        },
        "plugins": [
            {
                "name": PLUGIN_NAME,
                "source": {
                    "source": "local",
                    "path": f"./plugins/{PLUGIN_NAME}",
                },
            }
        ],
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest_path


def ensure_plugins_feature_enabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "plugins", "true")


def ensure_apps_feature_disabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "apps", "false")


def ensure_fast_mode_enabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "fast_mode", "true")


def ensure_fast_service_tier(config_text: str) -> str:
    config_text = remove_profile_service_tier_overrides(config_text)
    return upsert_top_level_toml_key(config_text, "service_tier", '"fast"')


def ensure_plugin_enabled(config_text: str) -> str:
    return set_plugin_enabled(config_text, PLUGIN_ID, enabled=True)


def set_plugin_enabled(config_text: str, plugin_id: str, *, enabled: bool) -> str:
    rendered = "true" if enabled else "false"
    return upsert_toml_key(config_text, f'plugins."{plugin_id}"', "enabled", rendered)


def ensure_release_marketplace_config(config_text: str, marketplace_root: Path) -> str:
    header = f"marketplaces.{RELEASE_MARKETPLACE_NAME}"
    config_text = upsert_toml_key(config_text, header, "last_updated", toml_string(utc_timestamp()))
    config_text = upsert_toml_key(config_text, header, "source_type", '"local"')
    return upsert_toml_key(
        config_text,
        header,
        "source",
        toml_string(codex_config_path(marketplace_root)),
    )


def toml_string(value: str) -> str:
    return json.dumps(value)


def codex_config_path(path: Path) -> str:
    resolved = str(path.resolve())
    if sys.platform == "win32" and not resolved.startswith("\\\\?\\"):
        return f"\\\\?\\{resolved}"
    return resolved


def utc_timestamp() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def upsert_top_level_toml_key(config_text: str, key: str, rendered_value: str) -> str:
    key_re = re.compile(rf"(?m)^\s*{re.escape(key)}\s*=.*$")
    first_section = re.search(r"(?m)^\[", config_text)
    section_start = first_section.start() if first_section else len(config_text)
    top_level = config_text[:section_start]
    rest = config_text[section_start:]

    if key_re.search(top_level):
        return key_re.sub(lambda _: f"{key} = {rendered_value}", top_level, count=1) + rest
    if top_level and not top_level.endswith("\n"):
        top_level += "\n"
    return f"{key} = {rendered_value}\n" + top_level + rest


def remove_profile_service_tier_overrides(config_text: str) -> str:
    lines = config_text.splitlines(keepends=True)
    output: list[str] = []
    in_profile_section = False
    service_tier_re = re.compile(r"^\s*service_tier\s*=.*$")

    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            header = stripped.strip("[]").strip()
            in_profile_section = header.startswith("profiles.")
        if in_profile_section and service_tier_re.match(line):
            continue
        output.append(line)
    return "".join(output)


def upsert_toml_key(config_text: str, header: str, key: str, rendered_value: str) -> str:
    header_line = f"[{header}]"
    section_re = re.compile(rf"(?ms)^\[{re.escape(header)}\]\r?\n(?P<body>.*?)(?=^\[|\Z)")
    key_re = re.compile(rf"(?m)^\s*{re.escape(key)}\s*=.*$")
    section = f"{header_line}\n{key} = {rendered_value}\n"

    match = section_re.search(config_text)
    if not match:
        if config_text and not config_text.endswith("\n"):
            config_text += "\n"
        if config_text and not config_text.endswith("\n\n"):
            config_text += "\n"
        return config_text + section

    body = match.group("body")
    if key_re.search(body):
        new_body = key_re.sub(lambda _: f"{key} = {rendered_value}", body, count=1)
    else:
        new_body = f"{key} = {rendered_value}\n{body}"
    return config_text[: match.start()] + header_line + "\n" + new_body + config_text[match.end() :]


def update_codex_config(
    config_path: Path,
    *,
    disable_apps: bool = False,
    fast_service_tier: bool = False,
    plugin_id: str = PLUGIN_ID,
    plugin_enabled: bool = True,
    disabled_plugin_ids: list[str] | None = None,
    marketplace_root: Path | None = None,
) -> None:
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_text = config_path.read_text() if config_path.exists() else ""
    config_text = ensure_plugins_feature_enabled(config_text)
    if disable_apps:
        config_text = ensure_apps_feature_disabled(config_text)
    if fast_service_tier:
        config_text = ensure_fast_mode_enabled(config_text)
        config_text = ensure_fast_service_tier(config_text)
    if marketplace_root is not None:
        config_text = ensure_release_marketplace_config(config_text, marketplace_root)
    config_text = set_plugin_enabled(config_text, plugin_id, enabled=plugin_enabled)
    for disabled_plugin_id in disabled_plugin_ids or []:
        if disabled_plugin_id != plugin_id:
            config_text = set_plugin_enabled(config_text, disabled_plugin_id, enabled=False)
    config_path.write_text(config_text)
