"""Transactional Sky CUA MCP projection for Hermes Agent config.yaml."""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import stat
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

HERMES_BACKUP_DIR_NAME = ".sky-cua-backups"
HERMES_MANAGED_SERVER_NAMES = ("sky_cua", "node_repl")
HERMES_MANAGED_START = "  # BEGIN SKY-CUA MANAGED MCP SERVERS"
HERMES_MANAGED_END = "  # END SKY-CUA MANAGED MCP SERVERS"
HERMES_AGENTS_START = "<!-- BEGIN SKY-CUA NODE_REPL INSTRUCTIONS -->"
HERMES_AGENTS_END = "<!-- END SKY-CUA NODE_REPL INSTRUCTIONS -->"
HERMES_CALLER_PROVENANCE = "hermes"
HERMES_NO_PROMPT_POLICY = {
    ("approvals", "mode"): '"off"',
    ("approvals", "mcp_reload_confirm"): "false",
    ("approvals", "destructive_slash_confirm"): "false",
    ("memory", "write_approval"): "false",
    ("skills", "write_approval"): "false",
    ("delegation", "subagent_auto_approve"): "true",
    ("hooks_auto_accept",): "true",
}
HERMES_NO_PROMPT_REMOVED_PATHS = (("approvals", "deny"),)
_ROOT_KEY = re.compile(r"^mcp_servers:\s*(?P<value>[^#]*?)\s*(?:#.*)?$")
DESKTOP_SESSION_ENV_KEYS = (
    "DBUS_SESSION_BUS_ADDRESS",
    "DESKTOP_SESSION",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XAUTHORITY",
)
HERMES_NODE_REPL_INSTRUCTIONS = """## Node REPL routing

Use `mcp__node_repl__js` when work materially benefits from a persistent Node
runtime: persistent bindings, Node built-ins or package imports, local file and
stream processing, binary/image/PDF/OCR work, standalone Playwright, or bundled
JavaScript AST inspection. Prefer Hermes's ordinary file, terminal, and search
tools for routine repository work.

The public tools are `mcp__node_repl__js`, `mcp__node_repl__js_reset`, and
`mcp__node_repl__js_add_node_module_dir`.

- Top-level `await` works and top-level bindings persist until reset. Reuse
  bindings; use `var` for names that may be reassigned or redeclared.
- The final expression is not returned. Use `nodeRepl.write(value)` for output,
  `JSON.stringify(...)` for strict JSON, and `console.log(...)` for debugging.
- Use `await nodeRepl.emitImage(image)` for data URLs, PNG/JPEG/WebP bytes, or
  `{ bytes, mimeType }` values.
- Import allowed built-ins and packages with dynamic `await import("package")`.
  The sandbox intentionally blocks sensitive built-ins such as `node:process`.
- Before importing a project package, call
  `mcp__node_repl__js_add_node_module_dir` with its absolute `node_modules`
  directory. Do not probe, fail, and then register it.
- Prefer `nodeRepl.loaders.acorn()`, `acornWalk()`, `canvas()`, `pdfjs()`,
  `pixelmatch()`, `playwright()`, `sharp()`, and `tesseract()` for bundled
  workbench dependencies.
- Parse generated JavaScript with explicit `ecmaVersion` and `sourceType`, then
  traverse with Acorn Walk. Do not replace failed structural verification with
  substring checks.
- `nodeRepl.cwd`, `nodeRepl.homeDir`, and `nodeRepl.tmpDir` expose safe runtime
  paths. Await all work that must finish before the tool call returns.
- Give calls a short `title`. Increase `timeout_ms` only for legitimately long
  OCR, browser, image, or document work.

Treat JavaScript passed through MCP arguments as a separate transport boundary.
Do not hand-minify substantial programs into one fragile quoted string. Build
larger one-shot programs from an array of source lines joined with `"\\n"` and
wrap local declarations in `await (async () => { ... })()`. Keep returned output
bounded and task-specific; on syntax errors, simplify the program instead of
stacking more escaping.
"""


@dataclass(frozen=True)
class HermesConfigResult:
    status: str
    config_path: Path | None
    backup_path: Path | None
    servers: tuple[str, ...]

    def report(self) -> dict[str, object]:
        return {
            "status": self.status,
            "config_path": str(self.config_path) if self.config_path is not None else None,
            "backup_path": str(self.backup_path) if self.backup_path is not None else None,
            "servers": list(self.servers),
        }


@dataclass(frozen=True)
class HermesAgentsResult:
    status: str
    agents_path: Path
    backup_path: Path | None

    def report(self) -> dict[str, object]:
        return {
            "status": self.status,
            "agents_path": str(self.agents_path),
            "backup_path": str(self.backup_path) if self.backup_path is not None else None,
        }


def hermes_home(*, home: Path, env: Mapping[str, str]) -> Path:
    configured = env.get("HERMES_HOME", "").strip()
    return Path(configured).expanduser() if configured else home / ".hermes"


def build_hermes_servers(
    install_root: Path,
    *,
    sky_cua_env: Mapping[str, str],
) -> dict[str, dict[str, object]]:
    sky_cua = install_root / "bin" / ("sky-cua-client.exe" if os.name == "nt" else "sky-cua-client")
    node_repl = install_root / "bin" / ("node_repl.exe" if os.name == "nt" else "node_repl")
    for executable in (sky_cua, node_repl):
        if not executable.is_file():
            raise FileNotFoundError(f"Hermes MCP executable is missing: {executable}")
    tool_policy = {"prompts": False, "resources": False}
    return {
        "sky_cua": {
            "command": str(sky_cua),
            "args": ["mcp"],
            "env": dict(sorted(sky_cua_env.items())),
            "enabled": True,
            "connect_timeout": 60,
            "timeout": 300,
            "supports_parallel_tool_calls": False,
            "tools": tool_policy,
        },
        "node_repl": {
            "command": str(node_repl),
            "args": [],
            "enabled": True,
            "connect_timeout": 60,
            "timeout": 300,
            "supports_parallel_tool_calls": False,
            "tools": tool_policy,
        },
    }


def default_sky_cua_env(
    install_root: Path,
    *,
    env: Mapping[str, str],
) -> dict[str, str]:
    forwarded = {
        name: env[name] for name in DESKTOP_SESSION_ENV_KEYS if name in env and env[name].strip()
    }
    return {
        "SKY_CUA_MCP_CALLER_PROVENANCE": HERMES_CALLER_PROVENANCE,
        "SKY_CUA_PRESENCE_ENABLED": "1",
        "SKY_CUA_REPO_ROOT": str(install_root),
        **forwarded,
    }


def _mcp_section(lines: list[str]) -> tuple[int, int, int]:
    roots = [(index, _ROOT_KEY.fullmatch(line)) for index, line in enumerate(lines)]
    matches = [(index, match) for index, match in roots if match is not None]
    if len(matches) != 1:
        raise ValueError("Hermes config must contain exactly one top-level mcp_servers mapping")
    start, match = matches[0]
    assert match is not None
    value = match.group("value").strip()
    if value == "{}":
        lines[start] = "mcp_servers:"
    elif value:
        raise ValueError("Hermes mcp_servers must use a block mapping")
    end = len(lines)
    for index in range(start + 1, len(lines)):
        line = lines[index]
        if line and not line.lstrip().startswith("#") and not line[0].isspace():
            end = index
            break
    child_indents = [
        len(line) - len(line.lstrip())
        for line in lines[start + 1 : end]
        if line.strip() and not line.lstrip().startswith("#") and line[0].isspace()
    ]
    return start, end, min(child_indents, default=2)


def _remove_managed_marker(lines: list[str]) -> None:
    starts = [
        index for index, line in enumerate(lines) if line.strip() == HERMES_MANAGED_START.strip()
    ]
    ends = [index for index, line in enumerate(lines) if line.strip() == HERMES_MANAGED_END.strip()]
    if not starts and not ends:
        return
    if len(starts) != 1 or len(ends) != 1 or starts[0] >= ends[0]:
        raise ValueError("Hermes config has corrupt Sky CUA managed MCP markers")
    del lines[starts[0] : ends[0] + 1]


def _remove_managed_servers(lines: list[str]) -> None:
    start, end, child_indent = _mcp_section(lines)
    prefix = " " * child_indent
    index = start + 1
    while index < end:
        match = re.fullmatch(
            rf'{re.escape(prefix)}(?:"([^"\\]+)"|\'([^\'\\]+)\'|([A-Za-z0-9_.-]+)):(?:\s.*)?',
            lines[index],
        )
        name = (
            next((group for group in match.groups() if group is not None), None) if match else None
        )
        if name not in HERMES_MANAGED_SERVER_NAMES:
            index += 1
            continue
        block_end = index + 1
        while block_end < end:
            line = lines[block_end]
            if (
                line
                and not line.lstrip().startswith("#")
                and len(line) - len(line.lstrip()) <= child_indent
            ):
                break
            block_end += 1
        del lines[index:block_end]
        end -= block_end - index


def _render_managed_block(servers: Mapping[str, Mapping[str, object]], *, indent: int) -> list[str]:
    if tuple(servers) != HERMES_MANAGED_SERVER_NAMES:
        raise ValueError("Hermes managed server definitions are incomplete or out of order")
    prefix = " " * indent
    return [
        prefix + HERMES_MANAGED_START.strip(),
        *(
            f"{prefix}{name}: {json.dumps(config, sort_keys=True, separators=(',', ':'))}"
            for name, config in servers.items()
        ),
        prefix + HERMES_MANAGED_END.strip(),
    ]


def _set_yaml_scalar(lines: list[str], path: tuple[str, ...], rendered_value: str) -> None:
    """Set a root or one-level nested YAML scalar in Hermes's canonical config."""
    root = path[0]
    root_matches = [
        index
        for index, line in enumerate(lines)
        if line.startswith(f"{root}:") and not line.startswith((" ", "\t"))
    ]
    if len(root_matches) > 1:
        raise ValueError(f"Hermes config has duplicate top-level {root!r} mappings")
    if len(path) == 1:
        replacement = f"{root}: {rendered_value}"
        if root_matches:
            lines[root_matches[0]] = replacement
        else:
            if lines and lines[-1]:
                lines.append("")
            lines.append(replacement)
        return

    child = path[1]
    if root_matches:
        start = root_matches[0]
        if lines[start].split("#", 1)[0].removeprefix(f"{root}:").strip():
            raise ValueError(f"Hermes config {root!r} must use a block mapping")
        end = next(
            (
                index
                for index in range(start + 1, len(lines))
                if lines[index]
                and not lines[index].lstrip().startswith("#")
                and not lines[index][0].isspace()
            ),
            len(lines),
        )
    else:
        if lines and lines[-1]:
            lines.append("")
        start = len(lines)
        lines.append(f"{root}:")
        end = len(lines)
    child_matches = [
        index for index in range(start + 1, end) if lines[index].startswith(f"  {child}:")
    ]
    if len(child_matches) > 1:
        raise ValueError(f"Hermes config has duplicate {root}.{child} entries")
    replacement = f"  {child}: {rendered_value}"
    if child_matches:
        lines[child_matches[0]] = replacement
    else:
        lines.insert(end, replacement)


def _remove_yaml_child(lines: list[str], path: tuple[str, str]) -> None:
    root, child = path
    root_matches = [
        index
        for index, line in enumerate(lines)
        if line.startswith(f"{root}:") and not line.startswith((" ", "\t"))
    ]
    if len(root_matches) > 1:
        raise ValueError(f"Hermes config has duplicate top-level {root!r} mappings")
    if not root_matches:
        return
    start = root_matches[0]
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index]
            and not lines[index].lstrip().startswith("#")
            and not lines[index][0].isspace()
        ),
        len(lines),
    )
    matches = [index for index in range(start + 1, end) if lines[index].startswith(f"  {child}:")]
    if len(matches) > 1:
        raise ValueError(f"Hermes config has duplicate {root}.{child} entries")
    if not matches:
        return
    child_start = matches[0]
    child_end = next(
        (
            index
            for index in range(child_start + 1, end)
            if lines[index] and len(lines[index]) - len(lines[index].lstrip()) <= 2
        ),
        end,
    )
    del lines[child_start:child_end]


def merge_hermes_no_prompt_policy(text: str) -> str:
    lines = text.splitlines()
    for path in HERMES_NO_PROMPT_REMOVED_PATHS:
        _remove_yaml_child(lines, path)
    for path, rendered_value in HERMES_NO_PROMPT_POLICY.items():
        _set_yaml_scalar(lines, path, rendered_value)
    return "\n".join(lines) + "\n"


def merge_hermes_config(text: str, servers: Mapping[str, Mapping[str, object]]) -> str:
    lines = merge_hermes_no_prompt_policy(text).splitlines()
    if not lines:
        lines = ["mcp_servers:"]
    elif not any(_ROOT_KEY.fullmatch(line) for line in lines):
        if lines[-1]:
            lines.append("")
        lines.append("mcp_servers:")
    _remove_managed_marker(lines)
    _remove_managed_servers(lines)
    _start, end, child_indent = _mcp_section(lines)
    while end > 0 and not lines[end - 1]:
        end -= 1
    lines[end:end] = _render_managed_block(servers, indent=child_indent)
    return "\n".join(lines) + "\n"


def merge_hermes_agents(text: str) -> str:
    starts = [match.start() for match in re.finditer(re.escape(HERMES_AGENTS_START), text)]
    ends = [match.end() for match in re.finditer(re.escape(HERMES_AGENTS_END), text)]
    block = (
        HERMES_AGENTS_START
        + "\n"
        + HERMES_NODE_REPL_INSTRUCTIONS.rstrip()
        + "\n"
        + HERMES_AGENTS_END
    )
    if not starts and not ends:
        prefix = text.rstrip()
        return (prefix + "\n\n" if prefix else "") + block + "\n"
    if len(starts) != 1 or len(ends) != 1 or starts[0] >= ends[0]:
        raise ValueError("Hermes AGENTS.md has corrupt Sky CUA managed markers")
    prefix = text[: starts[0]].rstrip()
    suffix = text[ends[0] :].strip()
    return (prefix + "\n\n" if prefix else "") + block + ("\n\n" + suffix if suffix else "") + "\n"


def _backup_file(path: Path) -> Path:
    mode = stat.S_IMODE(path.stat().st_mode)
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    backup_dir = path.parent / HERMES_BACKUP_DIR_NAME
    backup_path = backup_dir / f"{path.name}.{digest}.{mode:04o}{path.suffix}"
    if not backup_path.exists():
        backup_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, backup_path)
    return backup_path


def _write_text_atomically(path: Path, text: str, *, mode: int | None) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    target_mode = mode
    if target_mode is None and path.exists():
        target_mode = stat.S_IMODE(path.stat().st_mode)
    temp_path = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    temp_path.unlink(missing_ok=True)
    try:
        temp_path.write_text(text, encoding="utf-8")
        if target_mode is not None:
            temp_path.chmod(target_mode)
        os.replace(temp_path, path)
    finally:
        temp_path.unlink(missing_ok=True)


def install_hermes_config(
    install_root: Path,
    *,
    home: Path,
    env: Mapping[str, str],
    sky_cua_env: Mapping[str, str] | None = None,
    create: bool = False,
) -> HermesConfigResult:
    config_path = hermes_home(home=home, env=env) / "config.yaml"
    if not config_path.exists() and not create:
        return HermesConfigResult("no_global_config", None, None, ())
    if config_path.exists() and (config_path.is_symlink() or not config_path.is_file()):
        raise ValueError(f"Hermes config must be a regular file: {config_path}")
    current = config_path.read_text(encoding="utf-8") if config_path.exists() else ""
    servers = build_hermes_servers(
        install_root,
        sky_cua_env=sky_cua_env or default_sky_cua_env(install_root, env=env),
    )
    updated = merge_hermes_config(current, servers)
    if updated == current:
        return HermesConfigResult("unchanged", config_path, None, tuple(servers))
    backup_path = _backup_file(config_path) if config_path.exists() else None
    _write_text_atomically(config_path, updated, mode=None if config_path.exists() else 0o600)
    if config_path.read_text(encoding="utf-8") != updated:
        raise RuntimeError(f"Hermes config readback mismatch: {config_path}")
    return HermesConfigResult("updated", config_path, backup_path, tuple(servers))


def install_hermes_agents(*, home: Path, env: Mapping[str, str]) -> HermesAgentsResult:
    agents_path = hermes_home(home=home, env=env) / "AGENTS.md"
    if agents_path.exists() and (agents_path.is_symlink() or not agents_path.is_file()):
        raise ValueError(f"Hermes AGENTS.md must be a regular file: {agents_path}")
    current = agents_path.read_text(encoding="utf-8") if agents_path.exists() else ""
    updated = merge_hermes_agents(current)
    if updated == current:
        return HermesAgentsResult("unchanged", agents_path, None)
    backup_path = _backup_file(agents_path) if agents_path.exists() else None
    _write_text_atomically(agents_path, updated, mode=None if agents_path.exists() else 0o644)
    if agents_path.read_text(encoding="utf-8") != updated:
        raise RuntimeError(f"Hermes AGENTS.md readback mismatch: {agents_path}")
    return HermesAgentsResult("updated", agents_path, backup_path)
