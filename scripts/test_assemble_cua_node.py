from __future__ import annotations

import hashlib
import io
import json
import os
import tarfile
import threading
from pathlib import Path

import pytest

from assemble_cua_node import (
    TARGET,
    AssemblyError,
    PreparedPackageImport,
    _compose,
    _extract_package,
    _files,
    _generate_compliance,
    _import_prepared_packages,
    _import_seed,
    _normalize_checkout_text_paths,
    _prepared_package_lock_record,
    _producer_commit,
    _recover_publication,
    _remove_wrong_platform_content,
    _resolve_seed,
    _tree_hash,
    assemble,
    main,
)


def test_normalizes_checkout_paths_in_text_but_preserves_native_debug_bytes(
    tmp_path: Path,
) -> None:
    text = tmp_path / "provenance.md"
    native = tmp_path / "addon.node"
    checkout = b"/home/builder/projects/codex-desktop/vendor/source/file.cc"
    text.write_bytes(b"source=" + checkout + b"\n")
    native.write_bytes(b"\x00" + checkout + b"\x00")

    _normalize_checkout_text_paths(tmp_path)

    assert (
        text.read_bytes() == b"source=${SKY_CUA_SOURCE_ROOT}/codex-desktop/vendor/source/file.cc\n"
    )
    assert native.read_bytes() == b"\x00" + checkout + b"\x00"


def _write_package_tarball(path: Path, name: str, member_type: bytes) -> None:
    info = tarfile.TarInfo(name)
    info.type = member_type
    if member_type == tarfile.REGTYPE:
        payload = b"unsafe"
        info.size = len(payload)
        source: io.BytesIO | None = io.BytesIO(payload)
    else:
        source = None
        if member_type == tarfile.SYMTYPE:
            info.linkname = "../../outside"
    with tarfile.open(path, "w:gz") as archive:
        archive.addfile(info, source)


@pytest.mark.parametrize("entry_kind", ["symlink", "fifo"])
def test_files_rejects_symlinks_and_special_entries(tmp_path: Path, entry_kind: str) -> None:
    root = tmp_path / "tree"
    root.mkdir()
    if entry_kind == "symlink":
        target = root / "target"
        target.write_text("data", encoding="utf-8")
        (root / "link").symlink_to(target)
    else:
        os.mkfifo(root / "pipe")

    with pytest.raises(AssemblyError, match=r"symlink|unsupported entry"):
        _files(root)


@pytest.mark.parametrize(
    ("member_name", "member_type"),
    [
        ("package/../../escaped", tarfile.REGTYPE),
        ("package/link", tarfile.SYMTYPE),
        ("package/device", tarfile.CHRTYPE),
    ],
)
def test_extract_package_rejects_escaping_links_and_special_members(
    tmp_path: Path,
    member_name: str,
    member_type: bytes,
) -> None:
    tarball = tmp_path / "package.tgz"
    destination = tmp_path / "destination"
    outside = tmp_path / "escaped"
    outside.write_text("preserve", encoding="utf-8")
    _write_package_tarball(tarball, member_name, member_type)

    with pytest.raises(AssemblyError, match="unsafe package member"):
        _extract_package(tarball, destination)

    assert outside.read_text(encoding="utf-8") == "preserve"


def test_platform_cleanup_preserves_portable_win32_module_file(tmp_path: Path) -> None:
    modules = tmp_path / "node_modules"
    package = modules / "portable-package"
    package.mkdir(parents=True)
    portable_module = package / "win32.js"
    portable_module.write_text("export const separator = '\\\\';\n", encoding="utf-8")
    (package / "package.json").write_text(
        json.dumps(
            {
                "files": ["win32.js", "fsevents"],
                "optionalDependencies": {
                    "@img/sharp-linux-x64": "0.34.5",
                    "@img/sharp-win32-x64": "0.34.5",
                    "fsevents": "2.3.3",
                },
                "scripts": {"install": "node install.js", "verify": "node win32.js"},
            }
        ),
        encoding="utf-8",
    )

    _remove_wrong_platform_content(modules)

    sanitized = json.loads((package / "package.json").read_text(encoding="utf-8"))
    assert portable_module.is_file()
    assert sanitized["files"] == ["win32.js"]
    assert sanitized["optionalDependencies"] == {"@img/sharp-linux-x64": "0.34.5"}
    assert sanitized["scripts"] == {"verify": "node win32.js"}


def _write_prepared_package(
    repository: Path,
    *,
    name: str = "example",
    version: str = "1.2.3",
    license_expression: str = "MIT",
) -> Path:
    package = repository / "runtime" / "cua-node" / "node_modules" / name
    package.mkdir(parents=True)
    (package / "package.json").write_text(
        json.dumps({"name": name, "version": version, "license": license_expression}),
        encoding="utf-8",
    )
    (package / "index.js").write_text("export const prepared = true;\n", encoding="utf-8")
    return package


def _lock_prepared_package(package: Path, *, name: str = "example") -> PreparedPackageImport:
    digest, size_bytes, _ = _tree_hash(package)
    return PreparedPackageImport(name, "1.2.3", "MIT", digest, size_bytes)


def test_import_prepared_packages_copies_exact_allowlisted_tree(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repository = tmp_path / "repo"
    package = _write_prepared_package(repository)
    locked = _lock_prepared_package(package)
    monkeypatch.setattr("assemble_cua_node.REPO_ROOT", repository)
    monkeypatch.setattr("assemble_cua_node.PREPARED_PACKAGE_IMPORTS", (locked,))
    staging = tmp_path / "staging"

    _import_prepared_packages(staging)

    destination = staging / "lib" / "node_modules" / "example"
    assert _tree_hash(destination) == _tree_hash(package)
    assert not (staging / "lib" / "node_modules" / "not-allowlisted").exists()


def test_import_prepared_packages_reports_frozen_install_for_missing_source(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr("assemble_cua_node.REPO_ROOT", tmp_path / "repo")
    monkeypatch.setattr(
        "assemble_cua_node.PREPARED_PACKAGE_IMPORTS",
        (PreparedPackageImport("missing", "1.0.0", "MIT", "0" * 64, 0),),
    )

    with pytest.raises(
        AssemblyError,
        match=r"bun install --frozen-lockfile --cwd=runtime/cua-node",
    ):
        _import_prepared_packages(tmp_path / "staging")


@pytest.mark.parametrize(
    ("name", "version"),
    [("wrong-name", "1.2.3"), ("example", "9.9.9")],
)
def test_import_prepared_packages_rejects_wrong_identity_or_version(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    name: str,
    version: str,
) -> None:
    repository = tmp_path / "repo"
    package = _write_prepared_package(repository)
    locked = _lock_prepared_package(package)
    (package / "package.json").write_text(
        json.dumps({"name": name, "version": version, "license": "MIT"}), encoding="utf-8"
    )
    monkeypatch.setattr("assemble_cua_node.REPO_ROOT", repository)
    monkeypatch.setattr("assemble_cua_node.PREPARED_PACKAGE_IMPORTS", (locked,))

    with pytest.raises(AssemblyError, match="prepared package identity mismatch"):
        _import_prepared_packages(tmp_path / "staging")


def test_import_prepared_packages_rejects_altered_tree(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repository = tmp_path / "repo"
    package = _write_prepared_package(repository)
    locked = _lock_prepared_package(package)
    (package / "index.js").write_text("altered\n", encoding="utf-8")
    monkeypatch.setattr("assemble_cua_node.REPO_ROOT", repository)
    monkeypatch.setattr("assemble_cua_node.PREPARED_PACKAGE_IMPORTS", (locked,))

    with pytest.raises(AssemblyError, match="prepared package tree mismatch"):
        _import_prepared_packages(tmp_path / "staging")


def test_import_prepared_packages_rejects_symlinks(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    repository = tmp_path / "repo"
    package = _write_prepared_package(repository)
    locked = _lock_prepared_package(package)
    (package / "link").symlink_to(package / "index.js")
    monkeypatch.setattr("assemble_cua_node.REPO_ROOT", repository)
    monkeypatch.setattr("assemble_cua_node.PREPARED_PACKAGE_IMPORTS", (locked,))

    with pytest.raises(AssemblyError, match="tree contains symlink"):
        _import_prepared_packages(tmp_path / "staging")


def test_prepared_package_lock_record_uses_official_npm_registry_source(tmp_path: Path) -> None:
    package = tmp_path / "lib" / "node_modules" / "acorn-walk"
    package.mkdir(parents=True)
    (package / "package.json").write_text(
        json.dumps({"name": "acorn-walk", "version": "8.3.5", "license": "MIT"}),
        encoding="utf-8",
    )
    locked = PreparedPackageImport(
        "acorn-walk", "8.3.5", "MIT", "0" * 64, 0, "sha512-registry-integrity"
    )

    record = _prepared_package_lock_record(tmp_path, locked)

    assert record["source"] == {
        "type": "npm",
        "uri": "https://registry.npmjs.org/acorn-walk/-/acorn-walk-8.3.5.tgz",
        "provenance": (
            "official npm registry package prepared by the frozen runtime/cua-node "
            "Bun lockfile and copied from runtime/cua-node/node_modules"
        ),
        "resolved": True,
    }
    assert "migration" not in record["source"]["provenance"]
    assert record["integrity"] == "sha512-registry-integrity"
    assert record["license"]["notice_files"] == ["lib/node_modules/acorn-walk/LICENSE"]


def test_resolve_seed_rejects_missing_cache(tmp_path: Path) -> None:
    with pytest.raises(AssemblyError, match="no sky-cua cua_node seed cache"):
        _resolve_seed(tmp_path / "missing-cache", None)


def test_resolve_seed_rejects_tampered_cached_bytes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = tmp_path / "seed"
    source.mkdir()
    (source / "manifest.json").write_text(
        json.dumps({"target": TARGET, "node_version": "24.14.0"}), encoding="utf-8"
    )
    (source / "payload").write_bytes(b"verified")
    digest, size_bytes, file_count = _tree_hash(source)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_SHA256", digest)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_SIZE_BYTES", size_bytes)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_FILE_COUNT", file_count)
    cache = tmp_path / "cache"

    imported = _import_seed(cache, source)
    (imported / "payload").write_bytes(b"tampered")

    with pytest.raises(AssemblyError, match="cached seed inventory or content hash mismatch"):
        _resolve_seed(cache, None)


def _write_test_seed(source: Path) -> tuple[str, int, int]:
    source.mkdir()
    (source / "manifest.json").write_text(
        json.dumps({"target": TARGET, "node_version": "24.14.0"}), encoding="utf-8"
    )
    (source / "payload").write_bytes(b"verified")
    return _tree_hash(source)


@pytest.mark.parametrize(
    "forgery",
    [
        "pointer_digest",
        "pointer_path",
        "marker_digest",
        "content_digest",
        "marker_size",
        "marker_count",
    ],
)
def test_cache_only_seed_requires_independent_locked_identity(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, forgery: str
) -> None:
    source = tmp_path / "seed"
    digest, size_bytes, file_count = _write_test_seed(source)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_SHA256", digest)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_SIZE_BYTES", size_bytes)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_FILE_COUNT", file_count)
    cache = tmp_path / "cache"
    imported = _import_seed(cache, source)
    pointer_path = cache / "current-seed.json"
    marker_path = imported / "SKY_CUA_MIGRATION_INPUT.json"

    if forgery.startswith("pointer_"):
        pointer = json.loads(pointer_path.read_text(encoding="utf-8"))
        if forgery == "pointer_digest":
            pointer["tree_sha256"] = "0" * 64
            pointer["path"] = f"seeds/{'0' * 64}"
        else:
            pointer["path"] = f"seeds/{digest}/../forged"
        pointer_path.write_text(json.dumps(pointer), encoding="utf-8")
    elif forgery == "content_digest":
        (imported / "payload").write_bytes(b"tampered")
    else:
        marker = json.loads(marker_path.read_text(encoding="utf-8"))
        if forgery == "marker_digest":
            marker["source_tree_sha256"] = "0" * 64
        elif forgery == "marker_size":
            marker["source_size_bytes"] = size_bytes + 1
        else:
            marker["source_file_count"] = file_count + 1
        marker_path.write_text(json.dumps(marker), encoding="utf-8")

    with pytest.raises(
        AssemblyError,
        match=r"cached seed (?:pointer does not match|inventory or content hash mismatch)",
    ):
        _resolve_seed(cache, None)


def test_forged_cache_is_rejected_before_composition_or_native_audit(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    source = tmp_path / "seed"
    digest, size_bytes, file_count = _write_test_seed(source)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_SHA256", digest)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_SIZE_BYTES", size_bytes)
    monkeypatch.setattr("assemble_cua_node.MIGRATION_SEED_FILE_COUNT", file_count)
    cache = tmp_path / "cache"
    imported = _import_seed(cache, source)
    (imported / "payload").write_bytes(b"tampered")
    compose_called = False
    native_audit_called = False

    def compose(*_args: object, **_kwargs: object) -> tuple[Path, Path]:
        nonlocal compose_called
        compose_called = True
        raise AssertionError("composition must not inspect an unauthenticated seed")

    def native_audit(*_args: object, **_kwargs: object) -> tuple[dict[str, object], list[object]]:
        nonlocal native_audit_called
        native_audit_called = True
        raise AssertionError("native audit must not inspect an unauthenticated seed")

    monkeypatch.setattr("assemble_cua_node._assert_producer_sources", lambda *_a, **_kw: False)
    monkeypatch.setattr("assemble_cua_node._recover_publication", lambda *_a: None)
    monkeypatch.setattr("assemble_cua_node._rebuild_first_party_outputs", lambda: {"outputs": {}})
    monkeypatch.setattr("assemble_cua_node._compose", compose)
    monkeypatch.setattr("assemble_cua_node._native_audit", native_audit)

    with pytest.raises(AssemblyError, match="cached seed inventory or content hash mismatch"):
        assemble(
            cache=cache,
            seed_argument=None,
            output=tmp_path / "output",
            producer_commit="d" * 40,
            check=False,
            allow_development_dirty=True,
        )

    assert compose_called is False
    assert native_audit_called is False


def test_import_seed_rejects_an_unpinned_source(tmp_path: Path) -> None:
    source = tmp_path / "seed"
    source.mkdir()
    (source / "manifest.json").write_text(
        json.dumps({"target": TARGET, "node_version": "24.14.0"}), encoding="utf-8"
    )
    (source / "payload").write_bytes(b"not-the-locked-migration-input")

    with pytest.raises(AssemblyError, match="does not match the locked Codex Desktop input"):
        _import_seed(tmp_path / "cache", source)


def test_producer_commit_requires_full_lowercase_sha() -> None:
    commit = "dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a"
    assert _producer_commit(commit) == commit
    for invalid in (commit[:-1], commit.upper(), "not-a-commit"):
        with pytest.raises(AssemblyError, match="full 40-character Git commit"):
            _producer_commit(invalid)


def test_cli_rejects_non_locked_target() -> None:
    assert TARGET == "linux-x64-glibc"
    with pytest.raises(SystemExit, match="unsupported target"):
        main(["--target", "linux-arm64"])


def _publication_names(output: Path) -> tuple[str, str]:
    return (
        f".{output.name}.staging-{'a' * 32}",
        f".{output.name}.backup-{'b' * 32}",
    )


def _write_publication_journal(
    journal: Path,
    output: Path,
    *,
    phase: str,
    staging: str,
    backup: str,
) -> None:
    journal.write_text(
        json.dumps(
            {
                "schema_version": 1,
                "phase": phase,
                "output": output.name,
                "staging": staging,
                "backup": backup,
            }
        ),
        encoding="utf-8",
    )


def test_recover_promoting_restores_backup_and_removes_staging(tmp_path: Path) -> None:
    output = tmp_path / "cua-node"
    journal = tmp_path / "journal.json"
    staging_name, backup_name = _publication_names(output)
    staging = tmp_path / staging_name
    backup = tmp_path / backup_name
    output.mkdir()
    (output / "value").write_text("new", encoding="utf-8")
    staging.mkdir()
    (staging / "value").write_text("staged", encoding="utf-8")
    backup.mkdir()
    (backup / "value").write_text("old", encoding="utf-8")
    _write_publication_journal(
        journal,
        output,
        phase="promoting",
        staging=staging_name,
        backup=backup_name,
    )

    _recover_publication(output, journal)

    assert (output / "value").read_text(encoding="utf-8") == "old"
    assert not staging.exists()
    assert not backup.exists()
    assert not journal.exists()


def test_recover_committed_keeps_output_and_cleans_temporary_trees(tmp_path: Path) -> None:
    output = tmp_path / "cua-node"
    journal = tmp_path / "journal.json"
    staging_name, backup_name = _publication_names(output)
    staging = tmp_path / staging_name
    backup = tmp_path / backup_name
    output.mkdir()
    (output / "value").write_text("committed", encoding="utf-8")
    staging.mkdir()
    backup.mkdir()
    _write_publication_journal(
        journal,
        output,
        phase="committed",
        staging=staging_name,
        backup=backup_name,
    )

    _recover_publication(output, journal)

    assert (output / "value").read_text(encoding="utf-8") == "committed"
    assert not staging.exists()
    assert not backup.exists()
    assert not journal.exists()


def test_recover_promoting_before_backup_keeps_old_output(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    output = tmp_path / "cua-node"
    journal = tmp_path / "journal.json"
    staging_name, backup_name = _publication_names(output)
    staging = tmp_path / staging_name
    output.mkdir()
    (output / "value").write_text("old", encoding="utf-8")
    staging.mkdir()
    (staging / "value").write_text("candidate", encoding="utf-8")
    _write_publication_journal(
        journal,
        output,
        phase="promoting",
        staging=staging_name,
        backup=backup_name,
    )
    synced: list[Path] = []
    monkeypatch.setattr("assemble_cua_node._fsync_directory", synced.append)

    _recover_publication(output, journal)

    assert (output / "value").read_text(encoding="utf-8") == "old"
    assert not staging.exists()
    assert not journal.exists()
    assert synced == [tmp_path]


def test_recover_promoting_first_install_removes_uncommitted_output(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    output = tmp_path / "cua-node"
    journal = tmp_path / "journal.json"
    staging_name, backup_name = _publication_names(output)
    output.mkdir()
    (output / "value").write_text("uncommitted", encoding="utf-8")
    _write_publication_journal(
        journal,
        output,
        phase="promoting",
        staging=staging_name,
        backup=backup_name,
    )
    synced: list[Path] = []
    monkeypatch.setattr("assemble_cua_node._fsync_directory", synced.append)

    _recover_publication(output, journal)

    assert not output.exists()
    assert not journal.exists()
    assert synced == [tmp_path]


def test_recovery_fsyncs_publication_directory_after_journal_removal(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    output = tmp_path / "cua-node"
    output.mkdir()
    journal = tmp_path / "journal.json"
    staging_name, backup_name = _publication_names(output)
    _write_publication_journal(
        journal,
        output,
        phase="committed",
        staging=staging_name,
        backup=backup_name,
    )
    synced: list[Path] = []
    monkeypatch.setattr("assemble_cua_node._fsync_directory", synced.append)

    _recover_publication(output, journal)

    assert synced == [tmp_path]
    assert not journal.exists()


def test_compliance_binds_publisher_declared_license_when_text_is_absent(
    tmp_path: Path,
) -> None:
    root = tmp_path / "component"
    package = root / "lib" / "node_modules" / "example"
    package.mkdir(parents=True)
    (package / "package.json").write_text(
        json.dumps({"name": "example", "version": "1.0.0", "license": "MIT"}),
        encoding="utf-8",
    )
    for relative in (
        "bin/node",
        "lib/node_repl/cli.js",
        "share/tessdata/eng.traineddata",
        "share/tessdata/osd.traineddata",
    ):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(relative, encoding="utf-8")
    (root / "licenses").mkdir()
    (root / "licenses" / "Node.js-LICENSE.txt").write_text(
        "Node.js license notices", encoding="utf-8"
    )
    for directory in ("share/pdfjs/cmaps", "share/pdfjs/standard_fonts"):
        path = root / directory
        path.mkdir(parents=True)
        (path / "asset").write_text(directory, encoding="utf-8")
    (root / "share" / "pdfjs" / "cmaps" / "LICENSE").write_text(
        "PDF.js cmap license", encoding="utf-8"
    )
    (root / "share" / "pdfjs" / "standard_fonts" / "LICENSE_FOXIT").write_text(
        "PDF.js Foxit font license", encoding="utf-8"
    )
    (root / "share" / "pdfjs" / "standard_fonts" / "LICENSE_LIBERATION").write_text(
        "PDF.js Liberation font license", encoding="utf-8"
    )

    _generate_compliance(
        root,
        producer_commit="dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a",
        browser_hash="2" * 64,
        migration_input={"source_tree_sha256": "1" * 64},
    )

    inventory = json.loads((root / "licenses" / "LICENSES.json").read_text(encoding="utf-8"))
    assert len(inventory["packages"]) == 1
    record = root / inventory["packages"][0]["license_files"][0]
    assert record.name == "SPDX-MIT.txt"
    declared = root / inventory["packages"][0]["license_files"][1]
    assert json.loads(declared.read_text(encoding="utf-8"))["license_expression"] == "MIT"
    sbom = json.loads((root / "sbom.cdx.json").read_text(encoding="utf-8"))
    assert {component["name"] for component in sbom["components"]} >= {
        "Node.js",
        "PDF.js cmaps",
        "PDF.js standard fonts",
    }
    pdf_components = {
        component["name"]: component
        for component in sbom["components"]
        if component["name"].startswith("PDF.js")
    }
    assert pdf_components["PDF.js cmaps"]["licenses"] == [
        {"license": {"expression": "BSD-3-Clause"}}
    ]
    assert pdf_components["PDF.js standard fonts"]["licenses"] == [
        {"license": {"expression": "BSD-3-Clause AND OFL-1.1"}}
    ]
    package_record = inventory["packages"][0]
    for relative, expected in package_record["license_file_sha256s"].items():
        assert hashlib.sha256((root / relative).read_bytes()).hexdigest() == expected
    sbom_record = next(
        component for component in sbom["components"] if component["name"] == "example"
    )
    assert sbom_record["hashes"] == [
        {"alg": "SHA-256", "content": package_record["package_tree_sha256"]}
    ]
    assert package_record["package_tree_sha256"] == _tree_hash(package)[0]


@pytest.mark.parametrize(
    ("phase", "escaped_field"), [("promoting", "staging"), ("committed", "backup")]
)
def test_recover_publication_rejects_paths_outside_output_directory(
    tmp_path: Path,
    phase: str,
    escaped_field: str,
) -> None:
    parent = tmp_path / "publication"
    parent.mkdir()
    output = parent / "cua-node"
    output.mkdir()
    journal = parent / "journal.json"
    outside = tmp_path / "outside"
    outside.mkdir()
    marker = outside / "marker"
    marker.write_text("preserve", encoding="utf-8")
    staging_name, backup_name = _publication_names(output)
    paths = {"staging": staging_name, "backup": backup_name}
    paths[escaped_field] = "../outside"
    _write_publication_journal(journal, output, phase=phase, **paths)

    with pytest.raises(AssemblyError, match="journal paths are invalid"):
        _recover_publication(output, journal)

    assert marker.read_text(encoding="utf-8") == "preserve"
    assert output.is_dir()
    assert journal.is_file()


def test_platform_cleanup_physically_removes_wrong_platform_packages(tmp_path: Path) -> None:
    modules = tmp_path / "node_modules"
    packages = {
        "@img/sharp-linux-x64": True,
        "@img/sharp-win32-x64": False,
        "@img/sharp-linux-arm64": False,
        "@napi-rs/canvas": True,
        "@napi-rs/canvas-linux-x64-gnu": True,
        "@napi-rs/canvas-darwin-x64": False,
        "fsevents": False,
    }
    for name in packages:
        package = modules.joinpath(*name.split("/"))
        package.mkdir(parents=True)
        (package / "package.json").write_text(
            json.dumps({"name": name, "version": "1.0.0"}), encoding="utf-8"
        )

    _remove_wrong_platform_content(modules)

    for name, retained in packages.items():
        assert modules.joinpath(*name.split("/")).exists() is retained


def test_assemble_rejects_lexical_output_root_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    target = tmp_path / "real-output"
    target.mkdir()
    output = tmp_path / "output-link"
    output.symlink_to(target, target_is_directory=True)
    monkeypatch.setattr(
        "assemble_cua_node._assert_producer_sources", lambda *_args, **_kwargs: False
    )

    with pytest.raises(AssemblyError, match="output root must not be a symlink"):
        assemble(
            cache=tmp_path / "cache",
            seed_argument=None,
            output=output,
            producer_commit="d" * 40,
            check=False,
            allow_development_dirty=True,
        )


def _install_fast_assembly_fakes(
    monkeypatch: pytest.MonkeyPatch,
    *,
    candidate_payload: bytes = b"candidate",
) -> None:
    monkeypatch.setattr("assemble_cua_node._assert_producer_sources", lambda *_a, **_kw: False)
    monkeypatch.setattr("assemble_cua_node._recover_publication", lambda *_a: None)
    monkeypatch.setattr("assemble_cua_node._rebuild_first_party_outputs", lambda: {"outputs": {}})
    monkeypatch.setattr("assemble_cua_node._resolve_seed", lambda *_a: Path("unused-seed"))

    def compose(
        _seed: Path,
        staging: Path,
        _commit: str,
        **_kwargs: object,
    ) -> tuple[Path, Path]:
        (staging / "payload").write_bytes(candidate_payload)
        locks = staging / "share" / "locks"
        locks.mkdir(parents=True)
        runtime_lock = locks / "runtime-lock.json"
        native_lock = locks / "native-assets.lock.json"
        runtime_lock.write_text("{}\n", encoding="utf-8")
        native_lock.write_text("{}\n", encoding="utf-8")
        return runtime_lock, native_lock

    monkeypatch.setattr("assemble_cua_node._compose", compose)
    monkeypatch.setattr("assemble_cua_node._verify", lambda *_a: {"status": "passed"})
    monkeypatch.setattr("assemble_cua_node._verify_installed_transcript", lambda _root: None)


def _write_fast_output(output: Path, payload: bytes = b"candidate") -> None:
    (output / "share" / "locks").mkdir(parents=True)
    (output / "payload").write_bytes(payload)
    (output / "share" / "locks" / "runtime-lock.json").write_text("{}\n", encoding="utf-8")
    (output / "share" / "locks" / "native-assets.lock.json").write_text("{}\n", encoding="utf-8")


def test_check_rejects_whole_component_drift(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    output = tmp_path / "component"
    _write_fast_output(output)
    (output / "unexpected-file").write_text("drift", encoding="utf-8")
    _install_fast_assembly_fakes(monkeypatch)

    with pytest.raises(AssemblyError, match="assembled output drift"):
        assemble(
            cache=tmp_path / "cache",
            seed_argument=None,
            output=output,
            producer_commit="d" * 40,
            check=True,
            allow_development_dirty=True,
        )


def test_check_verifies_installed_modes_before_tree_comparison(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    output = tmp_path / "component"
    _write_fast_output(output)
    (output / "payload").chmod(0o777)
    _install_fast_assembly_fakes(monkeypatch)

    def verify(root: Path, *_args: Path) -> dict[str, str]:
        if root == output and (root / "payload").stat().st_mode & 0o777 != 0o644:
            raise AssemblyError("installed component mode drift")
        return {"status": "passed"}

    monkeypatch.setattr("assemble_cua_node._verify", verify)

    with pytest.raises(AssemblyError, match="mode drift"):
        assemble(
            cache=tmp_path / "cache",
            seed_argument=None,
            output=output,
            producer_commit="d" * 40,
            check=True,
            allow_development_dirty=True,
        )


@pytest.mark.parametrize("check", [False, True])
def test_assembly_runs_installed_transcript_once(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, check: bool
) -> None:
    output = tmp_path / "component"
    if check:
        _write_fast_output(output)
    _install_fast_assembly_fakes(monkeypatch)
    transcript_roots: list[Path] = []
    monkeypatch.setattr("assemble_cua_node._verify_installed_transcript", transcript_roots.append)

    assemble(
        cache=tmp_path / "cache",
        seed_argument=None,
        output=output,
        producer_commit="d" * 40,
        check=check,
        allow_development_dirty=True,
    )

    assert len(transcript_roots) == 1
    assert transcript_roots[0] != output


@pytest.mark.parametrize("release_eligible", [False, True])
def test_compose_writes_first_party_build_attestation(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    release_eligible: bool,
) -> None:
    repository = tmp_path / "repo"
    host = repository / "runtime" / "cua-node" / "dist" / "cli.js"
    host.parent.mkdir(parents=True)
    host.write_text("host", encoding="utf-8")
    monkeypatch.setattr("assemble_cua_node.REPO_ROOT", repository)
    seed = tmp_path / "seed"
    for member in ("bin", "lib", "share", "licenses"):
        (seed / member).mkdir(parents=True, exist_ok=True)
    (seed / "sbom.cdx.json").write_text("{}\n", encoding="utf-8")
    migration_input = {
        "schema_version": 1,
        "source_tree_sha256": "1" * 64,
        "source_size_bytes": 1,
        "source_file_count": 1,
        "migration_evidence": {"codex_desktop_commit": "2" * 40},
    }
    (seed / "SKY_CUA_MIGRATION_INPUT.json").write_text(
        json.dumps(migration_input), encoding="utf-8"
    )
    monkeypatch.setattr(
        "assemble_cua_node._compile_launcher",
        lambda root: (root / "bin" / "node_repl").write_bytes(b"launcher"),
    )
    monkeypatch.setattr(
        "assemble_cua_node._install_first_party_packages", lambda _root: ("3" * 64, "4" * 64)
    )
    monkeypatch.setattr("assemble_cua_node._import_prepared_packages", lambda _root: None)
    monkeypatch.setattr("assemble_cua_node._remove_wrong_platform_content", lambda _root: None)
    monkeypatch.setattr("assemble_cua_node._generate_compliance", lambda *_a, **_kw: None)
    source_inventory = [{"path": "runtime/source.ts", "sha256": "5" * 64, "size_bytes": 7}]
    monkeypatch.setattr("assemble_cua_node._producer_source_inventory", lambda: source_inventory)
    monkeypatch.setattr("assemble_cua_node._generate_locks", lambda *_a, **_kw: (b"{}\n", b"{}\n"))
    monkeypatch.setattr("assemble_cua_node._write_manifest", lambda *_a, **_kw: None)
    staging = tmp_path / "staging"
    staging.mkdir()
    first_party_build: dict[str, object] = {
        "schema_version": 1,
        "commands": [{"cwd": "runtime/cua-node", "argv": ["bun", "run", "build"]}],
        "toolchain": {"bun": "1.2.0", "cc": "cc 1.0"},
        "outputs": {},
    }

    _compose(
        seed,
        staging,
        "d" * 40,
        release_eligible=release_eligible,
        first_party_build=first_party_build,
    )

    attestation = json.loads(
        (staging / "share" / "provenance" / "SKY_CUA_BUILD_ATTESTATION.json").read_text(
            encoding="utf-8"
        )
    )
    assert attestation["schema_version"] == 1
    assert attestation["release_eligible"] is release_eligible
    assert attestation["producer_commit"] == "d" * 40
    assert attestation["migration_input"] == migration_input
    assert attestation["source_inventory"] == source_inventory
    assert (
        attestation["source_inventory_sha256"]
        == hashlib.sha256(
            json.dumps(
                source_inventory, ensure_ascii=False, sort_keys=True, separators=(",", ":")
            ).encode()
        ).hexdigest()
    )
    assert attestation["first_party_build"]["outputs"]["node_repl_launcher"] == {
        "path": "bin/node_repl",
        "sha256": hashlib.sha256(b"launcher").hexdigest(),
        "size_bytes": len(b"launcher"),
    }


def test_concurrent_checks_serialize_first_party_build_under_lock(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    output = tmp_path / "component"
    _write_fast_output(output)
    _install_fast_assembly_fakes(monkeypatch)
    ready = threading.Barrier(2)
    second_build_entered = threading.Event()
    state_lock = threading.Lock()
    build_calls = 0
    concurrent_build = False

    def assert_sources(*_args: object, **_kwargs: object) -> bool:
        ready.wait(timeout=2)
        return False

    def rebuild() -> dict[str, object]:
        nonlocal build_calls, concurrent_build
        with state_lock:
            build_calls += 1
            call = build_calls
            if call == 2:
                second_build_entered.set()
        if call == 1:
            concurrent_build = second_build_entered.wait(timeout=0.2)
        return {"outputs": {}}

    monkeypatch.setattr("assemble_cua_node._assert_producer_sources", assert_sources)
    monkeypatch.setattr("assemble_cua_node._rebuild_first_party_outputs", rebuild)
    errors: list[BaseException] = []

    def run() -> None:
        try:
            assemble(
                cache=tmp_path / "cache",
                seed_argument=None,
                output=output,
                producer_commit="d" * 40,
                check=True,
                allow_development_dirty=True,
            )
        except BaseException as error:
            errors.append(error)

    threads = [threading.Thread(target=run), threading.Thread(target=run)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=3)

    assert all(not thread.is_alive() for thread in threads)
    assert errors == []
    assert build_calls == 2
    assert concurrent_build is False
