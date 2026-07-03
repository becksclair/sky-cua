"""KDE ScreenCast portal recorder for the desktop overlay motion harness.

Opens an ``org.freedesktop.portal.ScreenCast`` session (monitor source,
persistent restore token), receives the PipeWire stream fd + node id, and
records the stream to MP4 with a ``gst-launch-1.0`` child. The portal calls go
through PyGObject (``Gio.DBusProxy``); typed via the ``pygobject-stubs`` dev
dependency.

Portal flow (ScreenCast interface version 4, verified on KDE Plasma Wayland):

1. ``CreateSession`` -> session handle.
2. ``SelectSources`` with ``types=1`` (monitor), ``persist_mode=2``
   (persistent) and the saved ``restore_token`` when one exists.
3. ``Start`` -> stream node id (+ a fresh ``restore_token`` to persist). The
   FIRST run without a token shows one interactive KDE share dialog;
   subsequent runs restore silently.
4. ``OpenPipeWireRemote`` -> the PipeWire remote fd handed to ``pipewiresrc``.

The recording child is stopped with SIGINT so ``gst-launch-1.0 -e`` flushes
EOS through the muxer and the MP4 finalizes cleanly.

Recordings capture the operator's live desktop — sensitive, never committed;
restore tokens are per-user portal state and belong under the gitignored
artifacts dir, never in the repo.
"""

from __future__ import annotations

import contextlib
import os
import secrets
import shutil
import signal
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any

PORTAL_BUS_NAME = "org.freedesktop.portal.Desktop"
PORTAL_OBJECT_PATH = "/org/freedesktop/portal/desktop"
SCREENCAST_INTERFACE = "org.freedesktop.portal.ScreenCast"
REQUEST_INTERFACE = "org.freedesktop.portal.Request"

#: ``SelectSources`` ``types`` bitmask: monitor sources only.
SOURCE_TYPE_MONITOR = 1
#: ``SelectSources`` ``persist_mode``: token persists across app restarts.
PERSIST_MODE_PERSISTENT = 2

#: Generous default: the first ``Start`` shows an interactive share dialog.
DEFAULT_REQUEST_TIMEOUT_S = 120.0


class PortalScreenCastError(RuntimeError):
    """A portal request failed, timed out, or was denied by the user."""


# ---------------------------------------------------------------------------
# Pure helpers (unit-testable without a portal, gi, or gstreamer)
# ---------------------------------------------------------------------------


def gst_pipeline_args(fd: int, node_id: int, output: Path) -> list[str]:
    """``gst-launch-1.0`` argv recording PipeWire node ``node_id`` on ``fd``.

    ``-e`` makes SIGINT flush EOS through ``mp4mux`` so the file finalizes
    cleanly instead of truncating without a moov atom.
    """
    return [
        "gst-launch-1.0",
        "-e",
        "pipewiresrc",
        f"fd={fd}",
        f"path={node_id}",
        "!",
        "videoconvert",
        "!",
        "x264enc",
        "tune=zerolatency",
        "!",
        "h264parse",
        "!",
        "mp4mux",
        "!",
        "filesink",
        f"location={output}",
    ]


def read_restore_token(path: Path) -> str | None:
    """The persisted restore token, or ``None`` when absent/blank."""
    try:
        token = path.read_text(encoding="utf-8").strip()
    except OSError:
        return None
    return token or None


def write_restore_token(path: Path, token: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(token + "\n", encoding="utf-8")


def probe() -> str | None:
    """``None`` when the portal recorder looks usable, else the human reason."""
    if shutil.which("gst-launch-1.0") is None:
        return "gst-launch-1.0 is not on PATH (install gstreamer + gst-plugin-pipewire)"
    try:
        import gi  # noqa: F401  # pyright: ignore[reportUnusedImport]
    except ImportError:
        return "PyGObject (gi) is not importable in this interpreter"
    if not os.environ.get("DBUS_SESSION_BUS_ADDRESS") and not os.environ.get("XDG_RUNTIME_DIR"):
        return "no session bus environment (DBUS_SESSION_BUS_ADDRESS/XDG_RUNTIME_DIR unset)"
    return None


# ---------------------------------------------------------------------------
# Portal session (requires gi + a desktop portal on the session bus)
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ScreenCastStream:
    """An opened monitor stream: the PipeWire remote fd (owned by the caller),
    the stream node id, and the restore token to persist (when granted)."""

    fd: int
    node_id: int
    restore_token: str | None


def _call_with_response(
    bus: Any,
    proxy: Any,
    method: str,
    build_params: Any,
    *,
    timeout_s: float,
) -> dict[str, Any]:
    """Portal request/response round trip.

    Portal methods return a ``Request`` object path and deliver results via its
    ``Response`` signal; the path is predictable from our unique name plus the
    ``handle_token``, so subscribe first, call, then pump a ``GLib.MainLoop``
    until the response (or the timeout) fires.
    """
    from gi.repository import Gio, GLib

    handle_token = f"skycua_{secrets.token_hex(4)}"
    sender = (bus.get_unique_name() or "").removeprefix(":").replace(".", "_")
    request_path = f"{PORTAL_OBJECT_PATH}/request/{sender}/{handle_token}"

    loop = GLib.MainLoop()
    outcome: dict[str, Any] = {}

    def on_response(
        _bus: Any,
        _sender: Any,
        _path: Any,
        _interface: Any,
        _signal: Any,
        parameters: Any,
    ) -> None:
        code, results = parameters.unpack()
        outcome["code"] = code
        outcome["results"] = results
        loop.quit()

    subscription = bus.signal_subscribe(
        PORTAL_BUS_NAME,
        REQUEST_INTERFACE,
        "Response",
        request_path,
        None,
        Gio.DBusSignalFlags.NO_MATCH_RULE,
        on_response,
    )

    def on_timeout() -> bool:
        # A Response that already arrived wins; never mark it timed out too.
        if "code" not in outcome:
            outcome["timed_out"] = True
        loop.quit()
        return False

    timeout_source = GLib.timeout_add(int(timeout_s * 1000), on_timeout)
    try:
        returned = proxy.call_sync(
            method,
            build_params(handle_token),
            Gio.DBusCallFlags.NONE,
            int(timeout_s * 1000),
            None,
        )
        returned_path = returned.unpack()[0] if returned is not None else None
        if isinstance(returned_path, str) and returned_path != request_path:
            # Old portals ignore handle_token and mint their own request path;
            # re-subscribe there (any response raced in between is covered by
            # the timeout below rather than a hang).
            bus.signal_unsubscribe(subscription)
            subscription = bus.signal_subscribe(
                PORTAL_BUS_NAME,
                REQUEST_INTERFACE,
                "Response",
                returned_path,
                None,
                Gio.DBusSignalFlags.NO_MATCH_RULE,
                on_response,
            )
        loop.run()
    finally:
        bus.signal_unsubscribe(subscription)
        with contextlib.suppress(Exception):
            GLib.source_remove(timeout_source)

    if outcome.get("timed_out"):
        raise PortalScreenCastError(f"portal {method} timed out after {timeout_s:.0f}s")
    code = outcome.get("code")
    if code != 0:
        raise PortalScreenCastError(f"portal {method} was denied or failed (response code {code})")
    results = outcome.get("results")
    return dict(results) if isinstance(results, dict) else {}


def open_monitor_stream(
    restore_token: str | None = None,
    *,
    timeout_s: float = DEFAULT_REQUEST_TIMEOUT_S,
) -> ScreenCastStream:
    """Opens a monitor ScreenCast stream and the PipeWire remote fd for it.

    The caller owns the returned fd (pass it to the gst child, then close it).
    Raises :class:`PortalScreenCastError` on denial/timeout and ``ImportError``
    when PyGObject is unavailable (see :func:`probe`).
    """
    from gi.repository import Gio, GLib

    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    proxy = Gio.DBusProxy.new_sync(
        bus,
        Gio.DBusProxyFlags.NONE,
        None,
        PORTAL_BUS_NAME,
        PORTAL_OBJECT_PATH,
        SCREENCAST_INTERFACE,
        None,
    )

    session_token = f"skycua_{secrets.token_hex(4)}"

    def create_session_params(handle_token: str) -> Any:
        return GLib.Variant(
            "(a{sv})",
            (
                {
                    "handle_token": GLib.Variant("s", handle_token),
                    "session_handle_token": GLib.Variant("s", session_token),
                },
            ),
        )

    created = _call_with_response(
        bus, proxy, "CreateSession", create_session_params, timeout_s=timeout_s
    )
    session_handle = created.get("session_handle")
    if not isinstance(session_handle, str) or not session_handle:
        raise PortalScreenCastError("portal CreateSession returned no session handle")

    def select_sources_params(handle_token: str) -> Any:
        options: dict[str, Any] = {
            "handle_token": GLib.Variant("s", handle_token),
            "types": GLib.Variant("u", SOURCE_TYPE_MONITOR),
            "multiple": GLib.Variant("b", False),
            "persist_mode": GLib.Variant("u", PERSIST_MODE_PERSISTENT),
        }
        if restore_token:
            options["restore_token"] = GLib.Variant("s", restore_token)
        return GLib.Variant("(oa{sv})", (session_handle, options))

    _call_with_response(bus, proxy, "SelectSources", select_sources_params, timeout_s=timeout_s)

    def start_params(handle_token: str) -> Any:
        return GLib.Variant(
            "(osa{sv})",
            (session_handle, "", {"handle_token": GLib.Variant("s", handle_token)}),
        )

    started = _call_with_response(bus, proxy, "Start", start_params, timeout_s=timeout_s)
    streams = started.get("streams")
    if not isinstance(streams, list) or not streams:
        raise PortalScreenCastError("portal Start returned no streams")
    node_id = int(streams[0][0])
    new_token = started.get("restore_token")

    fd_reply, fd_list = proxy.call_with_unix_fd_list_sync(
        "OpenPipeWireRemote",
        GLib.Variant("(oa{sv})", (session_handle, {})),
        Gio.DBusCallFlags.NONE,
        int(timeout_s * 1000),
        None,
        None,
    )
    if fd_list is None:
        raise PortalScreenCastError("portal OpenPipeWireRemote returned no fd list")
    (fd_index,) = fd_reply.unpack()
    fd = fd_list.steal_fds()[int(fd_index)]

    return ScreenCastStream(
        fd=int(fd),
        node_id=node_id,
        restore_token=new_token if isinstance(new_token, str) and new_token else None,
    )


class ScreenCastRecorder:
    """Owns one portal monitor stream and its ``gst-launch-1.0`` MP4 child."""

    def __init__(self, token_path: Path, *, timeout_s: float = DEFAULT_REQUEST_TIMEOUT_S) -> None:
        self._token_path = token_path
        self._timeout_s = timeout_s
        self._child: subprocess.Popen[bytes] | None = None
        self._fd: int | None = None

    def start(self, output: Path) -> None:
        """Opens the portal stream and starts recording to ``output``.

        On any failure after the stream opened, the PipeWire fd is closed
        (via :meth:`stop`) before the error propagates, so a failed start
        never leaks the portal stream.
        """
        stream = open_monitor_stream(
            read_restore_token(self._token_path), timeout_s=self._timeout_s
        )
        self._fd = stream.fd
        try:
            if stream.restore_token:
                write_restore_token(self._token_path, stream.restore_token)
            output.parent.mkdir(parents=True, exist_ok=True)
            output.unlink(missing_ok=True)
            self._child = subprocess.Popen(
                gst_pipeline_args(stream.fd, stream.node_id, output),
                pass_fds=(stream.fd,),
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except Exception:
            self.stop()
            raise

    def stop(self) -> None:
        """SIGINTs the recorder (clean EOS/mux) and closes the PipeWire fd."""
        child = self._child
        self._child = None
        if child is not None and child.poll() is None:
            child.send_signal(signal.SIGINT)
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill()
                with contextlib.suppress(subprocess.TimeoutExpired):
                    child.wait(timeout=5)
        if self._fd is not None:
            with contextlib.suppress(OSError):
                os.close(self._fd)
            self._fd = None
