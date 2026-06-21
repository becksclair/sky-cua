"""Tests for runtime artifact packaging and binary name contracts."""

from __future__ import annotations

from pathlib import Path

import pytest

import _plugin_bundle as plugin_bundle
import build_runtime_packages
import package_runtime_artifact
from _plugin_bundle import (
    REQUIRED_RUNTIME_PLATFORMS,
    all_runtime_binary_names,
    all_runtime_binary_paths,
    executable_name,
    merge_runtime_artifacts,
    runtime_binary_names,
    runtime_binary_path,
    runtime_binary_source_name,
)
from _test_support import (
    write_minimal_bundle_sources,
)


def test_runtime_binary_names_match_host_platform() -> None:
    suffix = ".exe" if executable_name("tool").endswith(".exe") else ""
    expected = [
        f"sky-cua-client{suffix}",
        f"sky-cua-service{suffix}",
        f"sky-cua-overlay-host{suffix}",
    ]
    if suffix == "":
        expected.append("sky-cua-cosmic-helper")
        expected.append("sky-cua-chrome-host")
        expected.append("sky-cua-input-helper")

    assert runtime_binary_names() == expected


def test_all_runtime_binary_names_include_linux_and_windows_binaries() -> None:
    assert all_runtime_binary_names() == [
        "runtimes/linux-x64/sky-cua-client",
        "runtimes/linux-x64/sky-cua-service",
        "runtimes/linux-x64/sky-cua-overlay-host",
        "runtimes/linux-x64/sky-cua-cosmic-helper",
        "runtimes/linux-x64/sky-cua-chrome-host",
        "runtimes/linux-x64/sky-cua-input-helper",
        "runtimes/linux-arm64/sky-cua-client",
        "runtimes/linux-arm64/sky-cua-service",
        "runtimes/linux-arm64/sky-cua-overlay-host",
        "runtimes/linux-arm64/sky-cua-cosmic-helper",
        "runtimes/linux-arm64/sky-cua-chrome-host",
        "runtimes/linux-arm64/sky-cua-input-helper",
        "sky-cua-client.exe",
        "sky-cua-service.exe",
        "sky-cua-overlay-host.exe",
    ]


def test_runtime_binary_paths_map_platform_variants() -> None:
    assert runtime_binary_path("linux-x64", "sky-cua-client") == Path(
        "bin/runtimes/linux-x64/sky-cua-client"
    )
    assert runtime_binary_path("linux-arm64", "sky-cua-service") == Path(
        "bin/runtimes/linux-arm64/sky-cua-service"
    )
    assert runtime_binary_path("linux-x64", "sky-cua-overlay-host") == Path(
        "bin/runtimes/linux-x64/sky-cua-overlay-host"
    )
    assert runtime_binary_path("linux-x64", "sky-cua-cosmic-helper") == Path(
        "bin/runtimes/linux-x64/sky-cua-cosmic-helper"
    )
    assert runtime_binary_path("linux-arm64", "sky-cua-chrome-host") == Path(
        "bin/runtimes/linux-arm64/sky-cua-chrome-host"
    )
    assert runtime_binary_path("windows-x64", "sky-cua-client") == Path("bin/sky-cua-client.exe")
    assert runtime_binary_path("windows-x64", "sky-cua-overlay-host") == Path(
        "bin/sky-cua-overlay-host.exe"
    )


def test_runtime_binary_source_names_reject_invalid_platform_or_binary() -> None:
    with pytest.raises(ValueError, match="unknown runtime platform"):
        runtime_binary_source_name("linux-riscv64", "sky-cua-client")
    with pytest.raises(ValueError, match="unknown runtime binary"):
        runtime_binary_source_name("windows-x64", "sky-cua-cosmic-helper")


def test_merge_runtime_artifacts_requires_all_platforms(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    artifacts_root = tmp_path / "artifacts"
    write_minimal_bundle_sources(bundle_root)
    for platform_id in REQUIRED_RUNTIME_PLATFORMS:
        platform_root = artifacts_root / platform_id
        platform_root.mkdir(parents=True)
        for binary_name in plugin_bundle.platform_runtime_binary_base_names(platform_id):
            source_name = runtime_binary_source_name(platform_id, binary_name)
            (platform_root / source_name).write_text(
                f"{platform_id}/{source_name}",
                encoding="utf-8",
            )

    merge_runtime_artifacts(bundle_root, artifacts_root)

    for relative_path in all_runtime_binary_paths():
        assert (bundle_root / relative_path).exists()
    linux_x64_host_path = plugin_bundle.chrome_extension_host_path("linux-x64")
    linux_arm64_host_path = plugin_bundle.chrome_extension_host_path("linux-arm64")
    assert linux_x64_host_path is not None
    assert linux_arm64_host_path is not None
    assert (bundle_root / linux_x64_host_path).read_text(
        encoding="utf-8"
    ) == "linux-x64/sky-cua-chrome-host"
    assert (bundle_root / linux_arm64_host_path).read_text(
        encoding="utf-8"
    ) == "linux-arm64/sky-cua-chrome-host"


def test_merge_runtime_artifacts_fails_when_variant_is_missing(tmp_path: Path) -> None:
    bundle_root = tmp_path / "bundle"
    artifacts_root = tmp_path / "artifacts"
    write_minimal_bundle_sources(bundle_root)
    for platform_id in REQUIRED_RUNTIME_PLATFORMS:
        platform_root = artifacts_root / platform_id
        platform_root.mkdir(parents=True)
        for binary_name in plugin_bundle.platform_runtime_binary_base_names(platform_id):
            if platform_id == "linux-arm64" and binary_name == "sky-cua-service":
                continue
            (platform_root / runtime_binary_source_name(platform_id, binary_name)).write_text(
                "binary",
                encoding="utf-8",
            )

    with pytest.raises(FileNotFoundError, match="linux-arm64"):
        merge_runtime_artifacts(bundle_root, artifacts_root)


def test_package_runtime_artifact_uses_platform_binary_contract(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    release_root = repo_root / "target" / "release"
    release_root.mkdir(parents=True)
    output_root = tmp_path / "artifacts"
    stale_linux_root = output_root / "linux-x64"
    stale_linux_root.mkdir(parents=True)
    (stale_linux_root / "stale-binary").write_text("stale", encoding="utf-8")

    for binary_name in plugin_bundle.platform_runtime_binary_base_names("linux-x64"):
        (release_root / runtime_binary_source_name("linux-x64", binary_name)).write_text(
            binary_name,
            encoding="utf-8",
        )
    (release_root / "sky-cua-cosmic-helper.exe").write_text(
        "windows should not package helper",
        encoding="utf-8",
    )

    monkeypatch.setattr(package_runtime_artifact, "REPO_ROOT", repo_root)

    linux_root = package_runtime_artifact.package_runtime_artifact("linux-x64", output_root)

    assert sorted(path.name for path in linux_root.iterdir()) == [
        "sky-cua-chrome-host",
        "sky-cua-client",
        "sky-cua-cosmic-helper",
        "sky-cua-input-helper",
        "sky-cua-overlay-host",
        "sky-cua-service",
    ]
    assert not (linux_root / "stale-binary").exists()

    for binary_name in plugin_bundle.platform_runtime_binary_base_names("windows-x64"):
        (release_root / runtime_binary_source_name("windows-x64", binary_name)).write_text(
            binary_name,
            encoding="utf-8",
        )

    windows_root = package_runtime_artifact.package_runtime_artifact("windows-x64", output_root)

    assert sorted(path.name for path in windows_root.iterdir()) == [
        "sky-cua-client.exe",
        "sky-cua-overlay-host.exe",
        "sky-cua-service.exe",
    ]


def test_package_runtime_artifact_rejects_invalid_platform_before_cleanup(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo_root = tmp_path / "repo"
    output_root = tmp_path / "artifacts"
    escaped = tmp_path / "escaped"
    escaped.mkdir()
    sentinel = escaped / "sentinel"
    sentinel.write_text("keep", encoding="utf-8")
    monkeypatch.setattr(package_runtime_artifact, "REPO_ROOT", repo_root)

    with pytest.raises(ValueError, match="unknown runtime platform"):
        package_runtime_artifact.package_runtime_artifact("../escaped", output_root)

    assert sentinel.read_text(encoding="utf-8") == "keep"


def test_build_runtime_packages_uses_packaging_contract(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    calls: list[list[str]] = []

    def fake_run(command: list[str], *, check: bool) -> None:
        calls.append(command)
        assert check is True

    monkeypatch.setattr(build_runtime_packages.subprocess, "run", fake_run)

    build_runtime_packages.build_runtime_packages("linux-x64")
    build_runtime_packages.build_runtime_packages("windows-x64")

    assert calls == [
        [
            "cargo",
            "build",
            "--release",
            "--package",
            "sky-cua-client",
            "--package",
            "sky-cua-service",
            "--package",
            "sky-cua-overlay-host",
            "--package",
            "sky-cua-cosmic-helper",
            "--package",
            "sky-cua-chrome-host",
            "--package",
            "sky-cua-input-helper",
        ],
        [
            "cargo",
            "build",
            "--release",
            "--package",
            "sky-cua-client",
            "--package",
            "sky-cua-service",
            "--package",
            "sky-cua-overlay-host",
        ],
    ]
