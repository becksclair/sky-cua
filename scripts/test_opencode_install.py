from __future__ import annotations

import json
import stat
from pathlib import Path

import pytest

import _opencode_install as installer
from _opencode_install import (
    OpenCodeInstallError,
    install_opencode_two_server_config,
    parse_jsonc,
    rollback_opencode_install,
)
from release_generation import VerifiedRelease

RELEASE_ID = "a" * 64
MANIFEST_SHA256 = "b" * 64
BROWSER_SHA256 = "c" * 64


def _executable(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("fixture", encoding="utf-8")
    path.chmod(0o755)


def _release(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[Path, Path, list[Path]]:
    generation = tmp_path / "store" / "releases" / RELEASE_ID
    core = generation / "components/core-linux-x64"
    cua_node = generation / "components/cua-node-linux-x64-glibc"
    _executable(core / "bin/sky-cua-client")
    _executable(cua_node / "bin/node_repl")
    _executable(cua_node / "bin/node")
    (cua_node / "lib/node_modules").mkdir(parents=True)
    (cua_node / "share/playwright").mkdir(parents=True)
    manifest = {
        "release_id": RELEASE_ID,
        "components": [
            {"name": "core-linux-x64", "path": "components/core-linux-x64"},
            {
                "name": "cua-node-linux-x64-glibc",
                "path": "components/cua-node-linux-x64-glibc",
            },
        ],
        "trusted_browser_client_sha256s": [BROWSER_SHA256],
    }
    (generation / "RELEASE.json").write_text(json.dumps(manifest), encoding="utf-8")
    current = generation.parents[1] / "current"
    current.symlink_to(generation, target_is_directory=True)
    verified_calls: list[Path] = []

    def verify(root: Path, *, profile: str, enforce_profile_shape: bool) -> VerifiedRelease:
        assert profile == "full"
        assert enforce_profile_shape is True
        verified_calls.append(root)
        return VerifiedRelease(
            root=root,
            release_id=RELEASE_ID,
            manifest_sha256=MANIFEST_SHA256,
            profile="full",
            component_names=(
                "core-linux-x64",
                "browser-js",
                "cua-node-linux-x64-glibc",
                "codex-compat",
                "compliance",
            ),
        )

    monkeypatch.setattr(installer, "verify_release_root", verify)
    return current, generation, verified_calls


def _install(
    current: Path,
    config_dir: Path,
    **kwargs: object,
) -> installer.OpenCodeInstallReport:
    return install_opencode_two_server_config(
        current,
        browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
        config_dir=config_dir,
        process_env={},
        **kwargs,  # type: ignore[arg-type]
    )


def test_jsonc_merge_preserves_unrelated_comments_and_pins_one_generation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, generation, verified_calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    config_dir.mkdir(parents=True)
    config = config_dir / "opencode.jsonc"
    config.write_text(
        """{
  // user model selection must survive byte-for-byte
  "model": "provider/model",
  "mcp": {
    // unrelated server must survive
    "context7": {"type": "remote", "url": "https://example.invalid/mcp"},
    "sky_cua": {"type": "local", "command": ["old"]}, // replace only this value
  },
}
""",
        encoding="utf-8",
    )

    report = _install(current, config_dir)

    text = config.read_text(encoding="utf-8")
    parsed = parse_jsonc(text)
    assert isinstance(parsed, dict)
    assert parsed["model"] == "provider/model"
    assert "// user model selection must survive byte-for-byte" in text
    assert "// unrelated server must survive" in text
    assert "// replace only this value" in text
    assert parsed["mcp"]["context7"] == {  # type: ignore[index]
        "type": "remote",
        "url": "https://example.invalid/mcp",
    }
    sky_cua = parsed["mcp"]["sky_cua"]  # type: ignore[index]
    node_repl = parsed["mcp"]["node_repl"]  # type: ignore[index]
    assert sky_cua["command"] == [
        str(generation / "components/core-linux-x64/bin/sky-cua-client"),
        "mcp",
    ]
    assert node_repl["command"] == [
        str(generation / "components/cua-node-linux-x64-glibc/bin/node_repl")
    ]
    for server in (sky_cua, node_repl):
        assert server["cwd"] == str(generation)
        assert server["enabled"] is True
        assert server["timeout"] == 30_000
        assert server["environment"]["SKY_CUA_RELEASE_ROOT"] == str(generation)
        assert server["environment"]["SKY_CUA_MCP_CALLER_PROVENANCE"] == "opencode"
        assert (
            server["environment"]["SKY_CUA_CODEX_BROWSER_SOCKET_PATH"]
            == "/run/user/1000/sky-cua/browser.sock"
        )
    node_env = node_repl["environment"]
    cua_root = generation / "components/cua-node-linux-x64-glibc"
    assert node_env["CODEX_NODE_REPL_PATH"] == str(cua_root / "bin/node_repl")
    assert node_env["NODE_REPL_NODE_PATH"] == str(cua_root / "bin/node")
    assert node_env["NODE_REPL_NODE_MODULE_DIRS"] == str(cua_root / "lib/node_modules")
    assert node_env["PLAYWRIGHT_BROWSERS_PATH"] == str(cua_root / "share/playwright")
    assert node_env["NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S"] == BROWSER_SHA256
    assert verified_calls == [generation]
    assert report.release_root == generation
    assert report.release_id == RELEASE_ID
    assert report.manifest_sha256 == MANIFEST_SHA256
    assert report.restart_required is True
    assert report.restart_scope == "all_running_opencode_processes"
    assert report.activation_status == "pending_full_process_restart"
    assert "session reload and MCP reconnect are not sufficient" in report.restart_instruction
    assert report.backup_path is not None and report.backup_path.is_file()
    serialized_report = json.loads(json.dumps(report.to_dict()))
    assert serialized_report["release_root"] == str(generation)
    assert serialized_report["server_names"] == ["sky_cua", "node_repl"]


def test_new_global_config_is_private_and_install_is_idempotent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation, verified_calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"

    first = _install(current, config_dir)
    first_bytes = first.config_path.read_bytes()
    first_backup = first.backup_path
    second = _install(current, config_dir)

    assert first.changed is True
    assert second.changed is False
    assert second.backup_path is None
    assert second.restart_required is False
    assert second.restart_scope == "none"
    assert second.activation_status == "unchanged"
    assert second.config_path.read_bytes() == first_bytes
    assert first_backup is not None and first_backup.is_file()
    assert stat.S_IMODE(first.config_path.stat().st_mode) == 0o600
    assert stat.S_IMODE(first_backup.stat().st_mode) == 0o600
    assert stat.S_IMODE(first_backup.parent.stat().st_mode) == 0o700
    assert verified_calls == [first.release_root, second.release_root]


def test_new_global_config_rollback_removes_the_installed_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation, _calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    report = _install(current, config_dir)
    assert report.backup_path is not None

    rollback_opencode_install(
        config_path=report.config_path,
        backup_path=report.backup_path,
        expected_installed_sha256=report.installed_config_sha256,
    )

    assert not report.config_path.exists()
    assert report.backup_path.is_file()


def test_rollback_restores_exact_original_and_rejects_stale_config(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation, _calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    config_dir.mkdir(parents=True)
    config = config_dir / "opencode.json"
    original = b'{"model":"keep"}\n'
    config.write_bytes(original)
    config.chmod(0o640)
    report = _install(current, config_dir)
    assert report.backup_path is not None

    rollback_opencode_install(
        config_path=report.config_path,
        backup_path=report.backup_path,
        expected_installed_sha256=report.installed_config_sha256,
    )
    assert config.read_bytes() == original
    assert stat.S_IMODE(config.stat().st_mode) == 0o640

    report = _install(current, config_dir)
    assert report.backup_path is not None
    config.write_text(config.read_text(encoding="utf-8") + "// later user edit\n", encoding="utf-8")
    with pytest.raises(OpenCodeInstallError, match="changed after install"):
        rollback_opencode_install(
            config_path=report.config_path,
            backup_path=report.backup_path,
            expected_installed_sha256=report.installed_config_sha256,
        )


def test_post_write_failure_rolls_back_before_propagating(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation, _calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    config_dir.mkdir(parents=True)
    config = config_dir / "opencode.jsonc"
    original = '{/* keep */\n  "model": "keep",\n}\n'
    config.write_text(original, encoding="utf-8")

    def fail(_path: Path) -> None:
        raise RuntimeError("controller validation failed")

    with pytest.raises(RuntimeError, match="controller validation failed"):
        _install(current, config_dir, after_write=fail)
    assert config.read_text(encoding="utf-8") == original


def test_precedence_hazards_fail_before_release_verification_or_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation, verified_calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    config_dir.mkdir(parents=True)
    (config_dir / "opencode.json").write_text("{}\n", encoding="utf-8")
    (config_dir / "opencode.jsonc").write_text("{}\n", encoding="utf-8")
    with pytest.raises(OpenCodeInstallError, match="both global"):
        _install(current, config_dir)
    assert verified_calls == []

    (config_dir / "opencode.jsonc").unlink()
    with pytest.raises(OpenCodeInstallError, match="OPENCODE_CONFIG"):
        install_opencode_two_server_config(
            current,
            browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
            config_dir=config_dir,
            process_env={"OPENCODE_CONFIG": "/tmp/custom.json"},
        )


def test_process_home_selects_only_the_isolated_global_config(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, generation, verified_calls = _release(tmp_path, monkeypatch)
    isolated_home = tmp_path / "isolated-home"

    report = install_opencode_two_server_config(
        current,
        browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
        process_env={"HOME": str(isolated_home)},
    )

    assert report.config_path == isolated_home / ".config/opencode/opencode.jsonc"
    assert report.config_path.is_file()
    parsed = parse_jsonc(report.config_path.read_text(encoding="utf-8"))
    assert isinstance(parsed, dict)
    assert sorted(parsed["mcp"]) == ["node_repl", "sky_cua"]  # type: ignore[arg-type]
    assert verified_calls == [generation]


def test_project_override_and_project_filename_ambiguity_are_detected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation, verified_calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    project = tmp_path / "project/subdir"
    project.mkdir(parents=True)
    (project.parent / ".git").mkdir()
    (project.parent / "opencode.jsonc").write_text(
        '{"mcp":{"node_repl":{"command":["shadow"]}}}\n', encoding="utf-8"
    )
    with pytest.raises(OpenCodeInstallError, match="overrides managed MCP"):
        _install(current, config_dir, effective_cwd=project)
    assert verified_calls == []

    (project.parent / "opencode.json").write_text("{}\n", encoding="utf-8")
    with pytest.raises(OpenCodeInstallError, match="both project config names"):
        _install(current, config_dir, effective_cwd=project)


def test_custom_directory_nested_project_and_managed_precedence_are_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation, verified_calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    project = tmp_path / "project"
    nested = project / ".opencode"
    nested.mkdir(parents=True)
    (project / ".git").mkdir()
    (nested / "opencode.json").write_text(
        '{"mcp":{"node_repl":{"command":["shadow"]}}}\n', encoding="utf-8"
    )

    with pytest.raises(OpenCodeInstallError, match="OPENCODE_CONFIG_DIR"):
        install_opencode_two_server_config(
            current,
            browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
            config_dir=config_dir,
            process_env={"OPENCODE_CONFIG_DIR": str(tmp_path / "custom")},
        )
    with pytest.raises(OpenCodeInstallError, match=r"project config .*node_repl"):
        _install(current, config_dir, effective_cwd=project)

    managed = tmp_path / "etc/opencode"
    managed.mkdir(parents=True)
    (managed / "opencode.jsonc").write_text(
        '{"mcp":{"sky_cua":{"command":["shadow"]}}}\n', encoding="utf-8"
    )
    monkeypatch.setattr(installer, "MANAGED_CONFIG_DIR", managed)
    with pytest.raises(OpenCodeInstallError, match=r"managed config .*sky_cua"):
        _install(current, config_dir)
    assert verified_calls == []


def test_release_root_must_resolve_to_release_id_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, generation, _calls = _release(tmp_path, monkeypatch)
    arbitrary = generation.parent / "mutable-alias"
    generation.rename(arbitrary)
    current.unlink()
    current.symlink_to(arbitrary, target_is_directory=True)

    with pytest.raises(OpenCodeInstallError, match="named by its release id"):
        _install(current, tmp_path / "home/.config/opencode")


def test_backup_identity_includes_original_mode(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, _generation, _calls = _release(tmp_path, monkeypatch)
    backup_names: list[str] = []
    for name, mode in (("first", 0o600), ("second", 0o640)):
        config_dir = tmp_path / name / ".config/opencode"
        config_dir.mkdir(parents=True)
        config = config_dir / "opencode.json"
        config.write_text('{"model":"keep"}\n', encoding="utf-8")
        config.chmod(mode)
        report = _install(current, config_dir)
        assert report.backup_path is not None
        backup_names.append(report.backup_path.name)
    assert len(set(backup_names)) == 2


@pytest.mark.parametrize(
    "source, message",
    [
        ('{"mcp": [], "other": 1}\n', "mcp must be an object"),
        ('{"mcp": {}, "mcp": {}}\n', "duplicate JSONC object key"),
        ('{"mcp": {/* unterminated }\n', "unterminated block comment"),
    ],
)
def test_ambiguous_or_invalid_jsonc_is_rejected(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    source: str,
    message: str,
) -> None:
    current, _generation, _calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    config_dir.mkdir(parents=True)
    config = config_dir / "opencode.jsonc"
    config.write_text(source, encoding="utf-8")
    before = config.read_bytes()
    with pytest.raises(OpenCodeInstallError, match=message):
        _install(current, config_dir)
    assert config.read_bytes() == before


def test_missing_browser_socket_or_core_only_generation_is_rejected(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    current, generation, _calls = _release(tmp_path, monkeypatch)
    config_dir = tmp_path / "home/.config/opencode"
    with pytest.raises(OpenCodeInstallError, match="socket path must be absolute"):
        install_opencode_two_server_config(
            current,
            browser_socket_path=Path("relative.sock"),
            config_dir=config_dir,
            process_env={},
        )

    with pytest.raises(OpenCodeInstallError, match="timeout must be a positive integer"):
        install_opencode_two_server_config(
            current,
            browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
            config_dir=config_dir,
            process_env={},
            timeout_ms=1.5,  # type: ignore[arg-type]
        )

    monkeypatch.setattr(
        installer,
        "verify_release_root",
        lambda root, **_kwargs: VerifiedRelease(
            root=root,
            release_id=RELEASE_ID,
            manifest_sha256=MANIFEST_SHA256,
            profile="core-only",
            component_names=("core-linux-x64", "compliance"),
        ),
    )
    with pytest.raises(OpenCodeInstallError, match="requires a verified full"):
        _install(current, config_dir)
    assert not config_dir.exists()
    assert generation.is_dir()
