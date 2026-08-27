//! Shared test-only overlay host support.

#[cfg(unix)]
pub(crate) fn write_fake_overlay_host(path: &std::path::Path) {
    write_overlay_host(path, 0.02, true);
}

#[cfg(unix)]
pub(crate) fn write_stalled_overlay_host(path: &std::path::Path) {
    write_overlay_host(path, 0.35, false);
}

#[cfg(unix)]
fn write_overlay_host(path: &std::path::Path, wait_delay_seconds: f64, reply_to_wait: bool) {
    use std::os::unix::fs::PermissionsExt;

    use sky_cua_overlay_host::OVERLAY_HOST_PROTOCOL_VERSION;

    let script = format!(
        r#"#!/usr/bin/env python3
import json
import os
import socket
import sys
import time

if len(sys.argv) != 4 or sys.argv[1:3] != ["serve", "--socket"]:
    raise SystemExit(f"unexpected argv: {{sys.argv!r}}")

socket_path = sys.argv[3]
try:
    os.unlink(socket_path)
except FileNotFoundError:
    pass

server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(socket_path)
server.listen(8)
state = None
motion = None
capabilities = {{
    "backend": "wayland_layer_shell",
    "visible_overlay": True,
    "screenshot_synthetic_cursor": False,
    "click_through": True,
    "capture_exclusion": False,
    "needs_user_install": False,
    "reason": "fake host",
}}

while True:
    conn, _ = server.accept()
    with conn:
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(4096)
            if not chunk:
                break
            data += chunk
        if not data.strip():
            continue
        message = json.loads(data.decode("utf-8"))
        kind = message["kind"]
        with open(socket_path + ".requests", "a") as request_log:
            request_log.write(kind + "\n")
        diagnostics = []
        applied_sequence = None
        arrival_wait = None
        if kind == "set_cursor":
            state = message.get("state")
            if state is not None and state.get("native_point") is not None:
                point = state["native_point"]
                motion = {{
                    "x": point["x"],
                    "y": point["y"],
                    "heading_deg": 0.0,
                    "speed": 100.0,
                    "settled": False,
                    "pending_gesture_feedback": False,
                }}
        elif kind == "animate_gesture":
            if motion is not None:
                motion["settled"] = False
                motion["pending_gesture_feedback"] = True
        elif kind == "wait_for_arrival":
            wait = message.get("arrival_wait")
            if wait is not None:
                time.sleep({wait_delay_seconds})
                if not {reply_to_wait}:
                    continue
                if motion is not None:
                    motion["speed"] = 0.0
                    motion["settled"] = True
                    motion["pending_gesture_feedback"] = False
                arrival_wait = {{
                    "sequence": wait["sequence"],
                    "outcome": "arrived",
                }}
                open(socket_path + ".arrived", "w").close()
            elif motion is not None:
                motion["speed"] = 0.0
                motion["settled"] = True
                motion["pending_gesture_feedback"] = False
        elif kind == "hide":
            if state is not None:
                state["visible"] = False
            applied_sequence = message.get("sequence")
            if message.get("reason"):
                diagnostics.append({{
                    "code": "OverlayCursorHidden",
                    "message": "Overlay host hid the cursor.",
                    "details": message["reason"],
                }})
        elif kind == "show":
            if state is not None:
                state["visible"] = True
        reply = {{
            "version": {version},
            "ok": True,
            "capabilities": capabilities,
            "lifecycle_state": "backend_ready",
            "applied_sequence": applied_sequence,
            "state": state,
            "motion": motion,
            "arrival_wait": arrival_wait,
            "diagnostics": diagnostics,
        }}
        conn.sendall(json.dumps(reply).encode("utf-8") + b"\n")
        if kind == "shutdown":
            break

server.close()
try:
    os.unlink(socket_path)
except FileNotFoundError:
    pass
"#,
        version = OVERLAY_HOST_PROTOCOL_VERSION,
        wait_delay_seconds = wait_delay_seconds,
        reply_to_wait = if reply_to_wait { "True" } else { "False" },
    );
    std::fs::write(path, script).expect("write fake overlay host");
    let mut permissions = std::fs::metadata(path)
        .expect("fake host metadata")
        .permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).expect("chmod fake overlay host");
}
