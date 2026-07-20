"""OpenCode consumer adapter for one verified immutable sky-cua generation.

This module owns only the global OpenCode JSON/JSONC projection.  Release
installation and verification remain in :mod:`release_generation`; callers
must restart every OpenCode process after a changed projection is committed.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import re
import stat
from collections.abc import Callable, Mapping
from dataclasses import asdict, dataclass, field
from pathlib import Path, PurePosixPath
from typing import cast

from _install_shared import write_text_atomically
from release_generation import (
    FULL_PROFILE,
    RELEASE_MANIFEST,
    VerifiedRelease,
    verify_release_root,
)

GLOBAL_CONFIG_NAMES = ("opencode.json", "opencode.jsonc")
MANAGED_SERVER_NAMES = ("sky_cua", "node_repl")
OPENCODE_CALLER_PROVENANCE = "opencode"
DEFAULT_MCP_TIMEOUT_MS = 30_000
RESTART_SCOPE = "all_running_opencode_processes"
MANAGED_CONFIG_DIR = Path("/etc/opencode")


class OpenCodeInstallError(RuntimeError):
    """The OpenCode projection could not be updated without ambiguity."""


@dataclass(frozen=True)
class OpenCodeInstallReport:
    """Machine-readable result consumed by the standalone installer controller."""

    config_path: Path
    release_root: Path
    release_id: str
    manifest_sha256: str
    changed: bool
    backup_path: Path | None
    installed_config_sha256: str
    server_names: tuple[str, ...] = MANAGED_SERVER_NAMES
    restart_required: bool = True
    restart_scope: str = RESTART_SCOPE
    activation_status: str = "pending_full_process_restart"
    restart_instruction: str = (
        "Terminate every running OpenCode CLI, TUI, server, and desktop process, then start "
        "a new process; session reload and MCP reconnect are not sufficient."
    )

    def to_dict(self) -> dict[str, object]:
        result = asdict(self)
        result["config_path"] = str(self.config_path)
        result["release_root"] = str(self.release_root)
        result["backup_path"] = str(self.backup_path) if self.backup_path else None
        result["server_names"] = list(self.server_names)
        return result


@dataclass(frozen=True)
class _Token:
    kind: str
    start: int
    end: int
    value: object = None


@dataclass(frozen=True)
class _Entry:
    key: str
    key_token: _Token
    value: _Node
    comma_end: int | None


@dataclass(frozen=True)
class _Node:
    kind: str
    start: int
    end: int
    value: object
    open_end: int | None = None
    close_start: int | None = None
    entries: tuple[_Entry, ...] = field(default_factory=tuple)

    def object_entries(self) -> dict[str, _Entry]:
        if self.kind != "object":
            raise OpenCodeInstallError("expected a JSONC object")
        return {entry.key: entry for entry in self.entries}


_NUMBER = re.compile(r"-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?")


def _tokenize_jsonc(text: str) -> list[_Token]:
    tokens: list[_Token] = []
    index = 0
    while index < len(text):
        char = text[index]
        if char.isspace():
            index += 1
            continue
        if text.startswith("//", index):
            newline = text.find("\n", index + 2)
            index = len(text) if newline == -1 else newline + 1
            continue
        if text.startswith("/*", index):
            end = text.find("*/", index + 2)
            if end == -1:
                raise OpenCodeInstallError("unterminated block comment in OpenCode config")
            index = end + 2
            continue
        if char in "{}[]:,":
            tokens.append(_Token(char, index, index + 1, char))
            index += 1
            continue
        if char == '"':
            end = index + 1
            escaped = False
            while end < len(text):
                current = text[end]
                if escaped:
                    escaped = False
                elif current == "\\":
                    escaped = True
                elif current == '"':
                    end += 1
                    break
                elif current in "\r\n":
                    raise OpenCodeInstallError("newline in JSONC string")
                end += 1
            else:
                raise OpenCodeInstallError("unterminated JSONC string")
            raw = text[index:end]
            try:
                value = json.loads(raw)
            except json.JSONDecodeError as error:
                raise OpenCodeInstallError(f"invalid JSONC string at byte {index}") from error
            tokens.append(_Token("string", index, end, value))
            index = end
            continue
        number = _NUMBER.match(text, index)
        if number is not None:
            raw = number.group(0)
            tokens.append(_Token("number", index, number.end(), json.loads(raw)))
            index = number.end()
            continue
        matched_literal = False
        for raw, value in (("true", True), ("false", False), ("null", None)):
            if text.startswith(raw, index):
                tokens.append(_Token("literal", index, index + len(raw), value))
                index += len(raw)
                matched_literal = True
                break
        if matched_literal:
            continue
        raise OpenCodeInstallError(f"invalid JSONC token at byte {index}")
    return tokens


class _JsoncParser:
    def __init__(self, text: str) -> None:
        self._tokens = _tokenize_jsonc(text)

    def parse(self) -> _Node:
        if not self._tokens:
            raise OpenCodeInstallError("OpenCode config is empty")
        node, index = self._parse_value(0)
        if index != len(self._tokens):
            raise OpenCodeInstallError(
                f"unexpected JSONC content at byte {self._tokens[index].start}"
            )
        return node

    def _parse_value(self, index: int) -> tuple[_Node, int]:
        if index >= len(self._tokens):
            raise OpenCodeInstallError("unexpected end of OpenCode config")
        token = self._tokens[index]
        if token.kind == "{":
            return self._parse_object(index)
        if token.kind == "[":
            return self._parse_array(index)
        if token.kind in {"string", "number", "literal"}:
            return _Node(token.kind, token.start, token.end, token.value), index + 1
        raise OpenCodeInstallError(f"expected JSONC value at byte {token.start}")

    def _parse_object(self, index: int) -> tuple[_Node, int]:
        opening = self._tokens[index]
        index += 1
        entries: list[_Entry] = []
        values: dict[str, object] = {}
        if index < len(self._tokens) and self._tokens[index].kind == "}":
            closing = self._tokens[index]
            return (
                _Node(
                    "object",
                    opening.start,
                    closing.end,
                    values,
                    opening.end,
                    closing.start,
                ),
                index + 1,
            )
        while index < len(self._tokens):
            key = self._tokens[index]
            if key.kind != "string" or not isinstance(key.value, str):
                raise OpenCodeInstallError(f"expected object key at byte {key.start}")
            if key.value in values:
                raise OpenCodeInstallError(f"duplicate JSONC object key {key.value!r}")
            index += 1
            if index >= len(self._tokens) or self._tokens[index].kind != ":":
                raise OpenCodeInstallError(f"expected ':' after object key {key.value!r}")
            value, index = self._parse_value(index + 1)
            comma_end: int | None = None
            if index < len(self._tokens) and self._tokens[index].kind == ",":
                comma_end = self._tokens[index].end
                index += 1
            entries.append(_Entry(key.value, key, value, comma_end))
            values[key.value] = value.value
            if index >= len(self._tokens):
                break
            if self._tokens[index].kind == "}":
                closing = self._tokens[index]
                return (
                    _Node(
                        "object",
                        opening.start,
                        closing.end,
                        values,
                        opening.end,
                        closing.start,
                        tuple(entries),
                    ),
                    index + 1,
                )
            if comma_end is None:
                raise OpenCodeInstallError(f"expected ',' before byte {self._tokens[index].start}")
        raise OpenCodeInstallError("unterminated JSONC object")

    def _parse_array(self, index: int) -> tuple[_Node, int]:
        opening = self._tokens[index]
        index += 1
        values: list[object] = []
        if index < len(self._tokens) and self._tokens[index].kind == "]":
            closing = self._tokens[index]
            return _Node("array", opening.start, closing.end, values), index + 1
        while index < len(self._tokens):
            value, index = self._parse_value(index)
            values.append(value.value)
            comma = index < len(self._tokens) and self._tokens[index].kind == ","
            if comma:
                index += 1
            if index < len(self._tokens) and self._tokens[index].kind == "]":
                closing = self._tokens[index]
                return _Node("array", opening.start, closing.end, values), index + 1
            if not comma:
                byte = self._tokens[index].start if index < len(self._tokens) else len(self._tokens)
                raise OpenCodeInstallError(f"expected ',' in array before byte {byte}")
        raise OpenCodeInstallError("unterminated JSONC array")


def parse_jsonc(text: str) -> object:
    """Parse JSONC while rejecting ambiguous duplicate object keys."""
    return _JsoncParser(text).parse().value


def _line_indent(text: str, position: int) -> str:
    start = text.rfind("\n", 0, position) + 1
    prefix = text[start:position]
    return prefix if prefix.strip() == "" else ""


def _render_nested(value: object, property_indent: str) -> str:
    rendered = json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False)
    lines = rendered.splitlines()
    return lines[0] + "".join(f"\n{property_indent}{line}" for line in lines[1:])


def _property_text(name: str, value: object, indent: str) -> str:
    return f"{indent}{json.dumps(name)}: {_render_nested(value, indent)}"


def _insert_object_properties(
    text: str, node: _Node, properties: Mapping[str, object]
) -> tuple[int, str]:
    if node.kind != "object" or node.open_end is None or node.close_start is None:
        raise OpenCodeInstallError("cannot add properties to a non-object")
    parent_indent = _line_indent(text, node.close_start)
    child_indent = parent_indent + "  "
    rendered = ",\n".join(
        _property_text(name, value, child_indent) for name, value in properties.items()
    )
    if not node.entries:
        return node.open_end, f"\n{rendered}\n{parent_indent}"
    last = node.entries[-1]
    insertion = last.comma_end if last.comma_end is not None else last.value.end
    prefix = "" if last.comma_end is not None else ","
    return insertion, f"{prefix}\n{rendered}"


def merge_managed_servers(text: str, servers: Mapping[str, object]) -> str:
    """Return a targeted JSONC edit that preserves every unrelated source byte."""
    root = _JsoncParser(text).parse()
    if root.kind != "object":
        raise OpenCodeInstallError("OpenCode config root must be an object")
    root_entries = root.object_entries()
    mcp_entry = root_entries.get("mcp")
    edits: list[tuple[int, int, str]] = []
    if mcp_entry is None:
        insertion, rendered = _insert_object_properties(text, root, {"mcp": dict(servers)})
        edits.append((insertion, insertion, rendered))
    else:
        mcp = mcp_entry.value
        if mcp.kind != "object":
            raise OpenCodeInstallError("OpenCode config mcp must be an object")
        existing = mcp.object_entries()
        missing: dict[str, object] = {}
        for name, server in servers.items():
            entry = existing.get(name)
            if entry is None:
                missing[name] = server
                continue
            indent = _line_indent(text, entry.key_token.start)
            edits.append((entry.value.start, entry.value.end, _render_nested(server, indent)))
        if missing:
            insertion, rendered = _insert_object_properties(text, mcp, missing)
            edits.append((insertion, insertion, rendered))

    merged = text
    for start, end, replacement in sorted(edits, reverse=True):
        merged = merged[:start] + replacement + merged[end:]
    parsed = parse_jsonc(merged)
    if not isinstance(parsed, dict):
        raise OpenCodeInstallError("updated OpenCode config root is not an object")
    parsed_mcp = parsed.get("mcp")
    if not isinstance(parsed_mcp, dict):
        raise OpenCodeInstallError("updated OpenCode config mcp is not an object")
    for name, expected in servers.items():
        if parsed_mcp.get(name) != expected:
            raise OpenCodeInstallError(f"updated OpenCode config did not preserve {name}")
    return merged


def _canonical_generation(release_root: Path) -> tuple[Path, VerifiedRelease, dict[str, object]]:
    try:
        canonical = release_root.expanduser().resolve(strict=True)
    except OSError as error:
        raise OpenCodeInstallError(f"release root is unavailable: {release_root}") from error
    if not canonical.is_dir() or canonical.is_symlink():
        raise OpenCodeInstallError(f"release root is not a real directory: {canonical}")
    try:
        verified = verify_release_root(
            canonical,
            profile=FULL_PROFILE,
            enforce_profile_shape=True,
        )
    except (OSError, ValueError) as error:
        raise OpenCodeInstallError(f"release verification failed: {error}") from error
    if verified.profile != "full":
        raise OpenCodeInstallError("OpenCode requires a verified full installation generation")
    if canonical.name != verified.release_id:
        raise OpenCodeInstallError(
            "resolved release generation directory must be named by its release id"
        )
    manifest_value = json.loads((canonical / RELEASE_MANIFEST).read_text(encoding="utf-8"))
    if not isinstance(manifest_value, dict):
        raise OpenCodeInstallError("verified RELEASE.json root must be an object")
    return canonical, verified, cast(dict[str, object], manifest_value)


def _component_root(
    release_root: Path, manifest: Mapping[str, object], component_name: str
) -> Path:
    components = manifest.get("components")
    if not isinstance(components, list):
        raise OpenCodeInstallError("RELEASE.json components must be an array")
    record = next(
        (
            item
            for item in components
            if isinstance(item, dict) and item.get("name") == component_name
        ),
        None,
    )
    if not isinstance(record, dict) or not isinstance(record.get("path"), str):
        raise OpenCodeInstallError(f"release component is missing: {component_name}")
    relative = PurePosixPath(record["path"])
    if relative.is_absolute() or ".." in relative.parts:
        raise OpenCodeInstallError(f"invalid component path for {component_name}")
    candidate = release_root.joinpath(*relative.parts)
    try:
        resolved = candidate.resolve(strict=True)
    except OSError as error:
        raise OpenCodeInstallError(f"component is unavailable: {component_name}") from error
    if not resolved.is_relative_to(release_root) or not resolved.is_dir() or candidate.is_symlink():
        raise OpenCodeInstallError(
            f"component is not an immutable release directory: {component_name}"
        )
    return resolved


def _required_file(root: Path, relative: str, *, executable: bool = False) -> Path:
    path = root.joinpath(*PurePosixPath(relative).parts)
    if path.is_symlink() or not path.is_file():
        raise OpenCodeInstallError(f"required release file is missing: {path}")
    if executable and not os.access(path, os.X_OK):
        raise OpenCodeInstallError(f"required release file is not executable: {path}")
    return path.resolve(strict=True)


def _required_directory(root: Path, relative: str) -> Path:
    path = root.joinpath(*PurePosixPath(relative).parts)
    if path.is_symlink() or not path.is_dir():
        raise OpenCodeInstallError(f"required release directory is missing: {path}")
    return path.resolve(strict=True)


def build_opencode_servers(
    release_root: Path,
    manifest: Mapping[str, object],
    *,
    browser_socket_path: Path,
    timeout_ms: int = DEFAULT_MCP_TIMEOUT_MS,
) -> dict[str, object]:
    """Build both local MCP definitions from one already-verified generation."""
    if not isinstance(timeout_ms, int) or isinstance(timeout_ms, bool) or timeout_ms < 1:
        raise OpenCodeInstallError("OpenCode MCP timeout must be a positive integer")
    if not browser_socket_path.is_absolute():
        raise OpenCodeInstallError("Browser bridge socket path must be absolute")
    normalized_socket_path = Path(os.path.abspath(browser_socket_path))

    core = _component_root(release_root, manifest, "core-linux-x64")
    cua_node = _component_root(release_root, manifest, "cua-node-linux-x64-glibc")
    client = _required_file(core, "bin/sky-cua-client", executable=True)
    node_repl = _required_file(cua_node, "bin/node_repl", executable=True)
    node = _required_file(cua_node, "bin/node", executable=True)
    modules = _required_directory(cua_node, "lib/node_modules")
    playwright = _required_directory(cua_node, "share/playwright")

    trusted = manifest.get("trusted_browser_client_sha256s")
    if (
        not isinstance(trusted, list)
        or not trusted
        or not all(
            isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value) for value in trusted
        )
    ):
        raise OpenCodeInstallError("release Browser trust hashes are invalid")
    if len(set(cast(list[str], trusted))) != len(trusted):
        raise OpenCodeInstallError("release Browser trust hashes contain duplicates")
    trust_value = ",".join(cast(list[str], trusted))
    shared_env = {
        "SKY_CUA_CODEX_BROWSER_SOCKET_PATH": str(normalized_socket_path),
        "SKY_CUA_MCP_CALLER_PROVENANCE": OPENCODE_CALLER_PROVENANCE,
        "SKY_CUA_RELEASE_ROOT": str(release_root),
        "SKY_CUA_REPO_ROOT": str(core),
    }
    return {
        "sky_cua": {
            "type": "local",
            "command": [str(client), "mcp"],
            "cwd": str(release_root),
            "environment": shared_env,
            "enabled": True,
            "timeout": timeout_ms,
        },
        "node_repl": {
            "type": "local",
            "command": [str(node_repl)],
            "cwd": str(release_root),
            "environment": {
                **shared_env,
                "CODEX_NODE_REPL_PATH": str(node_repl),
                "NODE_REPL_NODE_MODULE_DIRS": str(modules),
                "NODE_REPL_NODE_PATH": str(node),
                "NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S": trust_value,
                "PLAYWRIGHT_BROWSERS_PATH": str(playwright),
            },
            "enabled": True,
            "timeout": timeout_ms,
        },
    }


def _config_path(config_dir: Path) -> Path:
    if config_dir.is_symlink():
        raise OpenCodeInstallError(f"OpenCode config directory must not be a symlink: {config_dir}")
    existing = [config_dir / name for name in GLOBAL_CONFIG_NAMES if (config_dir / name).exists()]
    if len(existing) > 1:
        raise OpenCodeInstallError(
            "both global opencode.json and opencode.jsonc exist; remove the ambiguous duplicate"
        )
    path = existing[0] if existing else config_dir / "opencode.jsonc"
    if path.is_symlink():
        raise OpenCodeInstallError(f"OpenCode global config must not be a symlink: {path}")
    if path.exists() and not path.is_file():
        raise OpenCodeInstallError(f"OpenCode global config is not a regular file: {path}")
    return path


def _project_config_hazards(cwd: Path) -> list[str]:
    hazards: list[str] = []
    current = cwd.expanduser().resolve(strict=False)
    while True:
        candidates = [current / name for name in GLOBAL_CONFIG_NAMES if (current / name).is_file()]
        candidates.extend(
            current / ".opencode" / name
            for name in GLOBAL_CONFIG_NAMES
            if (current / ".opencode" / name).is_file()
        )
        if len(candidates) > 1:
            hazards.append(f"both project config names exist in {current}")
        for path in candidates:
            try:
                parsed = parse_jsonc(path.read_text(encoding="utf-8"))
            except (OSError, UnicodeError, OpenCodeInstallError) as error:
                hazards.append(f"cannot validate higher-precedence project config {path}: {error}")
                continue
            if isinstance(parsed, dict) and isinstance(parsed.get("mcp"), dict):
                collisions = sorted(set(parsed["mcp"]) & set(MANAGED_SERVER_NAMES))
                if collisions:
                    hazards.append(
                        f"project config {path} overrides managed MCP server(s): {', '.join(collisions)}"
                    )
        if (current / ".git").exists() or current.parent == current:
            break
        current = current.parent
    return hazards


def detect_precedence_hazards(
    *, process_env: Mapping[str, str], effective_cwd: Path | None
) -> tuple[str, ...]:
    """Return higher-precedence sources that can shadow the global projection."""
    hazards: list[str] = []
    if process_env.get("OPENCODE_CONFIG", "").strip():
        hazards.append("OPENCODE_CONFIG selects a higher-precedence custom config")
    if process_env.get("OPENCODE_CONFIG_CONTENT", "").strip():
        hazards.append("OPENCODE_CONFIG_CONTENT supplies a higher-precedence inline config")
    if process_env.get("OPENCODE_CONFIG_DIR", "").strip():
        hazards.append("OPENCODE_CONFIG_DIR selects a higher-precedence custom config directory")
    if effective_cwd is not None:
        hazards.extend(_project_config_hazards(effective_cwd))
    managed_candidates = [
        MANAGED_CONFIG_DIR / name
        for name in GLOBAL_CONFIG_NAMES
        if (MANAGED_CONFIG_DIR / name).is_file()
    ]
    if len(managed_candidates) > 1:
        hazards.append(f"both managed config names exist in {MANAGED_CONFIG_DIR}")
    for path in managed_candidates:
        try:
            parsed = parse_jsonc(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeError, OpenCodeInstallError) as error:
            hazards.append(f"cannot validate higher-precedence managed config {path}: {error}")
            continue
        if isinstance(parsed, dict) and isinstance(parsed.get("mcp"), dict):
            collisions = sorted(set(parsed["mcp"]) & set(MANAGED_SERVER_NAMES))
            if collisions:
                hazards.append(
                    f"managed config {path} overrides managed MCP server(s): "
                    + ", ".join(collisions)
                )
    return tuple(hazards)


def _sha256_bytes(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def _write_backup(config_path: Path, content: bytes | None, mode: int | None) -> Path:
    backup_dir = config_path.parent / ".sky-cua-backups"
    if backup_dir.is_symlink():
        raise OpenCodeInstallError(f"OpenCode backup directory must not be a symlink: {backup_dir}")
    backup_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    backup_dir.chmod(0o700)
    identity = (
        f"{_sha256_bytes(content)}-{mode:04o}"
        if content is not None and mode is not None
        else "absent"
    )
    backup_path = backup_dir / f"{config_path.name}.{identity}.json"
    if backup_path.is_symlink():
        raise OpenCodeInstallError(
            f"OpenCode rollback snapshot must not be a symlink: {backup_path}"
        )
    payload = {
        "schema_version": 1,
        "config_name": config_path.name,
        "existed": content is not None,
        "mode": mode,
        "sha256": _sha256_bytes(content) if content is not None else None,
        "content_base64": base64.b64encode(content).decode("ascii")
        if content is not None
        else None,
    }
    rendered = json.dumps(payload, sort_keys=True, separators=(",", ":")) + "\n"
    if backup_path.exists():
        if backup_path.read_text(encoding="utf-8") != rendered:
            raise OpenCodeInstallError(f"rollback snapshot collision: {backup_path}")
    else:
        write_text_atomically(backup_path, rendered, mode=0o600)
    backup_path.chmod(0o600)
    return backup_path


def _restore_snapshot(config_path: Path, content: bytes | None, mode: int | None) -> None:
    if content is None:
        config_path.unlink(missing_ok=True)
        return
    write_text_atomically(config_path, content.decode("utf-8"), mode=mode)


def install_opencode_two_server_config(
    release_root: Path,
    *,
    browser_socket_path: Path,
    config_dir: Path | None = None,
    process_env: Mapping[str, str] | None = None,
    effective_cwd: Path | None = None,
    timeout_ms: int = DEFAULT_MCP_TIMEOUT_MS,
    after_write: Callable[[Path], None] | None = None,
) -> OpenCodeInstallReport:
    """Merge both servers into the effective global config transactionally.

    ``after_write`` is a controller/test validation hook.  Any exception it
    raises restores the exact pre-install file before propagating.
    """
    env = process_env if process_env is not None else os.environ
    hazards = detect_precedence_hazards(process_env=env, effective_cwd=effective_cwd)
    if hazards:
        raise OpenCodeInstallError("OpenCode config precedence hazard(s): " + "; ".join(hazards))

    if config_dir is not None:
        selected_dir = config_dir.expanduser()
    else:
        config_home = env.get("XDG_CONFIG_HOME", "").strip()
        if not config_home:
            config_home = str(Path(env.get("HOME", str(Path.home()))) / ".config")
        selected_dir = Path(config_home).expanduser() / "opencode"
    if not selected_dir.is_absolute():
        raise OpenCodeInstallError("OpenCode global config directory must be absolute")
    config_path = _config_path(selected_dir)
    canonical, verified, manifest = _canonical_generation(release_root)
    servers = build_opencode_servers(
        canonical,
        manifest,
        browser_socket_path=browser_socket_path,
        timeout_ms=timeout_ms,
    )
    if config_path.exists():
        original_bytes = config_path.read_bytes()
        try:
            original_text = original_bytes.decode("utf-8")
        except UnicodeDecodeError as error:
            raise OpenCodeInstallError(f"OpenCode config is not UTF-8: {config_path}") from error
        original_mode = stat.S_IMODE(config_path.stat().st_mode)
    else:
        original_bytes = None
        original_text = '{\n  "$schema": "https://opencode.ai/config.json"\n}\n'
        original_mode = None
    merged = merge_managed_servers(original_text, servers)
    merged_bytes = merged.encode("utf-8")
    if original_bytes == merged_bytes:
        return OpenCodeInstallReport(
            config_path=config_path,
            release_root=canonical,
            release_id=verified.release_id,
            manifest_sha256=verified.manifest_sha256,
            changed=False,
            backup_path=None,
            installed_config_sha256=_sha256_bytes(merged_bytes),
            restart_required=False,
            restart_scope="none",
            activation_status="unchanged",
            restart_instruction="No restart is required because the effective managed definitions were unchanged.",
        )

    backup_path = _write_backup(config_path, original_bytes, original_mode)
    try:
        write_text_atomically(config_path, merged, mode=original_mode or 0o600)
        if parse_jsonc(config_path.read_text(encoding="utf-8")) != parse_jsonc(merged):
            raise OpenCodeInstallError("OpenCode config readback differs after atomic write")
        if after_write is not None:
            after_write(config_path)
    except BaseException:
        _restore_snapshot(config_path, original_bytes, original_mode)
        raise
    return OpenCodeInstallReport(
        config_path=config_path,
        release_root=canonical,
        release_id=verified.release_id,
        manifest_sha256=verified.manifest_sha256,
        changed=True,
        backup_path=backup_path,
        installed_config_sha256=_sha256_bytes(merged_bytes),
    )


def rollback_opencode_install(
    *, config_path: Path, backup_path: Path, expected_installed_sha256: str
) -> None:
    """Restore a reported snapshot unless the config changed after installation."""
    if config_path.is_symlink() or backup_path.is_symlink():
        raise OpenCodeInstallError("refusing rollback through a symlink")
    if (
        not config_path.is_file()
        or _sha256_bytes(config_path.read_bytes()) != expected_installed_sha256
    ):
        raise OpenCodeInstallError("OpenCode config changed after install; refusing stale rollback")
    try:
        payload = json.loads(backup_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise OpenCodeInstallError(f"invalid OpenCode rollback snapshot: {backup_path}") from error
    if not isinstance(payload, dict) or payload.get("schema_version") != 1:
        raise OpenCodeInstallError("unsupported OpenCode rollback snapshot")
    if payload.get("config_name") != config_path.name:
        raise OpenCodeInstallError("OpenCode rollback snapshot targets a different config")
    if payload.get("existed") is False:
        config_path.unlink()
        return
    content_base64 = payload.get("content_base64")
    mode = payload.get("mode")
    if not isinstance(content_base64, str) or not isinstance(mode, int):
        raise OpenCodeInstallError("OpenCode rollback snapshot is incomplete")
    try:
        content = base64.b64decode(content_base64, validate=True)
    except ValueError as error:
        raise OpenCodeInstallError("OpenCode rollback snapshot content is invalid") from error
    if payload.get("sha256") != _sha256_bytes(content):
        raise OpenCodeInstallError("OpenCode rollback snapshot hash mismatch")
    try:
        decoded = content.decode("utf-8")
    except UnicodeDecodeError as error:
        raise OpenCodeInstallError("OpenCode rollback snapshot is not UTF-8") from error
    write_text_atomically(config_path, decoded, mode=mode)
