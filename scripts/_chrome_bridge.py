"""Shared Chrome + Codex-extension + native-host bring-up helpers.

Factored out of ``live_chrome_host_client_smoke.py`` so both the chrome-host
smoke and the codex CUA smoke can launch a real browser with the sky-cua Codex
extension loaded, register the native-messaging host manifest, and wait for the
native-host UNIX socket to appear. ``google-chrome`` support (the testing VM's
browser) is added here alongside the existing ``brave``/``chromium`` paths.

These helpers are launch/transport plumbing only; the proof/assertion logic
stays in the individual smokes.
"""

from __future__ import annotations

import http.server
import json
import os
import shutil
import subprocess
import threading
import time
import urllib.request
from collections.abc import Iterator
from contextlib import contextmanager, suppress
from pathlib import Path
from typing import NamedTuple, cast

REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_EXTENSION_ID = "hehggadaopoacecdllhhajmbjkdcmajg"
DEFAULT_EXTENSION_VERSION_DIR = "1.2.27221.15725_0"
DEFAULT_EXTENSION_DIR = (
    Path.home()
    / ".config/google-chrome/Default/Extensions"
    / DEFAULT_EXTENSION_ID
    / DEFAULT_EXTENSION_VERSION_DIR
)
FALLBACK_EXTENSION_DIR = (
    REPO_ROOT / "resources/chrome-extension/codex" / DEFAULT_EXTENSION_VERSION_DIR
)
DEFAULT_HOST_PATH = REPO_ROOT / "target/debug/sky-cua-chrome-host"
HOST_NAME = "com.openai.codexextension"


class BrowserSelection(NamedTuple):
    name: str
    command: str


class ManifestRestore(NamedTuple):
    path: Path
    previous_content: bytes | None


def default_extension_dir() -> Path:
    if DEFAULT_EXTENSION_DIR.exists():
        return DEFAULT_EXTENSION_DIR
    return FALLBACK_EXTENSION_DIR


def browser_command(choice: str) -> BrowserSelection:
    # ``auto`` keeps the original brave/chromium preference first (the chrome-host
    # smoke's historical behavior) and falls back to google-chrome, which is the
    # browser shipped in the testing VM.
    chrome_candidates = [
        ("chrome", "google-chrome"),
        ("chrome", "google-chrome-stable"),
        ("chrome", "chrome"),
    ]
    candidates = [
        ("brave", "brave"),
        ("brave", "brave-browser"),
        ("chromium", "chromium"),
        ("chromium", "chromium-browser"),
        *chrome_candidates,
    ]
    if choice == "brave":
        candidates = [("brave", "brave"), ("brave", "brave-browser")]
    elif choice == "chromium":
        candidates = [("chromium", "chromium"), ("chromium", "chromium-browser")]
    elif choice == "chrome":
        candidates = chrome_candidates
    for browser_name, candidate in candidates:
        command = shutil.which(candidate)
        if command is not None:
            return BrowserSelection(browser_name, command)
    raise FileNotFoundError(f"no browser command found for {choice}")


def wait_for_devtools_port(user_data_dir: Path, proc: subprocess.Popen[str]) -> str:
    active_port = user_data_dir / "DevToolsActivePort"
    deadline = time.time() + 20
    while time.time() < deadline:
        if active_port.exists():
            # Chrome creates the file, then writes the port line; observing it
            # mid-write yields no lines. Keep polling until the first line lands.
            lines = active_port.read_text(encoding="utf-8").splitlines()
            if lines and lines[0].strip():
                return lines[0].strip()
        if proc.poll() is not None:
            stderr = proc.stderr.read() if proc.stderr is not None else ""
            raise RuntimeError(f"browser exited early with {proc.returncode}\n{stderr}")
        time.sleep(0.1)
    raise TimeoutError("DevToolsActivePort did not appear")


def load_targets(port: str) -> list[dict[str, object]]:
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/list", timeout=3) as response:
        value = json.loads(response.read().decode("utf-8"))
    if not isinstance(value, list):
        raise RuntimeError(f"unexpected DevTools target list: {value!r}")
    return [target for target in value if isinstance(target, dict)]


def wait_for_extension_target(port: str, extension_id: str) -> dict[str, object]:
    deadline = time.time() + 20
    while time.time() < deadline:
        for target in load_targets(port):
            target_type = target.get("type")
            target_url = str(target.get("url", ""))
            if target_type in {"service_worker", "background_page"} and extension_id in target_url:
                return target
        time.sleep(0.25)
    raise TimeoutError(f"extension target for {extension_id} did not appear")


def wait_for_socket(socket_dir: Path) -> Path:
    deadline = time.time() + 15
    while time.time() < deadline:
        sockets = sorted(socket_dir.glob("extension-*.sock"))
        if sockets:
            return sockets[0]
        time.sleep(0.1)
    raise TimeoutError(f"native host socket did not appear in {socket_dir}")


def native_manifest_path(browser_name: str) -> Path:
    home = Path.home()
    if browser_name == "brave":
        return (
            home / ".config/BraveSoftware/Brave-Browser/NativeMessagingHosts" / f"{HOST_NAME}.json"
        )
    if browser_name == "chromium":
        return home / ".config/chromium/NativeMessagingHosts" / f"{HOST_NAME}.json"
    if browser_name == "chrome":
        return home / ".config/google-chrome/NativeMessagingHosts" / f"{HOST_NAME}.json"
    raise ValueError(f"unsupported browser for native manifest: {browser_name}")


def install_temp_manifest(
    browser_name: str,
    extension_id: str,
    host_path: Path,
    *,
    user_data_dir: Path | None = None,
) -> ManifestRestore:
    manifest_path = native_manifest_path(browser_name)
    previous = manifest_path.read_bytes() if manifest_path.exists() else None
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest = {
        "name": HOST_NAME,
        "description": "sky-cua Chrome native messaging host live smoke",
        "type": "stdio",
        "path": str(host_path),
        "allowed_origins": [f"chrome-extension://{extension_id}/"],
    }
    manifest_text = json.dumps(manifest, indent=2) + "\n"
    manifest_path.write_text(manifest_text, encoding="utf-8")
    # When Chrome runs with --user-data-dir it searches <user-data-dir>/
    # NativeMessagingHosts/ for user-level host manifests, NOT the standard
    # ~/.config/<browser>/NativeMessagingHosts/. Write there too so the native
    # bridge connects under a custom profile. The profile dir is ephemeral, so no
    # restore is tracked for it (only the shared standard path is restored).
    if user_data_dir is not None:
        profile_manifest_dir = user_data_dir / "NativeMessagingHosts"
        profile_manifest_dir.mkdir(parents=True, exist_ok=True)
        (profile_manifest_dir / f"{HOST_NAME}.json").write_text(manifest_text, encoding="utf-8")
    return ManifestRestore(manifest_path, previous)


def restore_manifest(restore: ManifestRestore | None) -> bool:
    if restore is None:
        return False
    if restore.previous_content is None:
        with suppress(FileNotFoundError):
            restore.path.unlink()
        return True
    restore.path.write_bytes(restore.previous_content)
    return True


def host_process(host_path: Path, pid: int) -> dict[str, object] | None:
    resolved = str(host_path)
    proc_dir = Path("/proc") / str(pid)
    try:
        raw_cmdline = (proc_dir / "cmdline").read_bytes()
    except OSError:
        return None
    parts = [part.decode("utf-8", "replace") for part in raw_cmdline.split(b"\0") if part]
    if parts and parts[0] == resolved:
        return {"pid": pid, "cmdline": parts}
    return None


def host_pid_from_socket(socket_path: Path) -> int:
    parts = socket_path.stem.split("-")
    if len(parts) < 2 or not parts[1].isdecimal():
        raise RuntimeError(f"could not parse host pid from socket path: {socket_path}")
    return int(parts[1])


def wait_for_host_process(host_path: Path, pid: int) -> dict[str, object] | None:
    deadline = time.time() + 5
    while time.time() < deadline:
        match = host_process(host_path, pid)
        if match is not None:
            return match
        time.sleep(0.1)
    return None


def launch_browser(
    command: str,
    *,
    user_data_dir: Path,
    extension_dir: Path,
    socket_dir: Path,
    sessions_dir: Path,
    load_extension: bool = True,
    initial_url: str = "about:blank",
    stderr_path: Path | None = None,
) -> subprocess.Popen[str]:
    env = os.environ.copy()
    env["CODEX_BROWSER_USE_SOCKET_DIR"] = str(socket_dir)
    env["SKY_CUA_BROWSER_USE_SOCKET_DIR"] = str(socket_dir)
    env["CODEX_BROWSER_USE_SESSIONS_DIR"] = str(sessions_dir)
    env["SKY_CUA_BROWSER_USE_SESSIONS_DIR"] = str(sessions_dir)
    env["SKY_CUA_CHROME_HOST_TRACE"] = "1"
    args = [
        command,
        f"--user-data-dir={user_data_dir}",
        "--remote-debugging-port=0",
        "--remote-allow-origins=*",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-sync",
        # The Codex extension drives tabs via chrome.debugger; without this Chrome
        # shows an "[extension] started debugging this browser" infobar that, left
        # unattended, leads to the debugger detaching and CDP Page.* commands
        # (navigate/enable) timing out. Silence it so the bridge stays attached.
        "--silent-debugger-extension-api",
        # Always capture Chrome's verbose log alongside the run so debugger /
        # devtools / extension events (e.g. why chrome.debugger detaches or a CDP
        # command stalls) are inspectable after the fact. Goes to a file so the
        # piped stderr below does not flood and block Chrome under --v=1. The
        # vmodule bumps the browser-automation-relevant modules (the extension
        # chrome.debugger API, the DevTools agent/session backend, and the MV3
        # service-worker lifecycle) so a wedged/detached debugger session is
        # actually recorded rather than swallowed at the global level.
        "--enable-logging",
        "--v=1",
        "--vmodule=*debugger*=3,*devtools*=2,*service_worker*=1",
        f"--log-file={user_data_dir / 'chrome_debug.log'}",
    ]
    # When load_extension is False the caller installs the extension through the
    # Chrome UI ("Load unpacked"); the `--load-extension` command-line switch is
    # disabled by default in Chrome 137+ anyway, so the UI path is the durable one.
    if load_extension:
        args.append(f"--disable-extensions-except={extension_dir}")
        args.append(f"--load-extension={extension_dir}")
    args.append("--ozone-platform=wayland")
    args.append(initial_url)
    # Chrome inherits its stderr to any native-messaging host it spawns, so the
    # sky-cua-chrome-host trace (SKY_CUA_CHROME_HOST_TRACE) lands here. Redirect it
    # to a file when a path is given so the relay trace is a retrievable artifact;
    # otherwise keep the pipe that terminate_browser drains for its stderr tail.
    stderr: int | object = subprocess.PIPE
    if stderr_path is not None:
        stderr = open(stderr_path, "w", encoding="utf-8")  # noqa: SIM115 (child owns the fd)
    return subprocess.Popen(
        args,
        env=env,
        stdout=subprocess.PIPE,
        stderr=stderr,
        text=True,
    )


def terminate_browser(proc: subprocess.Popen[str], keep_open: bool) -> str:
    if keep_open:
        return ""
    proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)
    return proc.stderr.read() if proc.stderr is not None else ""


def _html_fixture_handler(body: bytes, route: str) -> type[http.server.BaseHTTPRequestHandler]:
    class _Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *_args: object) -> None:
            del format, _args
            return

    del route
    return _Handler


@contextmanager
def serve_html_fixture(html: bytes, *, route: str = "/fixture.html") -> Iterator[str]:
    """Serve a single static HTML page on a loopback port; yield its URL."""
    handler = _html_fixture_handler(html, route)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, name="sky-cua-html-fixture", daemon=True)
    thread.start()
    try:
        host, port = cast(tuple[str, int], server.server_address)
        yield f"http://{host}:{port}{route}"
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
