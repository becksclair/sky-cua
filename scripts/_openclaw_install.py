"""OpenClaw host registration for the sky-cua MCP server."""

from __future__ import annotations

import json
import os
import shlex
import stat
import subprocess
import sys
import tomllib
from pathlib import Path

import _install_shared
from _install_shared import (
    install_sky_cua_skills,
    restore_text_path_snapshot,
    snapshot_text_path,
    subprocess_error_detail,
    toml_basic_string,
    write_text_atomically,
)
from _plugin_bundle import remove_path

DEFAULT_OPENCLAW_DIR = Path.home() / ".openclaw"
# Codex per-tool approval semantics: "approve" = always approved with no user
# interaction; "auto" = gated on MCP tool annotations, prompting for
# destructive/open-world tools. Shared by the openclaw.json projection, the
# codex-home config.toml block, and the OpenClaw smoke validator.
CODEX_TOOLS_APPROVAL_MODE = "approve"
CODEX_MCP_SERVER_TOML_BEGIN = "# >>> sky-cua mcp_servers (managed by install_mcp_server.py) >>>"
CODEX_MCP_SERVER_TOML_END = "# <<< sky-cua mcp_servers <<<"
OPENCLAW_MCP_SET_TIMEOUT_SECONDS = 30


def install_openclaw(
    target_dir: Path,
    client_path: Path,
    openclaw_dir: Path | None = None,
    openclaw_bin: str = "openclaw",
    resource_root: Path | None = None,
) -> Path:
    """Register sky-cua with OpenClaw and copy sky-cua skills into its workspace."""
    openclaw_state_dir = (openclaw_dir or DEFAULT_OPENCLAW_DIR).expanduser().resolve()
    openclaw_state_dir.mkdir(parents=True, exist_ok=True)
    root = (resource_root or _install_shared.REPO_ROOT).resolve()
    codex_home_updates = plan_openclaw_agent_codex_mcp_servers(
        openclaw_state_dir, client_path, resource_root=root
    )
    server: dict[str, object] = {
        "enabled": True,
        "command": str(client_path),
        "args": ["mcp"],
        "cwd": str(target_dir),
        "env": {
            "SKY_CUA_REPO_ROOT": str(root),
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
    # back; after the registration commits, post-commit failures (reload,
    # skills copy) deliberately keep the consistent committed state.
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
        install_sky_cua_skills(openclaw_state_dir / "workspace" / "skills")
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
            openclaw_state_dir, client_path, resource_root=resource_root
        )
    )


def plan_openclaw_agent_codex_mcp_servers(
    openclaw_state_dir: Path,
    client_path: Path,
    resource_root: Path | None = None,
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
        planned = plan_codex_mcp_server_toml(config_path, client_path, resource_root=resource_root)
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
) -> str:
    root = (resource_root or _install_shared.REPO_ROOT).resolve()
    rendered_env = f"SKY_CUA_REPO_ROOT = {toml_basic_string(str(root))}"
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
) -> str | None:
    """Return updated config text for a marker-delimited sky_cua mcp_servers block.

    Returns None when the existing file cannot be updated
    safely: a corrupt marker pair, a stray marker line outside the managed
    span, an unmanaged ``[mcp_servers.sky_cua]`` table outside the markers
    (a duplicate table would make the whole agent config unparseable), or a
    result that fails TOML validation.
    """
    block = codex_mcp_server_toml_block(client_path, resource_root=resource_root)
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
    reported but non-fatal: the registration itself already succeeded.
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
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        detail = subprocess_error_detail(error)
        print(
            f"warning: openclaw mcp reload failed ({error}{detail}); "
            "restart the OpenClaw gateway or run 'openclaw mcp reload' manually "
            "so agent turns pick up the new sky-cua config.",
            file=sys.stderr,
        )
