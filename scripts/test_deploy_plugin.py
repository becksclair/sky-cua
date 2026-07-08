from __future__ import annotations

import argparse
import tomllib
from pathlib import Path

import pytest

import _kwin_effect as kwin_effect
import deploy_plugin
from _plugin_bundle import (
    COMPUTER_USE_COMPAT_PLUGIN_ID,
    PLUGIN_ID,
    update_codex_config,
)
from deploy_plugin import drop_retired_channel_caches


def test_update_codex_config_enables_local_id_when_compat_unavailable(tmp_path: Path) -> None:
    # The fast deploy's off-compat (e.g. Windows / bundles without openai-bundled
    # resources) path: no compat root, so update_codex_config must enable the
    # local channel id directly. Linux CI never exercises this through a real
    # deploy, so assert the branch here with no OS gating.
    config_path = tmp_path / "config.toml"

    update_codex_config(config_path, compat_enablement=False)

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["plugins"][PLUGIN_ID]["enabled"] is True
    assert parsed["plugins"][COMPUTER_USE_COMPAT_PLUGIN_ID]["enabled"] is False


def test_update_codex_config_enables_compat_id_when_available(tmp_path: Path) -> None:
    # Compat-first (Linux, the primary live path): the compat plugin id is the
    # single enabled computer-use plugin and the sky-cua@local channel id stays
    # a disabled payload carrier. Pin the toggle so a regression that silently
    # degraded compat-first to the channel-id fallback is caught.
    config_path = tmp_path / "config.toml"

    update_codex_config(config_path, compat_enablement=True)

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["plugins"][COMPUTER_USE_COMPAT_PLUGIN_ID]["enabled"] is True
    assert parsed["plugins"][PLUGIN_ID]["enabled"] is False


def test_update_codex_config_disables_retired_channels(tmp_path: Path) -> None:
    # The single-active-computer-use invariant: update_codex_config is the
    # chokepoint every codex setup path (install_plugin, deploy_plugin, installer)
    # funnels through, so a box left in the old debug/Heliasar-enabled state must
    # converge to one enabled id on the next write rather than double-serving.
    config_path = tmp_path / "config.toml"
    config_path.write_text(
        '[plugins."sky-cua@debug"]\nenabled = true\n\n'
        '[plugins."sky-cua@Heliasar"]\nenabled = true\n',
        encoding="utf-8",
    )

    update_codex_config(config_path, compat_enablement=True)

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["plugins"]["sky-cua@debug"]["enabled"] is False
    assert parsed["plugins"]["sky-cua@Heliasar"]["enabled"] is False
    assert parsed["plugins"][COMPUTER_USE_COMPAT_PLUGIN_ID]["enabled"] is True


def test_update_codex_config_does_not_synthesize_absent_retired_channels(
    tmp_path: Path,
) -> None:
    # A machine that only ever ran the live channel must not gain phantom disabled
    # stanzas (set_plugin_enabled/upsert would otherwise create them).
    config_path = tmp_path / "config.toml"

    update_codex_config(config_path, compat_enablement=True)

    plugins = tomllib.loads(config_path.read_text(encoding="utf-8")).get("plugins", {})
    assert "sky-cua@debug" not in plugins
    assert "sky-cua@Heliasar" not in plugins


def test_drop_retired_channel_caches_removes_debug_payload(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Cache hygiene only: config neutralization is update_codex_config's job, so
    # this just stops and drops the orphaned cache/<marketplace>/sky-cua tree.
    monkeypatch.setattr(deploy_plugin, "stop_unix_runtime_processes", lambda _roots: None)
    monkeypatch.setattr(deploy_plugin, "stop_windows_cache_processes", lambda _root: None)

    stale_root = tmp_path / "plugins" / "cache" / "debug" / "sky-cua" / "local"
    stale_root.mkdir(parents=True)

    drop_retired_channel_caches(tmp_path)
    drop_retired_channel_caches(tmp_path)  # idempotent: second run is a no-op

    assert not (tmp_path / "plugins" / "cache" / "debug" / "sky-cua").exists()


def test_drop_retired_channel_caches_preserves_sibling_marketplace_plugins(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # The shared Heliasar marketplace dir can hold sibling plugins (e.g.
    # clawpatch); only cache/Heliasar/sky-cua is dropped, never cache/Heliasar.
    monkeypatch.setattr(deploy_plugin, "stop_unix_runtime_processes", lambda _roots: None)
    monkeypatch.setattr(deploy_plugin, "stop_windows_cache_processes", lambda _root: None)

    cache = tmp_path / "plugins" / "cache" / "Heliasar"
    (cache / "sky-cua" / "0.1.0").mkdir(parents=True)
    sibling = cache / "clawpatch" / "0.1.0"
    sibling.mkdir(parents=True)

    drop_retired_channel_caches(tmp_path)

    assert not (cache / "sky-cua").exists()
    assert sibling.exists()  # sibling marketplace plugin left untouched


def test_drop_retired_channel_caches_noop_without_state(tmp_path: Path) -> None:
    # No stale cache dirs: must not raise and must not create anything.
    drop_retired_channel_caches(tmp_path)
    assert not (tmp_path / "plugins").exists()


def test_fast_deploy_offcompat_enables_local_and_refreshes_runtime(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    codex_home = tmp_path / "codex-home"
    config_path = codex_home / "config.toml"
    calls: dict[str, object] = {}
    atspi_refreshes = 0

    monkeypatch.setattr(deploy_plugin, "build_bundle", lambda: None)
    monkeypatch.setattr(deploy_plugin, "ensure_bundle_structure", lambda _root: None)
    monkeypatch.setattr(
        deploy_plugin,
        "drop_retired_channel_caches",
        lambda _home, **_kwargs: None,
    )
    monkeypatch.setattr(deploy_plugin, "install_bundle", lambda *_args: None)
    monkeypatch.setattr(deploy_plugin, "run_browser_preflight", lambda _dest, _home: None)
    monkeypatch.setattr(deploy_plugin, "stop_unix_runtime_processes", lambda _roots: None)
    monkeypatch.setattr(deploy_plugin, "stop_windows_cache_processes", lambda _root: None)
    monkeypatch.setattr(deploy_plugin, "compat_plugin_targets_payload", lambda _home, _dest: False)

    def fake_refresh_accessibility_bus() -> None:
        nonlocal atspi_refreshes
        atspi_refreshes += 1

    monkeypatch.setattr(deploy_plugin, "refresh_accessibility_bus", fake_refresh_accessibility_bus)

    def fake_install_local(
        target: Path,
        host: str,
        *,
        restart_runtime: bool = False,
        bundle_root: Path | None = None,
        refresh_accessibility: bool = True,
        browser_eval: str | None = None,
        model_supports_images: str | None = None,
        reap_all_runtime: bool = False,
    ) -> tuple[Path, Path]:
        calls["restart_runtime"] = restart_runtime
        calls["host"] = host
        calls["bundle_root"] = bundle_root
        calls["refresh_accessibility"] = refresh_accessibility
        calls["browser_eval"] = browser_eval
        calls["model_supports_images"] = model_supports_images
        calls["reap_all_runtime"] = reap_all_runtime
        return target / "bin" / "sky-cua-client", target / "claude_code_mcp.json"

    monkeypatch.setattr(deploy_plugin, "install_local_mcp_server", fake_install_local)
    # The companion device-setup handoff shells out to `adb`; stub it so the unit
    # test stays device-free (the lane is covered by `test_companion.py`).
    monkeypatch.setattr(deploy_plugin, "companion_setup_status", lambda: None)
    monkeypatch.setattr(deploy_plugin, "print_companion_setup_status", lambda _status: None)

    args = argparse.Namespace(
        codex_home=codex_home,
        no_build=True,
        symlink=False,
        kwin_effect=False,
        no_companion=False,
        force_companion=False,
        local_install_dir=tmp_path / "local-install",
        local_install_host="claude-code",
    )

    assert deploy_plugin.fast_deploy(args) == 0

    parsed = tomllib.loads(config_path.read_text(encoding="utf-8"))
    assert parsed["plugins"][PLUGIN_ID]["enabled"] is True
    assert parsed["plugins"][COMPUTER_USE_COMPAT_PLUGIN_ID]["enabled"] is False
    # The AT-SPI registry reset is opt-in (--refresh-accessibility); a default
    # deploy must not wipe running apps' accessibility registrations.
    assert atspi_refreshes == 0
    assert calls["restart_runtime"] is True
    assert calls["refresh_accessibility"] is False
    assert calls["host"] == "claude-code"
    # The deploy reaps the whole sky-cua stack so zombie dev-build processes
    # cannot survive to race the freshly deployed binaries.
    assert calls["reap_all_runtime"] is True


def test_fast_deploy_returns_failure_when_kwin_live_reload_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    codex_home = tmp_path / "codex-home"
    monkeypatch.setattr(deploy_plugin, "build_bundle", lambda: None)
    monkeypatch.setattr(deploy_plugin, "ensure_bundle_structure", lambda _root: None)
    monkeypatch.setattr(
        deploy_plugin,
        "drop_retired_channel_caches",
        lambda _home, **_kwargs: None,
    )
    monkeypatch.setattr(deploy_plugin, "install_bundle", lambda *_args: None)
    monkeypatch.setattr(deploy_plugin, "run_browser_preflight", lambda _dest, _home: None)
    monkeypatch.setattr(deploy_plugin, "stop_unix_runtime_processes", lambda _roots: None)
    monkeypatch.setattr(deploy_plugin, "stop_windows_cache_processes", lambda _root: None)
    monkeypatch.setattr(deploy_plugin, "compat_plugin_targets_payload", lambda _home, _dest: True)
    monkeypatch.setattr(deploy_plugin, "refresh_accessibility_bus", lambda: None)
    monkeypatch.setattr(
        deploy_plugin,
        "install_local_mcp_server",
        lambda target, _host, **_kwargs: (
            target / "bin" / "sky-cua-client",
            target / "claude_code_mcp.json",
        ),
    )

    def fake_deploy(**_kwargs: object) -> kwin_effect.ReloadOutcome:
        return kwin_effect.ReloadOutcome(
            converged=False,
            loaded=False,
            expected_build_id="new",
            running_build_id="old",
            effect_id="sky-cua-agent-cursor-000004",
            rollback_effect_id="sky-cua-agent-cursor-000003",
            live_load_attempted=True,
        )

    monkeypatch.setattr(deploy_plugin, "deploy_kwin_effect", fake_deploy)
    monkeypatch.setattr(deploy_plugin, "print_kwin_effect_deploy_outcome", lambda _outcome: None)

    args = argparse.Namespace(
        codex_home=codex_home,
        no_build=True,
        symlink=False,
        kwin_effect=True,
        no_companion=True,
        force_companion=False,
        local_install_dir=tmp_path / "local-install",
        local_install_host="claude-code",
    )

    assert deploy_plugin.fast_deploy(args) == 1
    assert "did not converge" in capsys.readouterr().err
