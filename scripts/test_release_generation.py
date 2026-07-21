from __future__ import annotations

import hashlib
import json
import os
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import pytest

from release_generation import (
    CHECKSUMS_FILE,
    COMPAT_VERSION,
    CORE_ONLY_PROFILE,
    FULL_PROFILE,
    INSTALLATION_STATE,
    RELEASE_MANIFEST,
    SCHEMA_VERSION,
    GenerationStore,
    InstallTransactionError,
    ReleaseValidationError,
    canonical_json_bytes,
    canonical_tree_digest,
    component_record,
    content_addressed_release_id,
    main,
    sha256_file,
    verify_release_root,
    write_deterministic_tar_gz,
)


def _write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(canonical_json_bytes(value) + b"\n")


def _release(root: Path, release_id: str, marker: str = "one") -> Path:
    components = root / "components"
    core = components / "core-linux-x64"
    browser = components / "browser-js"
    node = components / "cua-node-linux-x64-glibc"
    compat = components / "codex-compat"
    compliance = components / "compliance"
    documentation = components / "documentation"
    for path in (core, browser, node, compat, compliance, documentation):
        path.mkdir(parents=True)
    (core / "bin").mkdir()
    (core / "bin" / "sky-cua-client").write_text(marker, encoding="utf-8")
    (browser / "browser-client.mjs").write_text("export const browser = 1;\n", encoding="utf-8")
    (node / "bin").mkdir()
    (node / "bin" / "node").write_text("node-24.14.0", encoding="utf-8")
    (compat / "browser-client.mjs").write_bytes((browser / "browser-client.mjs").read_bytes())
    (compliance / "LICENSES.json").write_text("{}\n", encoding="utf-8")
    for name in (
        "api-inventory.json",
        "capability-inventory.json",
        "example-inventory.json",
        "routing-inventory.json",
    ):
        (documentation / name).write_text("{}\n", encoding="utf-8")

    locks = root / "locks"
    locks.mkdir()
    (locks / "runtime-lock.json").write_text('{"node":"24.14.0"}\n', encoding="utf-8")
    artifacts = root / "compliance"
    artifacts.mkdir()
    (artifacts / "sbom.json").write_text('{"bomFormat":"CycloneDX"}\n', encoding="utf-8")
    (artifacts / "provenance.json").write_text('{"builder":"sky-cua"}\n', encoding="utf-8")
    (artifacts / "licenses.json").write_text('{"packages":[]}\n', encoding="utf-8")

    archives = root / "archives"
    for name, source in (
        ("core-linux-x64", core),
        ("browser-js", browser),
        ("cua-node-linux-x64-glibc", node),
        ("codex-compat", compat),
        ("compliance", compliance),
        ("documentation", documentation),
    ):
        write_deterministic_tar_gz(source, archives / f"{name}.tar.gz", arcname=name)

    records = [
        component_record(
            root,
            name="core-linux-x64",
            path="components/core-linux-x64",
            archive="archives/core-linux-x64.tar.gz",
            profiles=(FULL_PROFILE, CORE_ONLY_PROFILE),
        ),
        component_record(
            root,
            name="documentation",
            path="components/documentation",
            archive="archives/documentation.tar.gz",
            dependencies=("browser-js", "cua-node-linux-x64-glibc"),
        ),
        component_record(
            root,
            name="browser-js",
            path="components/browser-js",
            archive="archives/browser-js.tar.gz",
            dependencies=("core-linux-x64",),
        ),
        component_record(
            root,
            name="cua-node-linux-x64-glibc",
            path="components/cua-node-linux-x64-glibc",
            archive="archives/cua-node-linux-x64-glibc.tar.gz",
            dependencies=("core-linux-x64", "browser-js"),
        ),
        component_record(
            root,
            name="codex-compat",
            path="components/codex-compat",
            archive="archives/codex-compat.tar.gz",
            dependencies=("browser-js",),
        ),
        component_record(
            root,
            name="compliance",
            path="components/compliance",
            archive="archives/compliance.tar.gz",
            required=True,
            profiles=(FULL_PROFILE, CORE_ONLY_PROFILE),
        ),
    ]
    browser_hash = sha256_file(browser / "browser-client.mjs")
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "compat_version": COMPAT_VERSION,
        "release_id": release_id,
        "producer": {"commit": "dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a"},
        "target": {
            "os": "linux",
            "arch": "x86_64",
            "libc": "glibc",
            "triple": "x86_64-unknown-linux-gnu",
        },
        "components": records,
        "runtime": {
            "node": "24.14.0",
            "node_repl": "1",
            "browser_use": "1",
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
        },
        "trusted_browser_client_sha256s": [browser_hash],
        "locks": {
            "runtime": {
                "path": "locks/runtime-lock.json",
                "sha256": sha256_file(locks / "runtime-lock.json"),
            }
        },
        "artifacts": {
            "sbom": {
                "path": "compliance/sbom.json",
                "sha256": sha256_file(artifacts / "sbom.json"),
            },
            "provenance": {
                "path": "compliance/provenance.json",
                "sha256": sha256_file(artifacts / "provenance.json"),
            },
            "licenses": {
                "path": "compliance/licenses.json",
                "sha256": sha256_file(artifacts / "licenses.json"),
            },
        },
        "capabilities": {
            "supported": ["linux-x64-glibc", "sky_cua", "node_repl"],
            "unsupported": ["linux-arm64", "linux-musl", "windows-node-repl"],
        },
        "browser_contract": {
            "api_schema_version": 1,
            "command_schema_version": 1,
            "caller_provenance": ["codex_desktop", "direct_mcp", "openclaw", "opencode"],
            "transport_identities": ["extension_native_host", "host_provided_iab"],
            "no_ambiguous_mutation_retry": True,
            "canonical_browser": {
                "component": "browser-js",
                "path": "components/browser-js/browser-client.mjs",
                "sha256": browser_hash,
            },
            "compatibility_projections": [
                {
                    "component": "codex-compat",
                    "path": "components/codex-compat/browser-client.mjs",
                    "sha256": browser_hash,
                }
            ],
        },
        "documentation": {
            "component": "documentation",
            "api_inventory": {
                "path": "components/documentation/api-inventory.json",
                "sha256": sha256_file(documentation / "api-inventory.json"),
            },
            "capability_inventory": {
                "path": "components/documentation/capability-inventory.json",
                "sha256": sha256_file(documentation / "capability-inventory.json"),
            },
            "example_inventory": {
                "path": "components/documentation/example-inventory.json",
                "sha256": sha256_file(documentation / "example-inventory.json"),
            },
            "routing_inventory": {
                "path": "components/documentation/routing-inventory.json",
                "sha256": sha256_file(documentation / "routing-inventory.json"),
            },
        },
    }
    manifest["release_id"] = content_addressed_release_id(manifest)
    _write_json(root / RELEASE_MANIFEST, manifest)
    checksum_paths = sorted(
        path
        for path in root.rglob("*")
        if path.is_file() and not path.is_symlink() and path.name != CHECKSUMS_FILE
    )
    (root / CHECKSUMS_FILE).write_text(
        "".join(
            f"{sha256_file(path)}  {path.relative_to(root).as_posix()}\n" for path in checksum_paths
        ),
        encoding="utf-8",
    )
    return root


def _release_id(root: Path) -> str:
    return json.loads((root / RELEASE_MANIFEST).read_text(encoding="utf-8"))["release_id"]


def _rewrite_manifest(root: Path, manifest: dict[str, object], *, readdress: bool = True) -> None:
    if readdress:
        manifest["release_id"] = content_addressed_release_id(manifest)
    _write_json(root / RELEASE_MANIFEST, manifest)
    checksum_paths = sorted(
        path
        for path in root.rglob("*")
        if path.is_file() and not path.is_symlink() and path.name != CHECKSUMS_FILE
    )
    (root / CHECKSUMS_FILE).write_text(
        "".join(
            f"{sha256_file(path)}  {path.relative_to(root).as_posix()}\n" for path in checksum_paths
        ),
        encoding="utf-8",
    )


def _fail_at(name: str):
    def failpoint(actual: str) -> None:
        if actual == name:
            raise RuntimeError(f"crash:{name}")

    return failpoint


def test_tree_digest_is_deterministic_and_mode_sensitive(tmp_path: Path) -> None:
    first = tmp_path / "first"
    second = tmp_path / "second"
    for root in (first, second):
        (root / "nested").mkdir(parents=True)
    (first / "nested" / "b").write_text("b", encoding="utf-8")
    (first / "a").write_text("a", encoding="utf-8")
    (second / "a").write_text("a", encoding="utf-8")
    (second / "nested" / "b").write_text("b", encoding="utf-8")

    assert canonical_tree_digest(first).sha256 == canonical_tree_digest(second).sha256
    (second / "a").chmod(0o755)
    assert canonical_tree_digest(first).sha256 != canonical_tree_digest(second).sha256


def test_tree_digest_rejects_symlinks(tmp_path: Path) -> None:
    root = tmp_path / "root"
    root.mkdir()
    (root / "file").write_text("safe", encoding="utf-8")
    (root / "link").symlink_to("file")

    with pytest.raises(ReleaseValidationError, match="component symlink is unsupported"):
        canonical_tree_digest(root)


def test_component_archive_is_deterministic_across_source_mtimes(tmp_path: Path) -> None:
    first = tmp_path / "first"
    second = tmp_path / "second"
    first.mkdir()
    second.mkdir()
    (first / "file").write_text("same", encoding="utf-8")
    (second / "file").write_text("same", encoding="utf-8")
    os.utime(first / "file", (1, 1))
    os.utime(second / "file", (2_000_000_000, 2_000_000_000))
    first_archive = tmp_path / "first.tar.gz"
    second_archive = tmp_path / "second.tar.gz"

    write_deterministic_tar_gz(first, first_archive, arcname="component")
    write_deterministic_tar_gz(second, second_archive, arcname="component")
    assert sha256_file(first_archive) == sha256_file(second_archive)


def test_checked_in_schema_matches_runtime_contract() -> None:
    schema_path = (
        Path(__file__).resolve().parents[1] / "resources" / "release" / "RELEASE.schema.json"
    )
    schema = json.loads(schema_path.read_text(encoding="utf-8"))
    assert schema["properties"]["schema_version"] == {"const": SCHEMA_VERSION}
    assert schema["properties"]["compat_version"] == {"const": COMPAT_VERSION}
    assert schema["properties"]["runtime"]["properties"]["node"] == {"const": "24.14.0"}
    assert set(schema["required"]) == {
        "schema_version",
        "compat_version",
        "release_id",
        "producer",
        "target",
        "components",
        "runtime",
        "trusted_browser_client_sha256s",
        "locks",
        "artifacts",
        "capabilities",
        "browser_contract",
    }


def test_manifest_requires_named_artifacts_and_docs_paths_inside_component(
    tmp_path: Path,
) -> None:
    release = _release(tmp_path / "candidate", "release-one")
    manifest = json.loads((release / RELEASE_MANIFEST).read_text(encoding="utf-8"))
    del manifest["artifacts"]["licenses"]
    _rewrite_manifest(release, manifest)
    with pytest.raises(ReleaseValidationError, match="required release artifacts"):
        verify_release_root(release)

    release = _release(tmp_path / "candidate-two", "release-two")
    manifest = json.loads((release / RELEASE_MANIFEST).read_text(encoding="utf-8"))
    manifest["documentation"]["api_inventory"]["path"] = "compliance/sbom.json"
    manifest["documentation"]["api_inventory"]["sha256"] = sha256_file(
        release / "compliance" / "sbom.json"
    )
    _rewrite_manifest(release, manifest)
    with pytest.raises(ReleaseValidationError, match="inside the documentation component"):
        verify_release_root(release)

    release = _release(tmp_path / "candidate-three", "release-three")
    manifest = json.loads((release / RELEASE_MANIFEST).read_text(encoding="utf-8"))
    manifest["capabilities"]["supported"].append("model-facing-documentation")
    docs = next(item for item in manifest["components"] if item["name"] == "documentation")
    docs["required"] = False
    docs["profiles"] = [CORE_ONLY_PROFILE]
    _rewrite_manifest(release, manifest)
    with pytest.raises(ReleaseValidationError, match="documentation in the full profile"):
        verify_release_root(release)


def test_verify_release_detects_component_tamper_before_install(tmp_path: Path) -> None:
    release = _release(tmp_path / "candidate", "release-one")
    verified = verify_release_root(release)
    assert verified.release_id == _release_id(release)
    assert verified.profile == FULL_PROFILE

    (release / "components" / "browser-js" / "browser-client.mjs").write_text(
        "tampered", encoding="utf-8"
    )
    with pytest.raises(ReleaseValidationError, match="browser-js tree hash mismatch"):
        verify_release_root(release)


def test_verify_release_rejects_release_id_not_derived_from_manifest(tmp_path: Path) -> None:
    release = _release(tmp_path / "candidate", "release-one")
    manifest = json.loads((release / RELEASE_MANIFEST).read_text(encoding="utf-8"))
    manifest["release_id"] = "0" * 64
    _rewrite_manifest(release, manifest, readdress=False)

    with pytest.raises(ReleaseValidationError, match="release_id mismatch"):
        verify_release_root(release)


def test_verify_release_rejects_wrong_equal_browser_bindings(tmp_path: Path) -> None:
    release = _release(tmp_path / "candidate", "release-one")
    manifest = json.loads((release / RELEASE_MANIFEST).read_text(encoding="utf-8"))
    wrong = hashlib.sha256(b"wrong-browser").hexdigest()
    manifest["trusted_browser_client_sha256s"] = [wrong]
    manifest["browser_contract"]["canonical_browser"]["sha256"] = wrong
    manifest["browser_contract"]["compatibility_projections"][0]["sha256"] = wrong
    _rewrite_manifest(release, manifest)

    with pytest.raises(ReleaseValidationError, match="canonical_browser hash mismatch"):
        verify_release_root(release)


def test_verify_release_rejects_incomplete_checksum_inventory(tmp_path: Path) -> None:
    release = _release(tmp_path / "candidate", "release-one")
    lines = (release / CHECKSUMS_FILE).read_text(encoding="utf-8").splitlines()
    (release / CHECKSUMS_FILE).write_text("\n".join(lines[1:]) + "\n", encoding="utf-8")

    with pytest.raises(ReleaseValidationError, match="SHA256SUMS path set mismatch"):
        verify_release_root(release)


def test_verify_release_rejects_nested_checksum_name_and_unbound_symlink(tmp_path: Path) -> None:
    release = _release(tmp_path / "candidate", "release-one")
    nested = release / "extra" / CHECKSUMS_FILE
    nested.parent.mkdir()
    nested.write_text("not-root-metadata\n", encoding="utf-8")
    with pytest.raises(ReleaseValidationError, match="SHA256SUMS path set mismatch"):
        verify_release_root(release)

    nested.unlink()
    nested.parent.rmdir()
    (release / "unbound-link").symlink_to(RELEASE_MANIFEST)
    with pytest.raises(ReleaseValidationError, match="release tree symlink is unsupported"):
        verify_release_root(release)


def test_verify_release_detects_missing_and_mixed_components(tmp_path: Path) -> None:
    one = _release(tmp_path / "one", "release-one", marker="one")
    two = _release(tmp_path / "two", "release-two", marker="two")
    mixed = tmp_path / "mixed"
    import shutil

    shutil.copytree(one, mixed)
    shutil.rmtree(mixed / "components" / "core-linux-x64")
    shutil.copytree(two / "components" / "core-linux-x64", mixed / "components" / "core-linux-x64")
    with pytest.raises(ReleaseValidationError, match="core-linux-x64 tree hash mismatch"):
        verify_release_root(mixed)

    shutil.rmtree(mixed / "components" / "browser-js")
    with pytest.raises(ReleaseValidationError, match="component path"):
        verify_release_root(mixed)


def test_verify_release_rejects_archive_that_does_not_match_expanded_tree(tmp_path: Path) -> None:
    release = _release(tmp_path / "candidate", "release-one")
    archive = release / "archives" / "browser-js.tar.gz"
    write_deterministic_tar_gz(
        release / "components" / "core-linux-x64",
        archive,
        arcname="browser-js",
    )
    manifest = json.loads((release / RELEASE_MANIFEST).read_text(encoding="utf-8"))
    browser = next(item for item in manifest["components"] if item["name"] == "browser-js")
    browser["sha256"] = sha256_file(archive)
    browser["size"] = archive.stat().st_size
    _rewrite_manifest(release, manifest)

    with pytest.raises(ReleaseValidationError, match="archive contents do not match"):
        verify_release_root(release)


def test_core_only_profile_verifies_core_dependency_closure(tmp_path: Path) -> None:
    release = _release(tmp_path / "candidate", "release-one")
    verified = verify_release_root(release, profile=CORE_ONLY_PROFILE)
    assert verified.component_names == ("compliance", "core-linux-x64")


def test_core_only_install_physically_omits_optional_components(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    installed = store.install(release, profile=CORE_ONLY_PROFILE)

    assert installed.component_names == ("compliance", "core-linux-x64")
    current = store.root / "current"
    assert (current / "components" / "core-linux-x64").is_dir()
    assert (current / "components" / "compliance").is_dir()
    assert not (current / "components" / "browser-js").exists()
    assert not (current / "components" / "cua-node-linux-x64-glibc").exists()
    assert not (current / "archives" / "browser-js.tar.gz").exists()


def test_existing_release_id_cannot_change_install_profile_in_place(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    store.install(release, profile=FULL_PROFILE)

    with pytest.raises(InstallTransactionError, match="cannot change profile"):
        store.install(release, profile=CORE_ONLY_PROFILE)
    assert (store.root / "current" / "components" / "browser-js").is_dir()


def test_installed_core_only_generation_rejects_readded_optional_component(
    tmp_path: Path,
) -> None:
    import shutil

    store = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    installed = store.install(release, profile=CORE_ONLY_PROFILE)
    generation = store.releases / installed.release_id
    shutil.copytree(
        release / "components" / "browser-js",
        generation / "components" / "browser-js",
    )

    with pytest.raises(ReleaseValidationError, match="omitted component is present"):
        store._verify_installed_generation(installed.release_id)


def test_generation_path_cannot_be_external_symlink(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    release_id = _release_id(release)
    store.initialize()
    (store.releases / release_id).symlink_to(release, target_is_directory=True)

    with pytest.raises(InstallTransactionError, match="must not be a symlink"):
        store.install(release)


def test_install_replaces_a_dangling_current_generation_without_validating_it(
    tmp_path: Path,
) -> None:
    store = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    store.initialize()
    missing_id = "a" * 64
    (store.root / "current").symlink_to(Path("releases") / missing_id)

    installed = store.install(release)

    assert store.current_release_id() == installed.release_id
    assert store.previous_release_id() is None
    assert {path.name for path in store.releases.iterdir()} == {installed.release_id}


def test_install_is_atomic_idempotent_and_prunes_prior_generations(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    one = _release(tmp_path / "one", "release-one", marker="one")
    two = _release(tmp_path / "two", "release-two", marker="two")
    three = _release(tmp_path / "three", "release-three", marker="three")
    one_id, two_id, three_id = _release_id(one), _release_id(two), _release_id(three)

    store.install(one)
    store.install(one)
    assert store.current_release_id() == one_id
    assert store.previous_release_id() is None

    store.install(two)
    assert store.current_release_id() == two_id
    assert store.previous_release_id() is None
    assert {path.name for path in store.releases.iterdir()} == {two_id}

    store.install(three)
    assert store.current_release_id() == three_id
    assert store.previous_release_id() is None
    assert {path.name for path in store.releases.iterdir()} == {three_id}


def test_complete_transaction_can_defer_pruning_until_activation_commits(
    tmp_path: Path,
) -> None:
    store = GenerationStore(tmp_path / "store")
    one = _release(tmp_path / "one", "release-one", marker="one")
    two = _release(tmp_path / "two", "release-two", marker="two")
    three = _release(tmp_path / "three", "release-three", marker="three")
    two_id, three_id = _release_id(two), _release_id(three)
    store.install(one)
    store.install(two)

    with store.transaction() as transaction:
        transaction.install(three, prune=False)
        assert {path.name for path in store.releases.iterdir()} == {
            two_id,
            three_id,
        }
        transaction.prune_generations({two_id, three_id})

    assert {path.name for path in store.releases.iterdir()} == {two_id, three_id}


def test_failed_first_consumer_cutover_can_deactivate_without_deleting_generation(
    tmp_path: Path,
) -> None:
    store = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    installed = store.install(release)

    deactivated = store.deactivate_initial_activation(installed.release_id)

    assert deactivated.release_id == installed.release_id
    assert store.current_release_id() is None
    assert store.previous_release_id() is None
    assert (store.releases / installed.release_id).is_dir()


def test_initial_deactivation_refuses_to_discard_a_prior_generation(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    one = _release(tmp_path / "one", "release-one", marker="one")
    two = _release(tmp_path / "two", "release-two", marker="two")
    store.install(one)
    with store.transaction() as transaction:
        installed = transaction.install(two, prune=False)

    with pytest.raises(InstallTransactionError, match="requires no previous"):
        store.deactivate_initial_activation(installed.release_id)


@pytest.mark.parametrize(
    "phase",
    ["after_staged_journal", "after_generation_commit", "after_current_switch"],
)
def test_journal_recovery_converges_without_mixed_generation(tmp_path: Path, phase: str) -> None:
    store = GenerationStore(tmp_path / "store")
    one = _release(tmp_path / "one", "release-one", marker="one")
    two = _release(tmp_path / "two", "release-two", marker="two")
    two_id = _release_id(two)
    store.install(one)

    with pytest.raises(RuntimeError, match=f"crash:{phase}"):
        store.install(two, failpoint=_fail_at(phase))

    recovered = store.recover()
    assert recovered is not None
    assert recovered.release_id == two_id
    assert store.current_release_id() == two_id
    assert store.previous_release_id() is None
    assert {path.name for path in store.releases.iterdir()} == {two_id}
    verify_release_root(store.releases / two_id)


def test_idempotent_reinstall_recovery_clears_stale_prior(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    one = _release(tmp_path / "one", "release-one", marker="one")
    two = _release(tmp_path / "two", "release-two", marker="two")
    two_id = _release_id(two)
    store.install(one)
    store.install(two)

    with pytest.raises(RuntimeError, match="crash:after_staged_journal"):
        store.install(two, failpoint=_fail_at("after_staged_journal"))
    recovered = store.recover()

    assert recovered is not None and recovered.release_id == two_id
    assert store.current_release_id() == two_id
    assert store.previous_release_id() is None
    assert {path.name for path in store.releases.iterdir()} == {two_id}


def test_installation_state_is_staged_before_current_switch_and_never_rewritten(
    tmp_path: Path,
) -> None:
    store = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    release_id = _release_id(release)
    with pytest.raises(RuntimeError, match="crash:after_current_switch"):
        store.install(release, failpoint=_fail_at("after_current_switch"))

    state = store.releases / release_id / "INSTALLATION.json"
    before = state.read_bytes()
    before_mtime = state.stat().st_mtime_ns
    store.recover()
    assert state.read_bytes() == before
    assert state.stat().st_mtime_ns == before_mtime


def test_generation_transaction_lock_serializes_independent_store_instances(
    tmp_path: Path,
) -> None:
    first = GenerationStore(tmp_path / "store")
    second = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    started = threading.Event()

    def install() -> str:
        started.set()
        return second.install(release).release_id

    with ThreadPoolExecutor(max_workers=1) as executor:
        with first.transaction() as transaction:
            assert transaction.install(release).release_id == _release_id(release)
            future = executor.submit(install)
            assert started.wait(timeout=1)
            assert not future.done()
        assert future.result(timeout=2) == _release_id(release)


@pytest.mark.parametrize(
    ("field", "value", "message"),
    [
        ("schema_version", 2, "schema mismatch"),
        ("components", ["core-linux-x64"], "component set mismatch"),
    ],
)
def test_installed_generation_rejects_false_installation_state_claims(
    tmp_path: Path, field: str, value: object, message: str
) -> None:
    store = GenerationStore(tmp_path / "store")
    release = _release(tmp_path / "candidate", "release-one")
    installed = store.install(release)
    state_path = store.releases / installed.release_id / INSTALLATION_STATE
    state = json.loads(state_path.read_text(encoding="utf-8"))
    state[field] = value
    _write_json(state_path, state)

    with pytest.raises(InstallTransactionError, match=message):
        store._verify_installed_generation(installed.release_id)


def test_rollback_swaps_two_verified_generations(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    one = _release(tmp_path / "one", "release-one", marker="one")
    two = _release(tmp_path / "two", "release-two", marker="two")
    one_id, two_id = _release_id(one), _release_id(two)
    store.install(one)
    with store.transaction() as transaction:
        transaction.install(two, prune=False)

    rolled_back = store.rollback()
    assert rolled_back.release_id == one_id
    assert store.current_release_id() == one_id
    assert store.previous_release_id() == two_id


@pytest.mark.parametrize(
    "phase",
    ["rollback_after_prepared_journal", "rollback_after_current_switch"],
)
def test_rollback_journal_recovery_converges_after_crash(tmp_path: Path, phase: str) -> None:
    store = GenerationStore(tmp_path / "store")
    one = _release(tmp_path / "one", "release-one", marker="one")
    two = _release(tmp_path / "two", "release-two", marker="two")
    one_id, two_id = _release_id(one), _release_id(two)
    store.install(one)
    with store.transaction() as transaction:
        transaction.install(two, prune=False)

    with pytest.raises(RuntimeError, match=f"crash:{phase}"):
        store.rollback(failpoint=_fail_at(phase))

    recovered = store.recover()
    assert recovered is not None and recovered.release_id == one_id
    assert store.current_release_id() == one_id
    assert store.previous_release_id() == two_id
    assert not store.journal.exists()


def test_wrong_expected_manifest_hash_rejects_before_store_mutation(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    candidate = _release(tmp_path / "candidate", "release-one")
    wrong = hashlib.sha256(b"wrong").hexdigest()

    with pytest.raises(ReleaseValidationError, match="manifest hash mismatch"):
        store.install(candidate, expected_manifest_sha256=wrong)
    assert store.current_release_id() is None
    assert list(store.releases.iterdir()) == []


def test_cli_verify_and_install_report_bound_release(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    candidate = _release(tmp_path / "candidate", "release-one")
    manifest_hash = sha256_file(candidate / RELEASE_MANIFEST)
    store = tmp_path / "store"

    assert main(["verify", str(candidate), "--manifest-sha256", manifest_hash]) == 0
    verified_output = json.loads(capsys.readouterr().out)
    assert verified_output["release_id"] == _release_id(candidate)
    assert verified_output["manifest_sha256"] == manifest_hash

    assert main(["install", str(candidate), "--store-root", str(store)]) == 1
    guarded_output = json.loads(capsys.readouterr().out)
    assert "install.py install" in guarded_output["error"]
    assert not (store / "current").exists()

    assert (
        main(
            [
                "install",
                str(candidate),
                "--store-root",
                str(store),
                "--manifest-sha256",
                manifest_hash,
                "--internal-generation-only",
            ]
        )
        == 0
    )
    installed_output = json.loads(capsys.readouterr().out)
    assert installed_output["release_id"] == _release_id(candidate)
    assert (store / "current" / RELEASE_MANIFEST).exists()


def test_cli_wrong_hash_fails_without_promotion(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    candidate = _release(tmp_path / "candidate", "release-one")
    store = tmp_path / "store"

    assert (
        main(
            [
                "install",
                str(candidate),
                "--store-root",
                str(store),
                "--manifest-sha256",
                "0" * 64,
            ]
        )
        == 1
    )
    output = json.loads(capsys.readouterr().out)
    assert output["status"] == "error"
    assert not (store / "current").exists()


def test_recovery_refuses_missing_target_and_staging(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    store.initialize()
    (store.journal).write_text(
        json.dumps(
            {
                "schema_version": 1,
                "operation": "install",
                "phase": "staged",
                "target_release_id": "1" * 64,
                "target_manifest_sha256": "0" * 64,
                "profile": FULL_PROFILE,
                "previous_release_id": None,
                "staging_name": f".{('1' * 64)}.staging-1",
            }
        ),
        encoding="utf-8",
    )

    with pytest.raises(InstallTransactionError, match="neither staging nor generation"):
        store.recover()


@pytest.mark.parametrize(
    ("field", "value", "message"),
    [
        ("target_release_id", "../../escape", "target_release_id"),
        ("previous_release_id", "/tmp/escape", "previous_release_id"),
        ("staging_name", "../../escape", "staging_name"),
        ("profile", "unknown", "unsupported journal profile"),
        ("phase", "unknown", "unsupported install journal phase"),
    ],
)
def test_recovery_rejects_untrusted_journal_fields(
    tmp_path: Path, field: str, value: str, message: str
) -> None:
    store = GenerationStore(tmp_path / "store")
    store.initialize()
    target = "1" * 64
    journal: dict[str, object] = {
        "schema_version": 1,
        "operation": "install",
        "phase": "staged",
        "target_release_id": target,
        "target_manifest_sha256": "0" * 64,
        "profile": FULL_PROFILE,
        "previous_release_id": None,
        "staging_name": f".{target}.staging-1",
    }
    journal[field] = value
    _write_json(store.journal, journal)

    with pytest.raises((InstallTransactionError, ReleaseValidationError), match=message):
        store.recover()


def test_current_link_cannot_escape_store(tmp_path: Path) -> None:
    store = GenerationStore(tmp_path / "store")
    store.initialize()
    os.symlink("/tmp", store.root / "current")
    with pytest.raises(InstallTransactionError, match="outside"):
        store.current_release_id()
