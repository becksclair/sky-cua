from __future__ import annotations

import errno
import json
import os
import platform
import re
import shutil
import signal
import stat
import subprocess
import sys
import time
from contextlib import suppress
from datetime import UTC, datetime
from pathlib import Path

PLUGIN_NAME = "sky-cua"
PLUGIN_CHANNEL = "local"
PLUGIN_ID = f"{PLUGIN_NAME}@{PLUGIN_CHANNEL}"
# Codex Desktop detects Computer Use plugins by the built-in plugin name
# "computer-use", so this compat id is the single enabled computer-use plugin
# for the codex host. The sky-cua channel id stays installed but disabled; the
# active payload is selected by retargeting the compat plugin root's .mcp.json
# at the local cache payload (see docs/operations/plugin-release.md).
COMPUTER_USE_COMPAT_PLUGIN_ID = "computer-use@openai-bundled"
SKY_CUA_SKILLS = ("computer-use", "browser-use", "phone-use")
SHARED_AGENT_SKILL_OVERRIDES_BEGIN = "# BEGIN sky-cua managed shared-agent skill overrides"
SHARED_AGENT_SKILL_OVERRIDES_END = "# END sky-cua managed shared-agent skill overrides"
REPO_ROOT = Path(__file__).resolve().parents[1]
DIST_PLUGIN_ROOT = REPO_ROOT / "dist" / "plugin" / PLUGIN_NAME
DEFAULT_CODEX_HOME = Path.home() / ".codex"
INSTALLED_PLUGIN_ROOT = (
    DEFAULT_CODEX_HOME / "plugins" / "cache" / PLUGIN_CHANNEL / PLUGIN_NAME / "local"
)
LINUX_X64 = "linux-x64"
LINUX_ARM64 = "linux-arm64"
WINDOWS_X64 = "windows-x64"
REQUIRED_RUNTIME_PLATFORMS = (LINUX_X64, LINUX_ARM64, WINDOWS_X64)
RUNTIME_BINARY_BASE_NAMES = ("sky-cua-client", "sky-cua-service", "sky-cua-overlay-host")
BUILD_STAMP_SUFFIX = ".buildstamp.json"
LINUX_RUNTIME_BINARY_BASE_NAMES = (
    *RUNTIME_BINARY_BASE_NAMES,
    "sky-cua-cosmic-helper",
    "sky-cua-chrome-host",
    "sky-cua-input-helper",
)
UNIX_RUNTIME_ENTRYPOINT_PATHS = tuple(Path("bin") / name for name in RUNTIME_BINARY_BASE_NAMES)
UNIX_PRE_FLIGHT_ENTRYPOINT_PATHS = (Path("bin") / "sky-cua-browser-preflight",)
TAG_VERSION_RE = re.compile(r"^v(?P<version>\d+\.\d+\.\d+)$")
SIGTERM = signal.SIGTERM
SIGKILL = getattr(signal, "SIGKILL", SIGTERM)


def executable_name(name: str) -> str:
    if sys.platform == "win32":
        return f"{name}.exe"
    return name


def platform_runtime_binary_names(*, windows: bool) -> list[str]:
    suffix = ".exe" if windows else ""
    base_names = RUNTIME_BINARY_BASE_NAMES if windows else LINUX_RUNTIME_BINARY_BASE_NAMES
    return [f"{name}{suffix}" for name in base_names]


def current_runtime_platform() -> str:
    if sys.platform == "win32":
        return WINDOWS_X64
    machine = platform.machine().lower()
    if machine in {"x86_64", "amd64"}:
        return LINUX_X64
    if machine in {"aarch64", "arm64"}:
        return LINUX_ARM64
    raise RuntimeError(f"unsupported sky-cua runtime platform: {sys.platform}/{machine}")


def runtime_binary_path(platform_id: str, binary_name: str) -> Path:
    if binary_name not in platform_runtime_binary_base_names(platform_id):
        raise ValueError(f"unknown runtime binary: {binary_name}")
    if platform_id == WINDOWS_X64:
        return Path("bin") / f"{binary_name}.exe"
    if platform_id in {LINUX_X64, LINUX_ARM64}:
        return Path("bin") / "runtimes" / platform_id / binary_name
    raise ValueError(f"unknown runtime platform: {platform_id}")


def runtime_binary_source_name(platform_id: str, binary_name: str) -> str:
    runtime_binary_path(platform_id, binary_name)
    return f"{binary_name}.exe" if platform_id == WINDOWS_X64 else binary_name


def chrome_extension_host_arch(platform_id: str) -> str | None:
    if platform_id == LINUX_X64:
        return "x64"
    if platform_id == LINUX_ARM64:
        return "arm64"
    return None


def chrome_extension_host_path(platform_id: str) -> Path | None:
    arch = chrome_extension_host_arch(platform_id)
    if arch is None:
        return None
    return (
        Path("resources")
        / "plugins"
        / "openai-bundled"
        / "plugins"
        / "chrome"
        / "extension-host"
        / "linux"
        / arch
        / "extension-host"
    )


def platform_runtime_binary_base_names(platform_id: str) -> tuple[str, ...]:
    if platform_id in {LINUX_X64, LINUX_ARM64}:
        return LINUX_RUNTIME_BINARY_BASE_NAMES
    if platform_id == WINDOWS_X64:
        return RUNTIME_BINARY_BASE_NAMES
    raise ValueError(f"unknown runtime platform: {platform_id}")


def platform_runtime_binary_paths(platform_id: str) -> list[Path]:
    return [
        runtime_binary_path(platform_id, name)
        for name in platform_runtime_binary_base_names(platform_id)
    ]


def all_runtime_binary_paths() -> list[Path]:
    return [
        path
        for platform_id in REQUIRED_RUNTIME_PLATFORMS
        for path in platform_runtime_binary_paths(platform_id)
    ]


def all_runtime_binary_names() -> list[str]:
    return [path.as_posix().removeprefix("bin/") for path in all_runtime_binary_paths()]


def runtime_binary_names() -> list[str]:
    return platform_runtime_binary_names(windows=sys.platform == "win32")


def runtime_entrypoint_paths() -> list[Path]:
    return [Path("bin") / executable_name(name) for name in RUNTIME_BINARY_BASE_NAMES]


def bundle_entrypoint_paths() -> list[Path]:
    return sorted(
        {
            *UNIX_RUNTIME_ENTRYPOINT_PATHS,
            *UNIX_PRE_FLIGHT_ENTRYPOINT_PATHS,
            *runtime_entrypoint_paths(),
        },
        key=lambda path: path.as_posix(),
    )


def version_from_tag(tag: str) -> str:
    match = TAG_VERSION_RE.match(tag)
    if match is None:
        raise ValueError(f"release tag must look like vX.Y.Z, got {tag!r}")
    return match.group("version")


def update_plugin_manifest_version(bundle_root: Path, version: str) -> None:
    for manifest_path in (
        bundle_root / ".codex-plugin" / "plugin.json",
        bundle_root / ".claude-plugin" / "plugin.json",
    ):
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["version"] = version
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def merge_runtime_artifacts(bundle_root: Path, artifacts_root: Path) -> None:
    """Merge per-platform runtime artifacts into one multi-platform bundle.

    Copies each platform's staged binaries (`<artifacts_root>/<platform>/`,
    produced by `package_runtime_artifact.py` on that platform's native host)
    into the bundle's `bin/runtimes/<platform>/` layout, requiring the full
    `REQUIRED_RUNTIME_PLATFORMS` set so a fat bundle is never shipped with a
    platform silently missing. This is the cross-build assembly step with no
    marketplace dependency; the single-platform `scripts/package.py` does not use
    it, but a future multi-platform/Windows package would.
    """
    missing: list[str] = []
    for platform_id in REQUIRED_RUNTIME_PLATFORMS:
        platform_root = artifacts_root / platform_id
        for binary_name in platform_runtime_binary_base_names(platform_id):
            source = platform_root / runtime_binary_source_name(platform_id, binary_name)
            destination = bundle_root / runtime_binary_path(platform_id, binary_name)
            if not source.exists():
                missing.append(str(source))
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            source_stamp = source.with_name(source.name + BUILD_STAMP_SUFFIX)
            destination_stamp = destination.with_name(destination.name + BUILD_STAMP_SUFFIX)
            if source_stamp.exists():
                shutil.copy2(source_stamp, destination_stamp)
            else:
                remove_path(destination_stamp)
            if not destination.name.endswith(".exe"):
                ensure_executable(destination)
            if binary_name == "sky-cua-chrome-host" and (
                host_destination := chrome_extension_host_path(platform_id)
            ):
                bundled_host = bundle_root / host_destination
                bundled_host.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, bundled_host)
                ensure_executable(bundled_host)
    if missing:
        raise FileNotFoundError(f"runtime artifact set is missing required binaries: {missing}")


def build_bundle() -> None:
    subprocess.run([sys.executable, str(REPO_ROOT / "scripts" / "build_plugin.py")], check=True)


def mcp_config_source() -> Path:
    return REPO_ROOT / ".mcp.json"


def ensure_executable(path: Path) -> None:
    if sys.platform == "win32":
        return
    try:
        mode = path.stat().st_mode
        path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    except FileNotFoundError:
        pass
    except PermissionError as exc:
        print(f"warning: cannot make {path} executable: {exc}", file=sys.stderr)


def remove_path(path: Path) -> None:
    if not path.exists() and not path.is_symlink():
        return
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    for attempt in range(3):
        try:
            shutil.rmtree(path)
            return
        except OSError as exc:
            if exc.errno != errno.ENOTEMPTY or attempt == 2:
                raise
            time.sleep(0.05)


def copytree_replace(src: Path, dst: Path) -> None:
    remove_path(dst)
    shutil.copytree(src, dst)


def copytree_replace_preserving_platform_binaries(src: Path, dst: Path) -> None:
    preserved_root = dst.parent / f".{dst.name}.preserved-bin"
    remove_path(preserved_root)
    preserved: list[tuple[Path, Path, Path | None]] = []
    for relative_binary_path in all_runtime_binary_paths():
        source_binary = src / relative_binary_path
        destination_binary = dst / relative_binary_path
        if source_binary.exists() or not destination_binary.exists():
            continue
        preserved_binary = preserved_root / relative_binary_path
        preserved_binary.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(destination_binary, preserved_binary)
        destination_stamp = destination_binary.with_name(
            destination_binary.name + BUILD_STAMP_SUFFIX
        )
        preserved_stamp = preserved_binary.with_name(preserved_binary.name + BUILD_STAMP_SUFFIX)
        if destination_stamp.exists():
            shutil.copy2(destination_stamp, preserved_stamp)
        else:
            preserved_stamp = None
        preserved.append((relative_binary_path, preserved_binary, preserved_stamp))

    try:
        copytree_replace(src, dst)
        for relative_binary_path, preserved_binary, preserved_stamp in preserved:
            restored_binary = dst / relative_binary_path
            if restored_binary.exists():
                continue
            restored_binary.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(preserved_binary, restored_binary)
            restored_stamp = restored_binary.with_name(restored_binary.name + BUILD_STAMP_SUFFIX)
            if preserved_stamp is not None:
                shutil.copy2(preserved_stamp, restored_stamp)
            else:
                remove_path(restored_stamp)
            if not relative_binary_path.name.endswith(".exe"):
                ensure_executable(restored_binary)
    finally:
        remove_path(preserved_root)


def stop_windows_cache_processes(cache_root: Path) -> None:
    if sys.platform != "win32":
        return
    resolved_cache_root = str(cache_root.resolve())
    escaped_cache_root = resolved_cache_root.replace("'", "''")
    script = (
        "$cacheRoot = '" + escaped_cache_root + "'; "
        "$cacheRoot = [System.IO.Path]::GetFullPath($cacheRoot).TrimEnd('\\'); "
        "$cachePrefix = ($cacheRoot + '\\').ToLowerInvariant(); "
        "$matches = Get-CimInstance Win32_Process | Where-Object { "
        "$path = $_.ExecutablePath; "
        "if ($path) { "
        "$full = [System.IO.Path]::GetFullPath($path).ToLowerInvariant(); "
        "$full.StartsWith($cachePrefix) "
        "} elseif ($_.CommandLine) { "
        "$commandLine = $_.CommandLine.TrimStart('\"').ToLowerInvariant(); "
        "$commandLine.StartsWith($cachePrefix) "
        "} else { $false } "
        "}; "
        "$matches | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }"
    )
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=True)


def stop_unix_runtime_processes(
    search_roots: list[Path],
    proc_root: Path = Path("/proc"),
    *,
    match_all_paths: bool = False,
) -> None:
    """Terminate sky-cua runtime processes so hosts respawn fresh binaries.

    By default only processes running from one of `search_roots` are stopped.
    With `match_all_paths`, every sky-cua stack process owned by the current
    user is stopped regardless of path — this reaps zombie processes left by
    an earlier dev build or a stale install (e.g. an overlay host still bound
    to the cursor socket from `target/release/`), which a path-scoped match
    silently misses and which then fight the freshly deployed stack.
    """
    if sys.platform == "win32":
        return
    root_prefixes = [
        _normalize_process_path(root.resolve()) + "/" for root in search_roots if root.exists()
    ]
    if not proc_root.exists() or (not root_prefixes and not match_all_paths):
        return

    current_pid = os.getpid()
    current_uid = os.getuid()
    matches: list[int] = []
    for entry in proc_root.iterdir():
        if not entry.name.isdigit():
            continue
        pid = int(entry.name)
        if pid == current_pid:
            continue
        exe = _read_process_link(entry / "exe")
        cwd = _read_process_link(entry / "cwd")
        cmdline = _read_process_cmdline(entry / "cmdline")
        if match_all_paths:
            # Name-only match, but only for processes this user owns so a
            # deploy never signals another user's sky-cua stack.
            if _process_uid(entry) == current_uid and _is_sky_cua_runtime_binary(exe, cmdline):
                matches.append(pid)
        elif _is_sky_cua_runtime_process(exe, cwd, cmdline, root_prefixes):
            matches.append(pid)

    for pid in matches:
        _terminate_process(pid)


def _process_uid(entry: Path) -> int | None:
    try:
        return entry.stat().st_uid
    except OSError:
        return None


SKY_CUA_RUNTIME_BINARIES = frozenset(
    {
        "sky-cua-client",
        "sky-cua-overlay-host",
        "sky-cua-service",
        "sky-cua-chrome-host",
        "sky-cua-cosmic-helper",
        "sky-cua-input-helper",
    }
)


def _is_sky_cua_runtime_binary(exe: str | None, cmdline: str) -> bool:
    if Path(exe or "").name in SKY_CUA_RUNTIME_BINARIES:
        return True
    # A process re-exec'd through a wrapper (bin/../target/release/...) may
    # report the wrapper as exe; fall back to the argv0 binary name.
    argv0 = cmdline.split(" ", 1)[0] if cmdline else ""
    return Path(argv0).name in SKY_CUA_RUNTIME_BINARIES


def _normalize_process_path(path: Path) -> str:
    rendered = str(path)
    if rendered.endswith(" (deleted)"):
        rendered = rendered[: -len(" (deleted)")]
    return os.path.abspath(rendered)


def _read_process_link(path: Path) -> str | None:
    try:
        return _normalize_process_path(path.readlink())
    except OSError:
        return None


def _read_process_cmdline(path: Path) -> str:
    try:
        return path.read_bytes().replace(b"\0", b" ").decode("utf-8", errors="replace")
    except OSError:
        return ""


def _is_sky_cua_runtime_process(
    exe: str | None,
    cwd: str | None,
    cmdline: str,
    root_prefixes: list[str],
) -> bool:
    if Path(exe or "").name not in SKY_CUA_RUNTIME_BINARIES:
        return False
    candidates = [candidate for candidate in [exe, cwd, cmdline] if candidate]
    return any(prefix in candidate for prefix in root_prefixes for candidate in candidates)


def _terminate_process(pid: int) -> None:
    try:
        os.kill(pid, SIGTERM)
    except ProcessLookupError:
        return
    deadline = datetime.now(tz=UTC).timestamp() + 3
    while datetime.now(tz=UTC).timestamp() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return
        time.sleep(0.05)
    with suppress(ProcessLookupError):
        os.kill(pid, SIGKILL)


def ensure_bundle_structure(root: Path) -> None:
    required = [
        root / ".claude-plugin" / "plugin.json",
        root / ".claude-plugin" / "marketplace.json",
        root / ".codex-plugin" / "plugin.json",
        root / ".mcp.json",
        *(root / "skills" / skill_name / "SKILL.md" for skill_name in SKY_CUA_SKILLS),
        root / "docs" / "operations" / "testing-vm-desktop-smokes.md",
        root / "resources" / "app-instructions" / "index.json",
    ]
    required.extend(root / path for path in bundle_entrypoint_paths())
    required.extend(
        root / path for path in platform_runtime_binary_paths(current_runtime_platform())
    )
    missing = [str(path) for path in required if not path.exists()]
    if missing:
        raise FileNotFoundError(f"plugin bundle is missing required paths: {missing}")
    for relative_path in [*bundle_entrypoint_paths(), *all_runtime_binary_paths()]:
        binary_path = root / relative_path
        if binary_path.exists() and not relative_path.name.endswith(".exe"):
            ensure_executable(binary_path)


def installed_plugin_root(codex_home: Path) -> Path:
    return codex_home / "plugins" / "cache" / PLUGIN_CHANNEL / PLUGIN_NAME / "local"


def ensure_plugins_feature_enabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "plugins", "true")


def ensure_apps_feature_disabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "apps", "false")


def ensure_fast_mode_enabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "fast_mode", "true")


def ensure_fast_service_tier(config_text: str) -> str:
    config_text = remove_profile_service_tier_overrides(config_text)
    return upsert_top_level_toml_key(config_text, "service_tier", '"fast"')


def set_plugin_enabled(config_text: str, plugin_id: str, *, enabled: bool) -> str:
    rendered = "true" if enabled else "false"
    return upsert_toml_key(config_text, f'plugins."{plugin_id}"', "enabled", rendered)


def upsert_top_level_toml_key(config_text: str, key: str, rendered_value: str) -> str:
    key_re = re.compile(rf"(?m)^\s*{re.escape(key)}\s*=.*$")
    first_section = re.search(r"(?m)^\[", config_text)
    section_start = first_section.start() if first_section else len(config_text)
    top_level = config_text[:section_start]
    rest = config_text[section_start:]

    if key_re.search(top_level):
        return key_re.sub(lambda _: f"{key} = {rendered_value}", top_level, count=1) + rest
    if top_level and not top_level.endswith("\n"):
        top_level += "\n"
    return f"{key} = {rendered_value}\n" + top_level + rest


def remove_profile_service_tier_overrides(config_text: str) -> str:
    lines = config_text.splitlines(keepends=True)
    output: list[str] = []
    in_profile_section = False
    service_tier_re = re.compile(r"^\s*service_tier\s*=.*$")

    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            header = stripped.strip("[]").strip()
            in_profile_section = header.startswith("profiles.")
        if in_profile_section and service_tier_re.match(line):
            continue
        output.append(line)
    return "".join(output)


def upsert_toml_key(config_text: str, header: str, key: str, rendered_value: str) -> str:
    header_line = f"[{header}]"
    section_re = re.compile(rf"(?ms)^\[{re.escape(header)}\]\r?\n(?P<body>.*?)(?=^\[|\Z)")
    key_re = re.compile(rf"(?m)^\s*{re.escape(key)}\s*=.*$")
    section = f"{header_line}\n{key} = {rendered_value}\n"

    match = section_re.search(config_text)
    if not match:
        if config_text and not config_text.endswith("\n"):
            config_text += "\n"
        if config_text and not config_text.endswith("\n\n"):
            config_text += "\n"
        return config_text + section

    body = match.group("body")
    if key_re.search(body):
        new_body = key_re.sub(lambda _: f"{key} = {rendered_value}", body, count=1)
    else:
        new_body = f"{key} = {rendered_value}\n{body}"
    return config_text[: match.start()] + header_line + "\n" + new_body + config_text[match.end() :]


def compat_plugin_cache_root(codex_home: Path) -> Path:
    """Cache root of the computer-use compat plugin.

    Layout owner: `resources/chrome_preflight.py` (`OPENAI_BUNDLED_MARKETPLACE`,
    `COMPUTER_USE_PLUGIN_NAME`, `sync_computer_use_compat_plugin`). The
    preflight ships inside the bundle and runs standalone, so it cannot import
    this module; keep the two spellings in sync.
    """
    return codex_home / "plugins" / "cache" / "openai-bundled" / "computer-use"


def compat_plugin_available(codex_home: Path) -> bool:
    """Whether a materialized computer-use compat plugin root exists.

    The chrome preflight only materializes the compat root when the payload
    ships the bundled openai-bundled resources (Linux installs). Without it,
    enabling the compat id would leave no active computer-use MCP server, so
    callers fall back to enabling a sky-cua channel id directly.
    """
    return (compat_plugin_cache_root(codex_home) / "latest" / ".mcp.json").exists()


def compat_plugin_target(codex_home: Path) -> str | None:
    """Command the materialized compat plugin launches, or None."""
    mcp_path = compat_plugin_cache_root(codex_home) / "latest" / ".mcp.json"
    try:
        servers = json.loads(mcp_path.read_text(encoding="utf-8"))["mcpServers"]
        command = servers["computer-use"]["command"]
    except (OSError, json.JSONDecodeError, KeyError, TypeError):
        return None
    return command if isinstance(command, str) else None


def compat_plugin_targets_payload(codex_home: Path, payload_root: Path) -> bool:
    """Whether the materialized compat plugin launches this payload's client."""
    expected = str((payload_root / "bin" / "sky-cua-client").resolve())
    return compat_plugin_target(codex_home) == expected


def apply_compat_plugin_enablement(config_text: str) -> str:
    """Apply compat-plugin-first enablement for the codex host.

    Enables `computer-use@openai-bundled` (the only computer-use plugin id the
    Codex Desktop UI detects) and disables the sky-cua channel id. The active
    payload is selected by regenerating the compat plugin root against the
    local cache payload, not by toggling the channel id.
    """
    config_text = set_plugin_enabled(config_text, COMPUTER_USE_COMPAT_PLUGIN_ID, enabled=True)
    config_text = set_plugin_enabled(config_text, PLUGIN_ID, enabled=False)
    return config_text


# Retired sky-cua plugin ids: the old local dev channel (`debug`, now `local`)
# and the retired private-marketplace publish id (`Heliasar`). Either stanza left
# enabled would make Codex launch a second computer-use MCP server alongside the
# active compat/local plugin, violating the single-active-server invariant. This
# is the single source of truth for both the config neutralization below and the
# cache-payload cleanup in the deploy loop. The marketplace cache dir for each id
# is the part after `@` (cache/<marketplace>/sky-cua).
RETIRED_PLUGIN_IDS: tuple[str, ...] = ("sky-cua@debug", "sky-cua@Heliasar")


def disable_retired_channels(config_text: str) -> str:
    """Disable any retired sky-cua channel stanzas present in the config.

    Stanzas absent from the config are left untouched - never synthesized - so
    this no-ops on machines that only ran the live channel. Folding this into
    every ``update_codex_config`` write enforces the single-active-computer-use
    invariant for all of its callers (``install_plugin``, ``deploy_plugin``,
    ``installer``), so an in-place upgrade of a box still carrying a stale
    ``sky-cua@debug``/``sky-cua@Heliasar`` stanza converges to one enabled id
    instead of producing a duplicate computer-use server.
    """
    for plugin_id in RETIRED_PLUGIN_IDS:
        if f'[plugins."{plugin_id}"]' in config_text:
            config_text = set_plugin_enabled(config_text, plugin_id, enabled=False)
    return config_text


def apply_shared_agent_skill_deduplication(config_text: str, skills_root: Path) -> str:
    """Disable shared sky-cua skill copies inside Codex only.

    sky-cua keeps canonical symlinks under ``~/.agents/skills`` so generic
    agents can discover the same skills. Codex also loads the copies bundled in
    the active plugin, however, which otherwise exposes every sky-cua skill
    twice. Codex canonicalizes path selectors before applying ``skills.config``
    rules, so selectors written against the stable symlink paths continue to
    follow whichever checkout ``sync_agent_skills.py`` currently targets while
    leaving the plugin-namespaced copies enabled.
    """
    managed_block_re = re.compile(
        rf"(?ms)^{re.escape(SHARED_AGENT_SKILL_OVERRIDES_BEGIN)}\r?\n"
        rf".*?^{re.escape(SHARED_AGENT_SKILL_OVERRIDES_END)}(?:\r?\n)?"
    )
    config_text = managed_block_re.sub("", config_text).rstrip()

    skills_root = skills_root.expanduser()
    lines = [
        SHARED_AGENT_SKILL_OVERRIDES_BEGIN,
        "# Shared copies stay available to non-Codex agents; plugin skills are canonical in Codex.",
    ]
    for skill_name in SKY_CUA_SKILLS:
        lines.extend(
            [
                "[[skills.config]]",
                f"path = {json.dumps(str(skills_root / skill_name / 'SKILL.md'))}",
                "enabled = false",
                "",
            ]
        )
    lines.append(SHARED_AGENT_SKILL_OVERRIDES_END)
    managed_block = "\n".join(lines)
    if not config_text:
        return f"{managed_block}\n"
    return f"{config_text}\n\n{managed_block}\n"


def update_codex_config(
    config_path: Path,
    *,
    disable_apps: bool = False,
    fast_service_tier: bool = False,
    plugin_id: str = PLUGIN_ID,
    plugin_enabled: bool = True,
    compat_enablement: bool = False,
    shared_agent_skills_root: Path | None = None,
) -> None:
    config_path.parent.mkdir(parents=True, exist_ok=True)
    try:
        config_text = config_path.read_text(encoding="utf-8")
    except FileNotFoundError:
        config_text = ""
    config_text = ensure_plugins_feature_enabled(config_text)
    if disable_apps:
        config_text = ensure_apps_feature_disabled(config_text)
    if fast_service_tier:
        config_text = ensure_fast_mode_enabled(config_text)
        config_text = ensure_fast_service_tier(config_text)
    if compat_enablement:
        # Compat-first mode owns the computer-use plugin toggles; the helper
        # enables the compat id and disables the sky-cua@local channel id, so
        # the per-channel plugin_id/plugin_enabled arguments do not apply.
        config_text = apply_compat_plugin_enablement(config_text)
    else:
        config_text = set_plugin_enabled(config_text, plugin_id, enabled=plugin_enabled)
        if plugin_enabled:
            # Channel-id fallback is the symmetric inverse: exactly one
            # enabled computer-use plugin id, so enabling the channel id turns
            # off a previously enabled compat id (e.g. after a cache wipe
            # removed its root). Disable-only staging calls leave the compat
            # id alone to avoid a transient zero-enabled window mid-deploy.
            config_text = set_plugin_enabled(
                config_text, COMPUTER_USE_COMPAT_PLUGIN_ID, enabled=False
            )
    # Retired channels are neutralized on every write so any caller converges to
    # exactly one enabled computer-use plugin id, even on an in-place upgrade of a
    # box left in the old debug/Heliasar-enabled state.
    config_text = disable_retired_channels(config_text)
    config_text = apply_shared_agent_skill_deduplication(
        config_text,
        shared_agent_skills_root or (Path.home() / ".agents" / "skills"),
    )
    config_path.write_text(config_text, encoding="utf-8")
