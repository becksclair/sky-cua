#!/usr/bin/env python3
"""Build the complete immutable sky-cua Linux x64 glibc release set."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

from _plugin_bundle import DIST_PLUGIN_ROOT, REPO_ROOT, remove_path
from release_builder import ComponentSource, FileSource, ReleaseBuild, build_release_set
from release_generation import sha256_file

CORE_COMPONENT = "core-linux-x64"
BROWSER_COMPONENT = "browser-js"
CUA_NODE_COMPONENT = "cua-node-linux-x64-glibc"
CODEX_COMPONENT = "codex-compat"
COMPLIANCE_COMPONENT = "compliance"
CANONICAL_BROWSER_ENTRYPOINT = "browser-client.mjs"
CORE_BUILD_INPUT_PROVENANCE = Path("resources/release/CORE_BUILD_INPUTS.json")
CODEX_PROJECTIONS = (
    "openai-bundled/plugins/browser-use/scripts/browser-client.mjs",
    "openai-bundled/plugins/chrome/scripts/browser-client.mjs",
)

RUNTIME_VERSIONS: dict[str, object] = {
    "node": "24.14.0",
    "node_repl": "0.1.0",
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
    "codecs": ["bmp", "jpeg", "png", "webp", "zlib"],
}

SUPPORTED_CAPABILITIES = (
    "browser-extension-bridge",
    "browser-host-provided-iab",
    "browser-persistent-js",
    "computer-use-persistent-js",
    "direct-sky-cua-mcp",
    "linux-x64-glibc",
    "node-repl-mcp",
    "ocr-pdf-image-file-toolbox",
    "openclaw-consumer",
    "opencode-consumer",
    "system-chrome-family-playwright",
)

UNSUPPORTED_CAPABILITIES = (
    "advanced-sky-cua-js",
    "linux-arm64-node-repl",
    "linux-musl-node-repl",
    "macos-node-repl-placeholder-only",
    "npm-publication",
    "windows-node-repl",
)


def _copy_component(source: Path, destination: Path) -> None:
    if source.is_symlink() or not source.is_dir():
        raise FileNotFoundError(f"component input is not a real directory: {source}")
    for path in sorted(source.rglob("*"), key=lambda item: item.as_posix()):
        if path.is_symlink():
            raise ValueError(
                f"component input contains a symlink: {path.relative_to(source).as_posix()}"
            )
        if not path.is_file() and not path.is_dir():
            raise ValueError(
                f"component input contains a special entry: {path.relative_to(source).as_posix()}"
            )
    shutil.copytree(source, destination)


def _build_core_from_commit(producer_commit: str, requested_source: Path) -> Path:
    """Rebuild the core in this invocation from an exact clean producer commit."""
    canonical_source = requested_source.expanduser().absolute()
    expected_source = DIST_PLUGIN_ROOT.absolute()
    if canonical_source != expected_source:
        raise ValueError(
            "complete releases must rebuild the canonical core output in this invocation: "
            f"expected {expected_source}, got {canonical_source}"
        )
    head = _git_value("rev-parse", "HEAD")
    if producer_commit != head:
        raise ValueError(
            f"core producer commit must equal current HEAD: producer={producer_commit}, head={head}"
        )
    status = subprocess.run(
        ["git", "status", "--porcelain=v1", "--untracked-files=all"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    if status:
        raise ValueError("core release build requires a clean producer working tree")
    isolated_root = Path(tempfile.mkdtemp(prefix=".complete-release-core-"))
    isolated_source = isolated_root / expected_source.name
    try:
        subprocess.run(
            [
                "python3",
                "scripts/build_plugin.py",
                "--dist-root",
                str(isolated_source),
                "--release-core-commit",
                producer_commit,
            ],
            cwd=REPO_ROOT,
            check=True,
        )
    except BaseException:
        remove_path(isolated_root)
        raise
    if not isolated_source.is_dir() or isolated_source.is_symlink():
        remove_path(isolated_root)
        raise ValueError("isolated core build did not produce a real output directory")
    return isolated_source


def _verify_core_input_provenance(component: Path, producer_commit: str) -> None:
    provenance_path = component / CORE_BUILD_INPUT_PROVENANCE
    try:
        provenance = json.loads(provenance_path.read_text(encoding="utf-8"))
    except (FileNotFoundError, json.JSONDecodeError) as error:
        raise ValueError("core build input provenance is missing or invalid") from error
    if provenance.get("producer_commit") != producer_commit or provenance.get("source") != {
        "kind": "git-archive",
        "commit": producer_commit,
    }:
        raise ValueError("core build input provenance is not bound to the producer commit")
    if provenance.get("external_inputs") != []:
        raise ValueError("core build contains unattested external inputs")


def _prepare_inputs(
    workspace: Path, *, core_source: Path, cua_node_source: Path
) -> dict[str, Path]:
    remove_path(workspace)
    workspace.mkdir(parents=True)

    core = workspace / CORE_COMPONENT
    _copy_component(core_source, core)
    remove_path(core / "resources" / "plugins" / "openai-bundled")
    remove_path(core / "resources" / "node_repl")
    if (core / "resources" / "plugins" / "openai-bundled").exists() or (
        core / "resources" / "node_repl"
    ).exists():
        raise RuntimeError(
            "legacy copied Browser or node_repl producer path survived core sanitation"
        )

    browser = workspace / BROWSER_COMPONENT
    _copy_component(REPO_ROOT / "packages" / "browser-use" / "build", browser)
    canonical = browser / CANONICAL_BROWSER_ENTRYPOINT
    component_metadata = json.loads(
        (browser / "BROWSER_COMPONENT.json").read_text(encoding="utf-8")
    )
    if component_metadata.get("sha256") != sha256_file(canonical):
        raise ValueError("Browser component metadata does not bind the canonical entrypoint")

    cua_node = workspace / CUA_NODE_COMPONENT
    _copy_component(cua_node_source, cua_node)
    embedded_browser = (
        cua_node
        / "lib"
        / "node_modules"
        / "@heliasar"
        / "browser-use"
        / "build"
        / "browser-client.mjs"
    )
    if embedded_browser.read_bytes() != canonical.read_bytes():
        raise ValueError("cua_node embedded Browser bytes differ from the canonical component")

    compat = workspace / CODEX_COMPONENT
    canonical_bytes = canonical.read_bytes()
    for relative in CODEX_PROJECTIONS:
        destination = compat / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(canonical_bytes)
    (compat / "PROJECTION.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "canonical_component": BROWSER_COMPONENT,
                "canonical_path": CANONICAL_BROWSER_ENTRYPOINT,
                "canonical_sha256": sha256_file(canonical),
                "projections": list(CODEX_PROJECTIONS),
                "implementation": "canonical-first-party-bytes",
            },
            indent=2,
            sort_keys=True,
        )
        + "\n",
        encoding="utf-8",
    )

    compliance = workspace / COMPLIANCE_COMPONENT
    compliance.mkdir()
    shutil.copytree(cua_node / "licenses", compliance / "licenses")
    shutil.copytree(cua_node / "share" / "provenance", compliance / "provenance")
    shutil.copytree(cua_node / "share" / "locks", compliance / "locks")
    shutil.copy2(cua_node / "sbom.cdx.json", compliance / "sbom.cdx.json")
    shutil.copy2(REPO_ROOT / "resources" / "release" / "RELEASE.schema.json", compliance)
    shutil.copy2(browser / "BROWSER_COMPONENT.json", compliance)

    return {
        CORE_COMPONENT: core,
        BROWSER_COMPONENT: browser,
        CUA_NODE_COMPONENT: cua_node,
        CODEX_COMPONENT: compat,
        COMPLIANCE_COMPONENT: compliance,
    }


def _verify_inner_cua_node(component: Path) -> None:
    command = [
        "bun",
        str(REPO_ROOT / "runtime/cua-node/tools/verify-cua-node.ts"),
        f"--root={component}",
        "--target=linux-x64-glibc",
        f"--enforce-lock={component / 'share/locks/runtime-lock.json'}",
        f"--enforce-lock={component / 'share/locks/native-assets.lock.json'}",
        "--json",
    ]
    result = subprocess.run(command, cwd=REPO_ROOT, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        raise ValueError(
            f"cua_node inner verifier failed: {result.stdout[-4000:]}{result.stderr[-4000:]}"
        )
    report = json.loads(result.stdout)
    if report.get("status") != "passed":
        raise ValueError("cua_node inner verifier did not report passed")


def _is_generated_source_path(path: str) -> bool:
    return (
        path.startswith("runtime/cua-node/dist/")
        or path.startswith("packages/browser-use/build/")
        or path.startswith("packages/sky-cua-js/dist/")
    )


def _verify_git_source_inventory(component: Path, producer_commit: str) -> None:
    attestation = json.loads(
        (component / "share/provenance/SKY_CUA_BUILD_ATTESTATION.json").read_text(encoding="utf-8")
    )
    inventory = attestation.get("source_inventory")
    if not isinstance(inventory, list) or not inventory:
        raise ValueError("cua_node source inventory is empty")
    bound_paths: set[str] = set()
    for record in inventory:
        if not isinstance(record, dict) or not isinstance(record.get("path"), str):
            raise ValueError("cua_node source inventory record is invalid")
        path = record["path"]
        if _is_generated_source_path(path):
            continue
        blob = subprocess.run(
            ["git", "show", f"{producer_commit}:{path}"],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
        )
        if blob.returncode != 0:
            raise ValueError(f"producer commit does not contain attested source: {path}")
        if record.get("sha256") != hashlib.sha256(blob.stdout).hexdigest() or record.get(
            "size_bytes"
        ) != len(blob.stdout):
            raise ValueError(f"attested source differs from producer commit: {path}")
        bound_paths.add(path)
    required = {
        "scripts/assemble_cua_node.py",
        "runtime/cua-node/src/cli.ts",
        "packages/browser-use/src/index.ts",
        "packages/sky-cua-js/src/index.ts",
    }
    if not required.issubset(bound_paths):
        raise ValueError(
            f"source inventory omits required producer sources: {sorted(required - bound_paths)}"
        )


def build_complete_release(
    output_root: Path,
    *,
    producer_commit: str,
    source_date_epoch: int,
    core_source: Path,
    cua_node_source: Path,
    include_fat_archive: bool = True,
) -> ReleaseBuild:
    requested_core_source = core_source.expanduser().resolve()
    core_source = _build_core_from_commit(producer_commit, core_source)
    isolated_core_root = core_source.parent if core_source != requested_core_source else None
    output_root.parent.mkdir(parents=True, exist_ok=True)
    workspace = Path(tempfile.mkdtemp(prefix=".complete-release-inputs-", dir=output_root.parent))
    assembly_lock = cua_node_source.parent / ".cua-node-assembly.lock"
    try:
        with assembly_lock.open("a+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
            inputs = _prepare_inputs(
                workspace,
                core_source=core_source,
                cua_node_source=cua_node_source,
            )
        _verify_inner_cua_node(inputs[CUA_NODE_COMPONENT])
        _verify_git_source_inventory(inputs[CUA_NODE_COMPONENT], producer_commit)
        _verify_core_input_provenance(inputs[CORE_COMPONENT], producer_commit)
    except BaseException:
        remove_path(workspace)
        if isolated_core_root is not None:
            remove_path(isolated_core_root)
        raise
    browser_hash = sha256_file(inputs[BROWSER_COMPONENT] / CANONICAL_BROWSER_ENTRYPOINT)
    node = inputs[CUA_NODE_COMPONENT]
    locks = (
        FileSource(
            "cua_node_runtime",
            node / "share/locks/runtime-lock.json",
            "locks/cua-node-runtime.json",
        ),
        FileSource(
            "cua_node_native_assets",
            node / "share/locks/native-assets.lock.json",
            "locks/cua-node-native-assets.json",
        ),
        FileSource(
            "cua_node_bun", REPO_ROOT / "runtime/cua-node/bun.lock", "locks/cua-node.bun.lock"
        ),
        FileSource(
            "cua_node_production_npm",
            REPO_ROOT / "runtime/cua-node/production/package-lock.json",
            "locks/cua-node-production.package-lock.json",
        ),
        FileSource(
            "browser_use_bun",
            REPO_ROOT / "packages/browser-use/bun.lock",
            "locks/browser-use.bun.lock",
        ),
        FileSource(
            "sky_cua_js_bun",
            REPO_ROOT / "packages/sky-cua-js/bun.lock",
            "locks/sky-cua-js.bun.lock",
        ),
    )
    artifacts = (
        FileSource("sbom", node / "sbom.cdx.json", "compliance/sbom.cdx.json"),
        FileSource("provenance", node / "licenses/PROVENANCE.json", "compliance/PROVENANCE.json"),
        FileSource("licenses", node / "licenses/LICENSES.json", "compliance/LICENSES.json"),
        FileSource(
            "build_attestation",
            node / "share/provenance/SKY_CUA_BUILD_ATTESTATION.json",
            "compliance/SKY_CUA_BUILD_ATTESTATION.json",
        ),
        FileSource(
            "browser_component",
            inputs[BROWSER_COMPONENT] / "BROWSER_COMPONENT.json",
            "compliance/BROWSER_COMPONENT.json",
        ),
        FileSource(
            "core_build_inputs",
            inputs[CORE_COMPONENT] / CORE_BUILD_INPUT_PROVENANCE,
            "compliance/CORE_BUILD_INPUTS.json",
        ),
    )
    try:
        return build_release_set(
            output_root,
            producer_commit=producer_commit,
            runtime=RUNTIME_VERSIONS,
            trusted_browser_client_sha256s=[browser_hash],
            capabilities_supported=SUPPORTED_CAPABILITIES,
            capabilities_unsupported=UNSUPPORTED_CAPABILITIES,
            browser_api_schema_version=1,
            browser_command_schema_version=1,
            canonical_browser_entrypoint=CANONICAL_BROWSER_ENTRYPOINT,
            compatibility_browser_projections=CODEX_PROJECTIONS,
            components=(
                ComponentSource(
                    CORE_COMPONENT, inputs[CORE_COMPONENT], profiles=("full", "core-only")
                ),
                ComponentSource(
                    BROWSER_COMPONENT, inputs[BROWSER_COMPONENT], dependencies=(CORE_COMPONENT,)
                ),
                ComponentSource(
                    CUA_NODE_COMPONENT,
                    inputs[CUA_NODE_COMPONENT],
                    dependencies=(CORE_COMPONENT, BROWSER_COMPONENT),
                ),
                ComponentSource(
                    CODEX_COMPONENT, inputs[CODEX_COMPONENT], dependencies=(BROWSER_COMPONENT,)
                ),
                ComponentSource(
                    COMPLIANCE_COMPONENT,
                    inputs[COMPLIANCE_COMPONENT],
                    profiles=("full", "core-only"),
                ),
            ),
            locks=locks,
            artifacts=artifacts,
            source_date_epoch=source_date_epoch,
            include_fat_archive=include_fat_archive,
        )
    finally:
        remove_path(workspace)
        if isolated_core_root is not None:
            remove_path(isolated_core_root)


def _git_value(*arguments: str) -> str:
    return subprocess.run(
        ["git", *arguments],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-root", type=Path, default=REPO_ROOT / "dist" / "complete-release")
    parser.add_argument(
        "--core-source", type=Path, default=REPO_ROOT / "dist" / "plugin" / "sky-cua"
    )
    parser.add_argument(
        "--cua-node-component",
        type=Path,
        default=REPO_ROOT / "out" / "components" / CUA_NODE_COMPONENT,
    )
    parser.add_argument("--producer-commit")
    parser.add_argument("--no-fat-archive", action="store_true")
    args = parser.parse_args(argv)
    producer_commit = args.producer_commit or _git_value("rev-parse", "HEAD")
    source_date_epoch = int(_git_value("show", "-s", "--format=%ct", producer_commit))
    result = build_complete_release(
        args.output_root.resolve(),
        producer_commit=producer_commit,
        source_date_epoch=source_date_epoch,
        core_source=args.core_source.resolve(),
        cua_node_source=args.cua_node_component.resolve(),
        include_fat_archive=not args.no_fat_archive,
    )
    print(
        json.dumps(
            {
                "release_id": result.release.release_id,
                "release_root": str(result.release.root),
                "manifest_sha256": result.release.manifest_sha256,
                "fat_archive": str(result.fat_archive) if result.fat_archive else None,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
