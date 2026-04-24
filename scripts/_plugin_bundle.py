from __future__ import annotations

import re
import shutil
import stat
from pathlib import Path

PLUGIN_NAME = "sky-cua"
PLUGIN_CHANNEL = "debug"
PLUGIN_ID = f"{PLUGIN_NAME}@{PLUGIN_CHANNEL}"
REPO_ROOT = Path(__file__).resolve().parents[1]
DIST_PLUGIN_ROOT = REPO_ROOT / "dist" / "plugin" / PLUGIN_NAME
DEFAULT_CODEX_HOME = Path.home() / ".codex"
INSTALLED_PLUGIN_ROOT = (
    DEFAULT_CODEX_HOME / "plugins" / "cache" / PLUGIN_CHANNEL / PLUGIN_NAME / "local"
)


def ensure_executable(path: Path) -> None:
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


def ensure_bundle_structure(root: Path) -> None:
    required = [
        root / ".codex-plugin" / "plugin.json",
        root / ".mcp.json",
        root / "bin" / "sky-cua-client",
        root / "bin" / "sky-cua-service",
        root / "skills" / "computer-use-workflows" / "SKILL.md",
        root / "resources" / "app-instructions" / "index.json",
    ]
    missing = [str(path) for path in required if not path.exists()]
    if missing:
        raise FileNotFoundError(f"plugin bundle is missing required paths: {missing}")
    ensure_executable(root / "bin" / "sky-cua-client")
    ensure_executable(root / "bin" / "sky-cua-service")


def installed_plugin_root(codex_home: Path) -> Path:
    return codex_home / "plugins" / "cache" / PLUGIN_CHANNEL / PLUGIN_NAME / "local"


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
    return upsert_toml_key(config_text, f'plugins."{PLUGIN_ID}"', "enabled", "true")


def upsert_top_level_toml_key(config_text: str, key: str, rendered_value: str) -> str:
    key_re = re.compile(rf"(?m)^\s*{re.escape(key)}\s*=.*$")
    first_section = re.search(r"(?m)^\[", config_text)
    section_start = first_section.start() if first_section else len(config_text)
    top_level = config_text[:section_start]
    rest = config_text[section_start:]

    if key_re.search(top_level):
        return key_re.sub(f"{key} = {rendered_value}", top_level, count=1) + rest
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
    section_re = re.compile(rf"(?ms)^\[{re.escape(header)}\]\n(?P<body>.*?)(?=^\[|\Z)")
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
        new_body = key_re.sub(f"{key} = {rendered_value}", body, count=1)
    else:
        new_body = f"{key} = {rendered_value}\n{body}"
    return config_text[: match.start()] + header_line + "\n" + new_body + config_text[match.end() :]


def update_codex_config(
    config_path: Path,
    *,
    disable_apps: bool = False,
    fast_service_tier: bool = False,
) -> None:
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_text = config_path.read_text() if config_path.exists() else ""
    config_text = ensure_plugins_feature_enabled(config_text)
    if disable_apps:
        config_text = ensure_apps_feature_disabled(config_text)
    if fast_service_tier:
        config_text = ensure_fast_mode_enabled(config_text)
        config_text = ensure_fast_service_tier(config_text)
    config_text = ensure_plugin_enabled(config_text)
    config_path.write_text(config_text)
