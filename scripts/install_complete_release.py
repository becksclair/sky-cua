#!/usr/bin/env python3
"""Install one complete sky-cua generation and configure generic MCP hosts."""

from __future__ import annotations

import argparse
import json
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path

from _native_messaging_install import (
    NativeMessagingInstallReport,
    install_native_messaging_manifests,
    rollback_native_messaging_manifests,
)
from _openclaw_install import (
    DEFAULT_OPENCLAW_DIR,
    GatewayActivationMode,
    OpenClawReleaseInstallReport,
    install_openclaw_release,
)
from _opencode_install import (
    OpenCodeInstallReport,
    install_opencode_two_server_config,
    rollback_opencode_install,
)
from _plugin_bundle import current_runtime_platform
from _release_activation import (
    ActivationReport,
    ActivationVerificationError,
    drain_stale_processes,
    install_stable_links,
    receipt_path,
    resolve_active_runtime,
    restore_path,
    snapshot_path,
    verify_activation,
    write_receipt,
)
from release_generation import (
    CORE_ONLY_PROFILE,
    FULL_PROFILE,
    GenerationStore,
    InstallTransactionError,
)

HOSTS = ("openclaw", "opencode")


class CompleteReleaseInstallError(RuntimeError):
    """Standalone promotion or consumer configuration did not converge."""


@dataclass(frozen=True)
class CompleteReleaseInstallReport:
    release_id: str
    manifest_sha256: str
    release_root: str
    profile: str
    previous_release_id: str | None
    configured_hosts: tuple[str, ...]
    native_messaging: NativeMessagingInstallReport
    browser_extension: Mapping[str, str]
    browser_reload_required: bool
    openclaw: OpenClawReleaseInstallReport | None
    opencode: OpenCodeInstallReport | None
    activation: ActivationReport

    def as_dict(self) -> dict[str, object]:
        return {
            "release_id": self.release_id,
            "manifest_sha256": self.manifest_sha256,
            "release_root": self.release_root,
            "profile": self.profile,
            "previous_release_id": self.previous_release_id,
            "configured_hosts": list(self.configured_hosts),
            "native_messaging": self.native_messaging.as_dict(),
            "browser_extension": dict(self.browser_extension),
            "browser_reload_required": self.browser_reload_required,
            "openclaw": self.openclaw.as_dict() if self.openclaw else None,
            "opencode": self.opencode.to_dict() if self.opencode else None,
            "activation": self.activation.as_dict(),
        }


def _host_set(hosts: Iterable[str]) -> tuple[str, ...]:
    selected = tuple(dict.fromkeys(hosts))
    unknown = sorted(set(selected) - set(HOSTS))
    if unknown:
        raise CompleteReleaseInstallError(f"unsupported complete-release host(s): {unknown}")
    return selected


def install_complete_release(
    candidate: Path,
    *,
    store_root: Path,
    profile: str = FULL_PROFILE,
    expected_manifest_sha256: str | None = None,
    hosts: Iterable[str] = (),
    browser_socket_path: Path | None = None,
    openclaw_dir: Path | None = None,
    openclaw_bin: str = "openclaw",
    openclaw_gateway_activation: GatewayActivationMode = "watcher",
    opencode_config_dir: Path | None = None,
    opencode_process_env: Mapping[str, str] | None = None,
    opencode_effective_cwd: Path | None = None,
    native_messaging_home: Path | None = None,
    bin_dir: Path | None = None,
    proc_root: Path = Path("/proc"),
) -> CompleteReleaseInstallReport:
    """Promote a generation, then transactionally project both MCP servers.

    OpenCode is configured first because it exposes a durable rollback handle.
    OpenClaw is last and rolls back its own definitions on registration or
    requested-restart failure. Any host failure then restores OpenCode and the
    previous generation; a failed first install deactivates ``current`` while
    retaining the verified generation for diagnosis.
    """
    selected_hosts = _host_set(hosts)
    if selected_hosts and profile != FULL_PROFILE:
        raise CompleteReleaseInstallError(
            "OpenClaw/OpenCode two-server configuration requires the full profile"
        )
    if selected_hosts and (browser_socket_path is None or not browser_socket_path.is_absolute()):
        raise CompleteReleaseInstallError(
            "an absolute browser socket path is required when configuring hosts"
        )

    store = GenerationStore(store_root.expanduser().resolve())
    with store.transaction() as transaction:
        transaction.recover()
        prior = transaction.current_release_id()
        installed = transaction.install(
            candidate.expanduser().resolve(),
            expected_manifest_sha256=expected_manifest_sha256,
            profile=profile,
            prune=False,
        )
        native_messaging: NativeMessagingInstallReport | None = None
        opencode_report: OpenCodeInstallReport | None = None
        openclaw_report: OpenClawReleaseInstallReport | None = None
        link_snapshots = ()
        prior_receipt = snapshot_path(receipt_path(store.root))
        activation: ActivationReport | None = None
        try:
            native_messaging = install_native_messaging_manifests(
                installed.root,
                home=native_messaging_home,
            )
            if "opencode" in selected_hosts:
                assert browser_socket_path is not None
                opencode_report = install_opencode_two_server_config(
                    installed.root,
                    browser_socket_path=browser_socket_path,
                    config_dir=opencode_config_dir,
                    process_env=opencode_process_env,
                    effective_cwd=opencode_effective_cwd,
                )
            if "openclaw" in selected_hosts:
                assert browser_socket_path is not None
                openclaw_report = install_openclaw_release(
                    installed.root,
                    browser_socket_path=browser_socket_path,
                    openclaw_dir=openclaw_dir,
                    openclaw_bin=openclaw_bin,
                    gateway_activation=openclaw_gateway_activation,
                )
            stable_links, link_snapshots = install_stable_links(
                store.root,
                installed,
                bin_dir=bin_dir,
            )
            write_receipt(
                store.root,
                installed,
                native_manifest_paths=native_messaging.manifest_paths,
                stable_links=stable_links,
            )
            drain_stale_processes(store.root, proc_root=proc_root)
            transaction.prune_generations({installed.release_id})
            activation = ActivationReport(
                release_id=installed.release_id,
                manifest_sha256=installed.manifest_sha256,
                release_root=str(installed.root),
                profile=installed.profile,
                platform=current_runtime_platform(),
                receipt_path=str(receipt_path(store.root)),
                native_manifest_paths=tuple(str(path) for path in native_messaging.manifest_paths),
                stable_links=stable_links,
                stale_processes_drained=True,
            )
        except BaseException as error:
            rollback_failures: list[str] = []
            for snapshot in reversed(link_snapshots):
                try:
                    restore_path(snapshot)
                except BaseException as rollback_error:
                    rollback_failures.append(f"stable-link {snapshot.path}: {rollback_error}")
            try:
                restore_path(prior_receipt)
            except BaseException as rollback_error:
                rollback_failures.append(f"activation-receipt: {rollback_error}")
            if opencode_report is not None and opencode_report.changed:
                assert opencode_report.backup_path is not None
                try:
                    rollback_opencode_install(
                        config_path=opencode_report.config_path,
                        backup_path=opencode_report.backup_path,
                        expected_installed_sha256=opencode_report.installed_config_sha256,
                    )
                except BaseException as rollback_error:
                    rollback_failures.append(f"opencode: {rollback_error}")
            if native_messaging is not None and native_messaging.changed_paths:
                try:
                    rollback_native_messaging_manifests(native_messaging)
                except BaseException as rollback_error:
                    rollback_failures.append(f"native-messaging: {rollback_error}")
            if prior != installed.release_id:
                try:
                    if prior is None:
                        transaction.deactivate_initial_activation(installed.release_id)
                    else:
                        restored = transaction.rollback()
                        if restored.release_id != prior:
                            raise InstallTransactionError(
                                f"rollback activated {restored.release_id}, expected {prior}"
                            )
                except BaseException as rollback_error:
                    rollback_failures.append(f"generation: {rollback_error}")
            detail = f"; rollback failure(s): {rollback_failures}" if rollback_failures else ""
            raise CompleteReleaseInstallError(
                f"complete-release consumer configuration failed: {error}{detail}"
            ) from error

        assert native_messaging is not None
        assert activation is not None
        manifest = json.loads((installed.root / "RELEASE.json").read_text(encoding="utf-8"))
        extension = manifest.get("browser_contract", {}).get("extension_bridge")
        if not isinstance(extension, dict) or not all(
            isinstance(extension.get(name), str)
            for name in (
                "extension_id",
                "manifest_sha256",
                "path",
                "tree_sha256",
                "version",
            )
        ):
            raise CompleteReleaseInstallError("verified release has no Browser extension binding")
        extension_path = installed.root.joinpath(*Path(extension["path"]).parts).resolve()
        browser_extension = {
            "activation": "web_store_preinstalled",
            "extension_id": extension["extension_id"],
            "manifest_sha256": extension["manifest_sha256"],
            "path": str(extension_path),
            "tree_sha256": extension["tree_sha256"],
            "version": extension["version"],
        }
        return CompleteReleaseInstallReport(
            release_id=installed.release_id,
            manifest_sha256=installed.manifest_sha256,
            release_root=str(installed.root),
            profile=installed.profile,
            previous_release_id=prior,
            configured_hosts=selected_hosts,
            native_messaging=native_messaging,
            browser_extension=browser_extension,
            browser_reload_required=False,
            openclaw=openclaw_report,
            opencode=opencode_report,
            activation=activation,
        )


def ensure_complete_release(
    candidate: Path,
    **kwargs: object,
) -> tuple[str, CompleteReleaseInstallReport | ActivationReport]:
    """Return unchanged when artifact-derived activation proof succeeds; repair otherwise."""
    verify_kwargs = {
        name: kwargs[name]
        for name in (
            "store_root",
            "profile",
            "expected_manifest_sha256",
            "native_messaging_home",
            "bin_dir",
            "proc_root",
        )
        if name in kwargs
    }
    try:
        return "unchanged", verify_activation(candidate, **verify_kwargs)  # type: ignore[arg-type]
    except ActivationVerificationError:
        return "repaired", install_complete_release(candidate, **kwargs)  # type: ignore[arg-type]


def main(
    argv: list[str] | None = None,
    *,
    operation: str = "install",
) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path, nargs="?")
    parser.add_argument(
        "--rollback",
        action="store_true",
        help="activate the retained prior generation and reproject selected consumers",
    )
    parser.add_argument("--store-root", type=Path, default=Path.home() / ".local/share/sky-cua")
    parser.add_argument(
        "--profile", choices=(FULL_PROFILE, CORE_ONLY_PROFILE), default=FULL_PROFILE
    )
    parser.add_argument("--manifest-sha256")
    parser.add_argument("--host", action="append", choices=HOSTS, default=[])
    parser.add_argument("--browser-socket", type=Path)
    parser.add_argument("--openclaw-dir", type=Path, default=DEFAULT_OPENCLAW_DIR)
    parser.add_argument("--openclaw-bin", default="openclaw")
    parser.add_argument(
        "--openclaw-gateway-activation",
        choices=("watcher", "restart", "deferred"),
        default="watcher",
    )
    parser.add_argument("--opencode-config-dir", type=Path)
    parser.add_argument("--opencode-effective-cwd", type=Path)
    parser.add_argument("--native-messaging-home", type=Path)
    parser.add_argument("--bin-dir", type=Path)
    parser.add_argument("--proc-root", type=Path, default=Path("/proc"))
    args = parser.parse_args(argv)
    try:
        if operation not in {"install", "ensure", "verify-activation", "resolve-active"}:
            raise CompleteReleaseInstallError(f"unsupported activation operation: {operation}")
        if args.rollback:
            if operation != "install":
                raise CompleteReleaseInstallError(f"--rollback cannot be used with {operation}")
            if args.candidate is not None:
                raise CompleteReleaseInstallError(
                    "a candidate path cannot be supplied together with --rollback"
                )
            store = GenerationStore(args.store_root.expanduser().resolve())
            previous = store.previous_release_id()
            if previous is None:
                raise CompleteReleaseInstallError("no retained prior generation is available")
            candidate = store.releases / previous
        elif args.candidate is None:
            raise CompleteReleaseInstallError("a candidate release path is required")
        else:
            candidate = args.candidate
        common = {
            "store_root": args.store_root,
            "profile": args.profile,
            "expected_manifest_sha256": args.manifest_sha256,
            "native_messaging_home": args.native_messaging_home,
            "bin_dir": args.bin_dir,
            "proc_root": args.proc_root,
        }
        if operation == "verify-activation":
            activation = verify_activation(candidate, **common)
            print(json.dumps({"status": "ok", "activation": activation.as_dict()}, sort_keys=True))
            return 0
        if operation == "resolve-active":
            runtime = resolve_active_runtime(candidate, **common)
            print(json.dumps({"status": "ok", "runtime": runtime.as_dict()}, sort_keys=True))
            return 0
        install_options = {
            **common,
            "hosts": args.host,
            "browser_socket_path": args.browser_socket,
            "openclaw_dir": args.openclaw_dir,
            "openclaw_bin": args.openclaw_bin,
            "openclaw_gateway_activation": args.openclaw_gateway_activation,
            "opencode_config_dir": args.opencode_config_dir,
            "opencode_effective_cwd": args.opencode_effective_cwd,
        }
        if operation == "ensure":
            status, outcome = ensure_complete_release(candidate, **install_options)
            activation = (
                outcome.activation if isinstance(outcome, CompleteReleaseInstallReport) else outcome
            )
            print(
                json.dumps(
                    {"status": status, "activation": activation.as_dict()},
                    sort_keys=True,
                )
            )
            return 0
        report = install_complete_release(candidate, **install_options)
    except (
        ActivationVerificationError,
        CompleteReleaseInstallError,
        InstallTransactionError,
        OSError,
        ValueError,
    ) as error:
        parser.exit(2, f"error: {error}\n")
    print(json.dumps(report.as_dict(), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
