"""Host-agnostic install helpers shared by the MCP server installers."""

from __future__ import annotations

import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tomllib
from pathlib import Path

from _plugin_bundle import SKY_CUA_SKILLS, remove_path

REPO_ROOT = Path(__file__).resolve().parents[1]
BROWSER_SELECTION_ENV = "SKY_CUA_BROWSER"
BROWSER_CONTROL_MODE_ENV = "SKY_CUA_BROWSER_CONTROL_MODE"
CODEX_BROWSER_SOCKET_PATH_ENV = "SKY_CUA_CODEX_BROWSER_SOCKET_PATH"
CANONICAL_BROWSER_SELECTIONS = frozenset({"all", "brave", "chrome", "chromium"})
BROWSER_CONTROL_MODES = frozenset({"legacy", "hybrid", "strict"})
LEGACY_BROWSER_SELECTION_ALIASES = {
    "brave-origin": "brave",
    "chrome-origin": "chrome",
    "chromium-origin": "chromium",
}
DEFAULT_LOCAL_INSTALL_DIR = Path.home() / ".local" / "share" / "sky-cua"
MCP_HOST_CHOICES = ("generic", "opencode", "claude-code", "claude-desktop", "pi", "openclaw")
GATEWAY_AUTH_ENV_KEYS = ("OPENCLAW_GATEWAY_TOKEN", "OPENCLAW_GATEWAY_PASSWORD")
MODEL_SKILL_NAMES = ("browser-use", "computer-use", "phone-use")
SKILL_PROJECTION_MARKER = "SKY_CUA_PROJECTION.json"


def project_model_skills(documentation_root: Path, skill_root: Path) -> tuple[Path, ...]:
    """Materialize small host-discoverable routers to one exact documentation generation."""
    documentation_root = documentation_root.resolve(strict=True)
    skill_root.mkdir(parents=True, exist_ok=True)
    destinations = tuple(skill_root / name for name in MODEL_SKILL_NAMES)
    for name, destination in zip(MODEL_SKILL_NAMES, destinations, strict=True):
        canonical = documentation_root / "skills" / name / "SKILL.md"
        if not canonical.is_file() or canonical.is_symlink():
            raise FileNotFoundError(f"canonical model skill is missing: {canonical}")
        if destination.exists() or destination.is_symlink():
            marker = destination / SKILL_PROJECTION_MARKER
            if destination.is_symlink() or not marker.is_file():
                raise ValueError(f"refusing to replace an unmanaged model skill: {destination}")
    for name, destination in zip(MODEL_SKILL_NAMES, destinations, strict=True):
        canonical = documentation_root / "skills" / name / "SKILL.md"
        temp = atomic_sibling_path(destination, "tmp")
        backup = atomic_sibling_path(destination, "backup")
        remove_path(temp)
        remove_path(backup)
        temp.mkdir()
        source = canonical.read_text(encoding="utf-8")
        front_matter, _, remainder = source.partition("---\n")
        if front_matter or not remainder:
            raise ValueError(f"canonical model skill has invalid front matter: {canonical}")
        metadata, separator, _body = remainder.partition("---\n")
        if not separator:
            raise ValueError(f"canonical model skill has invalid front matter: {canonical}")
        wrapper = (
            f"---\n{metadata}---\n\n"
            f"# Installed sky-cua routing\n\n"
            f"Read and follow the canonical generation-bound skill at `{canonical}`. "
            f"Resolve every `references/`, `recipes/`, `examples/`, and `inventories/` path "
            f"from `{documentation_root}`. Do not use checkout documentation.\n"
        )
        (temp / "SKILL.md").write_text(wrapper, encoding="utf-8")
        (temp / SKILL_PROJECTION_MARKER).write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "documentation_root": str(documentation_root),
                    "canonical_skill": str(canonical),
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        if destination.exists():
            os.replace(destination, backup)
        try:
            os.replace(temp, destination)
        except BaseException:
            if backup.exists():
                os.replace(backup, destination)
            raise
        remove_path(backup)
    return destinations


def resolve_gateway_auth_env(openclaw_dir: Path | None) -> dict[str, str]:
    """Resolve gateway auth credentials from the systemd env file, as a fallback.

    `openclaw mcp set` / `openclaw mcp reload` talk to the running Gateway, and
    when `gateway.auth` uses a secret reference the CLI needs
    OPENCLAW_GATEWAY_TOKEN or OPENCLAW_GATEWAY_PASSWORD in its environment.
    Interactive shells that already sourced the gateway env have them, but a
    plain install shell (or a service context) does not — the gateway keeps
    them in `<state_dir>/gateway.systemd.env` (the same file the gateway
    systemd unit loads via `EnvironmentFile=`). Gateway auth is token OR
    password, so if EITHER key is already set in the environment the shell is
    assumed authenticated and nothing is filled (an explicitly-set credential
    always wins); otherwise whatever auth keys the file provides are returned.
    The file is never written or logged, and only the two auth keys are read.
    """
    if any(os.environ.get(key) for key in GATEWAY_AUTH_ENV_KEYS):
        return {}
    state_dir = (openclaw_dir or (Path.home() / ".openclaw")).expanduser()
    env_file = state_dir / "gateway.systemd.env"
    if not env_file.exists():
        return {}
    resolved: dict[str, str] = {}
    for line in env_file.read_text(encoding="utf-8").splitlines():
        name, sep, value = line.partition("=")
        if not sep:
            continue
        if name.strip() in GATEWAY_AUTH_ENV_KEYS and value:
            resolved[name.strip()] = _unquote_env_value(value.strip())
    return resolved


def _unquote_env_value(value: str) -> str:
    """Strips one matching pair of single or double quotes (systemd
    ``EnvironmentFile`` permits either), leaving unquoted values untouched."""
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        return value[1:-1]
    return value


def subprocess_error_detail(error: Exception) -> str:
    """Render ': <stderr>' for subprocess errors that captured stderr."""
    if isinstance(error, subprocess.CalledProcessError) and error.stderr:
        stderr = error.stderr.decode(errors="replace").strip()
        if stderr:
            return f": {stderr}"
    return ""


def ensure_parent(path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def atomic_sibling_path(path: Path, suffix: str) -> Path:
    return path.with_name(f".{path.name}.{suffix}-{os.getpid()}")


def write_text_atomically(path: Path, text: str, mode: int | None = None) -> None:
    ensure_parent(path)
    write_path = atomic_write_target(path)
    if write_path != path:
        ensure_parent(write_path)
    target_mode = mode
    if target_mode is None and write_path.exists():
        target_mode = stat.S_IMODE(write_path.stat().st_mode)
    temp_path = atomic_sibling_path(write_path, "tmp")
    remove_path(temp_path)
    try:
        temp_path.write_text(text, encoding="utf-8")
        if target_mode is not None:
            temp_path.chmod(target_mode)
        os.replace(temp_path, write_path)
    finally:
        remove_path(temp_path)


def atomic_write_target(path: Path) -> Path:
    return path.resolve(strict=False) if path.is_symlink() else path


def replace_tree_atomically(source: Path, destination: Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temp_path = atomic_sibling_path(destination, "tmp")
    backup_path = atomic_sibling_path(destination, "backup")
    backup_in_use = False
    remove_path(temp_path)
    remove_path(backup_path)
    try:
        shutil.copytree(source, temp_path)
        if destination.exists() or destination.is_symlink():
            os.replace(destination, backup_path)
            backup_in_use = True
            try:
                os.replace(temp_path, destination)
            except OSError:
                os.replace(backup_path, destination)
                backup_in_use = False
                raise
            remove_path(backup_path)
            backup_in_use = False
        else:
            os.replace(temp_path, destination)
    finally:
        remove_path(temp_path)
        if not backup_in_use:
            remove_path(backup_path)


def snapshot_text_path(path: Path) -> tuple[Path, str | None, int | None]:
    target_path = atomic_write_target(path)
    return (
        target_path,
        target_path.read_text(encoding="utf-8") if target_path.exists() else None,
        stat.S_IMODE(target_path.stat().st_mode) if target_path.exists() else None,
    )


def restore_text_path_snapshot(_path: Path, snapshot: tuple[Path, str | None, int | None]) -> None:
    target_path, original_text, original_mode = snapshot
    if original_text is None:
        remove_path(target_path)
    else:
        write_text_atomically(target_path, original_text, mode=original_mode)


MACHINE_CONFIG_PATH_ENV = "SKY_CUA_CONFIG_PATH"


def toml_basic_string(value: str) -> str:
    """Render a TOML basic string with backslash/quote/control escaping."""
    escaped = value.replace("\\", "\\\\").replace('"', '\\"')
    escaped = "".join(
        f"\\u{ord(char):04X}"
        if (ord(char) < 0x20 and char not in "\t") or ord(char) == 0x7F
        else char
        for char in escaped
    )
    return f'"{escaped}"'


def machine_config_path() -> Path | None:
    """Mirror the runtime's machine-config resolution (platform config.rs)."""
    explicit = os.environ.get(MACHINE_CONFIG_PATH_ENV)
    if explicit is not None:
        return Path(explicit) if explicit else None
    if sys.platform == "win32":
        appdata = os.environ.get("APPDATA")
        base = Path(appdata) if appdata else None
    else:
        xdg = os.environ.get("XDG_CONFIG_HOME")
        # Mirror the Rust resolver: no $HOME means no machine config, rather
        # than falling back to the passwd database.
        home = os.environ.get("HOME")
        base = Path(xdg) if xdg else (Path(home) / ".config" if home else None)
    if base is None:
        return None
    return base / "sky-cua" / "sky-cua.toml"


def _replace_assignment(text: str, start: int, end: int, key: str, value: str) -> str:
    section = text[start:end]
    line = f"{key} = {toml_basic_string(value)}\n"
    updated, replacements = re.subn(
        rf"(?m)^{re.escape(key)}[ \t]*=.*(?:\n|$)", line, section, count=1
    )
    if replacements != 1:
        raise ValueError(f"could not locate the {key} assignment to replace")
    return text[:start] + updated + text[end:]


def _upsert_top_level_string(text: str, parsed: dict[str, object], key: str, value: str) -> str:
    first_table = re.search(r"(?m)^\s*\[[^\n]+\]", text)
    end = first_table.start() if first_table else len(text)
    if key in parsed:
        return _replace_assignment(text, 0, end, key, value)
    separator = "" if end == 0 or text[:end].endswith("\n") else "\n"
    return text[:end] + separator + f"{key} = {toml_basic_string(value)}\n" + text[end:]


def _upsert_table_strings(
    text: str, parsed: dict[str, object], table: str, values: dict[str, str]
) -> str:
    table_value = parsed.get(table)
    if table_value is not None and not isinstance(table_value, dict):
        raise ValueError(f"{table} is not a TOML table")
    header = re.search(rf"(?m)^\[{re.escape(table)}\][ \t]*(?:#.*)?$", text)
    if header is None:
        if table_value is not None:
            raise ValueError(f"could not locate the [{table}] table to update")
        separator = "" if not text or text.endswith("\n") else "\n"
        block = f"[{table}]\n" + "".join(
            f"{key} = {toml_basic_string(value)}\n" for key, value in values.items()
        )
        return text + separator + block

    existing = table_value if isinstance(table_value, dict) else {}
    for key, value in values.items():
        header = re.search(rf"(?m)^\[{re.escape(table)}\][ \t]*(?:#.*)?$", text)
        assert header is not None
        section_start = header.end()
        next_table = re.search(r"(?m)^\s*\[[^\n]+\]", text[section_start:])
        section_end = section_start + next_table.start() if next_table else len(text)
        if key in existing:
            text = _replace_assignment(text, section_start, section_end, key, value)
        else:
            insertion = f"{key} = {toml_basic_string(value)}\n"
            prefix = "" if text[:section_end].endswith("\n") else "\n"
            text = text[:section_end] + prefix + insertion + text[section_end:]
    return text


def _seed_machine_config(values: dict[str, str]) -> Path | None:
    path = machine_config_path()
    if path is None:
        return None
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    try:
        existing = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        print(
            f"warning: not updating machine config {path}: existing file fails TOML "
            f"validation ({error}); fix it by hand.",
            file=sys.stderr,
        )
        return None
    new_text = text
    try:
        if (browser := values.get("browser")) and existing.get("browser") != browser:
            new_text = _upsert_top_level_string(new_text, existing, "browser", browser)
        browser_control = {
            key: value for key, value in values.items() if key in {"mode", "codex_socket_path"}
        }
        existing_control = existing.get("browser_control", {})
        changed_control = {
            key: value
            for key, value in browser_control.items()
            if not isinstance(existing_control, dict) or existing_control.get(key) != value
        }
        if changed_control:
            new_text = _upsert_table_strings(new_text, existing, "browser_control", changed_control)
    except ValueError as error:
        print(
            f"warning: not updating machine config {path}: {error}; edit it by hand.",
            file=sys.stderr,
        )
        return None
    if new_text == text:
        return path
    try:
        tomllib.loads(new_text)
    except tomllib.TOMLDecodeError as error:
        print(
            f"warning: not updating machine config {path}: updated file would fail "
            f"TOML validation ({error}).",
            file=sys.stderr,
        )
        return None
    write_text_atomically(path, new_text)
    return path


def seed_machine_config_browser(value: str) -> Path | None:
    """Persist one already-validated browser selection."""
    return _seed_machine_config({"browser": value})


def seed_machine_config_from_environment() -> Path | None:
    """Persist explicitly supplied machine-owned browser settings atomically."""
    values: dict[str, str] = {}
    browser = os.environ.get(BROWSER_SELECTION_ENV, "").strip()
    if browser:
        browser = LEGACY_BROWSER_SELECTION_ALIASES.get(browser, browser)
        if browser not in CANONICAL_BROWSER_SELECTIONS:
            choices = ", ".join(sorted(CANONICAL_BROWSER_SELECTIONS))
            print(
                f"warning: not updating machine config: unsupported {BROWSER_SELECTION_ENV} "
                f"value {browser!r}; use {choices}.",
                file=sys.stderr,
            )
            return None
        values["browser"] = browser

    if BROWSER_CONTROL_MODE_ENV in os.environ:
        mode = os.environ[BROWSER_CONTROL_MODE_ENV].strip()
        if mode not in BROWSER_CONTROL_MODES:
            choices = ", ".join(sorted(BROWSER_CONTROL_MODES))
            print(
                f"warning: not updating machine config: unsupported {BROWSER_CONTROL_MODE_ENV} "
                f"value {mode!r}; use {choices}.",
                file=sys.stderr,
            )
            return None
        values["mode"] = mode

    if CODEX_BROWSER_SOCKET_PATH_ENV in os.environ:
        socket_path = os.environ[CODEX_BROWSER_SOCKET_PATH_ENV].strip()
        if not socket_path:
            print(
                f"warning: not updating machine config: {CODEX_BROWSER_SOCKET_PATH_ENV} "
                "must not be empty.",
                file=sys.stderr,
            )
            return None
        values["codex_socket_path"] = socket_path

    if not values:
        return None
    return _seed_machine_config(values)


def install_sky_cua_skills(skills_dir: Path) -> None:
    skills_dir.mkdir(parents=True, exist_ok=True)
    for skill_name in SKY_CUA_SKILLS:
        source = REPO_ROOT / "skills" / skill_name
        if not source.exists():
            raise FileNotFoundError(f"sky-cua skill source not found: {source}")
        destination = skills_dir / skill_name
        replace_tree_atomically(source, destination)
