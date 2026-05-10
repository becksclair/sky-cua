from __future__ import annotations

import json
import os
import platform
import re
import shutil
import stat
import subprocess
import sys
import time
from contextlib import suppress
from datetime import UTC, datetime
from pathlib import Path
from signal import SIGKILL, SIGTERM

PLUGIN_NAME = "sky-cua"
PLUGIN_CHANNEL = "debug"
RELEASE_MARKETPLACE_NAME = "Heliasar"
RELEASE_PLUGIN_ID = f"{PLUGIN_NAME}@{RELEASE_MARKETPLACE_NAME}"
PLUGIN_ID = f"{PLUGIN_NAME}@{PLUGIN_CHANNEL}"
PLUGIN_CATEGORY = "Coding"
REPO_ROOT = Path(__file__).resolve().parents[1]
DIST_PLUGIN_ROOT = REPO_ROOT / "dist" / "plugin" / PLUGIN_NAME
DEFAULT_CODEX_HOME = Path.home() / ".codex"
DEFAULT_MARKETPLACE_ROOT = Path.home() / "projects" / "heliasar-marketplace"
INSTALLED_PLUGIN_ROOT = (
    DEFAULT_CODEX_HOME / "plugins" / "cache" / PLUGIN_CHANNEL / PLUGIN_NAME / "local"
)
LINUX_X64 = "linux-x64"
LINUX_ARM64 = "linux-arm64"
WINDOWS_X64 = "windows-x64"
REQUIRED_RUNTIME_PLATFORMS = (LINUX_X64, LINUX_ARM64, WINDOWS_X64)
RUNTIME_BINARY_BASE_NAMES = ("sky-cua-client", "sky-cua-service")
UNIX_RUNTIME_ENTRYPOINT_PATHS = tuple(Path("bin") / name for name in RUNTIME_BINARY_BASE_NAMES)
TAG_VERSION_RE = re.compile(r"^v(?P<version>\d+\.\d+\.\d+)$")


def executable_name(name: str) -> str:
    if sys.platform == "win32":
        return f"{name}.exe"
    return name


def platform_runtime_binary_names(*, windows: bool) -> list[str]:
    suffix = ".exe" if windows else ""
    return [f"sky-cua-client{suffix}", f"sky-cua-service{suffix}"]


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
    if binary_name not in RUNTIME_BINARY_BASE_NAMES:
        raise ValueError(f"unknown runtime binary: {binary_name}")
    if platform_id == WINDOWS_X64:
        return Path("bin") / f"{binary_name}.exe"
    if platform_id in {LINUX_X64, LINUX_ARM64}:
        return Path("bin") / "runtimes" / platform_id / binary_name
    raise ValueError(f"unknown runtime platform: {platform_id}")


def runtime_binary_source_name(platform_id: str, binary_name: str) -> str:
    runtime_binary_path(platform_id, binary_name)
    return f"{binary_name}.exe" if platform_id == WINDOWS_X64 else binary_name


def platform_runtime_binary_paths(platform_id: str) -> list[Path]:
    return [runtime_binary_path(platform_id, name) for name in RUNTIME_BINARY_BASE_NAMES]


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
    return [Path("bin") / name for name in runtime_binary_names()]


def bundle_entrypoint_paths() -> list[Path]:
    return sorted(
        {*UNIX_RUNTIME_ENTRYPOINT_PATHS, *runtime_entrypoint_paths()},
        key=lambda path: path.as_posix(),
    )


def version_from_tag(tag: str) -> str:
    match = TAG_VERSION_RE.match(tag)
    if match is None:
        raise ValueError(f"release tag must look like vX.Y.Z, got {tag!r}")
    return match.group("version")


def update_plugin_manifest_version(bundle_root: Path, version: str) -> None:
    manifest_path = bundle_root / ".codex-plugin" / "plugin.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["version"] = version
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def merge_runtime_artifacts(bundle_root: Path, artifacts_root: Path) -> None:
    missing: list[str] = []
    for platform_id in REQUIRED_RUNTIME_PLATFORMS:
        platform_root = artifacts_root / platform_id
        for binary_name in RUNTIME_BINARY_BASE_NAMES:
            source = platform_root / runtime_binary_source_name(platform_id, binary_name)
            destination = bundle_root / runtime_binary_path(platform_id, binary_name)
            if not source.exists():
                missing.append(str(source))
                continue
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
            if not destination.name.endswith(".exe"):
                ensure_executable(destination)
    if missing:
        raise FileNotFoundError(f"runtime artifact set is missing required binaries: {missing}")


def build_bundle() -> None:
    subprocess.run([sys.executable, str(REPO_ROOT / "scripts" / "build_plugin.py")], check=True)


def mcp_config_source() -> Path:
    return REPO_ROOT / ".mcp.json"


def ensure_executable(path: Path) -> None:
    if sys.platform == "win32":
        return
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def remove_path(path: Path) -> None:
    if not path.exists() and not path.is_symlink():
        return
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    shutil.rmtree(path)


def copytree_replace(src: Path, dst: Path) -> None:
    remove_path(dst)
    shutil.copytree(src, dst)


def copytree_replace_preserving_platform_binaries(src: Path, dst: Path) -> None:
    preserved_root = dst.parent / f".{dst.name}.preserved-bin"
    remove_path(preserved_root)
    preserved: list[tuple[str, Path]] = []
    for relative_binary_path in all_runtime_binary_paths():
        source_binary = src / relative_binary_path
        destination_binary = dst / relative_binary_path
        if source_binary.exists() or not destination_binary.exists():
            continue
        preserved_binary = preserved_root / relative_binary_path
        preserved_binary.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(destination_binary, preserved_binary)
        preserved.append((relative_binary_path.as_posix(), preserved_binary))

    try:
        copytree_replace(src, dst)
        for relative_binary_path, preserved_binary in preserved:
            restored_binary = dst / relative_binary_path
            if restored_binary.exists():
                continue
            restored_binary.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(preserved_binary, restored_binary)
            if not relative_binary_path.endswith(".exe"):
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


def stop_unix_runtime_processes(search_roots: list[Path], proc_root: Path = Path("/proc")) -> None:
    if sys.platform == "win32":
        return
    root_prefixes = [
        _normalize_process_path(root.resolve()) + "/" for root in search_roots if root.exists()
    ]
    if not root_prefixes or not proc_root.exists():
        return

    current_pid = os.getpid()
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
        if _is_sky_cua_runtime_process(exe, cwd, cmdline, root_prefixes):
            matches.append(pid)

    for pid in matches:
        _terminate_process(pid)


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
    process_name = Path(exe or "").name
    if process_name not in {"sky-cua-client", "sky-cua-service"}:
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
        root / ".codex-plugin" / "plugin.json",
        root / ".mcp.json",
        root / "skills" / "computer-use-workflows" / "SKILL.md",
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


def release_plugin_root(marketplace_root: Path) -> Path:
    return marketplace_root / "plugins" / PLUGIN_NAME


def marketplace_manifest_path(marketplace_root: Path) -> Path:
    return marketplace_root / ".agents" / "plugins" / "marketplace.json"


def write_release_marketplace(marketplace_root: Path) -> Path:
    manifest_path = marketplace_manifest_path(marketplace_root)
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest = {
        "name": RELEASE_MARKETPLACE_NAME,
        "interface": {
            "displayName": RELEASE_MARKETPLACE_NAME,
        },
        "plugins": [
            {
                "name": PLUGIN_NAME,
                "source": {
                    "source": "local",
                    "path": f"./plugins/{PLUGIN_NAME}",
                },
                "policy": {
                    "installation": "AVAILABLE",
                    "authentication": "ON_INSTALL",
                },
                "category": PLUGIN_CATEGORY,
            }
        ],
    }
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return manifest_path


def ensure_plugins_feature_enabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "plugins", "true")


def ensure_apps_feature_disabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "apps", "false")


def ensure_fast_mode_enabled(config_text: str) -> str:
    return upsert_toml_key(config_text, "features", "fast_mode", "true")


def ensure_fast_service_tier(config_text: str) -> str:
    config_text = remove_profile_service_tier_overrides(config_text)
    return upsert_top_level_toml_key(config_text, "service_tier", '"fast"')


def ensure_plugin_enabled(config_text: str) -> str:
    return set_plugin_enabled(config_text, PLUGIN_ID, enabled=True)


def set_plugin_enabled(config_text: str, plugin_id: str, *, enabled: bool) -> str:
    rendered = "true" if enabled else "false"
    return upsert_toml_key(config_text, f'plugins."{plugin_id}"', "enabled", rendered)


def ensure_release_marketplace_config(config_text: str, marketplace_root: Path) -> str:
    header = f"marketplaces.{RELEASE_MARKETPLACE_NAME}"
    config_text = upsert_toml_key(config_text, header, "last_updated", toml_string(utc_timestamp()))
    config_text = upsert_toml_key(config_text, header, "source_type", '"local"')
    return upsert_toml_key(
        config_text,
        header,
        "source",
        toml_string(codex_config_path(marketplace_root)),
    )


def toml_string(value: str) -> str:
    return json.dumps(value)


def codex_config_path(path: Path) -> str:
    resolved = str(path.resolve())
    if sys.platform == "win32" and not resolved.startswith("\\\\?\\"):
        return f"\\\\?\\{resolved}"
    return resolved


def utc_timestamp() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


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


def update_codex_config(
    config_path: Path,
    *,
    disable_apps: bool = False,
    fast_service_tier: bool = False,
    plugin_id: str = PLUGIN_ID,
    plugin_enabled: bool = True,
    disabled_plugin_ids: list[str] | None = None,
    marketplace_root: Path | None = None,
) -> None:
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_text = config_path.read_text() if config_path.exists() else ""
    config_text = ensure_plugins_feature_enabled(config_text)
    if disable_apps:
        config_text = ensure_apps_feature_disabled(config_text)
    if fast_service_tier:
        config_text = ensure_fast_mode_enabled(config_text)
        config_text = ensure_fast_service_tier(config_text)
    if marketplace_root is not None:
        config_text = ensure_release_marketplace_config(config_text, marketplace_root)
    config_text = set_plugin_enabled(config_text, plugin_id, enabled=plugin_enabled)
    for disabled_plugin_id in disabled_plugin_ids or []:
        if disabled_plugin_id != plugin_id:
            config_text = set_plugin_enabled(config_text, disabled_plugin_id, enabled=False)
    config_path.write_text(config_text)
