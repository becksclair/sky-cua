"""Standalone Codex marketplace, plugin installation and compatibility projection."""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
from collections.abc import Callable, Mapping
from pathlib import Path
from types import ModuleType

import _plugin_bundle
from _codex_app_server import CodexAppServerClient
from _standalone_hosts import (
    _copy_tree,
    _link_or_copy_directory,
    _remove_path,
    _toml_string,
    _upsert_toml_key,
    _write_text_atomically,
)

SKILL_NAMES = ("computer-use", "browser-use", "phone-use")
PLUGIN_NAMES = ("computer-use", "browser")


def _native_browser_projection_module(install_root: Path) -> ModuleType:
    path = install_root / "resources/_native_browser_projection.py"
    spec = importlib.util.spec_from_file_location("_sky_native_browser_projection", path)
    if spec is None or spec.loader is None:
        raise ValueError(f"cannot load native Browser projection helper: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _sync_codex_bundled_marketplace(
    install_root: Path,
    codex_home: Path,
    *,
    native_browser_root: Path,
    plugin_names: tuple[str, ...] = PLUGIN_NAMES,
) -> None:
    """Stage the marketplace at Codex's reserved bundled-marketplace location."""
    source_root = install_root / "codex/openai-bundled"
    source_marketplace = source_root / ".agents/plugins/marketplace.json"
    if not source_marketplace.is_file():
        return
    projection = _native_browser_projection_module(install_root)
    native_browser = None
    if "browser" in plugin_names:
        native_browser = projection.validate_native_browser_plugin(native_browser_root)
    projection.sync_native_browser_skill(
        install_root,
        codex_home,
        enabled=native_browser is not None,
        native_browser_root=native_browser.root if native_browser is not None else None,
    )
    if native_browser is not None:
        projection.materialize_native_browser_plugin(
            native_browser,
            codex_home / ".tmp/bundled-marketplaces/openai-bundled/plugins/browser",
        )
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
    for plugin_name in plugin_names:
        if plugin_name == "browser" and native_browser is not None:
            continue
        source_plugin = source_root / "plugins" / plugin_name
        if not source_plugin.is_dir():
            continue
        dest_link = codex_home / ".tmp/bundled-marketplaces/openai-bundled/plugins" / plugin_name
        _remove_path(dest_link)
        _copy_tree(source_plugin, dest_link)
        mcp_path = dest_link / ".mcp.json"
        mcp = json.loads(mcp_path.read_text(encoding="utf-8"))
        if plugin_name == "computer-use":
            mcp["mcpServers"]["computer-use"].update(
                command=str(install_root / "bin/sky-cua-client"),
                args=["mcp"],
                cwd=str(install_root),
            )
        else:
            mcp["mcpServers"]["node_repl"].update(
                command=str(install_root / "bin/node_repl"),
                cwd=str(install_root),
            )
        mcp_path.write_text(json.dumps(mcp, indent=2) + "\n", encoding="utf-8")


def _update_codex_config_for_bundled(
    codex_home: Path,
    *,
    skills_root: Path | None = None,
    enabled_skill_names: tuple[str, ...] = SKILL_NAMES,
) -> None:
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
    config_text = _plugin_bundle.apply_compat_plugin_enablement(config_text)
    config_text = _plugin_bundle.disable_retired_channels(config_text)
    config_text = _plugin_bundle.apply_browser_companion_enablement(
        config_text, enabled="browser-use" in enabled_skill_names
    )
    config_text = _plugin_bundle.apply_shared_agent_skill_deduplication(
        config_text,
        skills_root or codex_home.parent / ".agents/skills",
        codex_home=codex_home,
        enabled_skill_names=enabled_skill_names,
    )
    _write_text_atomically(config_path, config_text)


def _install_codex_plugins(
    install_root: Path,
    *,
    home: Path,
    env: Mapping[str, str],
    which: Callable[[str], str | None],
    enabled_skill_names: tuple[str, ...],
    native_codex_binary: Path,
    native_browser_root: Path,
    client_factory: Callable[..., CodexAppServerClient] = CodexAppServerClient,
    product_version: str,
    report: dict[str, object] | None = None,
) -> tuple[str, ...]:
    codex = str(native_codex_binary) if native_codex_binary.is_file() else which("codex")
    if codex is None:
        return ()
    codex_home = home / ".codex"
    plugin_names = (
        ("computer-use", "browser") if "browser-use" in enabled_skill_names else ("computer-use",)
    )
    _sync_codex_bundled_marketplace(
        install_root,
        codex_home,
        native_browser_root=native_browser_root,
        plugin_names=plugin_names,
    )
    _update_codex_config_for_bundled(codex_home, enabled_skill_names=enabled_skill_names)
    marketplace = codex_home / ".tmp/bundled-marketplaces/openai-bundled"
    command_env = os.environ.copy()
    command_env.update(env)
    command_env.update(HOME=str(home), CODEX_HOME=str(codex_home))
    client: CodexAppServerClient | None = None
    completed: list[str] = []
    if report is not None:
        report.update(mode="native-api", completed=completed, unfinished=list(plugin_names))
    phase = "start app-server"
    try:
        client = client_factory([codex, "app-server"], env=command_env, cwd=install_root)
        phase = "initialize"
        client.initialize(
            client_name="sky-cua-installer",
            client_version=product_version,
            capabilities={"experimentalApi": True},
        )
        for name in plugin_names:
            phase = f"plugin/install {name}"
            client.request(
                "plugin/install",
                {
                    "pluginName": name,
                    "marketplacePath": str(marketplace / ".agents/plugins/marketplace.json"),
                },
            )
            completed.append(name)
            if report is not None:
                report["unfinished"] = [
                    plugin for plugin in plugin_names if plugin not in completed
                ]
        phase = "computer-use cache validation"
        manifest = json.loads(
            (marketplace / "plugins/computer-use/.codex-plugin/plugin.json").read_text(
                encoding="utf-8"
            )
        )
        version = manifest["version"]
        if not isinstance(version, str) or not version or Path(version).name != version:
            raise ValueError("installed Computer Use plugin has an invalid version")
        cache_root = _plugin_bundle.compat_plugin_cache_root(codex_home)
        installed = cache_root / version
        if not installed.is_dir():
            raise FileNotFoundError(f"Codex did not materialize its plugin cache: {installed}")
        _link_or_copy_directory(installed, cache_root / "latest")
    except Exception as error:
        unfinished = [name for name in plugin_names if name not in completed]
        raise RuntimeError(
            f"Codex installation failed during {phase}; "
            f"completed={completed}, unfinished={unfinished}: {error}"
        ) from error
    finally:
        if client is not None:
            client.close()
        # Installation enables plugins; restore the durable surface mask afterwards.
        _update_codex_config_for_bundled(codex_home, enabled_skill_names=enabled_skill_names)
    return plugin_names
