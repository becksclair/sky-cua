from __future__ import annotations

import fcntl
import hashlib
import json
import subprocess
import threading
from pathlib import Path

import pytest

from _plugin_bundle import REPO_ROOT
from build_complete_release import (
    CODEX_PROJECTIONS,
    _build_core_from_commit,
    build_complete_release,
)
from release_generation import canonical_json_bytes, sha256_file, verify_release_root

PRODUCER_COMMIT = "dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a"


@pytest.fixture(autouse=True)
def _stub_core_build(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        "build_complete_release._build_core_from_commit",
        lambda _commit, source: source,
    )


def _core(root: Path) -> Path:
    core = root / "core"
    (core / "resources" / "plugins" / "openai-bundled").mkdir(parents=True)
    (core / "resources" / "plugins" / "openai-bundled" / "copied.js").write_text(
        "legacy", encoding="utf-8"
    )
    (core / "resources" / "node_repl").mkdir(parents=True)
    (core / "resources" / "node_repl" / "launcher").write_text("legacy", encoding="utf-8")
    (core / "bin").mkdir()
    (core / "bin" / "sky-cua").write_text("core", encoding="utf-8")
    return core


def _cua_node(root: Path, *, release_eligible: bool = True) -> Path:
    component = root / "cua-node"
    for directory in ("licenses", "share/provenance", "share/locks"):
        (component / directory).mkdir(parents=True, exist_ok=True)
    migration_input = {
        "schema_version": 1,
        "source_tree_sha256": "1" * 64,
        "source_size_bytes": 1,
        "source_file_count": 1,
        "migration_evidence": {"codex_desktop_commit": "2" * 40},
    }
    inventory: list[object] = [
        {"path": "runtime/cua-node/src/cli.ts", "sha256": "4" * 64, "size_bytes": 1}
    ]
    runtime_cli = component / "lib/node_repl/cli.js"
    browser_client = component / "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs"
    launcher = component / "bin/node_repl"
    for path, content in (
        (runtime_cli, "runtime"),
        (launcher, "launcher"),
    ):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    browser_client.parent.mkdir(parents=True, exist_ok=True)
    browser_client.write_bytes(
        (REPO_ROOT / "packages/browser-use/build/browser-client.mjs").read_bytes()
    )
    attestation = {
        "schema_version": 1,
        "release_eligible": release_eligible,
        "producer_commit": PRODUCER_COMMIT,
        "migration_input": migration_input,
        "source_inventory_sha256": hashlib.sha256(canonical_json_bytes(inventory)).hexdigest(),
        "source_inventory": inventory,
        "first_party_build": {
            "schema_version": 1,
            "commands": [{"cwd": "runtime/cua-node", "argv": ["bun", "run", "build"]}],
            "toolchain": {"bun": "1.3.14", "cc": "cc test"},
            "outputs": {
                "runtime_cli": {"sha256": sha256_file(runtime_cli)},
                "browser_client": {"sha256": sha256_file(browser_client)},
                "node_repl_launcher": {"sha256": sha256_file(launcher)},
                "sky_cua_tarball": {"sha256": "3" * 64},
            },
        },
    }
    (component / "share/provenance/SKY_CUA_BUILD_ATTESTATION.json").write_text(
        json.dumps(attestation), encoding="utf-8"
    )
    runtime_lock = component / "share/locks/runtime-lock.json"
    native_lock = component / "share/locks/native-assets.lock.json"
    for path in (runtime_lock, native_lock):
        path.write_text(
            json.dumps({"release_ready": True, "release_blockers": []}), encoding="utf-8"
        )
    (component / "licenses/LICENSES.json").write_text('{"packages":[]}\n', encoding="utf-8")
    (component / "licenses/PROVENANCE.json").write_text(
        '{"producer":"sky-cua"}\n', encoding="utf-8"
    )
    (component / "sbom.cdx.json").write_text(
        '{"bomFormat":"CycloneDX","specVersion":"1.6"}\n', encoding="utf-8"
    )
    checksum_files = []
    for path in sorted(component.rglob("*"), key=lambda item: item.as_posix()):
        if path.is_file() and path.name != "manifest.json":
            checksum_files.append(
                {
                    "path": path.relative_to(component).as_posix(),
                    "sha256": sha256_file(path),
                    "size_bytes": path.stat().st_size,
                }
            )
    browser_hash = sha256_file(browser_client)
    (component / "manifest.json").write_text(
        json.dumps(
            {
                "target": "linux-x64-glibc",
                "node_version": "24.14.0",
                "source": {
                    "producer_commit": PRODUCER_COMMIT,
                    "migration_input": migration_input,
                },
                "components": {
                    "sky_cua": {"tarball_sha256": "3" * 64},
                    "browser_use": {"entrypoint_sha256": browser_hash},
                },
                "trusted_browser_client_sha256s": [browser_hash],
                "checksums": {
                    "algorithm": "sha256",
                    "files": checksum_files,
                    "lock_hashes": {
                        "runtime_lock_sha256": sha256_file(runtime_lock),
                        "native_assets_lock_sha256": sha256_file(native_lock),
                    },
                },
            }
        ),
        encoding="utf-8",
    )
    return component


def test_complete_release_sanitizes_core_and_materializes_exact_codex_projections(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    result = build_complete_release(
        tmp_path / "out",
        producer_commit=PRODUCER_COMMIT,
        source_date_epoch=1_784_500_000,
        core_source=_core(tmp_path / "inputs"),
        cua_node_source=_cua_node(tmp_path / "inputs"),
        include_fat_archive=False,
    )
    release = verify_release_root(result.release.root)
    core = release.root / "components/core-linux-x64"
    compat = release.root / "components/codex-compat"
    canonical = release.root / "components/browser-js/browser-client.mjs"

    assert not (core / "resources/plugins/openai-bundled").exists()
    assert not (core / "resources/node_repl").exists()
    for relative in CODEX_PROJECTIONS:
        assert (compat / relative).read_bytes() == canonical.read_bytes()


def test_complete_release_rejects_development_cua_node_component(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    with pytest.raises(ValueError, match="not release eligible"):
        build_complete_release(
            tmp_path / "out",
            producer_commit=PRODUCER_COMMIT,
            source_date_epoch=1_784_500_000,
            core_source=_core(tmp_path / "inputs"),
            cua_node_source=_cua_node(tmp_path / "inputs", release_eligible=False),
            include_fat_archive=False,
        )

    assert not (tmp_path / "out").exists()


def test_complete_release_rejects_cua_node_browser_generation_mismatch(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    component = _cua_node(tmp_path / "inputs")
    (component / "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs").write_text(
        "different generation", encoding="utf-8"
    )

    with pytest.raises(ValueError, match="embedded Browser bytes differ"):
        build_complete_release(
            tmp_path / "out",
            producer_commit=PRODUCER_COMMIT,
            source_date_epoch=1_784_500_000,
            core_source=_core(tmp_path / "inputs"),
            cua_node_source=component,
            include_fat_archive=False,
        )


def test_complete_release_workspace_never_collides_with_output_root(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    output = tmp_path / ".complete-release-inputs"
    result = build_complete_release(
        output,
        producer_commit=PRODUCER_COMMIT,
        source_date_epoch=1_784_500_000,
        core_source=_core(tmp_path / "inputs"),
        cua_node_source=_cua_node(tmp_path / "inputs"),
        include_fat_archive=False,
    )

    assert output.is_dir()
    assert result.release.root.is_dir()


def test_complete_release_rejects_nested_component_symlinks(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    core = _core(tmp_path / "inputs")
    outside = tmp_path / "private.txt"
    outside.write_text("must not be copied\n", encoding="utf-8")
    (core / "resources/private-link").symlink_to(outside)

    with pytest.raises(ValueError, match="component input contains a symlink"):
        build_complete_release(
            tmp_path / "out",
            producer_commit=PRODUCER_COMMIT,
            source_date_epoch=1_784_500_000,
            core_source=core,
            cua_node_source=_cua_node(tmp_path / "inputs"),
            include_fat_archive=False,
        )


def test_core_build_rejects_noncanonical_prebuilt_input(tmp_path: Path) -> None:
    arbitrary = tmp_path / "prebuilt-core"
    arbitrary.mkdir()

    with pytest.raises(ValueError, match="must rebuild the canonical core output"):
        _build_core_from_commit(PRODUCER_COMMIT, arbitrary)


def test_core_build_requires_matching_clean_head_and_runs_canonical_builder(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    core = tmp_path / "dist/plugin/sky-cua"
    core.mkdir(parents=True)
    monkeypatch.setattr("build_complete_release.DIST_PLUGIN_ROOT", core)
    monkeypatch.setattr("build_complete_release._git_value", lambda *_args: PRODUCER_COMMIT)
    commands: list[list[str]] = []

    def run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        commands.append(command)
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

    monkeypatch.setattr("build_complete_release.subprocess.run", run)
    assert _build_core_from_commit(PRODUCER_COMMIT, core) == core
    assert commands == [
        ["git", "status", "--porcelain=v1", "--untracked-files=all"],
        ["python3", "scripts/build_plugin.py"],
    ]

    monkeypatch.setattr("build_complete_release._git_value", lambda *_args: "f" * 40)
    with pytest.raises(ValueError, match="must equal current HEAD"):
        _build_core_from_commit(PRODUCER_COMMIT, core)


def test_core_build_rejects_dirty_producer_tree(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    core = tmp_path / "dist/plugin/sky-cua"
    core.mkdir(parents=True)
    monkeypatch.setattr("build_complete_release.DIST_PLUGIN_ROOT", core)
    monkeypatch.setattr("build_complete_release._git_value", lambda *_args: PRODUCER_COMMIT)
    monkeypatch.setattr(
        "build_complete_release.subprocess.run",
        lambda command, **_kwargs: subprocess.CompletedProcess(
            command, 0, stdout=" M crates/sky-cua-client/src/main.rs\n", stderr=""
        ),
    )

    with pytest.raises(ValueError, match="requires a clean producer working tree"):
        _build_core_from_commit(PRODUCER_COMMIT, core)


def test_complete_release_snapshot_holds_assembly_lock_and_excludes_later_generation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    component = _cua_node(tmp_path / "inputs")
    core = _core(tmp_path / "inputs")
    lock_path = component.parent / ".cua-node-assembly.lock"
    source_cli = component / "lib/node_repl/cli.js"
    late_only = component / "late-generation-only.txt"
    staged_cli = component.parent / "next-generation-cli.js"
    staged_cli.write_text("later generation", encoding="utf-8")

    snapshot_started = threading.Event()
    contender_blocked = threading.Event()
    promotion_complete = threading.Event()
    contender_observations: list[bool] = []
    contender_errors: list[BaseException] = []

    def promote_later_generation() -> None:
        try:
            assert snapshot_started.wait(timeout=5)
            with lock_path.open("a+b") as lock:
                try:
                    fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    contender_observations.append(True)
                else:
                    contender_observations.append(False)
                    fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
                finally:
                    contender_blocked.set()

                fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
                staged_cli.replace(source_cli)
                late_only.write_text("later generation", encoding="utf-8")
                fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
        except BaseException as error:
            contender_errors.append(error)
        finally:
            promotion_complete.set()

    contender = threading.Thread(target=promote_later_generation, daemon=True)
    contender.start()

    from build_complete_release import _prepare_inputs as real_prepare_inputs

    def observe_locked_snapshot(
        workspace: Path, *, core_source: Path, cua_node_source: Path
    ) -> dict[str, Path]:
        snapshot_started.set()
        assert contender_blocked.wait(timeout=5)
        assert contender_observations == [True]
        return real_prepare_inputs(
            workspace,
            core_source=core_source,
            cua_node_source=cua_node_source,
        )

    def verify_private_snapshot(snapshot: Path) -> None:
        assert promotion_complete.wait(timeout=5)
        assert snapshot != component
        assert (snapshot / "lib/node_repl/cli.js").read_text(encoding="utf-8") == "runtime"
        assert not (snapshot / late_only.name).exists()

    monkeypatch.setattr("build_complete_release._prepare_inputs", observe_locked_snapshot)
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", verify_private_snapshot)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )

    result = build_complete_release(
        tmp_path / "out",
        producer_commit=PRODUCER_COMMIT,
        source_date_epoch=1_784_500_000,
        core_source=core,
        cua_node_source=component,
        include_fat_archive=False,
    )
    contender.join(timeout=5)

    assert not contender.is_alive()
    assert contender_errors == []
    assert source_cli.read_text(encoding="utf-8") == "later generation"
    released = result.release.root / "components/cua-node-linux-x64-glibc"
    assert (released / "lib/node_repl/cli.js").read_text(encoding="utf-8") == "runtime"
    assert not (released / late_only.name).exists()
