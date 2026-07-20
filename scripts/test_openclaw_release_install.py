"""Focused immutable-generation OpenClaw two-server installer tests."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import cast

import pytest

import _openclaw_install
from release_generation import FULL_PROFILE, VerifiedRelease

RELEASE_ID = "a" * 64
MANIFEST_SHA256 = "b" * 64
BROWSER_SHA256 = "c" * 64


def _release_fixture(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[Path, Path]:
    releases = tmp_path / "store" / "releases"
    generation = releases / RELEASE_ID
    core = generation / "components" / "core-linux-x64"
    cua_node = generation / "components" / "cua-node-linux-x64-glibc"
    browser = generation / "components" / "browser-js"
    for path in (core, cua_node, browser):
        path.mkdir(parents=True)
    sky_cua = core / "bin" / "sky-cua-client"
    node_repl = cua_node / "bin" / "node_repl"
    node = cua_node / "bin" / "node"
    for executable in (sky_cua, node_repl, node):
        executable.parent.mkdir(parents=True, exist_ok=True)
        executable.write_text("fixture\n", encoding="utf-8")
        executable.chmod(0o755)
    (cua_node / "lib" / "node_modules").mkdir(parents=True)
    (cua_node / "share" / "playwright").mkdir(parents=True)
    documentation = generation / "components" / "documentation"
    for skill in ("browser-use", "computer-use", "phone-use"):
        path = documentation / "skills" / skill / "SKILL.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            f"---\nname: {skill}\ndescription: fixture\n---\n\n# Fixture\n", encoding="utf-8"
        )
    (cua_node / "manifest.json").write_text(
        json.dumps(
            {
                "target": "linux-x64-glibc",
                "node_version": "24.14.0",
                "node_repl_path": "bin/node_repl",
                "node_path": "bin/node",
                "node_modules": "lib/node_modules",
                "data": {"playwright": "share/playwright"},
                "trusted_browser_client_sha256s": [BROWSER_SHA256],
            }
        ),
        encoding="utf-8",
    )
    (generation / "RELEASE.json").write_text(
        json.dumps(
            {
                "trusted_browser_client_sha256s": [BROWSER_SHA256],
                "components": [
                    {
                        "name": "core-linux-x64",
                        "path": "components/core-linux-x64",
                    },
                    {
                        "name": "browser-js",
                        "path": "components/browser-js",
                    },
                    {
                        "name": "cua-node-linux-x64-glibc",
                        "path": "components/cua-node-linux-x64-glibc",
                    },
                    {
                        "name": "documentation",
                        "path": "components/documentation",
                    },
                ],
            }
        ),
        encoding="utf-8",
    )

    def fake_verify(
        root: Path,
        *,
        profile: str,
        enforce_profile_shape: bool,
    ) -> VerifiedRelease:
        assert root == generation
        assert profile == FULL_PROFILE
        assert enforce_profile_shape is True
        if (root / "TAMPERED").exists():
            raise ValueError("fixture tamper")
        return VerifiedRelease(
            root=root,
            release_id=RELEASE_ID,
            manifest_sha256=MANIFEST_SHA256,
            profile=FULL_PROFILE,
            component_names=(
                "core-linux-x64",
                "browser-js",
                "cua-node-linux-x64-glibc",
                "documentation",
            ),
        )

    monkeypatch.setattr(_openclaw_install, "verify_release_root", fake_verify)
    current = tmp_path / "store" / "current"
    current.symlink_to(Path("releases") / RELEASE_ID)
    return current, generation


class FakeOpenClaw:
    def __init__(self, servers: dict[str, dict[str, object]] | None = None) -> None:
        self.servers = dict(servers or {})
        self.calls: list[tuple[list[str], dict[str, object]]] = []
        self.fail_next_node_set = False
        self.restart_returncode = 0

    def __call__(self, command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        self.calls.append((list(command), dict(kwargs)))
        assert kwargs["check"] is False
        assert kwargs["capture_output"] is True
        assert kwargs["text"] is True
        if command[1:4] == ["mcp", "show", "--json"]:
            return subprocess.CompletedProcess(command, 0, json.dumps(self.servers), "")
        if command[1:3] == ["mcp", "set"]:
            name = command[3]
            definition = cast(dict[str, object], json.loads(command[4]))
            self.servers[name] = definition
            if name == "node_repl" and self.fail_next_node_set:
                self.fail_next_node_set = False
                return subprocess.CompletedProcess(command, 19, "", "node set failed")
            return subprocess.CompletedProcess(command, 0, "saved\n", "")
        if command[1:3] == ["mcp", "unset"]:
            self.servers.pop(command[3], None)
            return subprocess.CompletedProcess(command, 0, "removed\n", "")
        if command[1:] == ["gateway", "restart", "--wait", "120s"]:
            return subprocess.CompletedProcess(
                command,
                self.restart_returncode,
                "healthy\n" if self.restart_returncode == 0 else "",
                "restart failed\n" if self.restart_returncode else "",
            )
        raise AssertionError(f"unexpected OpenClaw command: {command}")


def test_plan_uses_one_verified_generation_for_all_paths_and_locked_env(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, generation = _release_fixture(tmp_path, monkeypatch)
    socket = tmp_path / "runtime" / "sky-cua" / "codex-browser.sock"
    plan = _openclaw_install.plan_openclaw_release_install(
        current,
        browser_socket_path=socket,
        openclaw_dir=tmp_path / "openclaw",
        launch_env={
            "KEEP_ME": "preserved",
            "SKY_CUA_RELEASE_ROOT": "/checkout/not-allowed",
            "NODE_REPL_NODE_PATH": "/usr/bin/node",
            "SKY_CUA_MCP_CALLER_PROVENANCE": "wrong",
        },
    )

    direct = plan.definitions["sky_cua"]
    repl = plan.definitions["node_repl"]
    direct_env = cast(dict[str, str], direct["env"])
    repl_env = cast(dict[str, str], repl["env"])
    core = generation / "components" / "core-linux-x64"
    runtime = generation / "components" / "cua-node-linux-x64-glibc"

    assert plan.release.root == generation
    assert direct["command"] == str(core / "bin" / "sky-cua-client")
    assert direct["args"] == ["mcp"]
    assert direct["cwd"] == str(generation)
    assert repl["command"] == str(runtime / "bin" / "node_repl")
    assert repl["args"] == []
    assert repl["cwd"] == str(generation)
    assert direct_env["KEEP_ME"] == "preserved"
    assert direct_env["SKY_CUA_RELEASE_ROOT"] == str(generation)
    assert direct_env["SKY_CUA_REPO_ROOT"] == str(core)
    assert direct_env["SKY_CUA_DOCUMENTATION_ROOT"] == str(
        generation / "components" / "documentation"
    )
    assert direct_env["SKY_CUA_MCP_CALLER_PROVENANCE"] == "openclaw"
    assert direct_env["SKY_CUA_CODEX_BROWSER_SOCKET_PATH"] == str(socket)
    assert "NODE_REPL_NODE_PATH" not in direct_env
    assert repl_env["CODEX_NODE_REPL_PATH"] == str(runtime / "bin" / "node_repl")
    assert repl_env["NODE_REPL_NODE_PATH"] == str(runtime / "bin" / "node")
    assert repl_env["NODE_REPL_NODE_MODULE_DIRS"] == str(runtime / "lib" / "node_modules")
    assert repl_env["NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S"] == BROWSER_SHA256
    assert repl_env["PLAYWRIGHT_BROWSERS_PATH"] == str(runtime / "share" / "playwright")
    assert repl["supportsParallelToolCalls"] is False
    assert repl["connectionTimeoutMs"] == 120_000
    assert repl["requestTimeoutMs"] == 3_600_000


def test_desktop_session_launch_env_preserves_only_required_runtime_keys() -> None:
    source = {
        "XDG_RUNTIME_DIR": "/run/user/1000",
        "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus",
        "DESKTOP_SESSION": "plasma",
        "XDG_CURRENT_DESKTOP": "KDE",
        "XDG_SESSION_TYPE": "wayland",
        "WAYLAND_DISPLAY": "wayland-0",
        "DISPLAY": ":0",
        "IGNORED": "not-forwarded",
        "EMPTY": "",
    }

    assert _openclaw_install._desktop_session_launch_env(source) == {
        name: source[name] for name in _openclaw_install.OPENCLAW_DESKTOP_SESSION_ENV
    }


def test_install_merges_two_definitions_idempotently_and_preserves_unrelated_server(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, generation = _release_fixture(tmp_path, monkeypatch)
    unrelated = {"command": "existing", "args": ["--operator"]}
    fake = FakeOpenClaw({"operator_server": unrelated})
    kwargs = {
        "browser_socket_path": tmp_path / "runtime" / "browser.sock",
        "openclaw_dir": tmp_path / "openclaw",
        "runner": fake,
    }

    first = _openclaw_install.install_openclaw_release(current, **kwargs)
    first_state = json.loads(json.dumps(fake.servers))
    second = _openclaw_install.install_openclaw_release(current, **kwargs)

    assert first.release_id == RELEASE_ID
    assert first.manifest_sha256 == MANIFEST_SHA256
    assert first.release_root == str(generation)
    assert first.changed_servers == ("sky_cua", "node_repl")
    assert first.gateway_activation == "gateway_watcher_pending_verification"
    assert "process-local 'openclaw mcp reload'" in first.gateway_detail
    assert second.changed_servers == ()
    assert second.gateway_activation == "unchanged"
    assert fake.servers == first_state
    assert fake.servers["operator_server"] == unrelated
    set_calls = [call for call, _kwargs in fake.calls if call[1:3] == ["mcp", "set"]]
    assert [call[3] for call in set_calls] == ["sky_cua", "node_repl"]
    assert not any(call[1:3] == ["mcp", "reload"] for call, _kwargs in fake.calls)
    for _command, call_kwargs in fake.calls:
        env = cast(dict[str, str], call_kwargs["env"])
        assert env["OPENCLAW_STATE_DIR"] == str(tmp_path / "openclaw")
        assert env["OPENCLAW_CONFIG_PATH"] == str(tmp_path / "openclaw" / "openclaw.json")


def test_partial_second_registration_failure_restores_both_named_snapshots(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation = _release_fixture(tmp_path, monkeypatch)
    original_sky: dict[str, object] = {"command": "/old/sky", "args": ["mcp"]}
    original_repl: dict[str, object] = {
        "command": "/old/repl",
        "supportsParallelToolCalls": False,
    }
    unrelated: dict[str, object] = {"url": "https://operator.invalid/mcp"}
    fake = FakeOpenClaw(
        {
            "sky_cua": original_sky,
            "node_repl": original_repl,
            "operator_server": unrelated,
        }
    )
    fake.fail_next_node_set = True

    with pytest.raises(
        _openclaw_install.OpenClawReleaseInstallError,
        match="failed to register OpenClaw MCP definition node_repl",
    ):
        _openclaw_install.install_openclaw_release(
            current,
            browser_socket_path=tmp_path / "runtime" / "browser.sock",
            openclaw_dir=tmp_path / "openclaw",
            runner=fake,
        )

    assert fake.servers["sky_cua"] == original_sky
    assert fake.servers["node_repl"] == original_repl
    assert fake.servers["operator_server"] == unrelated


def test_explicit_gateway_restart_reports_health_wait_outcome_without_mcp_reload(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    current, _generation = _release_fixture(tmp_path, monkeypatch)
    fake = FakeOpenClaw()

    report = _openclaw_install.install_openclaw_release(
        current,
        browser_socket_path=tmp_path / "runtime" / "browser.sock",
        openclaw_dir=tmp_path / "openclaw",
        gateway_activation="restart",
        runner=fake,
    )

    assert report.gateway_activation == "gateway_restart_verified"
    assert any(call[1:] == ["gateway", "restart", "--wait", "120s"] for call, _kwargs in fake.calls)
    assert not any(call[1:3] == ["mcp", "reload"] for call, _kwargs in fake.calls)


def test_failed_gateway_restart_rolls_back_both_definitions(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation = _release_fixture(tmp_path, monkeypatch)
    original_sky: dict[str, object] = {"command": "old-sky"}
    original_repl: dict[str, object] = {"command": "old-repl"}
    unrelated: dict[str, object] = {"command": "operator"}
    fake = FakeOpenClaw(
        {
            "sky_cua": original_sky,
            "node_repl": original_repl,
            "operator_server": unrelated,
        }
    )
    fake.restart_returncode = 1

    with pytest.raises(
        _openclaw_install.OpenClawReleaseInstallError,
        match="restart failed; definitions were rolled back",
    ):
        _openclaw_install.install_openclaw_release(
            current,
            browser_socket_path=tmp_path / "runtime" / "browser.sock",
            openclaw_dir=tmp_path / "openclaw",
            gateway_activation="restart",
            runner=fake,
        )

    assert fake.servers["sky_cua"] == original_sky
    assert fake.servers["node_repl"] == original_repl
    assert fake.servers["operator_server"] == unrelated


def test_release_verification_failure_precedes_openclaw_commands(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, generation = _release_fixture(tmp_path, monkeypatch)
    (generation / "TAMPERED").write_text("yes\n", encoding="utf-8")
    fake = FakeOpenClaw()

    with pytest.raises(
        _openclaw_install.OpenClawReleaseInstallError,
        match="release verification failed: fixture tamper",
    ):
        _openclaw_install.install_openclaw_release(
            current,
            browser_socket_path=tmp_path / "runtime" / "browser.sock",
            openclaw_dir=tmp_path / "openclaw",
            runner=fake,
        )

    assert fake.calls == []


def test_plan_rejects_relative_browser_socket(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation = _release_fixture(tmp_path, monkeypatch)
    with pytest.raises(
        _openclaw_install.OpenClawReleaseInstallError,
        match="browser socket path must be absolute",
    ):
        _openclaw_install.plan_openclaw_release_install(
            current,
            browser_socket_path=Path("relative/browser.sock"),
            openclaw_dir=tmp_path / "openclaw",
        )
