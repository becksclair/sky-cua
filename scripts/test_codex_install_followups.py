"""Codex install policy, complete projections and native API compatibility regressions."""

from __future__ import annotations

import json
import os
import shutil
import sys
import tomllib
from pathlib import Path
from typing import Any

import pytest

import _standalone_codex
import standalone_release
from test_plugin_bundle import load_chrome_preflight

REPO_ROOT = Path(__file__).resolve().parents[1]


def _write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def _payload(root: Path) -> Path:
    marketplace = root / "codex/openai-bundled"
    _write(
        marketplace / ".agents/plugins/marketplace.json",
        json.dumps(
            {
                "name": "openai-bundled",
                "plugins": [
                    {"name": name, "source": {"source": "local", "path": f"./plugins/{name}"}}
                    for name in ("computer-use", "browser")
                ],
            }
        ),
    )
    for name in ("computer-use", "browser"):
        plugin = marketplace / "plugins" / name
        _write(plugin / ".codex-plugin/plugin.json", json.dumps({"name": name, "version": "test"}))
        server = "computer-use" if name == "computer-use" else "node_repl"
        _write(plugin / ".mcp.json", json.dumps({"mcpServers": {server: {"command": server}}}))
        _write(plugin / "skills" / name / "SKILL.md", f"{name} instructions")
        _write(plugin / "scripts/browser-client.mjs", "export const fixture = true;\n")
    resources = root / "resources"
    resources.mkdir()
    for resource in ("_native_browser_projection.py",):
        shutil.copyfile(REPO_ROOT / "resources" / resource, resources / resource)
    _write(root / "browser/sky-enhancements.mjs", "export const fixture = true;\n")
    _write(
        root / "resources/sky-browser-enhancements/SKILL.md",
        "Sky @SKY_CUA_ROOT_JSON@\nNative @NATIVE_BROWSER_SKILL@\n",
    )
    return root


class _Client:
    def __init__(self, command: list[str], *, env: dict[str, str], cwd: Path) -> None:
        self.codex_home = Path(env["CODEX_HOME"])

    def initialize(self, **_kwargs: Any) -> dict[str, Any]:
        return {}

    def request(self, method: str, params: dict[str, str]) -> dict[str, Any]:
        assert method == "plugin/install"
        marketplace = self.codex_home / ".tmp/bundled-marketplaces/openai-bundled"
        assert Path(params["marketplacePath"]) == marketplace / ".agents/plugins/marketplace.json"
        name = params["pluginName"]
        # Codex's copier omits symlinks, so source projections must contain regular entries.
        shutil.copytree(
            marketplace / "plugins" / name,
            self.codex_home / "plugins/cache/openai-bundled" / name / "test",
            ignore=lambda directory, names: [
                entry for entry in names if (Path(directory) / entry).is_symlink()
            ],
        )
        return {}

    def close(self) -> None:
        pass


@pytest.mark.parametrize("native_state", ["absent", "malformed"])
def test_disabled_browser_never_discovers_native_package_and_installs_computer(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, native_state: str
) -> None:
    payload = _payload(tmp_path / "sky")
    native = tmp_path / "native/plugins/browser"
    native.parent.mkdir(parents=True)
    if native_state == "malformed":
        _write(native / ".codex-plugin/plugin.json", "{")
    monkeypatch.setattr(standalone_release, "NATIVE_BROWSER_PLUGIN_ROOT", native)
    monkeypatch.setattr(standalone_release, "NATIVE_CODEX_BINARY", tmp_path / "no-codex")
    monkeypatch.setattr(standalone_release, "CodexAppServerClient", _Client)

    projection = standalone_release._native_browser_projection_module(payload)

    def unexpected_probe(_native_root: Path) -> None:
        raise AssertionError("disabled Browser must not discover native packages")

    monkeypatch.setattr(projection, "validate_native_browser_plugin", unexpected_probe)
    monkeypatch.setattr(
        _standalone_codex, "_native_browser_projection_module", lambda _root: projection
    )
    home = tmp_path / "home"
    report: dict[str, object] = {}
    assert standalone_release._install_codex_plugins(
        payload,
        home=home,
        env={},
        which=lambda _name: "codex",
        enabled_skill_names=("computer-use",),
        report=report,
    ) == ("computer-use",)
    assert report == {"mode": "native-api", "completed": ["computer-use"], "unfinished": []}
    assert (home / ".codex/plugins/cache/openai-bundled/computer-use/latest/.mcp.json").is_file()
    assert not (home / ".codex/.tmp/bundled-marketplaces/openai-bundled/plugins/browser").exists()


@pytest.mark.parametrize("native_state", ["absent", "malformed"])
def test_disabled_preflight_does_not_probe_native_and_continues_computer_sync(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, native_state: str
) -> None:
    preflight = load_chrome_preflight(tmp_path)
    native = tmp_path / "native/plugins/browser"
    native.parent.mkdir(parents=True)
    if native_state == "malformed":
        _write(native / ".codex-plugin/plugin.json", "{")
    monkeypatch.setattr(preflight, "NATIVE_BROWSER_PLUGIN_ROOT", native)
    policy = tmp_path / "policy.toml"
    _write(policy, "[surfaces]\nbrowser = false\n")
    monkeypatch.setenv("SKY_CUA_CONFIG_PATH", str(policy))
    source = tmp_path / "source"
    _write(source / ".agents/plugins/marketplace.json", '{"plugins": []}')
    home = tmp_path / "codex"
    calls: list[str] = []

    def unexpected_probe(_candidate: Path) -> None:
        raise AssertionError("disabled Browser must not discover or validate native plugins")

    monkeypatch.setattr(preflight, "validate_native_browser_plugin", unexpected_probe)
    monkeypatch.setattr(
        preflight, "sync_computer_use_compat_plugin", lambda *_args: calls.append("computer")
    )
    monkeypatch.setattr(
        sys, "argv", ["preflight", "--source-root", str(source), "--codex-home", str(home)]
    )
    assert preflight.main() == 0
    assert calls == ["computer"]
    assert not (home / ".tmp/bundled-marketplaces/openai-bundled/plugins/browser").exists()


@pytest.mark.parametrize("chrome_exists", [False, True])
def test_browser_only_preflight_converges_existing_chrome_off_and_on(
    tmp_path: Path, chrome_exists: bool
) -> None:
    preflight = load_chrome_preflight(tmp_path)
    home = tmp_path / "codex"
    config_path = home / "config.toml"
    if chrome_exists:
        _write(config_path, '[plugins."chrome@openai-bundled"]\nenabled = true\n')
    for enabled in (False, True, False, True):
        preflight.update_codex_config(home, browser_only=True, browser_surface_enabled=enabled)
        config = tomllib.loads(config_path.read_text()) if config_path.exists() else {}
        chrome = config.get("plugins", {}).get("chrome@openai-bundled")
        if chrome_exists:
            assert chrome["enabled"] is enabled
        else:
            assert chrome is None


@pytest.mark.parametrize("existing_config", [False, True])
def test_disabled_sourceless_preflight_only_disables_existing_plugins(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, existing_config: bool
) -> None:
    preflight = load_chrome_preflight(tmp_path)
    policy = tmp_path / "policy.toml"
    _write(policy, "[surfaces]\nbrowser = false\n")
    monkeypatch.setenv("SKY_CUA_CONFIG_PATH", str(policy))
    home = tmp_path / "codex"
    config_path = home / "config.toml"
    marketplace_config = (
        "# Keep the operator marketplace and feature policy verbatim.\n"
        "[features]\nplugins = false # deliberately disabled\n\n"
        "[marketplaces.openai-bundled]\n"
        'source_type = "git"\nsource = "https://example.test/plugins.git"\n'
        'last_updated = "2026-01-02T03:04:05Z"\n\n'
    )
    plugin_config = "".join(
        f'[plugins."{name}@openai-bundled"]\nenabled = true\n'
        for name in ("browser", "chrome", "browser-use")
    )
    if existing_config:
        _write(config_path, marketplace_config + plugin_config)
    monkeypatch.setattr(
        sys,
        "argv",
        ["preflight", "--source-root", str(tmp_path / "missing"), "--codex-home", str(home)],
    )
    assert preflight.main() == 0
    if existing_config:
        assert config_path.read_text() == marketplace_config + plugin_config.replace(
            "enabled = true", "enabled = false"
        )
        # Repeating the disable operation remains stable.
        assert preflight.main() == 0
        assert config_path.read_text() == marketplace_config + plugin_config.replace(
            "enabled = true", "enabled = false"
        )
    else:
        assert not config_path.exists()
        assert not home.exists()


def test_absent_native_package_with_existing_parent_uses_complete_fallback(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = _payload(tmp_path / "sky")
    native = tmp_path / "native/plugins/browser"
    native.parent.mkdir(parents=True)
    monkeypatch.setattr(standalone_release, "NATIVE_BROWSER_PLUGIN_ROOT", native)
    home = tmp_path / "codex"
    standalone_release._sync_codex_bundled_marketplace(payload, home)
    browser = home / ".tmp/bundled-marketplaces/openai-bundled/plugins/browser"
    assert not [path for path in browser.rglob("*") if path.is_symlink()]
    assert (browser / "skills/browser/SKILL.md").read_text() == "browser instructions"
    server = json.loads((browser / ".mcp.json").read_text())["mcpServers"]["node_repl"]
    assert server["command"] == str(payload / "bin/node_repl")
    assert server["cwd"] == str(payload)


def test_standalone_preflight_binds_fallback_to_own_runtime_on_install_and_reinstall(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    preflight = load_chrome_preflight(tmp_path)
    payload = _payload(tmp_path / "sky")
    _write(payload / "RELEASE.json", "{}")
    monkeypatch.setattr(preflight, "__file__", str(payload / "resources/chrome_preflight.py"))
    home = tmp_path / "codex"
    source = payload / "codex/openai-bundled"
    for _revision in range(2):
        preflight.sync_browser_plugin(source, home)
        cached = home / "plugins/cache/openai-bundled/browser/test/.mcp.json"
        mcp = json.loads(cached.read_text())
        assert mcp["mcpServers"]["node_repl"]["command"] == str(payload / "bin/node_repl")
        assert mcp["mcpServers"]["node_repl"]["cwd"] == str(payload)
        # A reinstall must also correct stale PATH-dependent cache bindings.
        mcp["mcpServers"]["node_repl"]["command"] = "node_repl"
        cached.write_text(json.dumps(mcp), encoding="utf-8")


def test_present_malformed_native_package_fails_before_projection(tmp_path: Path) -> None:
    payload = _payload(tmp_path / "sky")
    native = tmp_path / "native/plugins/browser"
    _write(native / ".codex-plugin/plugin.json", "{")
    home = tmp_path / "codex"
    with pytest.raises(ValueError, match="cannot read native app Browser manifest"):
        standalone_release._sync_codex_bundled_marketplace(
            payload, home, native_browser_root=native
        )
    assert not home.exists()


def test_baseline_reuses_validated_native_manifest(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = _payload(tmp_path / "sky")
    native = tmp_path / "native/plugins/browser"
    _write(native / ".codex-plugin/plugin.json", '{"name":"browser","version":"native"}')
    for relative in (
        "scripts/browser-client.mjs",
        "scripts/browser-service.mjs",
        "docs/api.json",
        "skills/control-in-app-browser/SKILL.md",
    ):
        _write(native / relative, "native")
    projection = standalone_release._native_browser_projection_module(payload)
    original = projection._native_manifest
    reads: list[Path] = []

    def read_manifest(root: Path) -> dict[str, Any]:
        reads.append(root)
        return original(root)

    monkeypatch.setattr(projection, "_native_manifest", read_manifest)
    monkeypatch.setattr(
        _standalone_codex, "_native_browser_projection_module", lambda _root: projection
    )
    standalone_release._sync_codex_bundled_marketplace(
        payload, tmp_path / "codex", native_browser_root=native
    )
    assert reads == [native]


@pytest.mark.parametrize(
    "failure",
    [
        RuntimeError("reserved and cannot be loaded from this source"),
        RuntimeError("unrelated RPC failure"),
        TimeoutError("app-server timeout"),
    ],
)
def test_install_failure_reports_partial_completion_and_never_silently_falls_back(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, failure: Exception
) -> None:
    payload = _payload(tmp_path / "sky")

    class FailingClient(_Client):
        def request(self, method: str, params: dict[str, str]) -> dict[str, Any]:
            if params["pluginName"] == "browser":
                raise failure
            return super().request(method, params)

    monkeypatch.setattr(standalone_release, "NATIVE_CODEX_BINARY", tmp_path / "absent-codex")
    monkeypatch.setattr(
        standalone_release, "NATIVE_BROWSER_PLUGIN_ROOT", tmp_path / "absent-browser"
    )
    monkeypatch.setattr(standalone_release, "CodexAppServerClient", FailingClient)
    home = tmp_path / "home"
    with pytest.raises(
        RuntimeError, match="completed=\\['computer-use'\\], unfinished=\\['browser'\\]"
    ) as error:
        standalone_release._install_codex_plugins(payload, home=home, which=lambda _name: "codex")
    assert error.value.__cause__ is failure
    assert "plugin/install browser" in str(error.value)
    assert not (home / ".codex/plugins/cache/openai-bundled/browser").exists()


@pytest.mark.parametrize("phase", ["start app-server", "initialize"])
def test_app_server_startup_failure_reports_no_completed_plugins(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, phase: str
) -> None:
    payload = _payload(tmp_path / "sky")
    failure = RuntimeError("process failure")

    class FailingClient(_Client):
        def __init__(self, command: list[str], *, env: dict[str, str], cwd: Path) -> None:
            if phase == "start app-server":
                raise failure
            super().__init__(command, env=env, cwd=cwd)

        def initialize(self, **_kwargs: Any) -> dict[str, Any]:
            raise failure

    monkeypatch.setattr(standalone_release, "NATIVE_CODEX_BINARY", tmp_path / "absent-codex")
    monkeypatch.setattr(
        standalone_release, "NATIVE_BROWSER_PLUGIN_ROOT", tmp_path / "absent-browser"
    )
    monkeypatch.setattr(standalone_release, "CodexAppServerClient", FailingClient)
    with pytest.raises(RuntimeError) as error:
        standalone_release._install_codex_plugins(
            payload, home=tmp_path / "home", which=lambda _name: "codex"
        )
    assert phase in str(error.value)
    assert "completed=[], unfinished=['computer-use', 'browser']" in str(error.value)
    assert error.value.__cause__ is failure


def test_real_codex_native_plugin_api_uses_reserved_marketplace_location(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Opt-in isolated-HOME acceptance for an explicitly selected Codex binary."""
    binary = os.environ.get("SKY_CUA_TEST_CODEX_BINARY")
    if not binary:
        pytest.skip("set SKY_CUA_TEST_CODEX_BINARY to run real Codex plugin API acceptance")
    payload = _payload(tmp_path / "sky")
    monkeypatch.setattr(standalone_release, "NATIVE_CODEX_BINARY", Path(binary))
    monkeypatch.setattr(
        standalone_release, "NATIVE_BROWSER_PLUGIN_ROOT", tmp_path / "absent-browser"
    )
    home = tmp_path / "home"
    report: dict[str, object] = {}
    assert standalone_release._install_codex_plugins(
        payload, home=home, env={}, which=lambda _name: None, report=report
    ) == ("computer-use", "browser")
    assert report["mode"] == "native-api"
    assert report["unfinished"] == []
    for name in ("computer-use", "browser"):
        cached = home / ".codex/plugins/cache/openai-bundled" / name / "test"
        assert (cached / ".mcp.json").is_file()
        assert (cached / "skills" / name / "SKILL.md").is_file()
        assert not [path for path in cached.rglob("*") if path.is_symlink()]
    config = tomllib.loads((home / ".codex/config.toml").read_text())
    assert config["plugins"]["browser@openai-bundled"]["enabled"] is True


@pytest.mark.parametrize("reason", ["disabled", "missing-native"])
def test_native_skill_refreshes_and_removes_only_managed_skill(tmp_path: Path, reason: str) -> None:
    payload = _payload(tmp_path / 'root with "quotes" and spaces')
    native = tmp_path / "native/plugins/browser"
    _write(native / ".codex-plugin/plugin.json", '{"name":"browser","version":"native"}')
    for relative in (
        "scripts/browser-client.mjs",
        "scripts/browser-service.mjs",
        "docs/api.json",
        "skills/control-in-app-browser/SKILL.md",
    ):
        _write(native / relative, "native")
    home = tmp_path / "codex"
    skill = home / "skills/sky-browser-enhancements"
    unrelated = home / "skills/personal/SKILL.md"
    _write(unrelated, "preserve me")
    standalone_release._sync_codex_bundled_marketplace(payload, home, native_browser_root=native)
    assert (skill / ".sky-cua-managed").is_file()
    assert (skill / "SKILL.md").read_text() == (
        f"Sky {json.dumps(str(payload))}\n"
        f"Native {native / 'skills/control-in-app-browser/SKILL.md'}\n"
    )
    _write(payload / "resources/sky-browser-enhancements/SKILL.md", "Updated @SKY_CUA_ROOT_JSON@\n")
    standalone_release._sync_codex_bundled_marketplace(payload, home, native_browser_root=native)
    assert (skill / "SKILL.md").read_text() == f"Updated {json.dumps(str(payload))}\n"
    standalone_release._sync_codex_bundled_marketplace(
        payload,
        home,
        native_browser_root=(native if reason == "disabled" else tmp_path / "absent"),
        plugin_names=(("computer-use",) if reason == "disabled" else ("computer-use", "browser")),
    )
    assert not skill.exists()
    assert unrelated.read_text() == "preserve me"
    _write(skill / "SKILL.md", "user-owned")
    standalone_release._sync_codex_bundled_marketplace(
        payload, home, native_browser_root=tmp_path / "absent"
    )
    assert (skill / "SKILL.md").read_text() == "user-owned"


@pytest.mark.parametrize("defect", ["unmanaged-skill", "missing-source"])
def test_native_skill_fails_before_native_projection_when_unavailable(
    tmp_path: Path, defect: str
) -> None:
    payload = _payload(tmp_path / "sky")
    native = tmp_path / "native/plugins/browser"
    _write(native / ".codex-plugin/plugin.json", '{"name":"browser","version":"native"}')
    for relative in (
        "scripts/browser-client.mjs",
        "scripts/browser-service.mjs",
        "docs/api.json",
        "skills/control-in-app-browser/SKILL.md",
    ):
        _write(native / relative, "native")
    home = tmp_path / "codex"
    skill = home / "skills/sky-browser-enhancements/SKILL.md"
    if defect == "unmanaged-skill":
        _write(skill, "user-owned")
        match = "unmanaged Sky Browser skill"
    else:
        (payload / "resources/sky-browser-enhancements/SKILL.md").unlink()
        match = "supplemental skill is missing"
    with pytest.raises(ValueError, match=match):
        standalone_release._sync_codex_bundled_marketplace(
            payload, home, native_browser_root=native
        )
    assert not (home / ".tmp").exists()
    if defect == "unmanaged-skill":
        assert skill.read_text() == "user-owned"


def test_preflight_missing_native_removes_stale_owned_skill(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    preflight = load_chrome_preflight(tmp_path)
    home = tmp_path / "codex"
    owned = home / "skills/sky-browser-enhancements"
    _write(owned / ".sky-cua-managed", "sky-cua\n")
    _write(owned / "SKILL.md", "stale")
    monkeypatch.setattr(
        sys,
        "argv",
        ["preflight", "--source-root", str(tmp_path / "missing"), "--codex-home", str(home)],
    )
    assert preflight.main() == 0
    assert not owned.exists()
