from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from _cua_node_inputs import PREPARED_IMPORTS_LOCK
from _cua_node_tree import _tree_hash
from refresh_cua_node_pins import (
    RUNTIME,
    RefreshError,
    bun_lock_packages,
    bun_workspace_pins,
    compliance_gaps,
    refresh_sources,
    sync_production,
)

BUN_LOCK = """{
  "lockfileVersion": 1,
  "workspaces": {
    "": {
      "dependencies": {
        "plain": "2.0.0",
        "sharp": "0.36.0",
      },
    },
  },
  "packages": {
    "plain": ["plain@2.0.0", "", {}, "sha512-plain-new"],

    "sharp": ["sharp@0.36.0", "", { "optionalDependencies": { "@img/sharp-linux-x64": "0.36.0" } }, "sha512-sharp-new"],
  }
}
"""


def _lock(*entries: dict[str, Any]) -> dict[str, Any]:
    return {"schema_version": 1, "packages": list(entries)}


def _entry(name: str, version: str, *, native_asset: bool = False) -> dict[str, Any]:
    tree = {"sha256": "0" * 64, "size_bytes": 0}
    return {
        "name": name,
        "version": version,
        "license": "MIT",
        "integrity": "sha512-old",
        "native_asset": native_asset,
        "source_tree": dict(tree),
        "assembled_tree": dict(tree),
    }


def test_bun_lock_parser_reads_trailing_comma_lockfiles(tmp_path: Path) -> None:
    lockfile = tmp_path / "bun.lock"
    lockfile.write_text(BUN_LOCK, encoding="utf-8")

    packages = bun_lock_packages(lockfile)

    assert packages["plain"] == {
        "version": "2.0.0",
        "integrity": "sha512-plain-new",
        "metadata": {},
    }
    assert packages["sharp"]["metadata"]["optionalDependencies"] == {
        "@img/sharp-linux-x64": "0.36.0"
    }
    assert bun_workspace_pins(lockfile) == {"plain": "2.0.0", "sharp": "0.36.0"}


def test_prepared_lock_integrity_and_versions_match_bun_lock() -> None:
    lock = json.loads(PREPARED_IMPORTS_LOCK.read_text(encoding="utf-8"))
    packages = bun_lock_packages(RUNTIME / "bun.lock")
    pins = bun_workspace_pins(RUNTIME / "bun.lock")
    for entry in lock["packages"]:
        assert packages[entry["name"]]["integrity"] == entry["integrity"], entry["name"]
        assert pins[entry["name"]] == entry["version"], entry["name"]


def test_refresh_sources_rehashes_installed_trees(tmp_path: Path) -> None:
    lockfile = tmp_path / "bun.lock"
    lockfile.write_text(BUN_LOCK, encoding="utf-8")
    package = tmp_path / "node_modules" / "plain"
    package.mkdir(parents=True)
    (package / "package.json").write_text(
        json.dumps({"name": "plain", "version": "2.0.0", "license": "ISC"}), encoding="utf-8"
    )
    lock = _lock(_entry("plain", "1.0.0"))

    refresh_sources(
        lock, bun_lock_packages(lockfile), bun_workspace_pins(lockfile), tmp_path / "node_modules"
    )

    digest, size_bytes, _ = _tree_hash(package)
    assert lock["packages"][0] | {"assembled_tree": None} == {
        "name": "plain",
        "version": "2.0.0",
        "license": "ISC",
        "integrity": "sha512-plain-new",
        "native_asset": False,
        "source_tree": {"sha256": digest, "size_bytes": size_bytes},
        "assembled_tree": None,
    }


def test_refresh_sources_rejects_an_unfrozen_install(tmp_path: Path) -> None:
    lockfile = tmp_path / "bun.lock"
    lockfile.write_text(BUN_LOCK, encoding="utf-8")
    package = tmp_path / "node_modules" / "plain"
    package.mkdir(parents=True)
    (package / "package.json").write_text(
        json.dumps({"name": "plain", "version": "1.0.0", "license": "ISC"}), encoding="utf-8"
    )

    with pytest.raises(RefreshError, match="run a frozen install"):
        refresh_sources(
            _lock(_entry("plain", "1.0.0")),
            bun_lock_packages(lockfile),
            bun_workspace_pins(lockfile),
            tmp_path / "node_modules",
        )


def test_sync_production_moves_pins_lock_nodes_and_optional_dependencies(tmp_path: Path) -> None:
    lockfile = tmp_path / "bun.lock"
    lockfile.write_text(BUN_LOCK, encoding="utf-8")
    lock = _lock(_entry("sharp", "0.36.0"), _entry("@img/sharp-linux-x64", "0.36.0"))
    lock["packages"][0]["integrity"] = "sha512-sharp-new"
    production_package = {"dependencies": {"sharp": "0.35.5", "@img/sharp-linux-x64": "0.35.5"}}
    production_lock = {
        "packages": {
            "": {"dependencies": {"sharp": "0.35.5", "@img/sharp-linux-x64": "0.35.5"}},
            "node_modules/sharp": {
                "version": "0.35.5",
                "resolved": "https://registry.npmjs.org/sharp/-/sharp-0.35.5.tgz",
                "integrity": "sha512-old",
                "license": "Apache-2.0",
                "optionalDependencies": {"@img/sharp-linux-x64": "0.35.5"},
            },
        }
    }

    sync_production(lock, bun_lock_packages(lockfile), production_package, production_lock)

    assert production_package["dependencies"] == {
        "sharp": "0.36.0",
        "@img/sharp-linux-x64": "0.36.0",
    }
    assert production_lock["packages"][""]["dependencies"] == production_package["dependencies"]
    assert production_lock["packages"]["node_modules/sharp"] == {
        "version": "0.36.0",
        "resolved": "https://registry.npmjs.org/sharp/-/sharp-0.36.0.tgz",
        "integrity": "sha512-sharp-new",
        "license": "MIT",
        "optionalDependencies": {"@img/sharp-linux-x64": "0.36.0"},
    }


def test_compliance_gaps_name_every_stale_human_record() -> None:
    lock = _lock(_entry("acorn", "9.0.0"), _entry("@img/sharp-libvips-linux-x64", "9.9.9"))
    policy = {
        "components": [
            {"id": "acorn", "version": "8.16.0", "purl": "pkg:npm/acorn@8.16.0"},
            {
                "id": "sharp-libvips-linux-x64",
                "version": "9.9.9",
                "purl": "pkg:npm/@img/sharp-libvips-linux-x64@9.9.9",
            },
        ]
    }
    provenance = {
        "records": [
            {"component_id": "acorn", "source_uri": "https://example.test/acorn-8.16.0.tgz"},
            {
                "component_id": "sharp-libvips-linux-x64",
                "source_uri": "https://example.test/sharp-libvips-linux-x64-9.9.9.tgz",
            },
        ]
    }
    notices = {
        "entries": [
            {"component_ids": ["acorn"], "planned_path": "notices/acorn-8.16.0.LICENSE"},
            {"component_ids": ["acorn"], "planned_path": "notices/acorn-8.16.0.LICENSE"},
        ]
    }

    gaps = compliance_gaps(lock, policy, provenance, notices)

    assert gaps == [
        "compliance/policy.json component acorn must name acorn@9.0.0",
        "compliance/provenance.json acorn must cite the 9.0.0 tarball",
        "compliance/notices/acorn-8.16.0.LICENSE must be re-collected for acorn@9.0.0",
        "compliance/sharp-libvips-9.9.9-source-offer.json is required corresponding-source evidence",
        "compliance/notices/sharp-libvips-9.9.9.NOTICE.md is required corresponding-source evidence",
    ]


def test_compliance_gaps_are_empty_for_the_committed_pins() -> None:
    lock = json.loads(PREPARED_IMPORTS_LOCK.read_text(encoding="utf-8"))
    compliance = RUNTIME / "compliance"
    assert (
        compliance_gaps(
            lock,
            json.loads((compliance / "policy.json").read_text(encoding="utf-8")),
            json.loads((compliance / "provenance.json").read_text(encoding="utf-8")),
            json.loads((compliance / "notice-inventory.json").read_text(encoding="utf-8")),
        )
        == []
    )
