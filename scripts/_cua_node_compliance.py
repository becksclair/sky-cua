"""License inventory, notices, SBOM, and provenance generation for cua_node."""

from __future__ import annotations

import hashlib
import json
import re
import shutil
from pathlib import Path
from typing import cast

from _artifact_helpers import (
    sha256_file,
)
from _cua_node_inputs import (
    HOST_VERSION,
    MIGRATION_COMMIT,
    NODE_VERSION,
    SHARP_LIBVIPS_RECORDS,
    TARGET,
)
from _cua_node_packages import (
    _license_expression,
    _package_roots,
)
from _cua_node_tree import (
    AssemblyError,
    _canonical_json,
    _sha256_bytes,
    _tree_hash,
)
from _plugin_bundle import REPO_ROOT, remove_path


def _generate_compliance(
    root: Path,
    *,
    producer_commit: str,
    browser_hash: str,
    migration_input: dict[str, object],
) -> None:
    modules = root / "lib" / "node_modules"
    package_roots = _package_roots(modules)
    if not package_roots:
        raise AssemblyError("compliance discovery found no shipped packages")
    notices_root = root / "licenses" / "packages"
    remove_path(notices_root)
    notices_root.mkdir(parents=True)
    inventory: list[dict[str, object]] = []
    accounted_package_paths: set[str] = set()
    for package_root in package_roots:
        package_path = package_root / "package.json"
        package = json.loads(package_path.read_text(encoding="utf-8"))
        name = package.get("name")
        version = package.get("version")
        if not isinstance(name, str) or not name or not isinstance(version, str) or not version:
            raise AssemblyError(f"compliance package identity is missing: {package_path}")
        package_relative = package_root.relative_to(root).as_posix()
        if package_relative in accounted_package_paths:
            raise AssemblyError(f"duplicate compliance package path: {package_relative}")
        accounted_package_paths.add(package_relative)
        license_files = sorted(
            path
            for path in package_root.iterdir()
            if path.is_file()
            and re.fullmatch(
                r"(?i)(?:licen[cs]e|copying|notice)(?:[-._][A-Za-z0-9._-]+)?", path.name
            )
        )
        path_suffix = hashlib.sha256(package_relative.encode()).hexdigest()[:12]
        destination = notices_root / (name.replace("@", "").replace("/", "--") + "--" + path_suffix)
        destination.mkdir(parents=True)
        relative_files: list[str] = []
        for source in license_files:
            target = destination / source.name
            shutil.copy2(source, target)
            relative_files.append(target.relative_to(root).as_posix())
        if not relative_files:
            bound_records = {
                "@img/sharp-libvips-linux-x64": [
                    f"licenses/{relative}" for relative in SHARP_LIBVIPS_RECORDS
                ],
                "@napi-rs/canvas-linux-x64-gnu": [
                    "licenses/canvas-0.1.91-source-offer.json",
                    "licenses/canvas-0.1.91-build-record.json",
                    "licenses/notices/canvas-0.1.91-NATIVE-NOTICES.md",
                    "licenses/notices/canvas-0.1.91-RUST-NOTICES.md",
                    "licenses/notices/canvas-0.1.91.NOTICE.md",
                ],
            }.get(name, [])
            for relative in bound_records:
                if not (root / relative).is_file():
                    raise AssemblyError(f"compliance record is missing: {relative}")
            relative_files.extend(bound_records)
        if not relative_files:
            expression = (
                "LicenseRef-Heliasar-Proprietary"
                if name.startswith("@heliasar/")
                else _license_expression(package.get("license"))
            )
            if expression == "NOASSERTION":
                raise AssemblyError(f"compliance package has no declared license: {name}")
            license_text = (
                REPO_ROOT / "resources" / "release" / "license-texts" / f"{expression}.txt"
            )
            if not license_text.is_file() or license_text.is_symlink():
                raise AssemblyError(f"canonical declared license text is missing: {expression}")
            license_target = destination / f"SPDX-{expression}.txt"
            shutil.copy2(license_text, license_target)
            relative_files.append(license_target.relative_to(root).as_posix())
            declared_record = destination / "DECLARED-LICENSE.json"
            declared_record.write_bytes(
                _canonical_json(
                    {
                        "schema_version": 1,
                        "record_kind": "publisher-declared-license",
                        "name": name,
                        "version": version,
                        "license_expression": expression,
                        "package_json_sha256": sha256_file(package_path),
                        "package_path": package_relative,
                    }
                )
            )
            relative_files.append(declared_record.relative_to(root).as_posix())
        inventory.append(
            {
                "name": name,
                "version": version,
                "package_path": package_relative,
                "license": (
                    "LicenseRef-Heliasar-Proprietary"
                    if name.startswith("@heliasar/")
                    else _license_expression(package.get("license"))
                ),
                "license_files": relative_files,
                "license_file_sha256s": {
                    relative: sha256_file(root / relative) for relative in relative_files
                },
                "package_tree_sha256": _tree_hash(package_root)[0],
            }
        )
    inventory.sort(key=lambda item: (cast(str, item["name"]), cast(str, item["package_path"])))
    if accounted_package_paths != {path.relative_to(root).as_posix() for path in package_roots}:
        raise AssemblyError("compliance inventory does not cover every shipped package root")
    (root / "licenses" / "LICENSES.json").write_bytes(
        _canonical_json({"schema_version": 1, "packages": inventory})
    )
    notices = [
        "# cua_node third-party notices",
        "",
        "This inventory is generated from the exact assembled package trees. Exact package license/notice files are copied under `licenses/packages/`; when a publisher omits license text, the package declaration and canonical SPDX text are both bound there.",
        "",
    ]
    notices.extend(
        f"- {item['name']} {item['version']}: {item['license']} ({', '.join(cast(list[str], item['license_files']))})"
        for item in inventory
    )
    notices.extend(
        [
            "",
            "Native Canvas and libvips source-offer/build records are bundled beside this file. Node.js license text is `Node.js-LICENSE.txt`. Playwright uses a system Chrome-family browser; no browser or FFmpeg download is bundled.",
            "",
        ]
    )
    (root / "licenses" / "THIRD_PARTY_NOTICES.md").write_text("\n".join(notices), encoding="utf-8")
    components = [
        {
            "type": "library",
            "name": item["name"],
            "version": item["version"],
            "licenses": [{"license": {"expression": item["license"]}}],
            "hashes": [{"alg": "SHA-256", "content": item["package_tree_sha256"]}],
            "properties": [{"name": "sky-cua:provenance-status", "value": "verified"}],
        }
        for item in inventory
    ]
    for name, version, path, license_path in (
        ("Node.js", NODE_VERSION, "bin/node", "licenses/Node.js-LICENSE.txt"),
        ("cua_node host/kernel", HOST_VERSION, "lib/node_repl/cli.js", None),
        ("Tesseract English traineddata", "4.1.0", "share/tessdata/eng.traineddata", None),
        ("Tesseract OSD traineddata", "4.1.0", "share/tessdata/osd.traineddata", None),
    ):
        component: dict[str, object] = {
            "type": "file",
            "name": name,
            "version": version,
            "hashes": [{"alg": "SHA-256", "content": sha256_file(root / path)}],
            "properties": [{"name": "sky-cua:provenance-status", "value": "verified"}],
        }
        if license_path is not None:
            component["licenses"] = [{"license": {"name": "Node.js bundled license notices"}}]
            cast(list[dict[str, str]], component["properties"]).extend(
                [
                    {"name": "sky-cua:license-path", "value": license_path},
                    {"name": "sky-cua:license-sha256", "value": sha256_file(root / license_path)},
                ]
            )
        components.append(component)
    for name, version, path, license_expression, license_paths in (
        (
            "PDF.js cmaps",
            "5.4.624",
            "share/pdfjs/cmaps",
            "BSD-3-Clause",
            ("share/pdfjs/cmaps/LICENSE",),
        ),
        (
            "PDF.js standard fonts",
            "5.4.624",
            "share/pdfjs/standard_fonts",
            "BSD-3-Clause AND OFL-1.1",
            (
                "share/pdfjs/standard_fonts/LICENSE_FOXIT",
                "share/pdfjs/standard_fonts/LICENSE_LIBERATION",
            ),
        ),
    ):
        tree_sha256, tree_size, tree_files = _tree_hash(root / path)
        properties = [
            {"name": "sky-cua:provenance-status", "value": "verified"},
            {"name": "sky-cua:path", "value": path},
            {"name": "sky-cua:tree-size", "value": str(tree_size)},
            {"name": "sky-cua:file-count", "value": str(tree_files)},
        ]
        for index, license_path in enumerate(license_paths):
            properties.extend(
                [
                    {"name": f"sky-cua:license:{index}:path", "value": license_path},
                    {
                        "name": f"sky-cua:license:{index}:sha256",
                        "value": sha256_file(root / license_path),
                    },
                ]
            )
        components.append(
            {
                "type": "data",
                "name": name,
                "version": version,
                "hashes": [{"alg": "SHA-256", "content": tree_sha256}],
                "licenses": [{"license": {"expression": license_expression}}],
                "properties": properties,
            }
        )
    components.sort(key=lambda item: (cast(str, item["name"]), cast(str, item["version"])))
    serial_digest = hashlib.sha256(_canonical_json(components)).hexdigest()
    sbom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "serialNumber": f"urn:uuid:{serial_digest[:8]}-{serial_digest[8:12]}-{serial_digest[12:16]}-{serial_digest[16:20]}-{serial_digest[20:32]}",
        "version": 1,
        "metadata": {
            "component": {
                "type": "application",
                "name": "sky-cua cua_node",
                "version": HOST_VERSION,
            },
            "properties": [
                {"name": "sky-cua:producer-commit", "value": producer_commit},
                {"name": "sky-cua:component-status", "value": "verified-candidate"},
                {"name": "sky-cua:browser-sha256", "value": browser_hash},
            ],
        },
        "components": components,
    }
    (root / "sbom.cdx.json").write_bytes(_canonical_json(sbom))
    provenance = {
        "schema_version": 1,
        "producer": "sky-cua",
        "producer_commit": producer_commit,
        "target": TARGET,
        "migration_evidence": {"codex_desktop_commit": MIGRATION_COMMIT},
        "canonical_browser_sha256": browser_hash,
        "migration_input": migration_input,
        "package_inventory_sha256": _sha256_bytes(_canonical_json(inventory)),
        "absolute_checkout_paths": {
            "embedded_native_build_debug_metadata": True,
            "runtime_path_dependencies": False,
        },
    }
    (root / "licenses" / "PROVENANCE.json").write_bytes(_canonical_json(provenance))
