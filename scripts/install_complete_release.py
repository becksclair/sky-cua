#!/usr/bin/env python3
"""Install one complete sky-cua generation and configure generic MCP hosts."""

from __future__ import annotations

import argparse
import json
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path

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
    openclaw: OpenClawReleaseInstallReport | None
    opencode: OpenCodeInstallReport | None

    def as_dict(self) -> dict[str, object]:
        return {
            "release_id": self.release_id,
            "manifest_sha256": self.manifest_sha256,
            "release_root": self.release_root,
            "profile": self.profile,
            "previous_release_id": self.previous_release_id,
            "configured_hosts": list(self.configured_hosts),
            "openclaw": self.openclaw.as_dict() if self.openclaw else None,
            "opencode": self.opencode.to_dict() if self.opencode else None,
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
    prior = store.current_release_id()
    installed = store.install(
        candidate.expanduser().resolve(),
        expected_manifest_sha256=expected_manifest_sha256,
        profile=profile,
    )
    opencode_report: OpenCodeInstallReport | None = None
    openclaw_report: OpenClawReleaseInstallReport | None = None
    try:
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
    except BaseException as error:
        rollback_failures: list[str] = []
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
        if prior != installed.release_id:
            try:
                if prior is None:
                    store.deactivate_initial_activation(installed.release_id)
                else:
                    restored = store.rollback()
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

    return CompleteReleaseInstallReport(
        release_id=installed.release_id,
        manifest_sha256=installed.manifest_sha256,
        release_root=str(installed.root),
        profile=installed.profile,
        previous_release_id=prior,
        configured_hosts=selected_hosts,
        openclaw=openclaw_report,
        opencode=opencode_report,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path)
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
    args = parser.parse_args(argv)
    try:
        report = install_complete_release(
            args.candidate,
            store_root=args.store_root,
            profile=args.profile,
            expected_manifest_sha256=args.manifest_sha256,
            hosts=args.host,
            browser_socket_path=args.browser_socket,
            openclaw_dir=args.openclaw_dir,
            openclaw_bin=args.openclaw_bin,
            openclaw_gateway_activation=args.openclaw_gateway_activation,
            opencode_config_dir=args.opencode_config_dir,
            opencode_effective_cwd=args.opencode_effective_cwd,
        )
    except (CompleteReleaseInstallError, InstallTransactionError, OSError, ValueError) as error:
        parser.exit(2, f"error: {error}\n")
    print(json.dumps(report.as_dict(), sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
