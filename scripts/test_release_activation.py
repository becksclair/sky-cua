from __future__ import annotations

import json
import os
from dataclasses import replace
from pathlib import Path

import pytest

import _plugin_bundle as plugin_bundle
import _release_activation as activation
from release_generation import VerifiedRelease, canonical_tree_digest, sha256_file


def _codex_contract(root: Path) -> dict[str, object]:
    marketplace = root / "components/codex-compat/openai-bundled"
    marketplace_manifest = marketplace / ".agents/plugins/marketplace.json"
    marketplace_manifest.parent.mkdir(parents=True)
    marketplace_manifest.write_text('{"name":"openai-bundled","plugins":[]}\n')
    plugins: list[dict[str, object]] = []
    for name, version, server in (
        ("computer-use", "0.1.0-sky-cua", "computer-use"),
        ("browser-use", "1.0.0-sky-cua-openclaw", "node_repl"),
    ):
        plugin_root = marketplace / "plugins" / name
        plugin_root.mkdir(parents=True)
        (plugin_root / "payload.txt").write_text(name)
        plugins.append(
            {
                "id": f"{name}@openai-bundled",
                "name": name,
                "version": version,
                "path": f"components/codex-compat/openai-bundled/plugins/{name}",
                "tree_sha256": canonical_tree_digest(plugin_root).sha256,
                "mcp_servers": [server],
            }
        )
    return {
        "marketplace": "openai-bundled",
        "path": "components/codex-compat/openai-bundled",
        "manifest_path": (
            "components/codex-compat/openai-bundled/.agents/plugins/marketplace.json"
        ),
        "manifest_sha256": sha256_file(marketplace_manifest),
        "plugins": plugins,
    }


def _installed_release(tmp_path: Path) -> tuple[Path, VerifiedRelease]:
    store = tmp_path / "store"
    release_id = "a" * 64
    root = store / "releases" / release_id
    binary = root / "components/core-linux-x64/bin/runtimes/linux-x64/sky-cua-client"
    binary.parent.mkdir(parents=True)
    binary.write_bytes(b"client")
    binary.chmod(0o755)
    service = binary.with_name("sky-cua-service")
    service.write_bytes(b"service")
    service.chmod(0o755)
    store.mkdir(exist_ok=True)
    (store / "current").symlink_to(Path("releases") / release_id)
    return store, VerifiedRelease(
        root=root,
        release_id=release_id,
        manifest_sha256="b" * 64,
        profile="full",
        component_names=("core-linux-x64",),
    )


def _fake_process(proc_root: Path, pid: int, executable: str) -> None:
    entry = proc_root / str(pid)
    entry.mkdir(parents=True)
    (entry / "cmdline").write_bytes(executable.encode() + b"\0serve")
    (entry / "exe").symlink_to(executable)
    (entry / "cwd").symlink_to(str(Path(executable.removesuffix(" (deleted)")).parent))


def _fake_node_repl_process(
    proc_root: Path,
    pid: int,
    runtime_root: Path,
    *,
    deleted: bool = False,
) -> str:
    entry = proc_root / str(pid)
    entry.mkdir(parents=True)
    executable = runtime_root / "bin/node"
    cli = runtime_root / "lib/node_repl/cli.js"
    (entry / "cmdline").write_bytes(str(executable).encode() + b"\0" + str(cli).encode() + b"\0")
    rendered_executable = f"{executable} (deleted)" if deleted else str(executable)
    (entry / "exe").symlink_to(rendered_executable)
    (entry / "cwd").symlink_to(str(runtime_root))
    return str(executable)


def test_stable_links_resolve_only_through_current_and_repair_stale_copy(
    tmp_path: Path,
) -> None:
    store, release = _installed_release(tmp_path)
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    stale = bin_dir / "sky-cua-client"
    stale.write_bytes(b"obsolete mutable copy")
    legacy = store / "bin/sky-cua-client"
    legacy.parent.mkdir()
    legacy.write_bytes(b"legacy store copy")
    unknown = store / "bin/user-helper"
    unknown.write_bytes(b"preserve me")

    links, snapshots = activation.install_stable_links(
        store,
        release,
        bin_dir=bin_dir,
    )

    assert stale.is_symlink()
    assert (
        stale.resolve()
        == release.root / "components/core-linux-x64/bin/runtimes/linux-x64/sky-cua-client"
    )
    assert "current" in os.readlink(stale)
    assert links[str(stale)] == os.readlink(stale)
    assert legacy.is_symlink()
    assert legacy.resolve() == stale.resolve()
    assert unknown.read_bytes() == b"preserve me"
    assert not (bin_dir / "node_repl").exists()
    assert not (store / "bin/node_repl").exists()
    activation.restore_path(snapshots[0])
    assert stale.read_bytes() == b"obsolete mutable copy"
    assert not stale.is_symlink()


def test_versioned_receipt_is_atomic_and_restorable(tmp_path: Path) -> None:
    store, release = _installed_release(tmp_path)
    manifest = tmp_path / "home/native.json"
    links = {str(tmp_path / "bin/sky-cua-client"): "../store/current/client"}

    prior = activation.write_receipt(
        store,
        release,
        native_manifest_paths=(manifest,),
        stable_links=links,
    )
    payload = json.loads((store / activation.ACTIVATION_RECEIPT).read_text())
    assert payload["schema_version"] == 1
    assert payload["release_id"] == release.release_id
    assert payload["stable_links"] == links
    activation.restore_path(prior)
    assert not (store / activation.ACTIVATION_RECEIPT).exists()


def test_active_runtime_resolves_from_current_without_selector_environment(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store, release = _installed_release(tmp_path)
    home = tmp_path / "home"
    bin_dir = tmp_path / "bin"
    for name in (
        "SKY_CUA_RELEASE_ROOT",
        "SKY_CUA_RELEASE_ID",
        "NODE_REPL_NODE_PATH",
        "NODE_REPL_NODE_MODULE_DIRS",
        "CUA_NODE_BROWSER_CLIENT_PATH",
    ):
        monkeypatch.delenv(name, raising=False)
    installer = release.root / "install.py"
    installer.write_text("#!/usr/bin/env python3\n", encoding="utf-8")
    installer.chmod(0o755)
    cua_root = release.root / "components/cua-node-linux-x64-glibc"
    for path in (
        cua_root / "bin/node",
        cua_root / "bin/node_repl",
        cua_root / "lib/node_modules/.keep",
    ):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"runtime")
    release = replace(
        release,
        component_names=("core-linux-x64", "cua-node-linux-x64-glibc"),
    )
    browser = release.root / "components/browser-js/browser-client.mjs"
    browser.parent.mkdir(parents=True)
    browser.write_text("export {};\n", encoding="utf-8")
    (release.root / "RELEASE.json").write_text(
        json.dumps(
            {
                "browser_contract": {
                    "canonical_browser": {"path": "components/browser-js/browser-client.mjs"}
                },
                "trusted_browser_client_sha256s": ["f" * 64],
                "codex_plugin_contract": _codex_contract(release.root),
            }
        ),
        encoding="utf-8",
    )
    host = release.root / activation.HOST_RELATIVE_PATH
    host.parent.mkdir(parents=True, exist_ok=True)
    host.write_bytes(b"host")
    links, _ = activation.install_stable_links(store, release, bin_dir=bin_dir)
    manifests = tuple(
        home / relative / f"{activation.HOST_NAME}.json"
        for relative in activation.MANIFEST_RELATIVE_DIRS
    )
    for path in manifests:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(activation.native_messaging_manifest_bytes(host))
        path.chmod(0o600)
    activation.write_receipt(
        store,
        release,
        native_manifest_paths=manifests,
        stable_links=links,
    )
    monkeypatch.setattr("release_generation.verify_release_root", lambda *_args, **_kwargs: release)
    monkeypatch.setattr(
        activation.GenerationStore, "verify_installed_generation", lambda *_args: release
    )
    monkeypatch.setattr(activation, "find_unix_runtime_processes", lambda *_args, **_kwargs: [])

    resolved = activation.resolve_active_runtime(
        release.root,
        store_root=store,
        profile="full",
        expected_manifest_sha256=release.manifest_sha256,
        native_messaging_home=home,
        bin_dir=bin_dir,
        proc_root=tmp_path / "proc",
    )

    assert resolved.release_root == str(release.root)
    assert resolved.node_path == str(cua_root / "bin/node")
    assert resolved.node_repl_path == str(cua_root / "bin/node_repl")
    assert resolved.node_module_dirs == (str(cua_root / "lib/node_modules"),)
    assert resolved.browser_client_path == str(browser)
    assert resolved.codex_marketplace_path.endswith("codex-compat/openai-bundled")
    assert [plugin.id for plugin in resolved.codex_plugins] == [
        "computer-use@openai-bundled",
        "browser-use@openai-bundled",
    ]
    assert (bin_dir / "sky-cua-release").resolve() == installer
    assert (store / "bin/sky-cua-release").resolve() == installer
    assert (bin_dir / "node_repl").resolve() == cua_root / "bin/node_repl"
    assert (store / "bin/node_repl").resolve() == cua_root / "bin/node_repl"
    assert "current" in os.readlink(bin_dir / "node_repl")
    assert resolved.node_repl_path == str((bin_dir / "node_repl").resolve())


def test_node_repl_stable_link_follows_current_generation_rollover(
    tmp_path: Path,
) -> None:
    store = tmp_path / "store"
    bin_dir = tmp_path / "bin"
    component = "cua-node-linux-x64-glibc"
    releases: list[VerifiedRelease] = []
    for release_id, payload in (("a" * 64, b"old"), ("c" * 64, b"new")):
        root = store / "releases" / release_id
        launcher = root / "components" / component / "bin/node_repl"
        launcher.parent.mkdir(parents=True)
        launcher.write_bytes(payload)
        launcher.chmod(0o755)
        releases.append(
            VerifiedRelease(
                root=root,
                release_id=release_id,
                manifest_sha256="b" * 64,
                profile="full",
                component_names=(component,),
            )
        )
    store.mkdir(exist_ok=True)
    current = store / "current"
    current.symlink_to(Path("releases") / releases[0].release_id)
    links, _ = activation.install_stable_links(store, releases[0], bin_dir=bin_dir)
    user_launcher = bin_dir / "node_repl"
    store_launcher = store / "bin/node_repl"
    user_target = os.readlink(user_launcher)
    store_target = os.readlink(store_launcher)

    assert user_launcher.resolve().read_bytes() == b"old"
    assert store_launcher.resolve().read_bytes() == b"old"
    assert links[str(user_launcher)] == user_target
    current.unlink()
    current.symlink_to(Path("releases") / releases[1].release_id)

    assert os.readlink(user_launcher) == user_target
    assert os.readlink(store_launcher) == store_target
    assert user_launcher.resolve().read_bytes() == b"new"
    assert store_launcher.resolve().read_bytes() == b"new"


def test_verify_rejects_obsolete_process_and_receipt_skew(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store, release = _installed_release(tmp_path)
    home = tmp_path / "home"
    bin_dir = tmp_path / "bin"
    host = release.root / activation.HOST_RELATIVE_PATH
    host.parent.mkdir(parents=True, exist_ok=True)
    host.write_bytes(b"host")
    links, _ = activation.install_stable_links(store, release, bin_dir=bin_dir)
    manifests = tuple(
        home / relative / f"{activation.HOST_NAME}.json"
        for relative in activation.MANIFEST_RELATIVE_DIRS
    )
    for path in manifests:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(activation.native_messaging_manifest_bytes(host))
        path.chmod(0o600)
    activation.write_receipt(
        store,
        release,
        native_manifest_paths=manifests,
        stable_links=links,
    )
    monkeypatch.setattr("release_generation.verify_release_root", lambda *_args, **_kwargs: release)
    monkeypatch.setattr(
        activation.GenerationStore, "verify_installed_generation", lambda *_args: release
    )
    monkeypatch.setattr(activation, "find_unix_runtime_processes", lambda *_args, **_kwargs: [])

    report = activation.verify_activation(
        release.root,
        store_root=store,
        profile="full",
        expected_manifest_sha256=release.manifest_sha256,
        native_messaging_home=home,
        bin_dir=bin_dir,
    )
    assert report.release_id == release.release_id

    monkeypatch.setattr(
        activation,
        "find_unix_runtime_processes",
        lambda *_args, **_kwargs: [(42, "/deleted/sky-cua-service")],
    )
    with pytest.raises(
        activation.ActivationVerificationError,
        match="obsolete sky-cua runtime process",
    ):
        activation.verify_activation(
            release.root,
            store_root=store,
            profile="full",
            expected_manifest_sha256=release.manifest_sha256,
            native_messaging_home=home,
            bin_dir=bin_dir,
        )


def test_verify_and_drain_cover_deleted_opt_and_legacy_store_processes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store, release = _installed_release(tmp_path)
    home = tmp_path / "home"
    bin_dir = tmp_path / "bin"
    host = release.root / activation.HOST_RELATIVE_PATH
    host.parent.mkdir(parents=True, exist_ok=True)
    host.write_bytes(b"host")
    links, _ = activation.install_stable_links(store, release, bin_dir=bin_dir)
    manifests = tuple(
        home / relative / f"{activation.HOST_NAME}.json"
        for relative in activation.MANIFEST_RELATIVE_DIRS
    )
    for path in manifests:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(activation.native_messaging_manifest_bytes(host))
        path.chmod(0o600)
    activation.write_receipt(
        store,
        release,
        native_manifest_paths=manifests,
        stable_links=links,
    )
    monkeypatch.setattr("release_generation.verify_release_root", lambda *_args, **_kwargs: release)
    monkeypatch.setattr(
        activation.GenerationStore, "verify_installed_generation", lambda *_args: release
    )

    proc_root = tmp_path / "proc"
    deleted_opt = "/opt/sky-cua/releases/old/sky-cua-service (deleted)"
    legacy_store = str(store / "bin/sky-cua-service")
    _fake_process(proc_root, 41, deleted_opt)
    _fake_process(proc_root, 42, legacy_store)
    deleted_node = _fake_node_repl_process(
        proc_root,
        43,
        Path("/opt/chatgpt-desktop/resources/sky-cua")
        / ("e" * 64)
        / "components/cua-node-linux-x64-glibc",
        deleted=True,
    )
    old_store_node = _fake_node_repl_process(
        proc_root,
        44,
        store / "releases" / ("d" * 64) / "components/cua-node-linux-x64-glibc",
    )
    unrelated_node = _fake_node_repl_process(
        proc_root,
        45,
        Path("/opt/chatgpt-desktop/resources/sky-cua/not-a-release")
        / "components/cua-node-linux-x64-glibc",
    )

    with pytest.raises(
        activation.ActivationVerificationError,
        match="obsolete sky-cua runtime process",
    ) as caught:
        activation.verify_activation(
            release.root,
            store_root=store,
            profile="full",
            expected_manifest_sha256=release.manifest_sha256,
            native_messaging_home=home,
            bin_dir=bin_dir,
            proc_root=proc_root,
        )
    assert "/opt/sky-cua/releases/old/sky-cua-service" in str(caught.value)
    assert legacy_store in str(caught.value)
    assert deleted_node in str(caught.value)
    assert old_store_node in str(caught.value)
    assert unrelated_node not in str(caught.value)

    calls: list[tuple[int, int]] = []
    terminated: set[int] = set()

    def fake_kill(pid: int, signal: int) -> None:
        calls.append((pid, signal))
        if signal == plugin_bundle.SIGTERM:
            terminated.add(pid)
        if signal == 0 and pid in terminated:
            raise ProcessLookupError

    monkeypatch.setattr(plugin_bundle.os, "kill", fake_kill)
    activation.drain_stale_processes(store, proc_root=proc_root)
    assert (41, plugin_bundle.SIGTERM) in calls
    assert (42, plugin_bundle.SIGTERM) in calls
    assert (43, plugin_bundle.SIGTERM) in calls
    assert (44, plugin_bundle.SIGTERM) in calls
    assert all(pid != 45 for pid, _signal in calls)

    monkeypatch.setattr(activation, "find_unix_runtime_processes", lambda *_args, **_kwargs: [])
    receipt = store / activation.ACTIVATION_RECEIPT
    payload = json.loads(receipt.read_text())
    payload["release_id"] = "c" * 64
    receipt.write_text(json.dumps(payload), encoding="utf-8")

    with pytest.raises(
        activation.ActivationVerificationError,
        match="receipt does not match",
    ):
        activation.verify_activation(
            release.root,
            store_root=store,
            profile="full",
            expected_manifest_sha256=release.manifest_sha256,
            native_messaging_home=home,
            bin_dir=bin_dir,
        )
