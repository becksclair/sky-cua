"""OpenClaw host registration for the sky-cua MCP server."""

from __future__ import annotations

import json
import os
import shlex
import stat
import subprocess
import sys
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Literal, cast

import _install_shared
from _install_shared import (
    project_model_skills,
    resolve_gateway_auth_env,
    restore_text_path_snapshot,
    snapshot_text_path,
    subprocess_error_detail,
    toml_basic_string,
    write_text_atomically,
)
from _openclaw_cli_transaction import (
    CommandRunner,
    OpenClawCliTransactionError,
    command_result_detail,
    restore_servers,
    run_openclaw_command,
    set_server,
    snapshot_servers,
)
from _plugin_bundle import remove_path
from release_generation import FULL_PROFILE, RELEASE_MANIFEST, VerifiedRelease, verify_release_root

DEFAULT_OPENCLAW_DIR = Path.home() / ".openclaw"
# Codex per-tool approval semantics: "approve" = always approved with no user
# interaction; "auto" = gated on MCP tool annotations, prompting for
# destructive/open-world tools. Shared by the openclaw.json projection, the
# codex-home config.toml block, and the OpenClaw smoke validator.
CODEX_TOOLS_APPROVAL_MODE = "approve"
CODEX_MCP_SERVER_TOML_BEGIN = "# >>> sky-cua mcp_servers (managed by install_mcp_server.py) >>>"
CODEX_MCP_SERVER_TOML_END = "# <<< sky-cua mcp_servers <<<"
OPENCLAW_MCP_SET_TIMEOUT_SECONDS = 30
MCP_CALLER_PROVENANCE_ENV = "SKY_CUA_MCP_CALLER_PROVENANCE"
OPENCLAW_CALLER_PROVENANCE = "openclaw"
OPTIONAL_MCP_RUNTIME_ENV = (
    "SKY_CUA_BROWSER_CONTROL_MODE",
    "SKY_CUA_CODEX_BROWSER_SOCKET_PATH",
)

OPENCLAW_RELEASE_SERVER_NAMES = ("sky_cua", "node_repl")
OPENCLAW_NODE_REPL_REQUEST_TIMEOUT_MS = 3_600_000
OPENCLAW_NODE_REPL_CONNECTION_TIMEOUT_MS = 120_000
OPENCLAW_GATEWAY_RESTART_TIMEOUT_SECONDS = 180
OPENCLAW_GATEWAY_RESTART_WAIT = "120s"
OPENCLAW_RELEASE_ROOT_ENV = "SKY_CUA_RELEASE_ROOT"
DOCUMENTATION_ROOT_ENV = "SKY_CUA_DOCUMENTATION_ROOT"
OPENCLAW_DESKTOP_SESSION_ENV = (
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "WAYLAND_DISPLAY",
    "DISPLAY",
)
NODE_REPL_PATH_ENV = "CODEX_NODE_REPL_PATH"
NODE_PATH_ENV = "NODE_REPL_NODE_PATH"
NODE_MODULE_DIRS_ENV = "NODE_REPL_NODE_MODULE_DIRS"
TRUSTED_BROWSER_SHA256S_ENV = "NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S"
PLAYWRIGHT_BROWSERS_PATH_ENV = "PLAYWRIGHT_BROWSERS_PATH"

GatewayActivationMode = Literal["watcher", "restart", "deferred"]


class OpenClawReleaseInstallError(RuntimeError):
    """The two-server OpenClaw registration transaction did not commit."""


@dataclass(frozen=True)
class OpenClawReleasePlan:
    """Verified immutable-generation paths and the two OpenClaw definitions."""

    release: VerifiedRelease
    config_path: Path
    definitions: dict[str, dict[str, object]]


@dataclass(frozen=True)
class OpenClawReleaseInstallReport:
    """Machine-readable handoff from the local config transaction."""

    release_id: str
    manifest_sha256: str
    release_root: str
    config_path: str
    registered_servers: tuple[str, ...]
    changed_servers: tuple[str, ...]
    gateway_activation: str
    gateway_detail: str
    skill_root: str
    projected_skills: tuple[str, ...]

    def as_dict(self) -> dict[str, object]:
        return {
            "release_id": self.release_id,
            "manifest_sha256": self.manifest_sha256,
            "release_root": self.release_root,
            "config_path": self.config_path,
            "registered_servers": list(self.registered_servers),
            "changed_servers": list(self.changed_servers),
            "gateway_activation": self.gateway_activation,
            "gateway_detail": self.gateway_detail,
            "skill_root": self.skill_root,
            "projected_skills": list(self.projected_skills),
        }


def _load_json_object(path: Path, *, label: str) -> dict[str, object]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise OpenClawReleaseInstallError(f"could not read {label} at {path}: {error}") from error
    if not isinstance(value, dict):
        raise OpenClawReleaseInstallError(f"{label} must contain a JSON object: {path}")
    return cast(dict[str, object], value)


def _component_root(release_root: Path, manifest: Mapping[str, object], name: str) -> Path:
    components = manifest.get("components")
    if not isinstance(components, list):
        raise OpenClawReleaseInstallError("release manifest components must be an array")
    for raw in components:
        if not isinstance(raw, dict) or raw.get("name") != name:
            continue
        relative = raw.get("path")
        if not isinstance(relative, str):
            break
        root = (release_root / relative).resolve(strict=True)
        if not root.is_relative_to(release_root) or not root.is_dir():
            break
        return root
    raise OpenClawReleaseInstallError(f"verified release is missing component {name}")


def _required_component_path(root: Path, relative: object, *, label: str) -> Path:
    if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
        raise OpenClawReleaseInstallError(f"{label} must be a relative path")
    path = (root / relative).resolve(strict=True)
    if not path.is_relative_to(root):
        raise OpenClawReleaseInstallError(f"{label} escapes its component root")
    return path


def _required_executable(root: Path, relative: object, *, label: str) -> Path:
    path = _required_component_path(root, relative, label=label)
    if not path.is_file() or path.is_symlink() or path.stat().st_mode & 0o111 == 0:
        raise OpenClawReleaseInstallError(f"{label} is not a real executable: {path}")
    return path


def _validated_launch_env(values: Mapping[str, str] | None) -> dict[str, str]:
    result: dict[str, str] = {}
    for key, value in (values or {}).items():
        if not key or "=" in key or "\x00" in key or "\x00" in value:
            raise OpenClawReleaseInstallError(f"invalid OpenClaw MCP environment entry: {key!r}")
        result[key] = value
    return result


def _desktop_session_launch_env(source: Mapping[str, str] | None = None) -> dict[str, str]:
    """Preserve the session identity OpenClaw removes from MCP child processes."""
    values = os.environ if source is None else source
    return {name: values[name] for name in OPENCLAW_DESKTOP_SESSION_ENV if values.get(name)}


def plan_openclaw_release_install(
    release_root: Path,
    *,
    browser_socket_path: Path,
    openclaw_dir: Path | None = None,
    launch_env: Mapping[str, str] | None = None,
) -> OpenClawReleasePlan:
    """Build two definitions from one fully verified immutable generation.

    ``release_root`` may be the standalone ``current`` pointer; it is resolved
    once, then every executable, module, data, and trust value is derived from
    that exact generation. No checkout or ambient Node path participates.
    """
    try:
        generation = release_root.expanduser().resolve(strict=True)
    except OSError as error:
        raise OpenClawReleaseInstallError(f"release root is unavailable: {release_root}") from error
    try:
        verified = verify_release_root(
            generation,
            profile=FULL_PROFILE,
            enforce_profile_shape=True,
        )
    except (OSError, ValueError) as error:
        raise OpenClawReleaseInstallError(f"release verification failed: {error}") from error
    required = {"core-linux-x64", "browser-js", "cua-node-linux-x64-glibc"}
    missing = required.difference(verified.component_names)
    if missing:
        raise OpenClawReleaseInstallError(
            f"OpenClaw two-server installation requires the full profile: {sorted(missing)}"
        )
    if generation.name != verified.release_id:
        raise OpenClawReleaseInstallError(
            "resolved release generation directory must be named by its release id"
        )

    socket = browser_socket_path.expanduser()
    if not socket.is_absolute():
        raise OpenClawReleaseInstallError("browser socket path must be absolute")
    socket = socket.resolve(strict=False)
    manifest = _load_json_object(generation / RELEASE_MANIFEST, label="release manifest")
    core = _component_root(generation, manifest, "core-linux-x64")
    cua_node = _component_root(generation, manifest, "cua-node-linux-x64-glibc")
    documentation = _component_root(generation, manifest, "documentation")
    runtime_manifest = _load_json_object(cua_node / "manifest.json", label="cua_node manifest")
    if (
        runtime_manifest.get("target") != "linux-x64-glibc"
        or runtime_manifest.get("node_version") != "24.14.0"
    ):
        raise OpenClawReleaseInstallError("cua_node target or Node version is incompatible")

    sky_cua = _required_executable(core, "bin/sky-cua-client", label="sky_cua MCP executable")
    node_repl = _required_executable(
        cua_node,
        runtime_manifest.get("node_repl_path"),
        label="node_repl MCP executable",
    )
    node = _required_executable(
        cua_node, runtime_manifest.get("node_path"), label="bundled Node executable"
    )
    node_modules = _required_component_path(
        cua_node, runtime_manifest.get("node_modules"), label="bundled Node module directory"
    )
    if not node_modules.is_dir():
        raise OpenClawReleaseInstallError("bundled Node module path is not a directory")
    data = runtime_manifest.get("data")
    if not isinstance(data, dict):
        raise OpenClawReleaseInstallError("cua_node manifest data inventory is missing")
    playwright = _required_component_path(
        cua_node, data.get("playwright"), label="bundled Playwright data directory"
    )
    if not playwright.is_dir():
        raise OpenClawReleaseInstallError("bundled Playwright path is not a directory")

    release_trust = manifest.get("trusted_browser_client_sha256s")
    runtime_trust = runtime_manifest.get("trusted_browser_client_sha256s")
    if (
        not isinstance(release_trust, list)
        or not release_trust
        or release_trust != runtime_trust
        or any(not isinstance(value, str) for value in release_trust)
    ):
        raise OpenClawReleaseInstallError(
            "release and cua_node trusted Browser SHA inventories do not match"
        )

    supplied_env = _validated_launch_env(launch_env)
    generation_owned_env = {
        OPENCLAW_RELEASE_ROOT_ENV,
        DOCUMENTATION_ROOT_ENV,
        "SKY_CUA_REPO_ROOT",
        MCP_CALLER_PROVENANCE_ENV,
        "SKY_CUA_CODEX_BROWSER_SOCKET_PATH",
        NODE_REPL_PATH_ENV,
        NODE_PATH_ENV,
        "CODEX_BROWSER_USE_NODE_PATH",
        NODE_MODULE_DIRS_ENV,
        TRUSTED_BROWSER_SHA256S_ENV,
        PLAYWRIGHT_BROWSERS_PATH_ENV,
    }
    for name in generation_owned_env:
        supplied_env.pop(name, None)
    common_env = {
        **supplied_env,
        OPENCLAW_RELEASE_ROOT_ENV: str(generation),
        DOCUMENTATION_ROOT_ENV: str(documentation),
        "SKY_CUA_REPO_ROOT": str(core),
        MCP_CALLER_PROVENANCE_ENV: OPENCLAW_CALLER_PROVENANCE,
        "SKY_CUA_CODEX_BROWSER_SOCKET_PATH": str(socket),
    }
    definitions: dict[str, dict[str, object]] = {
        "sky_cua": {
            "enabled": True,
            "command": str(sky_cua),
            "args": ["mcp"],
            "cwd": str(generation),
            "env": dict(common_env),
            "codex": {"defaultToolsApprovalMode": CODEX_TOOLS_APPROVAL_MODE},
        },
        "node_repl": {
            "enabled": True,
            "command": str(node_repl),
            "args": [],
            "cwd": str(generation),
            "env": {
                **common_env,
                NODE_REPL_PATH_ENV: str(node_repl),
                NODE_PATH_ENV: str(node),
                NODE_MODULE_DIRS_ENV: str(node_modules),
                TRUSTED_BROWSER_SHA256S_ENV: ",".join(cast(list[str], release_trust)),
                PLAYWRIGHT_BROWSERS_PATH_ENV: str(playwright),
            },
            "connectionTimeoutMs": OPENCLAW_NODE_REPL_CONNECTION_TIMEOUT_MS,
            "requestTimeoutMs": OPENCLAW_NODE_REPL_REQUEST_TIMEOUT_MS,
            "supportsParallelToolCalls": False,
            "codex": {"defaultToolsApprovalMode": CODEX_TOOLS_APPROVAL_MODE},
        },
    }
    state_dir = (openclaw_dir or DEFAULT_OPENCLAW_DIR).expanduser().resolve()
    return OpenClawReleasePlan(
        release=verified,
        config_path=state_dir / "openclaw.json",
        definitions=definitions,
    )


def _openclaw_command_env(openclaw_state_dir: Path) -> dict[str, str]:
    env = os.environ.copy()
    env.update(resolve_gateway_auth_env(openclaw_state_dir))
    env["OPENCLAW_STATE_DIR"] = str(openclaw_state_dir)
    env["OPENCLAW_CONFIG_PATH"] = str(openclaw_state_dir / "openclaw.json")
    return env


def install_openclaw_release(
    release_root: Path,
    *,
    browser_socket_path: Path,
    openclaw_dir: Path | None = None,
    openclaw_bin: str = "openclaw",
    launch_env: Mapping[str, str] | None = None,
    gateway_activation: GatewayActivationMode = "watcher",
    runner: CommandRunner = subprocess.run,
) -> OpenClawReleaseInstallReport:
    """Transactionally register both immutable-generation MCP definitions.

    OpenClaw's Gateway watches ``openclaw.json`` and hot-applies ``mcp``
    changes by disposing its own cached runtimes. The separate
    ``openclaw mcp reload`` command only disposes caches in that short-lived CLI
    process, so this installer never presents it as a Gateway reload. Callers
    may explicitly request a health-checked Gateway restart for deterministic
    cutover, or leave activation to the watcher and prove it later.
    """
    if gateway_activation not in {"watcher", "restart", "deferred"}:
        raise ValueError(f"unsupported Gateway activation mode: {gateway_activation}")
    effective_launch_env = {
        **_desktop_session_launch_env(),
        **dict(launch_env or {}),
    }
    plan = plan_openclaw_release_install(
        release_root,
        browser_socket_path=browser_socket_path,
        openclaw_dir=openclaw_dir,
        launch_env=effective_launch_env,
    )
    state_dir = plan.config_path.parent
    state_dir.mkdir(parents=True, exist_ok=True)
    env = _openclaw_command_env(state_dir)
    try:
        snapshots = snapshot_servers(
            runner,
            openclaw_bin,
            env,
            OPENCLAW_RELEASE_SERVER_NAMES,
            timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS,
        )
    except OpenClawCliTransactionError as error:
        raise OpenClawReleaseInstallError(str(error)) from error
    changed = tuple(
        name for name in OPENCLAW_RELEASE_SERVER_NAMES if snapshots[name] != plan.definitions[name]
    )
    try:
        for name in changed:
            try:
                result = set_server(
                    runner,
                    openclaw_bin,
                    name,
                    plan.definitions[name],
                    env,
                    timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS,
                )
            except OpenClawCliTransactionError as error:
                raise OpenClawReleaseInstallError(str(error)) from error
            if result.returncode != 0:
                raise OpenClawReleaseInstallError(
                    f"failed to register OpenClaw MCP definition {name}"
                    f"{command_result_detail(result)}"
                )
        try:
            committed = snapshot_servers(
                runner,
                openclaw_bin,
                env,
                OPENCLAW_RELEASE_SERVER_NAMES,
                timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS,
            )
        except OpenClawCliTransactionError as error:
            raise OpenClawReleaseInstallError(str(error)) from error
        if any(committed[name] != plan.definitions[name] for name in OPENCLAW_RELEASE_SERVER_NAMES):
            raise OpenClawReleaseInstallError(
                "OpenClaw did not persist the exact two-server definition set"
            )
    except BaseException as error:
        rollback_failures = restore_servers(
            runner,
            openclaw_bin,
            env,
            OPENCLAW_RELEASE_SERVER_NAMES,
            snapshots,
            timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS,
        )
        if rollback_failures:
            raise OpenClawReleaseInstallError(
                f"OpenClaw registration failed and rollback failed for {rollback_failures}"
            ) from error
        raise

    if not changed:
        activation = "unchanged"
        detail = "definitions already matched the verified generation; Gateway activation unchanged"
    elif gateway_activation == "watcher":
        activation = "gateway_watcher_pending_verification"
        detail = (
            "OpenClaw Gateway hot-reloads mcp config through its config watcher; "
            "the installer did not claim process-local 'openclaw mcp reload' as Gateway proof"
        )
    elif gateway_activation == "deferred":
        activation = "deferred"
        detail = "definitions committed; Gateway activation intentionally deferred"
    else:
        command = [
            openclaw_bin,
            "gateway",
            "restart",
            "--wait",
            OPENCLAW_GATEWAY_RESTART_WAIT,
        ]
        try:
            result = run_openclaw_command(
                runner,
                command,
                env=env,
                timeout=OPENCLAW_GATEWAY_RESTART_TIMEOUT_SECONDS,
            )
        except OpenClawCliTransactionError as error:
            rollback_failures = restore_servers(
                runner,
                openclaw_bin,
                env,
                OPENCLAW_RELEASE_SERVER_NAMES,
                snapshots,
                timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS,
            )
            if rollback_failures:
                raise OpenClawReleaseInstallError(
                    "OpenClaw Gateway restart failed and definition rollback failed for "
                    f"{rollback_failures}: {error}"
                ) from error
            raise OpenClawReleaseInstallError(
                f"OpenClaw Gateway restart failed; definitions were rolled back: {error}"
            ) from error
        else:
            if result.returncode == 0:
                activation = "gateway_restart_verified"
                detail = "OpenClaw Gateway restart command completed with its health wait"
            else:
                rollback_failures = restore_servers(
                    runner,
                    openclaw_bin,
                    env,
                    OPENCLAW_RELEASE_SERVER_NAMES,
                    snapshots,
                    timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS,
                )
                if rollback_failures:
                    raise OpenClawReleaseInstallError(
                        "OpenClaw Gateway restart failed and definition rollback failed for "
                        f"{rollback_failures}: {shlex.join(command)}"
                        f"{command_result_detail(result)}"
                    )
                raise OpenClawReleaseInstallError(
                    "OpenClaw Gateway restart failed; definitions were rolled back: "
                    f"{shlex.join(command)}{command_result_detail(result)}"
                )

    documentation_root = Path(
        cast(dict[str, str], plan.definitions["node_repl"]["env"])[DOCUMENTATION_ROOT_ENV]
    )
    try:
        projected = project_model_skills(documentation_root, state_dir / "skills")
    except BaseException as error:
        rollback_failures = restore_servers(
            runner,
            openclaw_bin,
            env,
            OPENCLAW_RELEASE_SERVER_NAMES,
            snapshots,
            timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS,
        )
        detail = (
            f"; definition rollback failed for {rollback_failures}" if rollback_failures else ""
        )
        raise OpenClawReleaseInstallError(
            f"OpenClaw model-skill projection failed: {error}{detail}"
        ) from error

    return OpenClawReleaseInstallReport(
        release_id=plan.release.release_id,
        manifest_sha256=plan.release.manifest_sha256,
        release_root=str(plan.release.root),
        config_path=str(plan.config_path),
        registered_servers=OPENCLAW_RELEASE_SERVER_NAMES,
        changed_servers=changed,
        gateway_activation=activation,
        gateway_detail=detail,
        skill_root=str(state_dir / "skills"),
        projected_skills=tuple(path.name for path in projected),
    )


def install_openclaw(
    target_dir: Path,
    client_path: Path,
    openclaw_dir: Path | None = None,
    openclaw_bin: str = "openclaw",
    resource_root: Path | None = None,
    launch_env: dict[str, str] | None = None,
) -> Path:
    """Register sky-cua with OpenClaw."""
    openclaw_state_dir = (openclaw_dir or DEFAULT_OPENCLAW_DIR).expanduser().resolve()
    openclaw_state_dir.mkdir(parents=True, exist_ok=True)
    root = (resource_root or _install_shared.REPO_ROOT).resolve()
    runtime_env = {
        name: os.environ[name] for name in OPTIONAL_MCP_RUNTIME_ENV if name in os.environ
    }
    policy_env = {
        **runtime_env,
        **dict(launch_env or {}),
        MCP_CALLER_PROVENANCE_ENV: OPENCLAW_CALLER_PROVENANCE,
    }
    codex_home_updates = plan_openclaw_agent_codex_mcp_servers(
        openclaw_state_dir, client_path, resource_root=root, launch_env=policy_env
    )
    server: dict[str, object] = {
        "enabled": True,
        "command": str(client_path),
        "args": ["mcp"],
        "cwd": str(target_dir),
        "env": {
            "SKY_CUA_REPO_ROOT": str(root),
            **policy_env,
        },
        # OpenClaw's native codex runtime projects this as Codex
        # default_tools_approval_mode; see CODEX_TOOLS_APPROVAL_MODE.
        "codex": {"defaultToolsApprovalMode": CODEX_TOOLS_APPROVAL_MODE},
    }
    snippet = {"mcp": {"servers": {"sky_cua": server}}}
    path = target_dir / "openclaw_mcp.json"

    command = [
        openclaw_bin,
        "mcp",
        "set",
        "sky_cua",
        json.dumps(server, separators=(",", ":")),
    ]
    env = os.environ.copy()
    # `openclaw mcp set` / `mcp reload` authenticate to the running Gateway; a
    # plain install shell rarely exports the gateway credentials, so fill them
    # in from the gateway env file (a value already in the environment wins).
    # Without this, the CLI can fail against a password-protected gateway.
    env.update(resolve_gateway_auth_env(openclaw_state_dir))
    env["OPENCLAW_STATE_DIR"] = str(openclaw_state_dir)
    env["OPENCLAW_CONFIG_PATH"] = str(openclaw_state_dir / "openclaw.json")
    codex_home_snapshots = snapshot_openclaw_agent_codex_mcp_server_updates(codex_home_updates)
    snippet_snapshot = snapshot_text_path(path)
    registration_committed = False
    pins_applied = False
    snippet_written = False

    def rollback() -> None:
        if pins_applied:
            restore_openclaw_agent_codex_mcp_server_snapshots(codex_home_snapshots)
        if snippet_written:
            restore_text_path_snapshot(path, snippet_snapshot)

    # Catch BaseException so an operator Ctrl-C mid-registration still rolls
    # back; after the registration commits, a post-commit failure (reload)
    # deliberately keeps the consistent committed state.
    try:
        apply_openclaw_agent_codex_mcp_server_updates(
            codex_home_updates, codex_home_snapshots, emit_messages=False
        )
        pins_applied = True
        write_text_atomically(path, json.dumps(snippet, indent=2) + "\n")
        snippet_written = True
        try:
            subprocess.run(command, check=True, env=env, timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS)
        except subprocess.TimeoutExpired as error:
            # Translate only the registration timeout; a timeout from a
            # post-commit step must not be mislabeled as a registration one.
            command_text = shlex.join(command)
            raise TimeoutError(
                "timed out registering sky-cua with OpenClaw after "
                f"{OPENCLAW_MCP_SET_TIMEOUT_SECONDS} seconds: {command_text} "
                f"(OPENCLAW_STATE_DIR={openclaw_state_dir})"
            ) from error
        registration_committed = True
        print_openclaw_agent_codex_mcp_server_updates(codex_home_updates)
        reload_openclaw_mcp_runtimes(openclaw_bin, env)
    except BaseException:
        if not registration_committed:
            rollback()
        raise
    return path


def openclaw_agent_codex_config_paths(openclaw_state_dir: Path) -> list[Path]:
    """codex-home config.toml files for every configured OpenClaw agent."""
    agents_dir = openclaw_state_dir / "agents"
    if not agents_dir.is_dir():
        return []
    return sorted(agents_dir.glob("*/agent/codex-home/config.toml"))


def install_openclaw_agent_codex_mcp_servers(
    openclaw_state_dir: Path,
    client_path: Path,
    resource_root: Path | None = None,
    launch_env: dict[str, str] | None = None,
) -> None:
    """Pin sky_cua into each agent's codex-home config.toml mcp_servers table.

    OpenClaw's native codex runtime projects mcp.servers into per-thread
    config, but that projection has runtime-state gates that can drop the
    server from a turn. The codex app-server also reads CODEX_HOME/config.toml
    at process level, which applies to every thread unconditionally, so the
    deploy pins the server in both places.
    """
    apply_openclaw_agent_codex_mcp_server_updates(
        plan_openclaw_agent_codex_mcp_servers(
            openclaw_state_dir, client_path, resource_root=resource_root, launch_env=launch_env
        )
    )


def plan_openclaw_agent_codex_mcp_servers(
    openclaw_state_dir: Path,
    client_path: Path,
    resource_root: Path | None = None,
    launch_env: dict[str, str] | None = None,
) -> list[tuple[Path, str]]:
    """Validate every OpenClaw agent codex-home config before any writes."""
    planned_updates: list[tuple[Path, str]] = []
    refused_paths: list[Path] = []
    for config_path in openclaw_agent_codex_config_paths(openclaw_state_dir):
        if config_path.is_symlink() and not config_path.exists():
            print(
                f"warning: refusing to update {config_path}: config.toml is a "
                "broken symlink; repair the link target and rerun the installer.",
                file=sys.stderr,
            )
            refused_paths.append(config_path)
            continue
        planned = plan_codex_mcp_server_toml(
            config_path, client_path, resource_root=resource_root, launch_env=launch_env
        )
        if planned is None:
            refused_paths.append(config_path)
        else:
            planned_updates.append((config_path, planned))
    if refused_paths:
        refused = ", ".join(str(path) for path in refused_paths)
        raise RuntimeError(
            "refused to update OpenClaw agent codex-home config(s): "
            f"{refused}; fix the warning(s) above and rerun the installer."
        )
    return planned_updates


def apply_openclaw_agent_codex_mcp_server_updates(
    planned_updates: list[tuple[Path, str]],
    snapshots: dict[Path, tuple[str | None, int | None]] | None = None,
    emit_messages: bool = True,
) -> None:
    if snapshots is None:
        snapshots = snapshot_openclaw_agent_codex_mcp_server_updates(planned_updates)
    written_paths: list[Path] = []
    try:
        for config_path, text in planned_updates:
            write_text_atomically(config_path, text)
            written_paths.append(config_path)
    except Exception:
        restore_openclaw_agent_codex_mcp_server_snapshots(snapshots, written_paths)
        raise
    if emit_messages:
        print_openclaw_agent_codex_mcp_server_updates(planned_updates)


def print_openclaw_agent_codex_mcp_server_updates(
    planned_updates: list[tuple[Path, str]],
) -> None:
    for config_path, _text in planned_updates:
        print(f"Pinned sky_cua mcp_servers entry in {config_path}")


def snapshot_openclaw_agent_codex_mcp_server_updates(
    planned_updates: list[tuple[Path, str]],
) -> dict[Path, tuple[str | None, int | None]]:
    return {
        config_path: (
            config_path.read_text(encoding="utf-8") if config_path.exists() else None,
            stat.S_IMODE(config_path.stat().st_mode) if config_path.exists() else None,
        )
        for config_path, _text in planned_updates
    }


def restore_openclaw_agent_codex_mcp_server_snapshots(
    snapshots: dict[Path, tuple[str | None, int | None]],
    paths: list[Path] | None = None,
) -> None:
    for path in reversed(paths or list(snapshots)):
        original_text, original_mode = snapshots[path]
        if original_text is None:
            remove_path(path)
        else:
            write_text_atomically(path, original_text, mode=original_mode)


def codex_mcp_server_toml_block(
    client_path: Path,
    resource_root: Path | None = None,
    launch_env: dict[str, str] | None = None,
) -> str:
    root = (resource_root or _install_shared.REPO_ROOT).resolve()
    env_values = {
        "SKY_CUA_REPO_ROOT": str(root),
        **dict(launch_env or {}),
        MCP_CALLER_PROVENANCE_ENV: OPENCLAW_CALLER_PROVENANCE,
    }
    rendered_env = "\n".join(
        f"{key} = {toml_basic_string(value)}" for key, value in sorted(env_values.items())
    )
    return (
        f"{CODEX_MCP_SERVER_TOML_BEGIN}\n"
        "[mcp_servers.sky_cua]\n"
        f"command = {toml_basic_string(str(client_path))}\n"
        'args = ["mcp"]\n'
        "startup_timeout_sec = 30\n"
        # Always-allow: codex "approve" mode approves every tool call without
        # user interaction. "auto" would prompt for unannotated MCP tools,
        # which codex treats as destructive and open-world by default.
        f'default_tools_approval_mode = "{CODEX_TOOLS_APPROVAL_MODE}"\n'
        "[mcp_servers.sky_cua.env]\n"
        f"{rendered_env}\n"
        f"{CODEX_MCP_SERVER_TOML_END}\n"
    )


def has_codex_mcp_server_table(text: str) -> bool:
    try:
        parsed = tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        return False
    mcp_servers = parsed.get("mcp_servers")
    return isinstance(mcp_servers, dict) and "sky_cua" in mcp_servers


def has_stray_marker_line(text: str) -> bool:
    """True when a line outside the managed span is exactly a marker line.

    Line-exact matching (after trimming whitespace) keeps marker text inside
    TOML comments and strings legal while catching stray or duplicated marker
    lines that would otherwise survive every rewrite.
    """
    markers = (CODEX_MCP_SERVER_TOML_BEGIN, CODEX_MCP_SERVER_TOML_END)
    return any(line.strip() in markers for line in text.splitlines())


def plan_codex_mcp_server_toml(
    config_path: Path,
    client_path: Path,
    resource_root: Path | None = None,
    launch_env: dict[str, str] | None = None,
) -> str | None:
    """Return updated config text for a marker-delimited sky_cua mcp_servers block.

    Returns None when the existing file cannot be updated
    safely: a corrupt marker pair, a stray marker line outside the managed
    span, an unmanaged ``[mcp_servers.sky_cua]`` table outside the markers
    (a duplicate table would make the whole agent config unparseable), or a
    result that fails TOML validation.
    """
    block = codex_mcp_server_toml_block(
        client_path, resource_root=resource_root, launch_env=launch_env
    )
    text = config_path.read_text(encoding="utf-8") if config_path.exists() else ""
    begin = text.find(CODEX_MCP_SERVER_TOML_BEGIN)
    end = text.find(CODEX_MCP_SERVER_TOML_END)
    if (begin == -1) != (end == -1) or (begin != -1 and end < begin):
        print(
            f"warning: {config_path} has a corrupt sky-cua marker block; "
            "remove the stray marker line(s) and rerun the installer.",
            file=sys.stderr,
        )
        return None
    if begin != -1:
        end += len(CODEX_MCP_SERVER_TOML_END)
        if end < len(text) and text[end] == "\n":
            end += 1
        if CODEX_MCP_SERVER_TOML_BEGIN in text[begin + 1 : end - 1]:
            print(
                f"warning: {config_path} has nested sky-cua marker blocks; "
                "remove the managed block(s) by hand and rerun the installer.",
                file=sys.stderr,
            )
            return None
        unmanaged = text[:begin] + text[end:]
        new_text = text[:begin] + block + text[end:]
    else:
        unmanaged = text
        separator = (
            "" if not text or text.endswith("\n\n") else ("\n" if text.endswith("\n") else "\n\n")
        )
        new_text = text + separator + block
    if has_stray_marker_line(unmanaged):
        print(
            f"warning: {config_path} has a corrupt sky-cua marker block; "
            "remove the stray marker line(s) and rerun the installer.",
            file=sys.stderr,
        )
        return None
    if has_codex_mcp_server_table(unmanaged):
        print(
            f"warning: {config_path} already defines [mcp_servers.sky_cua] outside "
            "the managed block; remove the hand-written table and rerun the "
            "installer (a duplicate table would break the whole config).",
            file=sys.stderr,
        )
        return None
    try:
        tomllib.loads(new_text)
    except tomllib.TOMLDecodeError as error:
        print(
            f"warning: refusing to write {config_path}: updated config fails TOML "
            f"validation ({error}); fix the file by hand and rerun the installer.",
            file=sys.stderr,
        )
        return None
    return new_text


def upsert_codex_mcp_server_toml(config_path: Path, client_path: Path) -> bool:
    """Replace or append the marker-delimited sky_cua mcp_servers block."""
    new_text = plan_codex_mcp_server_toml(config_path, client_path)
    if new_text is None:
        return False
    write_text_atomically(config_path, new_text)
    return True


def reload_openclaw_mcp_runtimes(openclaw_bin: str, env: dict[str, str]) -> None:
    """Dispose cached OpenClaw MCP runtimes so the next turn uses the new config.

    Without this, a running OpenClaw gateway keeps serving the previously
    cached sky-cua process and config until restarted. Reload failures are
    reported but non-fatal: the registration itself already succeeded. This
    includes a reload that cannot even spawn the openclaw binary (OSError /
    FileNotFoundError) — the committed registration must not be undone by a
    best-effort reload step.
    """
    command = [openclaw_bin, "mcp", "reload"]
    try:
        subprocess.run(
            command,
            check=True,
            env=env,
            timeout=OPENCLAW_MCP_SET_TIMEOUT_SECONDS,
            capture_output=True,
        )
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, OSError) as error:
        detail = subprocess_error_detail(error)
        print(
            f"warning: openclaw mcp reload failed ({error}{detail}); "
            "restart the OpenClaw gateway or run 'openclaw mcp reload' manually "
            "so agent turns pick up the new sky-cua config.",
            file=sys.stderr,
        )
