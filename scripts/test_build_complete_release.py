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
    CANONICAL_EXTENSION_COMPONENT_PATH,
    CANONICAL_EXTENSION_VERSION,
    CODEX_PROJECTIONS,
    CORE_BUILD_INPUT_PROVENANCE,
    _build_core_from_commit,
    build_complete_release,
)
from release_generation import (
    canonical_json_bytes,
    sha256_file,
    verify_release_root,
)

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
    (core / "bin" / "sky-cua-browser-preflight").write_text("legacy", encoding="utf-8")
    (core / "resources/chrome_preflight.py").write_text("legacy", encoding="utf-8")
    runtime = core / "bin/runtimes/linux-x64"
    runtime.mkdir(parents=True)
    for name in ("sky-cua-client", "sky-cua-service", "sky-cua-chrome-host"):
        binary = runtime / name
        binary.write_text(name, encoding="utf-8")
        binary.chmod(0o755)
    (runtime / "sky-cua-client.buildstamp.json").write_text(
        json.dumps(
            {
                "version": 1,
                "source_fingerprint": "f" * 64,
                "git_sha": PRODUCER_COMMIT,
                "git_dirty": False,
                "repo_root": str(REPO_ROOT),
            }
        ),
        encoding="utf-8",
    )
    extension = core / Path(CANONICAL_EXTENSION_COMPONENT_PATH).relative_to(
        "components/core-linux-x64"
    )
    extension.mkdir(parents=True)
    (extension / "manifest.json").write_bytes(
        (
            REPO_ROOT
            / "resources/chrome-extension/codex"
            / f"{CANONICAL_EXTENSION_VERSION}_0/manifest.json"
        ).read_bytes()
    )
    provenance = core / CORE_BUILD_INPUT_PROVENANCE
    provenance.parent.mkdir(parents=True, exist_ok=True)
    provenance.write_text(
        json.dumps(
            {
                "producer_commit": PRODUCER_COMMIT,
                "source": {"kind": "git-archive", "commit": PRODUCER_COMMIT},
                "external_inputs": [],
            }
        ),
        encoding="utf-8",
    )
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
    phone_declarations = component / "lib/node_modules/@heliasar/sky-cua/dist/phone"
    phone_declarations.mkdir(parents=True)
    for name in ("client.d.ts", "index.d.ts", "protocol.d.ts", "screenshot.d.ts"):
        (phone_declarations / name).write_text(f"export type {name.split('.')[0]} = unknown;\n")
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
    browser_license = component / "licenses/packages/browser/LICENSE.txt"
    browser_license.parent.mkdir(parents=True)
    browser_license.write_text("test proprietary license\n", encoding="utf-8")
    (component / "licenses/LICENSES.json").write_text(
        json.dumps(
            {
                "packages": [
                    {
                        "name": "@heliasar/browser-use",
                        "version": "1.0.0",
                        "license": "LicenseRef-Heliasar-Proprietary",
                        "license_files": ["licenses/packages/browser/LICENSE.txt"],
                        "license_file_sha256s": {
                            "licenses/packages/browser/LICENSE.txt": sha256_file(browser_license)
                        },
                    }
                ]
            }
        )
        + "\n",
        encoding="utf-8",
    )
    (component / "licenses/PROVENANCE.json").write_text(
        '{"producer":"sky-cua"}\n', encoding="utf-8"
    )
    (component / "sbom.cdx.json").write_text(
        '{"bomFormat":"CycloneDX","specVersion":"1.6","metadata":{"properties":[]},"components":[{"type":"library","name":"@heliasar/browser-use","version":"1.0.0"}]}\n',
        encoding="utf-8",
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
    manifest = json.loads((release.root / "RELEASE.json").read_text(encoding="utf-8"))
    documented_capabilities = json.loads(
        (release.root / "components/documentation/inventories/capability-inventory.json").read_text(
            encoding="utf-8"
        )
    )["supported"]

    assert "phone-use-persistent-js" in manifest["capabilities"]["supported"]
    assert set(documented_capabilities) <= set(manifest["capabilities"]["supported"])
    phone_api = json.loads(
        (release.root / "components/documentation/inventories/api-inventory.json").read_text(
            encoding="utf-8"
        )
    )["phone"]
    installed_phone_declarations = (
        release.root
        / "components/cua-node-linux-x64-glibc/lib/node_modules/@heliasar/sky-cua/dist/phone"
    )
    for record in phone_api["declarations"]:
        declaration = installed_phone_declarations / record["name"]
        assert record["sha256"] == sha256_file(declaration)
        assert record["size_bytes"] == declaration.stat().st_size

    assert not (core / "resources/plugins/openai-bundled").exists()
    assert not (core / "resources/node_repl").exists()
    assert not (core / "bin/sky-cua-browser-preflight").exists()
    assert not (core / "resources/chrome_preflight.py").exists()
    stamp = json.loads(
        (core / "bin/runtimes/linux-x64/sky-cua-client.buildstamp.json").read_text(encoding="utf-8")
    )
    assert "repo_root" not in stamp
    assert stamp["source"] == {"kind": "git-archive", "commit": PRODUCER_COMMIT}
    provenance = json.loads(
        (release.root / "compliance/PROVENANCE.json").read_text(encoding="utf-8")
    )
    assert {record["name"] for record in provenance["release_inventory"]} == {
        "sky-cua-core",
        "sky-cua-client",
        "sky-cua-service",
        "sky-cua-chrome-host",
        "browser-extension-assets",
        "browser-js",
        "cua-node",
        "codex-compat",
        "installer",
        "installer-entrypoint",
        "model-facing-documentation",
    }
    sbom = json.loads((release.root / "compliance/sbom.cdx.json").read_text(encoding="utf-8"))
    assert sbom["metadata"]["component"]["name"] == "sky-cua complete CUA stack"
    for relative in CODEX_PROJECTIONS:
        assert (compat / relative).read_bytes() == canonical.read_bytes()


def test_complete_release_ships_checkout_free_verified_controller(
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
        include_fat_archive=True,
    )
    root = result.release.root
    manifest = json.loads((root / "RELEASE.json").read_text(encoding="utf-8"))
    components = {record["name"]: record for record in manifest["components"]}

    assert "installer" in components
    assert set(components["installer"]["profiles"]) == {"core-only", "full"}
    assert manifest["artifacts"]["installer_entrypoint"]["path"] == "install.py"
    extension = manifest["browser_contract"]["extension_bridge"]
    assert extension["extension_id"] == "hehggadaopoacecdllhhajmbjkdcmajg"
    assert extension["version"] == CANONICAL_EXTENSION_VERSION
    assert extension["path"] == CANONICAL_EXTENSION_COMPONENT_PATH
    assert len(extension["manifest_sha256"]) == 64
    assert len(extension["tree_sha256"]) == 64
    assert (root / "components/installer/install_complete_release.py").is_file()
    assert (root / "components/installer/_native_messaging_install.py").is_file()

    neutral = tmp_path / "neutral"
    neutral.mkdir()
    completed = subprocess.run(
        [
            "python3",
            str(root / "install.py"),
            "verify",
            "--manifest-sha256",
            result.release.manifest_sha256,
        ],
        cwd=neutral,
        env={"HOME": str(tmp_path / "home"), "PATH": "/usr/bin:/bin", "PYTHONPATH": ""},
        check=True,
        capture_output=True,
        text=True,
    )
    report = json.loads(completed.stdout)
    assert report["status"] == "ok"
    assert report["release_id"] == result.release.release_id
    assert "installer" in report["components"]

    installed = subprocess.run(
        [
            "python3",
            str(root / "install.py"),
            "install",
            "--store-root",
            str(tmp_path / "store"),
            "--native-messaging-home",
            str(tmp_path / "browser-home"),
        ],
        cwd=neutral,
        env={"HOME": str(tmp_path / "home"), "PATH": "/usr/bin:/bin", "PYTHONPATH": ""},
        check=True,
        capture_output=True,
        text=True,
    )
    install_report = json.loads(installed.stdout)
    assert install_report["release_id"] == result.release.release_id
    assert install_report["browser_reload_required"] is False
    assert install_report["browser_extension"]["activation"] == "web_store_preinstalled"
    assert install_report["browser_extension"]["path"].startswith(
        str(tmp_path / "store" / "releases" / result.release.release_id)
    )
    assert (tmp_path / "store/current").resolve() == (
        tmp_path / "store" / "releases" / result.release.release_id
    )
    brave_origin_manifest = (
        tmp_path
        / "browser-home/.config/BraveSoftware/Brave-Origin/NativeMessagingHosts"
        / "com.openai.codexextension.json"
    )
    assert json.loads(brave_origin_manifest.read_text(encoding="utf-8"))["path"].startswith(
        str(tmp_path / "store" / "releases" / result.release.release_id)
    )


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


def test_complete_release_normalizes_checkout_shaped_paths_in_packaged_text(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    core = _core(tmp_path / "inputs")
    leak = core / "docs/leak.md"
    leak.parent.mkdir()
    leak.write_text("producer path: /home/alice/projects/sky-cua/private\n", encoding="utf-8")

    result = build_complete_release(
        tmp_path / "out",
        producer_commit=PRODUCER_COMMIT,
        source_date_epoch=1_784_500_000,
        core_source=core,
        cua_node_source=_cua_node(tmp_path / "inputs"),
        include_fat_archive=False,
    )

    packaged = result.release.root / "components/core-linux-x64/docs/leak.md"
    assert packaged.read_text(encoding="utf-8") == (
        "producer path: ${SKY_CUA_SOURCE_ROOT}/sky-cua/private\n"
    )


def test_complete_release_rejects_producer_path_embedded_in_binary(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    core = _core(tmp_path / "inputs")
    (core / "bin/runtimes/linux-x64/sky-cua-service").write_bytes(
        b"\x00" + str(REPO_ROOT).encode() + b"\x00"
    )

    with pytest.raises(ValueError, match="packaged binary contains"):
        build_complete_release(
            tmp_path / "out",
            producer_commit=PRODUCER_COMMIT,
            source_date_epoch=1_784_500_000,
            core_source=core,
            cua_node_source=_cua_node(tmp_path / "inputs"),
            include_fat_archive=False,
        )


def test_complete_release_allows_attested_native_debug_build_paths(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    core = _core(tmp_path / "inputs")
    binary = core / "bin/runtimes/linux-x64/sky-cua-service"
    binary.write_bytes(b"\x00/home/builder/projects/codex-desktop/vendor/native/source/file.cc\x00")

    result = build_complete_release(
        tmp_path / "out",
        producer_commit=PRODUCER_COMMIT,
        source_date_epoch=1_784_500_000,
        core_source=core,
        cua_node_source=_cua_node(tmp_path / "inputs"),
        include_fat_archive=False,
    )

    assert result.release.release_id


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


def test_complete_release_rejects_unattested_core_external_input(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    core = _core(tmp_path / "inputs")
    provenance_path = core / CORE_BUILD_INPUT_PROVENANCE
    provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
    provenance["external_inputs"] = [
        {"path": "resources/android/phone-companion.apk", "binding": None}
    ]
    provenance_path.write_text(json.dumps(provenance), encoding="utf-8")

    with pytest.raises(ValueError, match="unattested external inputs"):
        build_complete_release(
            tmp_path / "out",
            producer_commit=PRODUCER_COMMIT,
            source_date_epoch=1_784_500_000,
            core_source=core,
            cua_node_source=_cua_node(tmp_path / "inputs"),
            include_fat_archive=False,
        )


def test_complete_release_core_input_build_is_deterministic(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("build_complete_release._verify_inner_cua_node", lambda _root: None)
    monkeypatch.setattr(
        "build_complete_release._verify_git_source_inventory", lambda _root, _commit: None
    )
    inputs = tmp_path / "inputs"
    core = _core(inputs)
    cua_node = _cua_node(inputs)

    first = build_complete_release(
        tmp_path / "first",
        producer_commit=PRODUCER_COMMIT,
        source_date_epoch=1_784_500_000,
        core_source=core,
        cua_node_source=cua_node,
        include_fat_archive=False,
    )
    second = build_complete_release(
        tmp_path / "second",
        producer_commit=PRODUCER_COMMIT,
        source_date_epoch=1_784_500_000,
        core_source=core,
        cua_node_source=cua_node,
        include_fat_archive=False,
    )

    assert first.release.release_id == second.release.release_id
    assert first.release.manifest_sha256 == second.release.manifest_sha256


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
    private = tmp_path / "private.txt"
    private.write_text("must stay private\n", encoding="utf-8")
    (core / "nested-private").symlink_to(private)
    monkeypatch.setattr("build_complete_release.DIST_PLUGIN_ROOT", core)
    monkeypatch.setattr("build_complete_release._git_value", lambda *_args: PRODUCER_COMMIT)
    commands: list[list[str]] = []

    def run(command: list[str], **_kwargs: object) -> subprocess.CompletedProcess[str]:
        commands.append(command)
        if command[0] == "python3":
            isolated = Path(command[command.index("--dist-root") + 1])
            isolated.mkdir(parents=True)
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

    monkeypatch.setattr("build_complete_release.subprocess.run", run)
    isolated = _build_core_from_commit(PRODUCER_COMMIT, core)
    assert isolated != core
    assert isolated.parent.name.startswith(".complete-release-core-")
    assert not (isolated / "nested-private").exists()
    assert (core / "nested-private").is_symlink()
    assert commands[0] == ["git", "status", "--porcelain=v1", "--untracked-files=all"]
    assert commands[1] == [
        "python3",
        "scripts/build_plugin.py",
        "--dist-root",
        str(isolated),
        "--release-core-commit",
        PRODUCER_COMMIT,
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


def _commit_fixture_repo(root: Path, *, tracked_symlink: bool = False) -> tuple[Path, str]:
    repo = root / "repo"
    (repo / "resources").mkdir(parents=True)
    (repo / ".gitignore").write_text(
        "/resources/android/\n/resources/private-link\n", encoding="utf-8"
    )
    (repo / "resources/tracked.txt").write_text("committed\n", encoding="utf-8")
    if tracked_symlink:
        (repo / "tracked-target.txt").write_text("private\n", encoding="utf-8")
        (repo / "resources/tracked-link").symlink_to(repo / "tracked-target.txt")
    subprocess.run(["git", "init", "-q"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=repo, check=True)
    subprocess.run(["git", "add", "."], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-qm", "fixture"], cwd=repo, check=True)
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repo,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    return repo, commit


def test_release_core_commit_archive_excludes_ignored_and_stale_worktree_inputs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import build_plugin

    repo, commit = _commit_fixture_repo(tmp_path)
    private = tmp_path / "private.txt"
    private.write_text("must not be dereferenced\n", encoding="utf-8")
    (repo / "resources/private-link").symlink_to(private)
    android = repo / "resources/android"
    android.mkdir()
    (android / "phone-companion.apk").write_bytes(b"stale ignored apk")
    (android / "phone-companion.json").write_text('{"stale":true}\n', encoding="utf-8")
    (repo / "resources/tracked.txt").write_text("uncommitted replacement\n", encoding="utf-8")
    monkeypatch.setattr(build_plugin, "REPO_ROOT", repo)

    first = tmp_path / "first"
    second = tmp_path / "second"
    build_plugin.copy_commit_bundle_sources(first, commit, source_paths=[Path("resources")])
    build_plugin.copy_commit_bundle_sources(second, commit, source_paths=[Path("resources")])

    assert (first / "resources/tracked.txt").read_text(encoding="utf-8") == "committed\n"
    assert not (first / "resources/private-link").exists()
    assert not (first / "resources/android").exists()
    assert (first / "resources/tracked.txt").read_bytes() == (
        second / "resources/tracked.txt"
    ).read_bytes()


def test_release_core_commit_archive_rejects_tracked_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import build_plugin

    repo, commit = _commit_fixture_repo(tmp_path, tracked_symlink=True)
    monkeypatch.setattr(build_plugin, "REPO_ROOT", repo)

    with pytest.raises(ValueError, match="non-regular entry"):
        build_plugin.copy_commit_bundle_sources(
            tmp_path / "out", commit, source_paths=[Path("resources")]
        )


def test_release_core_bundle_skips_all_optional_and_preexisting_fallback_inputs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import build_plugin

    output = tmp_path / "sky-cua"
    private = tmp_path / "private.txt"
    private.write_text("private\n", encoding="utf-8")
    output.mkdir()
    (output / "nested-private").symlink_to(private)

    def committed_sources(root: Path, _commit: str) -> None:
        (root / ".codex-plugin").mkdir(parents=True)
        (root / ".codex-plugin/plugin.json").write_text("{}\n", encoding="utf-8")

    def forbidden(*_args: object, **_kwargs: object) -> None:
        raise AssertionError("release-core mode consumed an optional or fallback input")

    monkeypatch.setattr(build_plugin, "copy_commit_bundle_sources", committed_sources)
    monkeypatch.setattr(build_plugin, "copy_tracked_bundle_sources", forbidden)
    monkeypatch.setattr(build_plugin, "copy_worktree_bundle_files", forbidden)
    monkeypatch.setattr(build_plugin, "copy_worktree_bundle_dirs", forbidden)
    monkeypatch.setattr(build_plugin, "copy_companion_apk_if_present", forbidden)
    monkeypatch.setattr(build_plugin, "stage_openai_bundled_plugins", forbidden)
    monkeypatch.setattr(build_plugin, "platform_runtime_binary_base_names", lambda _platform: ())
    monkeypatch.setattr(build_plugin, "bundle_entrypoint_paths", lambda: [])
    monkeypatch.setattr(build_plugin, "ensure_bundle_structure", lambda _root: None)

    build_plugin.stage_bundle(output, release_core_commit=PRODUCER_COMMIT)

    provenance = json.loads(
        (output / build_plugin.RELEASE_CORE_INPUT_PROVENANCE).read_text(encoding="utf-8")
    )
    assert provenance["producer_commit"] == PRODUCER_COMMIT
    assert provenance["external_inputs"] == []
    assert not (output / "nested-private").exists()
    assert private.read_text(encoding="utf-8") == "private\n"


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
