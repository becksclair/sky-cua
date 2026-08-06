from __future__ import annotations

import json
import subprocess
import sys
import tarfile
from collections.abc import Sequence
from pathlib import Path
from typing import Any, cast

import pytest

import _opencode_config
import standalone_release
from standalone_release import (
    ARCHIVE_NAME,
    PAYLOAD_DIR_NAME,
    assemble_payload,
    build_payload,
    install_payload,
)


def _write(path: Path, content: str = "fixture\n", *, executable: bool = False) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    if executable:
        path.chmod(0o755)


def test_bundled_codex_mcp_servers_are_preapproved() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    for plugin_name, server_name in (("computer-use", "computer-use"), ("browser", "node_repl")):
        config = json.loads(
            (
                repo_root
                / "resources/codex-compat/openai-bundled/plugins"
                / plugin_name
                / ".mcp.json"
            ).read_text(encoding="utf-8")
        )
        assert config["mcpServers"][server_name]["default_tools_approval_mode"] == "approve"


def _fixture_repo(root: Path) -> tuple[Path, Path]:
    for relative in (
        "packages/browser-use/build/browser-client.mjs",
        "resources/codex-compat/openai-bundled/.agents/plugins/marketplace.json",
        "resources/codex-compat/openai-bundled/plugins/computer-use/.mcp.json",
        "resources/codex-compat/openai-bundled/plugins/computer-use/.codex-plugin/plugin.json",
        "resources/codex-compat/openai-bundled/plugins/computer-use/assets/app-icon.png",
        "resources/codex-compat/openai-bundled/plugins/browser/.mcp.json",
        "resources/codex-compat/openai-bundled/plugins/browser/.codex-plugin/plugin.json",
        "resources/codex-compat/openai-bundled/plugins/browser/assets/browser.png",
        "resources/codex-compat/openai-bundled/plugins/browser/assets/composer-icon.png",
        "resources/codex-compat/openai-bundled/plugins/browser/scripts/browser-client.mjs",
        "resources/codex-compat/openai-bundled/plugins/browser/skills/control-in-app-browser/SKILL.md",
        "resources/model-documentation/README.md",
        "scripts/_codex_app_server.py",
        "scripts/_hermes_config.py",
        "scripts/_opencode_config.py",
        "scripts/_plugin_bundle.py",
        "scripts/_standalone_release_command.py",
        "install.py",
    ):
        _write(root / relative, "{}\n" if relative.endswith(".json") else "fixture\n")
    _write(
        root / "resources/codex-compat/openai-bundled/.agents/plugins/marketplace.json",
        json.dumps(
            {
                "name": "openai-bundled",
                "plugins": [
                    {
                        "name": "computer-use",
                        "source": {"source": "local", "path": "./plugins/computer-use"},
                    },
                    {
                        "name": "browser",
                        "source": {"source": "local", "path": "./plugins/browser"},
                    },
                ],
            }
        ),
    )
    _write(
        root
        / "resources/codex-compat/openai-bundled/plugins/computer-use/.codex-plugin/plugin.json",
        json.dumps(
            {
                "name": "computer-use",
                "version": "test",
                "interface": {"logo": "./assets/app-icon.png"},
            }
        ),
    )
    _write(
        root / "resources/codex-compat/openai-bundled/plugins/browser/.codex-plugin/plugin.json",
        json.dumps(
            {
                "name": "browser",
                "version": "test",
                "interface": {
                    "composerIcon": "./assets/composer-icon.png",
                    "logo": "./assets/browser.png",
                },
            }
        ),
    )
    _write(
        root
        / "resources/codex-compat/openai-bundled/plugins/browser/skills/control-in-app-browser/SKILL.md",
        (
            "setupBrowserRuntime\n"
            'entry.type === "iab"\n'
            'entry.transport === "host_provided_iab"\n'
            "extension_native_host\n"
        ),
    )
    _write(
        root / "resources/codex-compat/openai-bundled/plugins/browser/scripts/browser-client.mjs",
        "const semanticPath = release?.paths?.browser_client;\nsetupBrowserRuntime;\n",
    )
    for name in standalone_release.SKILL_NAMES:
        _write(root / f"skills/{name}/SKILL.md", f"# {name}\n")
    _write(root / "out/components/model-documentation/README.md", "# model docs\n")
    for name in ("api", "capability", "example", "routing"):
        _write(
            root / f"out/components/model-documentation/inventories/{name}-inventory.json",
            "{}\n",
        )

    core = root / "core"
    for name in ("sky-cua-client", "sky-cua-service", "sky-cua-overlay-host"):
        _write(core / f"bin/{name}", executable=True)
    for name in ("sky-cua-cosmic-helper", "sky-cua-input-helper"):
        _write(core / f"bin/runtimes/linux-x64/{name}", executable=True)
    _write(core / "bin/runtimes/linux-x64/sky-cua-chrome-host", executable=True)
    _write(core / "resources/chrome-extension/codex/1_0/manifest.json", "{}\n")

    cua_node = root / "cua-node"
    for name in ("node", "node_repl"):
        _write(cua_node / f"bin/{name}", executable=True)
    _write(cua_node / "lib/node_modules/package/index.js")
    _write(cua_node / "share/playwright/README")
    _write(cua_node / "manifest.json", "{}\n")
    _write(cua_node / "sbom.cdx.json", "{}\n")
    return core, cua_node


def test_computer_use_marketplace_metadata_includes_phone_capability() -> None:
    manifest_path = (
        standalone_release.REPO_ROOT
        / "resources/codex-compat/openai-bundled/plugins/computer-use/.codex-plugin/plugin.json"
    )
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))

    assert {"android", "phone", "mobile-automation"} <= set(manifest["keywords"])
    interface = manifest["interface"]
    assert interface["shortDescription"] == "Control desktop apps and Android devices"
    assert "connected Android devices" in interface["longDescription"]
    assert any("Android phone" in prompt for prompt in interface["defaultPrompt"])


def test_build_owns_generated_inputs_and_emits_one_fixed_archive(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core_fixture, cua_fixture = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)

    calls: list[tuple[str, ...]] = []

    def fake_runner(command: Sequence[str]) -> None:
        calls.append(tuple(command))
        if command[0] == "bun":
            return
        if "build_model_documentation.py" in command[1]:
            return
        output_flag = "--output-root" if "assemble_cua_node.py" in command[1] else "--dist-root"
        output = Path(command[command.index(output_flag) + 1])
        source = cua_fixture if output_flag == "--output-root" else core_fixture
        standalone_release._copy_tree(source, output)

    payload, archive = build_payload(
        tmp_path / "dist",
        create_archive=True,
        portable_x86_64_v3=True,
        runner=fake_runner,
    )

    assert archive == tmp_path / "dist" / ARCHIVE_NAME
    assert archive is not None
    assert archive.is_file()
    assert [command[:3] for command in calls[:3]] == [
        ("bun", "install", "--frozen-lockfile"),
        ("bun", "install", "--frozen-lockfile"),
        ("bun", "install", "--frozen-lockfile"),
    ]
    assert [Path(command[1]).name for command in calls[3:]] == [
        "build_model_documentation.py",
        "assemble_cua_node.py",
        "build_plugin.py",
    ]
    assert calls[-1][-1] == "--portable-x86-64-v3"
    assert (payload / "browser/browser-client.mjs").is_file()
    assert (payload / "scripts/_standalone_release_command.py").is_file()
    assert (payload / "docs/inventories/routing-inventory.json").is_file()
    assert not (payload / "resources/release").exists()
    assert not (payload / "resources/model-documentation").exists()
    assert not (payload / "resources/chrome-extension").exists()
    assert not (payload / "current").exists()
    assert not (payload / "releases").exists()
    with tarfile.open(archive, "r:gz") as bundle:
        names = set(bundle.getnames())
    assert f"{PAYLOAD_DIR_NAME}/bin/node_repl" in names
    assert f"{PAYLOAD_DIR_NAME}/codex/openai-bundled/plugins/computer-use/.mcp.json" in names
    assert (
        f"{PAYLOAD_DIR_NAME}/codex/openai-bundled/plugins/computer-use/assets/app-icon.png" in names
    )
    assert f"{PAYLOAD_DIR_NAME}/codex/openai-bundled/plugins/browser/.mcp.json" in names
    assert (
        f"{PAYLOAD_DIR_NAME}/codex/openai-bundled/plugins/browser/scripts/browser-client.mjs"
        in names
    )
    assert f"{PAYLOAD_DIR_NAME}/codex/openai-bundled/plugins/browser/assets/browser.png" in names
    assert (
        f"{PAYLOAD_DIR_NAME}/codex/openai-bundled/plugins/browser/assets/composer-icon.png" in names
    )
    assert not any(
        name.startswith(f"{PAYLOAD_DIR_NAME}/codex/openai-bundled/plugins/browser-use/")
        for name in names
    )
    marketplace = json.loads(
        (payload / "codex/openai-bundled/.agents/plugins/marketplace.json").read_text(
            encoding="utf-8"
        )
    )
    assert [plugin["name"] for plugin in marketplace["plugins"]] == [
        "computer-use",
        "browser",
    ]
    computer_use_plugin = payload / "codex/openai-bundled/plugins/computer-use"
    computer_use_manifest = json.loads(
        (computer_use_plugin / ".codex-plugin/plugin.json").read_text(encoding="utf-8")
    )
    assert computer_use_manifest["interface"]["logo"] == "./assets/app-icon.png"
    assert (computer_use_plugin / "assets/app-icon.png").read_text(encoding="utf-8") == "fixture\n"
    browser_plugin = payload / "codex/openai-bundled/plugins/browser"
    plugin_manifest = json.loads(
        (browser_plugin / ".codex-plugin/plugin.json").read_text(encoding="utf-8")
    )
    assert plugin_manifest["name"] == "browser"
    assert plugin_manifest["interface"]["composerIcon"] == "./assets/composer-icon.png"
    assert plugin_manifest["interface"]["logo"] == "./assets/browser.png"
    skill = (browser_plugin / "skills/control-in-app-browser/SKILL.md").read_text(encoding="utf-8")
    assert "setupBrowserRuntime" in skill
    assert 'entry.type === "iab"' in skill
    assert 'entry.transport === "host_provided_iab"' in skill
    assert "extension_native_host" in skill
    adapter = (browser_plugin / "scripts/browser-client.mjs").read_text(encoding="utf-8")
    assert "paths?.browser_client" in adapter
    assert "setupBrowserRuntime" in adapter


def test_local_build_payload_omits_portable_release_mode(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core_fixture, cua_fixture = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)

    calls: list[tuple[str, ...]] = []

    def fake_runner(command: Sequence[str]) -> None:
        calls.append(tuple(command))
        if command[0] == "bun" or "build_model_documentation.py" in command[1]:
            return
        output_flag = "--output-root" if "assemble_cua_node.py" in command[1] else "--dist-root"
        output = Path(command[command.index(output_flag) + 1])
        source = cua_fixture if output_flag == "--output-root" else core_fixture
        standalone_release._copy_tree(source, output)

    build_payload(
        tmp_path / "dist",
        create_archive=False,
        portable_x86_64_v3=False,
        runner=fake_runner,
    )

    assert Path(calls[-1][1]).name == "build_plugin.py"
    assert "--portable-x86-64-v3" not in calls[-1]


def test_install_replaces_one_tree_and_projects_stable_paths(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core, cua_node = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)
    first = tmp_path / "first"
    assemble_payload(first, core_root=core, cua_node_root=cua_node)
    _write(first / "old-only-marker")

    home = tmp_path / "home"
    env = {"HOME": str(home), "XDG_DATA_HOME": str(tmp_path / "xdg-data")}
    first_report = install_payload(first, home=home, env=env, configure_hosts=False)
    install_root = Path(str(first_report["install_root"]))
    assert (install_root / "old-only-marker").is_file()
    legacy_node = home / ".local/bin/node"
    legacy_node.symlink_to(install_root / "bin/node")

    second = tmp_path / "second"
    assemble_payload(second, core_root=core, cua_node_root=cua_node)
    _write(second / "new-marker")
    install_payload(second, home=home, env=env, configure_hosts=False)
    assert not legacy_node.exists()
    assert not legacy_node.is_symlink()

    host_node = legacy_node
    _write(host_node, "#!/bin/sh\nprintf 'host node\\n'\n", executable=True)
    second_report = install_payload(second, home=home, env=env, configure_hosts=False)

    assert second_report["install_root"] == str(install_root)
    assert (install_root / "new-marker").is_file()
    assert not (install_root / "old-only-marker").exists()
    assert not (install_root / "current").exists()
    assert not (install_root / "releases").exists()
    assert host_node.read_text(encoding="utf-8") == "#!/bin/sh\nprintf 'host node\\n'\n"
    assert (home / ".local/bin/node_repl").resolve() == install_root / "bin/node_repl"
    for name in ("sky-cua-cosmic-helper", "sky-cua-input-helper"):
        assert (home / ".local/bin" / name).resolve() == (
            install_root / "bin/runtimes/linux-x64" / name
        )
    for name in standalone_release.SKILL_NAMES:
        assert (home / ".agents/skills" / name).resolve() == install_root / "skills" / name
    native_manifest = json.loads(
        (
            home / ".config/google-chrome/NativeMessagingHosts/com.openai.codexextension.json"
        ).read_text(encoding="utf-8")
    )
    assert native_manifest["path"] == str(home / ".local/bin/sky-cua-chrome-host")

    install_payload(second, home=home, env=env, configure_hosts=False)
    assert (install_root / "new-marker").is_file()


def test_install_preserves_unresolvable_user_node_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    root = tmp_path / "fixture"
    core, cua_node = _fixture_repo(root)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", root)
    payload = tmp_path / "payload"
    assemble_payload(payload, core_root=core, cua_node_root=cua_node)
    home = tmp_path / "home"
    legacy_node = home / ".local/bin/node"
    legacy_node.parent.mkdir(parents=True)
    legacy_node.symlink_to("node")

    install_payload(payload, home=home, env={}, configure_hosts=False)

    assert legacy_node.is_symlink()
    assert legacy_node.readlink() == Path("node")


def test_install_stops_fixed_root_runtime_before_replacing_tree(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core, cua_node = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)
    payload = tmp_path / "payload"
    assemble_payload(payload, core_root=core, cua_node_root=cua_node)
    home = tmp_path / "home"
    env = {"HOME": str(home), "XDG_DATA_HOME": str(tmp_path / "xdg-data")}
    install_root = Path(env["XDG_DATA_HOME"]) / "sky-cua"
    _write(install_root / "stale-runtime-marker")
    calls: list[Path] = []

    def fake_stop(search_roots: list[Path]) -> None:
        assert (install_root / "stale-runtime-marker").is_file()
        calls.extend(search_roots)

    monkeypatch.setattr(
        standalone_release._plugin_bundle,
        "stop_unix_runtime_processes",
        fake_stop,
    )

    install_payload(payload, home=home, env=env, configure_hosts=False)

    assert calls == [install_root]
    assert not (install_root / "stale-runtime-marker").exists()


def test_install_projects_both_mcp_servers_into_existing_hermes_config(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core, cua_node = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)
    payload = tmp_path / "payload"
    assemble_payload(payload, core_root=core, cua_node_root=cua_node)
    home = tmp_path / "home"
    hermes_config = home / ".hermes/config.yaml"
    hermes_config.parent.mkdir(parents=True)
    hermes_config.write_text("mcp_servers:\n  unrelated:\n    command: keep-me\n", encoding="utf-8")
    env = {"HOME": str(home), "XDG_DATA_HOME": str(tmp_path / "xdg-data")}

    report = install_payload(
        payload,
        home=home,
        env=env,
        which=lambda _name: None,
    )

    install_root = Path(str(report["install_root"]))
    hermes_report = cast(dict[str, object], report["hermes_config"])
    assert hermes_report["status"] == "updated"
    assert hermes_report["servers"] == ["sky_cua", "node_repl"]
    agents_report = cast(dict[str, object], hermes_report["agents"])
    assert agents_report["status"] == "updated"
    assert "mcp__node_repl__js" in (home / ".hermes/AGENTS.md").read_text(encoding="utf-8")
    text = hermes_config.read_text(encoding="utf-8")
    assert "  unrelated:\n    command: keep-me\n" in text
    sky_line = next(line for line in text.splitlines() if line.startswith("  sky_cua: "))
    node_line = next(line for line in text.splitlines() if line.startswith("  node_repl: "))
    sky_cua = json.loads(sky_line.removeprefix("  sky_cua: "))
    node_repl = json.loads(node_line.removeprefix("  node_repl: "))
    assert sky_cua["command"] == str(install_root / "bin/sky-cua-client")
    assert sky_cua["args"] == ["mcp"]
    assert sky_cua["env"]["SKY_CUA_MCP_CALLER_PROVENANCE"] == "hermes"
    assert node_repl["command"] == str(install_root / "bin/node_repl")


def test_extracted_payload_install_imports_hermes_adapter(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core, cua_node = _fixture_repo(repo)
    real_root = Path(__file__).resolve().parents[1]
    for relative in (
        "install.py",
        "scripts/_codex_app_server.py",
        "scripts/_hermes_config.py",
        "scripts/_opencode_config.py",
        "scripts/_plugin_bundle.py",
        "scripts/_standalone_release_command.py",
    ):
        source = real_root / relative
        destination = repo / relative
        destination.write_bytes(source.read_bytes())
        destination.chmod(source.stat().st_mode)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)
    payload = tmp_path / "payload"
    assemble_payload(payload, core_root=core, cua_node_root=cua_node)
    home = tmp_path / "home"
    env = {
        "HOME": str(home),
        "PATH": "",
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
    }

    result = subprocess.run(
        [sys.executable, "install.py", "install"],
        cwd=payload,
        env=env,
        check=False,
        capture_output=True,
        text=True,
    )

    assert result.returncode == 0, result.stderr
    report = json.loads(result.stdout)
    assert report["hermes_config"]["status"] == "no_global_config"

    release_result = subprocess.run(
        [sys.executable, "install.py", "release"],
        cwd=payload,
        env=env,
        check=False,
        capture_output=True,
        text=True,
    )

    assert release_result.returncode == 1
    assert release_result.stderr.strip() == "error: release requires a source checkout"


def test_payload_rejects_legacy_browser_use_plugin_alias(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core, cua_node = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)
    payload = tmp_path / "payload"
    assemble_payload(payload, core_root=core, cua_node_root=cua_node)
    _write(payload / "codex/openai-bundled/plugins/browser-use/.mcp.json", "{}\n")

    with pytest.raises(
        ValueError,
        match="standalone Codex plugin tree must contain exactly",
    ):
        standalone_release.validate_payload(payload)


def test_install_rejects_incomplete_payload_before_replacing_fixed_tree(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core, cua_node = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)
    first = tmp_path / "first"
    assemble_payload(first, core_root=core, cua_node_root=cua_node)
    _write(first / "old-marker")
    incomplete = tmp_path / "incomplete"
    assemble_payload(incomplete, core_root=core, cua_node_root=cua_node)
    (incomplete / "bin/sky-cua-overlay-host").unlink()
    home = tmp_path / "home"
    env = {"HOME": str(home), "XDG_DATA_HOME": str(tmp_path / "xdg-data")}
    report = install_payload(first, home=home, env=env, configure_hosts=False)
    install_root = Path(str(report["install_root"]))

    with pytest.raises(FileNotFoundError, match="standalone payload is incomplete"):
        install_payload(incomplete, home=home, env=env, configure_hosts=False)

    assert (install_root / "old-marker").is_file()


def test_detected_hosts_receive_native_plugins_and_hash_free_openclaw_definition(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repo = tmp_path / "repo"
    core, cua_node = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)
    payload = tmp_path / "payload"
    assemble_payload(payload, core_root=core, cua_node_root=cua_node)
    home = tmp_path / "home"
    (home / ".codex").mkdir(parents=True)
    (home / ".openclaw").mkdir(parents=True)
    main_codex_config = home / ".openclaw/agents/main/agent/codex-home/config.toml"
    _write(
        main_codex_config,
        'model = "gpt-5"\napproval_policy = "on-request"\n\n[plugins.example]\nenabled = true\n',
    )
    sky_codex_config = home / ".openclaw/agents/sky/agent/codex-home/config.toml"
    _write(sky_codex_config, '[plugins."computer-use@openai-bundled"]\nenabled = true\n')
    env = {
        "HOME": str(home),
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
        "PATH": f"{home}/.local/bin:/usr/bin",
    }
    plugin_calls: list[str] = []
    monkeypatch.setattr(
        standalone_release,
        "_install_codex_plugins",
        lambda _root, **_kwargs: (
            plugin_calls.extend(standalone_release.PLUGIN_NAMES) or standalone_release.PLUGIN_NAMES
        ),
    )
    openclaw_calls: list[tuple[list[str], dict[str, Any]]] = []

    def fake_which(name: str) -> str | None:
        return f"/usr/bin/{name}" if name in {"codex", "openclaw"} else None

    def fake_run(command: list[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
        openclaw_calls.append((command, kwargs))
        return subprocess.CompletedProcess(command, 0, "", "")

    report = install_payload(
        payload,
        home=home,
        env=env,
        which=fake_which,
        runner=fake_run,
    )

    assert plugin_calls == ["computer-use", "browser"]
    assert report["codex_plugins"] == ["computer-use", "browser"]
    assert report["openclaw_node_repl"] is True
    permission_command, permission_kwargs = openclaw_calls[0]
    assert permission_command[1:4] == ["config", "set", "--batch-json"]
    permission_batch = json.loads(permission_command[4])
    assert permission_batch == [
        {"path": "plugins.entries.codex.config.appServer.mode", "value": "yolo"},
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
    assert permission_kwargs["env"]["HOME"] == str(home)
    for config_path in (main_codex_config, sky_codex_config):
        config = config_path.read_text(encoding="utf-8")
        assert config.count('approval_policy = "never"') == 1
        assert config.count('sandbox_mode = "danger-full-access"') == 1
    assert "[plugins.example]\nenabled = true" in main_codex_config.read_text(encoding="utf-8")
    assert report["openclaw_permission_configs"] == [
        str(main_codex_config),
        str(sky_codex_config),
    ]

    openclaw_command, openclaw_kwargs = openclaw_calls[1]
    definition = json.loads(openclaw_command[-1])
    assert openclaw_command[1:4] == ["mcp", "set", "node_repl"]
    assert openclaw_kwargs["env"]["PATH"] == "/usr/bin"
    assert "NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S" not in definition["env"]
    assert definition["command"].endswith("/sky-cua/bin/node_repl")
    for skill_root in (home / ".codex/skills", home / ".openclaw/skills"):
        assert sorted(path.name for path in skill_root.iterdir()) == sorted(
            standalone_release.SKILL_NAMES
        )


def test_openclaw_no_prompt_permissions_are_idempotent_and_preserve_agent_config(
    tmp_path: Path,
) -> None:
    home = tmp_path / "home"
    config_path = home / ".openclaw/agents/sky/agent/codex-home/config.toml"
    _write(
        config_path,
        'model = "gpt-5"\napproval_policy = "on-request"\n\n[plugins.example]\nenabled = true\n',
    )
    calls: list[list[str]] = []

    def fake_run(command: list[str], **_kwargs: Any) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        return subprocess.CompletedProcess(command, 0, "", "")

    for _ in range(2):
        inode_before = config_path.stat().st_ino
        assert standalone_release._configure_openclaw_no_prompt_permissions(
            home=home,
            env={"HOME": str(home)},
            openclaw="/usr/bin/openclaw",
            runner=fake_run,
        ) == (config_path,)
        if _ == 1:
            assert config_path.stat().st_ino == inode_before

    config = config_path.read_text(encoding="utf-8")
    assert config.count('approval_policy = "never"') == 1
    assert config.count('sandbox_mode = "danger-full-access"') == 1
    assert "[plugins.example]\nenabled = true" in config
    assert len(calls) == 2


def test_openclaw_permission_command_failure_preserves_agent_config(tmp_path: Path) -> None:
    home = tmp_path / "home"
    config_path = home / ".openclaw/agents/sky/agent/codex-home/config.toml"
    original = 'approval_policy = "on-request"\n'
    _write(config_path, original)

    def fake_run(command: list[str], **_kwargs: Any) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(command, 1, "", "invalid policy")

    with pytest.raises(RuntimeError, match="invalid policy"):
        standalone_release._configure_openclaw_no_prompt_permissions(
            home=home,
            env={"HOME": str(home)},
            openclaw="/usr/bin/openclaw",
            runner=fake_run,
        )

    assert config_path.read_text(encoding="utf-8") == original


def _assemble_payload(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    repo = tmp_path / "repo"
    core, cua_node = _fixture_repo(repo)
    monkeypatch.setattr(standalone_release, "REPO_ROOT", repo)
    payload = tmp_path / "payload"
    assemble_payload(payload, core_root=core, cua_node_root=cua_node)
    return payload


def _seed_opencode_config(home: Path, *, body: str) -> Path:
    config_dir = home / ".config/opencode"
    config_dir.mkdir(parents=True, exist_ok=True)
    config_path = config_dir / "opencode.jsonc"
    config_path.write_text(body, encoding="utf-8")
    config_path.chmod(0o600)
    return config_path


def test_install_rewrites_existing_opencode_config_to_flat_root(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = _assemble_payload(tmp_path, monkeypatch)

    home = tmp_path / "home"
    legacy_body = (
        "{\n"
        '  "$schema": "https://opencode.ai/config.json",\n'
        '  "model": "opencode-go/minimax-m3",\n'
        '  "permission": {"*": "allow"},\n'
        '  "agent": {"explore": {"permission": {"edit": "deny"}}},\n'
        '  "mcp": {\n'
        '    "sky_cua": {\n'
        '      "type": "local",\n'
        '      "command": ["/home/bex/.local/share/sky-cua/bin/sky-cua-client", "mcp"],\n'
        '      "cwd": "/home/bex/.local/share/sky-cua",\n'
        '      "environment": {\n'
        '        "SKY_CUA_DOCUMENTATION_ROOT": "/home/bex/.local/share/sky-cua/components/documentation",\n'
        '        "SKY_CUA_REPO_ROOT": "/home/bex/.local/share/sky-cua/components/core-linux-x64",\n'
        '        "SKY_CUA_RELEASE_ROOT": "/home/bex/.local/share/sky-cua",\n'
        '        "SKY_CUA_MCP_CALLER_PROVENANCE": "opencode",\n'
        '        "SKY_CUA_CODEX_BROWSER_SOCKET_PATH": "/run/user/1000/sky-cua/codex-browser.sock"\n'
        "      },\n"
        '      "enabled": true,\n'
        '      "timeout": 30000\n'
        "    },\n"
        '    "node_repl": {\n'
        '      "type": "local",\n'
        '      "command": ["/home/bex/.local/share/sky-cua/bin/node_repl"],\n'
        '      "cwd": "/home/bex/.local/share/sky-cua",\n'
        '      "environment": {\n'
        '        "CODEX_NODE_REPL_PATH": "/home/bex/.local/share/sky-cua/bin/node_repl",\n'
        '        "NODE_REPL_NODE_PATH": "/home/bex/.local/share/sky-cua/bin/node",\n'
        '        "NODE_REPL_NODE_MODULE_DIRS": "/home/bex/.local/share/sky-cua/lib/node_modules",\n'
        '        "PLAYWRIGHT_BROWSERS_PATH": "/home/bex/.local/share/sky-cua/components/cua-node-linux-x64-glibc/share/playwright",\n'
        '        "NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S": "41e1151f1e50f096c7561da32bb01123e74b6ecdd38f081e34da30091fc4f193,6d25aa7656feac858f3a3bdaea5bcbab0dbfd426c9de8e6931ce90c399ee8e4f",\n'
        '        "SKY_CUA_DOCUMENTATION_ROOT": "/home/bex/.local/share/sky-cua/components/documentation",\n'
        '        "SKY_CUA_REPO_ROOT": "/home/bex/.local/share/sky-cua/components/core-linux-x64",\n'
        '        "SKY_CUA_RELEASE_ROOT": "/home/bex/.local/share/sky-cua/",\n'
        '        "SKY_CUA_MCP_CALLER_PROVENANCE": "opencode",\n'
        '        "SKY_CUA_CODEX_BROWSER_SOCKET_PATH": "/run/user/1000/sky-cua/codex-browser.sock"\n'
        "      },\n"
        '      "enabled": true,\n'
        '      "timeout": 30000\n'
        "    },\n"
        '    "context7": {"type": "remote", "url": "https://mcp.context7.com/mcp"}\n'
        "  }\n"
        "}\n"
    )
    config_path = _seed_opencode_config(home, body=legacy_body)
    env = {
        "HOME": str(home),
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
        "XDG_RUNTIME_DIR": "/run/user/1000",
        "WAYLAND_DISPLAY": "wayland-0",
        "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus",
    }

    report = install_payload(payload, home=home, env=env, configure_hosts=False)
    install_root = Path(str(report["install_root"]))

    new_text = config_path.read_text(encoding="utf-8")
    assert "components/core-linux-x64" not in new_text
    assert "components/cua-node-linux-x64-glibc" not in new_text
    assert "NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S" not in new_text
    assert "trusted_browser_client_sha256s" not in new_text

    new_parsed = json.loads(new_text)
    assert new_parsed["model"] == "opencode-go/minimax-m3"
    assert new_parsed["agent"]["explore"]["permission"]["edit"] == "deny"
    assert new_parsed["mcp"]["context7"]["url"] == "https://mcp.context7.com/mcp"

    sky_cua = new_parsed["mcp"]["sky_cua"]
    assert sky_cua["type"] == "local"
    assert sky_cua["command"] == [str(install_root / "bin/sky-cua-client"), "mcp"]
    assert sky_cua["cwd"] == str(install_root)
    assert sky_cua["environment"]["SKY_CUA_REPO_ROOT"] == str(install_root)
    assert sky_cua["environment"]["SKY_CUA_RELEASE_ROOT"] == str(install_root)
    assert sky_cua["environment"]["SKY_CUA_DOCUMENTATION_ROOT"] == str(install_root / "docs")
    assert (
        sky_cua["environment"]["SKY_CUA_CODEX_BROWSER_SOCKET_PATH"]
        == "/run/user/1000/sky-cua/codex-browser.sock"
    )
    assert sky_cua["environment"]["SKY_CUA_MCP_CALLER_PROVENANCE"] == "opencode"
    assert sky_cua["environment"]["XDG_RUNTIME_DIR"] == "/run/user/1000"
    assert sky_cua["environment"]["WAYLAND_DISPLAY"] == "wayland-0"
    assert sky_cua["environment"]["DBUS_SESSION_BUS_ADDRESS"] == "unix:path=/run/user/1000/bus"

    node_repl = new_parsed["mcp"]["node_repl"]
    assert node_repl["command"] == [str(install_root / "bin/node_repl")]
    assert node_repl["environment"]["CODEX_NODE_REPL_PATH"] == str(install_root / "bin/node_repl")
    assert node_repl["environment"]["NODE_REPL_NODE_PATH"] == str(install_root / "bin/node")
    assert node_repl["environment"]["NODE_REPL_NODE_MODULE_DIRS"] == str(
        install_root / "lib/node_modules"
    )
    assert node_repl["environment"]["PLAYWRIGHT_BROWSERS_PATH"] == str(
        install_root / "share/playwright"
    )
    assert "NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S" not in node_repl["environment"]
    assert node_repl["environment"]["SKY_CUA_REPO_ROOT"] == str(install_root)

    opencode = cast("dict[str, object]", report["opencode_config"])
    assert opencode["status"] == "updated"
    backup_path = Path(str(opencode["backup_path"]))
    assert backup_path.parent == config_path.parent / _opencode_config.OPENCODE_BACKUP_DIR_NAME
    assert backup_path.read_bytes() == legacy_body.encode("utf-8")

    second_report = install_payload(payload, home=home, env=env, configure_hosts=False)
    second_opencode = cast("dict[str, object]", second_report["opencode_config"])
    assert second_opencode["status"] == "unchanged"


def test_install_opencode_no_op_when_no_global_config_exists(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = _assemble_payload(tmp_path, monkeypatch)

    home = tmp_path / "home"
    env = {"HOME": str(home), "XDG_DATA_HOME": str(tmp_path / "xdg-data")}

    report = install_payload(payload, home=home, env=env, configure_hosts=False)

    opencode = cast("dict[str, object]", report["opencode_config"])
    assert opencode["status"] == "no_global_config"
    assert opencode["config_path"] is None
    assert not (home / ".config/opencode/opencode.jsonc").exists()
    assert not (home / ".config/opencode").exists()


def test_install_opencode_refuses_when_opencode_config_env_override_is_set(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    payload = _assemble_payload(tmp_path, monkeypatch)

    home = tmp_path / "home"
    config_path = _seed_opencode_config(home, body='{"$schema": "x", "mcp": {}}\n')
    env = {
        "HOME": str(home),
        "XDG_DATA_HOME": str(tmp_path / "xdg-data"),
        "OPENCODE_CONFIG": "/some/other/opencode.json",
    }
    original_bytes = config_path.read_bytes()

    with pytest.raises(RuntimeError, match="OPENCODE_CONFIG"):
        install_payload(payload, home=home, env=env, configure_hosts=False)

    assert config_path.read_bytes() == original_bytes


def test_install_opencode_drops_jsonc_comments_but_keeps_user_keys_on_first_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A first install into a fresh ``opencode.jsonc`` drops ``//`` comments
    and preserves the user's existing top-level keys while adding the managed
    MCP block."""
    payload = _assemble_payload(tmp_path, monkeypatch)

    home = tmp_path / "home"
    config_path = _seed_opencode_config(
        home,
        body=(
            "{\n"
            "  // top-level schema + model\n"
            '  "$schema": "https://opencode.ai/config.json",\n'
            '  "model": "opencode-go/minimax-m3",\n'
            '  "permission": {"*": "allow"}\n'
            "}\n"
        ),
    )
    env = {"HOME": str(home), "XDG_DATA_HOME": str(tmp_path / "xdg-data")}

    report = install_payload(payload, home=home, env=env, configure_hosts=False)
    install_root = Path(str(report["install_root"]))

    new_parsed = json.loads(config_path.read_text(encoding="utf-8"))
    assert new_parsed["model"] == "opencode-go/minimax-m3"
    assert new_parsed["permission"] == {"*": "allow"}
    assert set(new_parsed["mcp"]) == set(_opencode_config.OPENCODE_MANAGED_SERVERS)

    sky_cua = new_parsed["mcp"]["sky_cua"]
    assert sky_cua["type"] == "local"
    assert sky_cua["command"] == [str(install_root / "bin/sky-cua-client"), "mcp"]
    assert sky_cua["enabled"] is True
    assert sky_cua["timeout"] == 30_000
    node_repl = new_parsed["mcp"]["node_repl"]
    assert node_repl["type"] == "local"
    assert node_repl["enabled"] is True
    assert node_repl["timeout"] == 30_000
    opencode = cast("dict[str, object]", report["opencode_config"])
    assert opencode["status"] == "updated"
