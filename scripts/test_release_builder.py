from __future__ import annotations

import hashlib
import json
import os
import subprocess
from pathlib import Path

import pytest

from release_builder import ComponentSource, FileSource, build_release_set
from release_generation import (
    ReleaseValidationError,
    canonical_json_bytes,
    sha256_file,
    verify_release_root,
)


def _inputs(root: Path) -> tuple[list[ComponentSource], list[FileSource], list[FileSource]]:
    core = root / "core"
    browser = root / "browser"
    node = root / "node"
    compat = root / "compat"
    compliance = root / "compliance-component"
    for path in (core, browser, node, compat, compliance):
        path.mkdir(parents=True)
    (core / "sky-cua-client").write_text("core", encoding="utf-8")
    (browser / "browser-client.mjs").write_text("export const browser = 1;\n", encoding="utf-8")
    (node / "node").write_text("24.14.0", encoding="utf-8")
    runtime_cli = node / "lib/node_repl/cli.js"
    browser_client = node / "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs"
    launcher = node / "bin/node_repl"
    for path, content in (
        (runtime_cli, "runtime"),
        (browser_client, "browser"),
        (launcher, "launcher"),
    ):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    producer_commit = "dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a"
    migration_input = {
        "schema_version": 1,
        "source_tree_sha256": "1" * 64,
        "source_size_bytes": 1,
        "source_file_count": 1,
        "migration_evidence": {"codex_desktop_commit": "65c69a3f1afc9f81274189901bc72e80682ea03a"},
    }
    inventory: list[object] = [
        {"path": "runtime/cua-node/src/cli.ts", "sha256": "4" * 64, "size_bytes": 1}
    ]
    attestation = {
        "schema_version": 1,
        "release_eligible": True,
        "producer_commit": producer_commit,
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
    attestation_path = node / "share" / "provenance" / "SKY_CUA_BUILD_ATTESTATION.json"
    attestation_path.parent.mkdir(parents=True)
    attestation_path.write_text(json.dumps(attestation), encoding="utf-8")
    runtime_lock = node / "share" / "locks" / "runtime-lock.json"
    native_lock = node / "share" / "locks" / "native-assets.lock.json"
    runtime_lock.parent.mkdir(parents=True)
    for path in (runtime_lock, native_lock):
        path.write_text(
            json.dumps({"release_ready": True, "release_blockers": []}), encoding="utf-8"
        )
    checksum_files = []
    for path in sorted(node.rglob("*"), key=lambda item: item.as_posix()):
        if path.is_file() and path.name != "manifest.json":
            checksum_files.append(
                {
                    "path": path.relative_to(node).as_posix(),
                    "sha256": sha256_file(path),
                    "size_bytes": path.stat().st_size,
                }
            )
    browser_hash = sha256_file(browser_client)
    (node / "manifest.json").write_text(
        json.dumps(
            {
                "target": "linux-x64-glibc",
                "node_version": "24.14.0",
                "source": {
                    "producer_commit": producer_commit,
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
    (compat / "browser-client.mjs").write_bytes((browser / "browser-client.mjs").read_bytes())
    marketplace = compat / "openai-bundled"
    marketplace_manifest = marketplace / ".agents/plugins/marketplace.json"
    marketplace_manifest.parent.mkdir(parents=True)
    marketplace_manifest.write_text(
        json.dumps(
            {
                "name": "openai-bundled",
                "plugins": [
                    {"name": "computer-use"},
                    {"name": "browser-use"},
                ],
            }
        ),
        encoding="utf-8",
    )
    for name, version, server in (
        ("computer-use", "0.1.0-sky-cua", "computer-use"),
        ("browser-use", "1.0.0-sky-cua-openclaw", "node_repl"),
    ):
        plugin = marketplace / "plugins" / name
        manifest = plugin / ".codex-plugin/plugin.json"
        manifest.parent.mkdir(parents=True)
        manifest.write_text(json.dumps({"name": name, "version": version}), encoding="utf-8")
        (plugin / ".mcp.json").write_text(
            json.dumps({"mcpServers": {server: {"command": "test"}}}),
            encoding="utf-8",
        )
    (compliance / "LICENSES.json").write_text("{}\n", encoding="utf-8")

    lock = root / "runtime-lock.json"
    lock.write_text('{"node":"24.14.0"}\n', encoding="utf-8")
    sbom = root / "sbom.json"
    provenance = root / "provenance.json"
    licenses = root / "licenses.json"
    sbom.write_text('{"bomFormat":"CycloneDX"}\n', encoding="utf-8")
    provenance.write_text('{"producer":"sky-cua"}\n', encoding="utf-8")
    licenses.write_text('{"packages":[]}\n', encoding="utf-8")
    return (
        [
            ComponentSource("core-linux-x64", core, profiles=("full", "core-only")),
            ComponentSource("browser-js", browser, dependencies=("core-linux-x64",)),
            ComponentSource(
                "cua-node-linux-x64-glibc",
                node,
                dependencies=("core-linux-x64", "browser-js"),
            ),
            ComponentSource("codex-compat", compat, dependencies=("browser-js",)),
            ComponentSource("compliance", compliance, profiles=("full", "core-only")),
        ],
        [FileSource("runtime", lock, "locks/runtime-lock.json")],
        [
            FileSource("sbom", sbom, "compliance/sbom.json"),
            FileSource("provenance", provenance, "compliance/provenance.json"),
            FileSource("licenses", licenses, "compliance/licenses.json"),
        ],
    )


def _runtime() -> dict[str, object]:
    return {
        "node": "24.14.0",
        "node_repl": "1.0.0",
        "browser_use": "1.0.0",
        "sky_cua_js": "0.1.0",
        "playwright": "1.57.0",
        "pdfjs": "5.4.624",
        "tesseract_js": "7.0.0",
        "sharp": "0.34.5",
        "sharp_linux_x64": "0.34.5",
        "sharp_libvips_linux_x64": "1.2.4",
        "canvas_linux_x64_gnu": "0.1.91",
        "pixelmatch": "7.1.0",
        "codecs": ["jpeg", "png", "webp"],
    }


def _build(output: Path, inputs_root: Path):
    components, locks, artifacts = _inputs(inputs_root)
    return _build_inputs(output, inputs_root, components, locks, artifacts)


def _build_inputs(
    output: Path,
    inputs_root: Path,
    components: list[ComponentSource],
    locks: list[FileSource],
    artifacts: list[FileSource],
):
    browser_hash = sha256_file(inputs_root / "browser" / "browser-client.mjs")
    return build_release_set(
        output,
        producer_commit="dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a",
        runtime=_runtime(),
        trusted_browser_client_sha256s=[browser_hash],
        capabilities_supported=["linux-x64-glibc", "sky_cua", "node_repl"],
        capabilities_unsupported=["linux-arm64", "linux-musl", "windows-node-repl"],
        browser_api_schema_version=1,
        browser_command_schema_version=1,
        canonical_browser_entrypoint="browser-client.mjs",
        compatibility_browser_projections=["browser-client.mjs"],
        components=components,
        locks=locks,
        artifacts=artifacts,
        source_date_epoch=1_784_500_000,
    )


def test_builder_emits_verified_component_set_and_fat_archive(tmp_path: Path) -> None:
    result = _build(tmp_path / "out", tmp_path / "inputs")
    root = result.release.root
    manifest = json.loads((root / "RELEASE.json").read_text(encoding="utf-8"))

    assert root.name == result.release.release_id
    assert result.fat_archive is not None and result.fat_archive.is_file()
    assert {component["name"] for component in manifest["components"]} == {
        "core-linux-x64",
        "browser-js",
        "cua-node-linux-x64-glibc",
        "codex-compat",
        "compliance",
    }
    assert all(component["archive"].startswith("archives/") for component in manifest["components"])
    assert verify_release_root(root).manifest_sha256 == result.release.manifest_sha256


def test_builder_normalizes_modes_for_fat_archive_extraction(tmp_path: Path) -> None:
    inputs_root = tmp_path / "inputs"
    components, locks, artifacts = _inputs(inputs_root)
    installer = inputs_root / "install.py"
    installer.write_text("#!/usr/bin/env python3\n", encoding="utf-8")
    installer.chmod(0o664)
    artifacts.append(FileSource("installer", installer, "install.py", executable=True))
    core_file = inputs_root / "core/sky-cua-client"
    launcher = inputs_root / "node/bin/node_repl"
    core_file.chmod(0o664)
    launcher.chmod(0o775)

    result = _build_inputs(tmp_path / "out", inputs_root, components, locks, artifacts)
    assert result.fat_archive is not None
    assert core_file.stat().st_mode & 0o777 == 0o664
    assert launcher.stat().st_mode & 0o777 == 0o775
    assert (
        result.release.root / "components/core-linux-x64/sky-cua-client"
    ).stat().st_mode & 0o777 == 0o644
    assert (
        result.release.root / "components/cua-node-linux-x64-glibc/bin/node_repl"
    ).stat().st_mode & 0o777 == 0o755
    assert (result.release.root / "install.py").stat().st_mode & 0o777 == 0o755

    extracted = tmp_path / "extracted"
    extracted.mkdir()
    subprocess.run(["tar", "xzf", str(result.fat_archive)], cwd=extracted, check=True)
    verified = verify_release_root(extracted / f"sky-cua-{result.release.release_id}")
    assert verified.manifest_sha256 == result.release.manifest_sha256
    assert (
        extracted / f"sky-cua-{result.release.release_id}/install.py"
    ).stat().st_mode & 0o777 == 0o755


def test_builder_is_content_addressed_and_deterministic(tmp_path: Path) -> None:
    first = _build(tmp_path / "first-out", tmp_path / "first-inputs")
    second = _build(tmp_path / "second-out", tmp_path / "second-inputs")

    assert first.release.release_id == second.release.release_id
    assert first.release.manifest_sha256 == second.release.manifest_sha256
    assert first.fat_archive is not None and second.fat_archive is not None
    assert sha256_file(first.fat_archive) == sha256_file(second.fat_archive)


def test_builder_is_idempotent_for_existing_release(tmp_path: Path) -> None:
    first = _build(tmp_path / "out", tmp_path / "inputs")
    components, locks, artifacts = _inputs(tmp_path / "inputs-again")
    browser_hash = sha256_file(tmp_path / "inputs-again" / "browser" / "browser-client.mjs")
    second = build_release_set(
        tmp_path / "out",
        producer_commit="dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a",
        runtime=_runtime(),
        trusted_browser_client_sha256s=[browser_hash],
        capabilities_supported=["linux-x64-glibc", "sky_cua", "node_repl"],
        capabilities_unsupported=["linux-arm64", "linux-musl", "windows-node-repl"],
        browser_api_schema_version=1,
        browser_command_schema_version=1,
        canonical_browser_entrypoint="browser-client.mjs",
        compatibility_browser_projections=["browser-client.mjs"],
        components=components,
        locks=locks,
        artifacts=artifacts,
        source_date_epoch=1_784_500_000,
    )
    assert second.release.release_id == first.release.release_id


def test_component_archive_tamper_is_rejected(tmp_path: Path) -> None:
    result = _build(tmp_path / "out", tmp_path / "inputs")
    archive = result.release.root / "archives" / "browser-js.tar.gz"
    archive.write_bytes(archive.read_bytes() + b"tamper")

    with pytest.raises(ReleaseValidationError, match="browser-js archive hash mismatch"):
        verify_release_root(result.release.root)


def test_builder_rejects_noncanonical_trusted_browser_hash(tmp_path: Path) -> None:
    components, locks, artifacts = _inputs(tmp_path / "inputs")
    with pytest.raises(ValueError, match="exactly the canonical Browser"):
        build_release_set(
            tmp_path / "out",
            producer_commit="dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a",
            runtime=_runtime(),
            trusted_browser_client_sha256s=["0" * 64],
            capabilities_supported=["linux-x64-glibc", "sky_cua", "node_repl"],
            capabilities_unsupported=["linux-arm64"],
            browser_api_schema_version=1,
            browser_command_schema_version=1,
            canonical_browser_entrypoint="browser-client.mjs",
            compatibility_browser_projections=["browser-client.mjs"],
            components=components,
            locks=locks,
            artifacts=artifacts,
        )


def test_builder_rejects_projection_bytes_that_differ_from_canonical(tmp_path: Path) -> None:
    components, locks, artifacts = _inputs(tmp_path / "inputs")
    (tmp_path / "inputs" / "compat" / "browser-client.mjs").write_text(
        "export const browser = 2;\n", encoding="utf-8"
    )
    browser_hash = sha256_file(tmp_path / "inputs" / "browser" / "browser-client.mjs")
    with pytest.raises(ValueError, match="differs from canonical bytes"):
        build_release_set(
            tmp_path / "out",
            producer_commit="dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a",
            runtime=_runtime(),
            trusted_browser_client_sha256s=[browser_hash],
            capabilities_supported=["linux-x64-glibc", "sky_cua", "node_repl"],
            capabilities_unsupported=["linux-arm64"],
            browser_api_schema_version=1,
            browser_command_schema_version=1,
            canonical_browser_entrypoint="browser-client.mjs",
            compatibility_browser_projections=["browser-client.mjs"],
            components=components,
            locks=locks,
            artifacts=artifacts,
        )


def test_builder_rejects_development_prepared_cua_node_component(tmp_path: Path) -> None:
    components, locks, artifacts = _inputs(tmp_path / "inputs")
    attestation_path = (
        tmp_path / "inputs" / "node" / "share" / "provenance" / "SKY_CUA_BUILD_ATTESTATION.json"
    )
    attestation = json.loads(attestation_path.read_text(encoding="utf-8"))
    attestation["release_eligible"] = False
    attestation_path.write_text(json.dumps(attestation), encoding="utf-8")
    browser_hash = sha256_file(tmp_path / "inputs" / "browser" / "browser-client.mjs")

    with pytest.raises(ValueError, match="not release eligible"):
        build_release_set(
            tmp_path / "out",
            producer_commit="dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a",
            runtime=_runtime(),
            trusted_browser_client_sha256s=[browser_hash],
            capabilities_supported=["linux-x64-glibc", "sky_cua", "node_repl"],
            capabilities_unsupported=["linux-arm64"],
            browser_api_schema_version=1,
            browser_command_schema_version=1,
            canonical_browser_entrypoint="browser-client.mjs",
            compatibility_browser_projections=["browser-client.mjs"],
            components=components,
            locks=locks,
            artifacts=artifacts,
        )

    assert not (tmp_path / "out").exists()


def test_builder_rejects_cua_node_file_inventory_tamper(tmp_path: Path) -> None:
    inputs = tmp_path / "inputs"
    components, locks, artifacts = _inputs(inputs)
    (inputs / "node/share/locks/runtime-lock.json").write_text(
        '{"release_ready":true,"release_blockers":[],"tampered":true}\n',
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="file inventory does not match"):
        _build_inputs(tmp_path / "out", inputs, components, locks, artifacts)

    assert not (tmp_path / "out").exists()


def test_builder_rejects_empty_cua_node_source_inventory(tmp_path: Path) -> None:
    inputs = tmp_path / "inputs"
    components, locks, artifacts = _inputs(inputs)
    attestation_path = inputs / "node/share/provenance/SKY_CUA_BUILD_ATTESTATION.json"
    attestation = json.loads(attestation_path.read_text(encoding="utf-8"))
    attestation["source_inventory"] = []
    attestation["source_inventory_sha256"] = hashlib.sha256(canonical_json_bytes([])).hexdigest()
    attestation_path.write_text(json.dumps(attestation), encoding="utf-8")

    with pytest.raises(ValueError, match="source inventory is invalid"):
        _build_inputs(tmp_path / "out", inputs, components, locks, artifacts)

    assert not (tmp_path / "out").exists()


def test_fat_archive_is_deterministic_across_umasks(tmp_path: Path) -> None:
    components, locks, artifacts = _inputs(tmp_path / "inputs")
    browser_hash = sha256_file(tmp_path / "inputs" / "browser" / "browser-client.mjs")

    def under_umask(mask: int, output: Path):
        previous = os.umask(mask)
        try:
            return build_release_set(
                output,
                producer_commit="dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a",
                runtime=_runtime(),
                trusted_browser_client_sha256s=[browser_hash],
                capabilities_supported=["linux-x64-glibc", "sky_cua", "node_repl"],
                capabilities_unsupported=["linux-arm64"],
                browser_api_schema_version=1,
                browser_command_schema_version=1,
                canonical_browser_entrypoint="browser-client.mjs",
                compatibility_browser_projections=["browser-client.mjs"],
                components=components,
                locks=locks,
                artifacts=artifacts,
                source_date_epoch=1_784_500_000,
            )
        finally:
            os.umask(previous)

    first = under_umask(0o022, tmp_path / "first")
    second = under_umask(0o077, tmp_path / "second")
    assert first.fat_archive is not None and second.fat_archive is not None
    assert sha256_file(first.fat_archive) == sha256_file(second.fat_archive)


def test_builder_rejects_escaping_names_and_destinations_before_copy(tmp_path: Path) -> None:
    components, locks, artifacts = _inputs(tmp_path / "inputs")
    browser_hash = sha256_file(tmp_path / "inputs" / "browser" / "browser-client.mjs")

    common = {
        "producer_commit": "dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a",
        "runtime": _runtime(),
        "trusted_browser_client_sha256s": [browser_hash],
        "capabilities_supported": ["linux-x64-glibc", "sky_cua", "node_repl"],
        "capabilities_unsupported": ["linux-arm64"],
        "browser_api_schema_version": 1,
        "browser_command_schema_version": 1,
        "canonical_browser_entrypoint": "browser-client.mjs",
        "compatibility_browser_projections": ["browser-client.mjs"],
        "artifacts": artifacts,
    }
    escaped_component = [
        ComponentSource("../escape", components[0].source),
        *components[1:],
    ]
    with pytest.raises(ValueError, match="component name"):
        build_release_set(
            tmp_path / "component-out",
            components=escaped_component,
            locks=locks,
            **common,
        )
    assert not (tmp_path / "component-out").exists()

    escaped_lock = [FileSource("runtime", locks[0].source, "../escape.json")]
    with pytest.raises(ValueError, match="file destination"):
        build_release_set(
            tmp_path / "file-out",
            components=components,
            locks=escaped_lock,
            **common,
        )
    assert not (tmp_path / "file-out").exists()
