#!/usr/bin/env python3
"""Place the desktop agent cursor at a screen fraction and capture it with
spectacle, for visual testing/debugging of the wgpu layer-shell overlay.

It starts an ISOLATED smoke service on a private socket (never the operator's
installed daemon), drives the cursor via the proven snapshot_id flow, captures
the real compositor with spectacle (KWin has no wlr-screencopy, so grim fails),
and crops at the monitor's TRUE native resolution (the spectacle full capture is
2x the logical virtual desktop). It owns and tears down the service itself, so
no background process lingers for the caller to wait on.

Usage:
    python3 capture.py [FX FY]
        FX FY  cursor position as a fraction of the primary capture
               (default 0.4 0.45). Pick a spot over light content to judge the
               shadow; the glyph/smoke read on any background.

Run it with the Bash sandbox DISABLED so the service can reach the KDE portal.
Build first: cargo build --release -p sky-cua-overlay-host
"""

from __future__ import annotations

import os
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
SERVICE_BIN = REPO / "target/release/sky-cua-service"
OVERLAY_HOST_BIN = REPO / "target/release/sky-cua-overlay-host"
ART = Path(os.environ.get("SKY_CUA_CURSOR_DEBUG_DIR", "/tmp/agent-cursor-debug"))
SOCK = ART / "svc.sock"
TIMEOUT = 20.0


def fail(message: str) -> None:
    print(f"ERROR: {message}", file=sys.stderr)
    raise SystemExit(1)


def connectable() -> bool:
    try:
        with socket.socket(socket.AF_UNIX) as probe:
            probe.settimeout(0.4)
            probe.connect(str(SOCK))
        return True
    except OSError:
        return False


def crop_native(full_path: Path, native_point: dict | None) -> None:
    """Crop the glyph at TRUE native resolution. The spectacle full capture is 2x
    the logical desktop, so judging the cursor at a high nearest-neighbour zoom of
    the raw capture fakes aliasing; downsample 2x first to see real pixels."""
    if native_point is None:
        print("(no native point — inspect the full capture)")
        return
    try:
        from PIL import Image
    except ImportError:
        print("(PIL unavailable — inspect the full capture directly)")
        return
    image = Image.open(full_path).convert("RGB")
    native = image.resize((image.size[0] // 2, image.size[1] // 2), Image.LANCZOS)
    nx, ny = int(native_point["x"]), int(native_point["y"])
    glyph = native.crop((nx - 30, ny - 30, nx + 50, ny + 56))
    glyph.resize((glyph.size[0] * 7, glyph.size[1] * 7), Image.NEAREST).save(
        ART / "cursor_native7x.png"
    )
    cx, cy = nx * 2, ny * 2
    context = image.crop((cx - 260, cy - 210, cx + 320, cy + 340))
    context.resize((int(context.size[0] * 1.25), int(context.size[1] * 1.25)), Image.LANCZOS).save(
        ART / "cursor_context.png"
    )
    print(f"  {ART / 'cursor_native7x.png'}  (true-res glyph zoom)")
    print(f"  {ART / 'cursor_context.png'}  (in-context view)")


def main() -> None:
    positional = [arg for arg in sys.argv[1:] if not arg.startswith("-")]
    fx = float(positional[0]) if len(positional) >= 1 else 0.4
    fy = float(positional[1]) if len(positional) >= 2 else 0.45

    for binary in (SERVICE_BIN, OVERLAY_HOST_BIN):
        if not binary.exists():
            fail(f"missing {binary} — run `cargo build --release -p sky-cua-overlay-host` first")

    sys.path.insert(0, str(REPO / "scripts"))
    os.environ["SKY_CUA_SKIP_LOCAL_BUILD"] = "1"
    os.environ["SKY_CUA_SERVICE_BIN"] = str(SERVICE_BIN)
    os.environ["SKY_CUA_OVERLAY_HOST_BIN"] = str(OVERLAY_HOST_BIN)
    import live_agent_cursor_kde_smoke as smoke
    from _overlay_host import terminate_leftover_hosts

    ART.mkdir(parents=True, exist_ok=True)
    # The isolated service derives its overlay host socket from its own IPC
    # socket dir (SOCK's parent), so the host lives on <ART>/agent-cursor.sock
    # — not the operator's $XDG_RUNTIME_DIR one. Clear only a leftover host on
    # THAT socket, never the operator's live service-owned host.
    host_socket = SOCK.parent / "agent-cursor.sock"
    terminate_leftover_hosts(host_socket)
    SOCK.unlink(missing_ok=True)

    env = dict(os.environ)
    env.update(
        {
            "SKY_CUA_SERVICE_SOCKET_PATH": str(SOCK),
            "SKY_CUA_AGENT_CURSOR": "always",
            "SKY_CUA_OVERLAY_BACKEND": "wayland-layer-shell",
            "SKY_CUA_OVERLAY_HOST_PATH": str(OVERLAY_HOST_BIN),
            # debug-visible: do not hide the cursor for capture.
            "SKY_CUA_SCREENSHOT_CURSOR": "never",
            "SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE": "never",
        }
    )
    log_handle = (ART / "service.log").open("wb")
    service = subprocess.Popen(
        [str(SERVICE_BIN), "daemon"],
        cwd=str(REPO),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=log_handle,
    )
    try:
        deadline = time.time() + 20.0
        while time.time() < deadline and not connectable():
            if service.poll() is not None:
                fail(
                    f"service exited at startup (rc={service.returncode}); see {ART / 'service.log'}"
                )
            time.sleep(0.2)
        if not connectable():
            fail("service never accepted connections")
        time.sleep(0.5)

        with smoke.ServiceClient(SOCK, timeout=TIMEOUT) as client:
            _r, snap, cap, _p = smoke.screenshot_capture(client, request_timeout=TIMEOUT)
            pixel_size = cap.get("pixel_size") or {}
            width = float(pixel_size.get("width") or 2560)
            height = float(pixel_size.get("height") or 1440)
            point = (round(fx * width, 1), round(fy * height, 1))
            native_point = smoke.native_point_from_capture(cap, point)
            state = {
                "visible": True,
                "sequence": 1,
                "model_point": {
                    "x": point[0],
                    "y": point[1],
                    "coordinate_space": "stream_pixels",
                    "mapping_id": cap.get("mapping_id"),
                },
                "snapshot_id": snap["snapshot_id"],
                "source_action": "click",
                # MUST be ~now: a stale/zero timestamp reads as a decayed cursor
                # and renders nothing. snapshot_id is likewise required.
                "updated_at_ms": int(time.time() * 1000),
            }
            if native_point:
                state["native_point"] = native_point
            client.call({"type": "set_agent_cursor", "state": state}, timeout=TIMEOUT)
            time.sleep(0.5)
            state["sequence"] = 2
            state["updated_at_ms"] = int(time.time() * 1000)
            client.call({"type": "set_agent_cursor", "state": state}, timeout=TIMEOUT)
            time.sleep(0.4)
            full = ART / "capture.png"
            subprocess.run(["spectacle", "-b", "-n", "-f", "-o", str(full)], check=True, timeout=25)
            time.sleep(0.3)
        print(f"captured {full}  native={native_point}")
        crop_native(full, native_point)
    finally:
        service.send_signal(signal.SIGTERM)
        try:
            service.wait(timeout=5)
        except Exception:
            service.kill()
        # Reap the isolated host on this run's socket only.
        terminate_leftover_hosts(host_socket)


if __name__ == "__main__":
    main()
