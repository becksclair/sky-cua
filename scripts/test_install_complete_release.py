from __future__ import annotations

import json
from pathlib import Path
from typing import ClassVar

import pytest

import install_complete_release as controller
from _native_messaging_install import HOST_RELATIVE_PATH
from _openclaw_install import OpenClawReleaseInstallReport
from _opencode_install import OpenCodeInstallReport
from release_generation import VerifiedRelease

RELEASE_ID = "a" * 64
PRIOR_ID = "b" * 64


class FakeStore:
    prior: str | None = PRIOR_ID
    events: ClassVar[list[str]] = []
    create_host: ClassVar[bool] = True

    def __init__(self, root: Path) -> None:
        self.root = root
        self.releases = root / "releases"

    def current_release_id(self) -> str | None:
        return self.prior

    def previous_release_id(self) -> str | None:
        return self.prior

    def install(self, _candidate: Path, **_kwargs: object) -> VerifiedRelease:
        self.events.append("install")
        release_root = self.root / "releases" / RELEASE_ID
        if self.create_host:
            host = release_root / HOST_RELATIVE_PATH
            host.parent.mkdir(parents=True, exist_ok=True)
            host.write_bytes(b"native-host")
            host.chmod(0o755)
        extension_relative = "components/core-linux-x64/resources/chrome-extension/codex/1.2.3_0"
        extension = release_root / extension_relative
        extension.mkdir(parents=True, exist_ok=True)
        (release_root / "RELEASE.json").write_text(
            json.dumps(
                {
                    "browser_contract": {
                        "extension_bridge": {
                            "extension_id": "hehggadaopoacecdllhhajmbjkdcmajg",
                            "manifest_sha256": "1" * 64,
                            "path": extension_relative,
                            "tree_sha256": "2" * 64,
                            "version": "1.2.3",
                        }
                    }
                }
            ),
            encoding="utf-8",
        )
        return VerifiedRelease(
            root=release_root,
            release_id=RELEASE_ID,
            manifest_sha256="c" * 64,
            profile="full",
            component_names=("core-linux-x64", "cua-node-linux-x64-glibc"),
        )

    def rollback(self) -> VerifiedRelease:
        self.events.append("rollback-generation")
        return VerifiedRelease(
            root=self.root / "releases" / PRIOR_ID,
            release_id=PRIOR_ID,
            manifest_sha256="d" * 64,
            profile="full",
            component_names=("core-linux-x64", "cua-node-linux-x64-glibc"),
        )

    def deactivate_initial_activation(self, release_id: str) -> VerifiedRelease:
        assert release_id == RELEASE_ID
        self.events.append("deactivate-generation")
        return VerifiedRelease(
            root=self.root / "releases" / RELEASE_ID,
            release_id=RELEASE_ID,
            manifest_sha256="c" * 64,
            profile="full",
            component_names=("core-linux-x64", "cua-node-linux-x64-glibc"),
        )


def _opencode_report(root: Path, *, changed: bool = True) -> OpenCodeInstallReport:
    return OpenCodeInstallReport(
        config_path=root / "opencode.jsonc",
        release_root=root / "releases" / RELEASE_ID,
        release_id=RELEASE_ID,
        manifest_sha256="c" * 64,
        changed=changed,
        backup_path=root / "backup.json" if changed else None,
        installed_config_sha256="e" * 64,
    )


@pytest.fixture(autouse=True)
def _reset_store(monkeypatch: pytest.MonkeyPatch) -> None:
    FakeStore.prior = PRIOR_ID
    FakeStore.events = []
    FakeStore.create_host = True
    monkeypatch.setattr(controller, "GenerationStore", FakeStore)


def test_controller_promotes_then_configures_opencode_before_openclaw(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    opencode = _opencode_report(tmp_path)

    def install_opencode(*_args: object, **_kwargs: object) -> OpenCodeInstallReport:
        FakeStore.events.append("opencode")
        return opencode

    def install_openclaw(*_args: object, **_kwargs: object) -> OpenClawReleaseInstallReport:
        FakeStore.events.append("openclaw")
        return OpenClawReleaseInstallReport(
            release_id=RELEASE_ID,
            manifest_sha256="c" * 64,
            release_root=str(tmp_path / "store/releases" / RELEASE_ID),
            config_path=str(tmp_path / "openclaw/openclaw.json"),
            registered_servers=("sky_cua", "node_repl"),
            changed_servers=("sky_cua", "node_repl"),
            gateway_activation="gateway_watcher_pending_verification",
            gateway_detail="pending",
        )

    monkeypatch.setattr(controller, "install_opencode_two_server_config", install_opencode)
    monkeypatch.setattr(controller, "install_openclaw_release", install_openclaw)
    report = controller.install_complete_release(
        tmp_path / "candidate",
        store_root=tmp_path / "store",
        hosts=("openclaw", "opencode"),
        browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
        native_messaging_home=tmp_path / "home",
    )

    assert FakeStore.events == ["install", "opencode", "openclaw"]
    assert report.previous_release_id == PRIOR_ID
    assert report.configured_hosts == ("openclaw", "opencode")
    assert report.as_dict()["release_id"] == RELEASE_ID
    assert report.as_dict()["browser_reload_required"] is True
    assert report.browser_extension["activation"] == "load_unpacked"
    assert report.browser_extension["version"] == "1.2.3"


@pytest.mark.parametrize(
    ("prior", "generation_event"),
    [(PRIOR_ID, "rollback-generation"), (None, "deactivate-generation")],
)
def test_host_failure_rolls_back_opencode_and_generation(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    prior: str | None,
    generation_event: str,
) -> None:
    FakeStore.prior = prior
    opencode = _opencode_report(tmp_path)
    monkeypatch.setattr(
        controller, "install_opencode_two_server_config", lambda *_args, **_kwargs: opencode
    )

    def fail_openclaw(*_args: object, **_kwargs: object) -> OpenClawReleaseInstallReport:
        raise RuntimeError("gateway cutover failed")

    monkeypatch.setattr(controller, "install_openclaw_release", fail_openclaw)

    def rollback(**_kwargs: object) -> None:
        FakeStore.events.append("rollback-opencode")

    monkeypatch.setattr(controller, "rollback_opencode_install", rollback)
    native_manifest = (
        tmp_path / "home/.config/google-chrome/NativeMessagingHosts/com.openai.codexextension.json"
    )
    native_manifest.parent.mkdir(parents=True)
    original_manifest = b"preexisting manifest bytes\n"
    native_manifest.write_bytes(original_manifest)
    with pytest.raises(
        controller.CompleteReleaseInstallError,
        match="consumer configuration failed: gateway cutover failed",
    ):
        controller.install_complete_release(
            tmp_path / "candidate",
            store_root=tmp_path / "store",
            hosts=("opencode", "openclaw"),
            browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
            native_messaging_home=tmp_path / "home",
        )

    assert FakeStore.events == ["install", "rollback-opencode", generation_event]
    assert native_manifest.read_bytes() == original_manifest
    for relative in (
        ".config/chromium/NativeMessagingHosts/com.openai.codexextension.json",
        ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts/com.openai.codexextension.json",
        ".config/BraveSoftware/Brave-Origin/NativeMessagingHosts/com.openai.codexextension.json",
    ):
        assert not (tmp_path / "home" / relative).exists()


def test_host_configuration_requires_full_profile_and_absolute_socket(tmp_path: Path) -> None:
    with pytest.raises(controller.CompleteReleaseInstallError, match="requires the full profile"):
        controller.install_complete_release(
            tmp_path / "candidate",
            store_root=tmp_path / "store",
            profile="core-only",
            hosts=("opencode",),
            browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
        )
    with pytest.raises(controller.CompleteReleaseInstallError, match="absolute browser socket"):
        controller.install_complete_release(
            tmp_path / "candidate",
            store_root=tmp_path / "store",
            hosts=("openclaw",),
            browser_socket_path=Path("relative.sock"),
        )
    assert FakeStore.events == []


def test_opencode_failure_restores_manifests_before_generation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    existing = (
        tmp_path / "home/.config/chromium/NativeMessagingHosts/com.openai.codexextension.json"
    )
    existing.parent.mkdir(parents=True)
    original = b"chromium prior bytes"
    existing.write_bytes(original)

    def fail_opencode(*_args: object, **_kwargs: object) -> OpenCodeInstallReport:
        raise RuntimeError("opencode projection failed")

    monkeypatch.setattr(controller, "install_opencode_two_server_config", fail_opencode)
    with pytest.raises(
        controller.CompleteReleaseInstallError,
        match="consumer configuration failed: opencode projection failed",
    ):
        controller.install_complete_release(
            tmp_path / "candidate",
            store_root=tmp_path / "store",
            hosts=("opencode",),
            browser_socket_path=Path("/run/user/1000/sky-cua/browser.sock"),
            native_messaging_home=tmp_path / "home",
        )

    assert existing.read_bytes() == original
    assert FakeStore.events == ["install", "rollback-generation"]


def test_missing_generation_host_rolls_back_without_manifest_mutation(tmp_path: Path) -> None:
    FakeStore.create_host = False
    home = tmp_path / "home"

    with pytest.raises(
        controller.CompleteReleaseInstallError,
        match="missing native messaging host",
    ):
        controller.install_complete_release(
            tmp_path / "candidate",
            store_root=tmp_path / "store",
            native_messaging_home=home,
        )

    assert FakeStore.events == ["install", "rollback-generation"]
    assert not home.exists()


def test_cli_rollback_reprojects_retained_generation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    captured: dict[str, object] = {}

    class Report:
        @staticmethod
        def as_dict() -> dict[str, object]:
            return {"release_id": PRIOR_ID, "status": "reprojected"}

    def install(candidate: Path, **kwargs: object) -> Report:
        captured["candidate"] = candidate
        captured.update(kwargs)
        return Report()

    monkeypatch.setattr(controller, "install_complete_release", install)
    assert (
        controller.main(
            [
                "--rollback",
                "--store-root",
                str(tmp_path / "store"),
                "--host",
                "openclaw",
                "--host",
                "opencode",
                "--browser-socket",
                "/run/user/1000/sky-cua/browser.sock",
            ]
        )
        == 0
    )

    assert captured["candidate"] == tmp_path / "store" / "releases" / PRIOR_ID
    assert captured["hosts"] == ["openclaw", "opencode"]
    assert captured["browser_socket_path"] == Path("/run/user/1000/sky-cua/browser.sock")
    assert '"release_id": "bbbb' in capsys.readouterr().out
