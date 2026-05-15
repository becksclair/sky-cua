#!/usr/bin/env bash
set -euo pipefail

helper="${SKY_CUA_COSMIC_HELPER:-/workspace/target/release/sky-cua-cosmic-helper}"
export XDG_CURRENT_DESKTOP="${XDG_CURRENT_DESKTOP:-COSMIC}"
export XDG_SESSION_DESKTOP="${XDG_SESSION_DESKTOP:-COSMIC}"
export DESKTOP_SESSION="${DESKTOP_SESSION:-COSMIC}"
export XDG_SESSION_TYPE=wayland

if [[ ! -x "$helper" ]]; then
  printf 'missing host-built COSMIC helper artifact: %s\n' "$helper" >&2
  printf 'run scripts/run_gui_testing_vm_smoke.py without --skip-host-build, or build the release artifact on the host first\n' >&2
  exit 66
fi

if [[ ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
  printf 'COSMIC helper profile requires a real Wayland session socket: %s/%s\n' "$XDG_RUNTIME_DIR" "$WAYLAND_DISPLAY" >&2
  exit 67
fi

artifact_dir="/workspace/artifacts/gui-desktop-smoke/cosmic-helper/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$artifact_dir"

client_pid=""
cleanup() {
  if [[ -n "$client_pid" ]] && kill -0 "$client_pid" 2>/dev/null; then
    kill "$client_pid" 2>/dev/null || true
    wait "$client_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

weston-flower >"$artifact_dir/weston-flower.log" 2>&1 &
client_pid=$!
printf '%s\n' "$client_pid" >"$artifact_dir/weston-flower.pid"
sleep 2

"$helper" probe | tee "$artifact_dir/probe.json"
"$helper" list-windows | tee "$artifact_dir/list-windows.json"
"$helper" focused-window | tee "$artifact_dir/focused-before-activate.json"

window_id="$(python - "$artifact_dir/list-windows.json" <<'PY'
import json
import pathlib
import sys

windows = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
for window in windows:
    if window.get("app_id") == "org.freedesktop.weston.flower":
        print(window["window_id"])
        raise SystemExit(0)
raise SystemExit("COSMIC helper did not list the weston-flower Wayland client")
PY
)"

"$helper" activate-window --window-id "$window_id" | tee "$artifact_dir/activate-window.json"
"$helper" focused-window | tee "$artifact_dir/focused-after-activate.json"

python - "$artifact_dir" "$window_id" <<'PY'
import json
import pathlib
import sys

artifact_dir = pathlib.Path(sys.argv[1])
window_id = int(sys.argv[2])

probe = json.loads((artifact_dir / "probe.json").read_text(encoding="utf-8"))
if not probe.get("ok"):
    raise SystemExit(f"COSMIC helper probe failed: {probe}")
if not probe.get("can_list_windows"):
    raise SystemExit(f"COSMIC helper cannot list windows: {probe}")
if not probe.get("can_activate_windows"):
    raise SystemExit(f"COSMIC helper cannot activate windows: {probe}")

windows = json.loads((artifact_dir / "list-windows.json").read_text(encoding="utf-8"))
matched = [window for window in windows if int(window["window_id"]) == window_id]
if not matched:
    raise SystemExit(f"COSMIC helper did not return window_id {window_id}")
window = matched[0]
if window.get("backend") != "cosmic-wayland":
    raise SystemExit(f"wrong COSMIC backend: {window}")
if window.get("client_type") != "wayland":
    raise SystemExit(f"wrong COSMIC client type: {window}")

activation = json.loads((artifact_dir / "activate-window.json").read_text(encoding="utf-8"))
if not activation.get("ok"):
    raise SystemExit(f"COSMIC helper activation failed: {activation}")

focused = json.loads((artifact_dir / "focused-after-activate.json").read_text(encoding="utf-8"))
if not focused:
    raise SystemExit("COSMIC helper focused-window returned null after activation")
if int(focused["window_id"]) != window_id:
    raise SystemExit(f"COSMIC helper focused the wrong window: {focused}")
if not focused.get("focused"):
    raise SystemExit(f"COSMIC helper did not report focused=true after activation: {focused}")
PY

{
  printf 'profile=cosmic-helper\n'
  printf 'artifact_dir=%s\n' "$artifact_dir"
  printf 'window_id=%s\n' "$window_id"
  printf 'desktop_session=%s\n' "${XDG_CURRENT_DESKTOP:-}"
  printf 'session_type=%s\n' "${XDG_SESSION_TYPE:-}"
  printf 'wayland_display=%s\n' "${WAYLAND_DISPLAY:-}"
  printf 'display=%s\n' "${DISPLAY:-}"
} >"$artifact_dir/summary.env"

printf 'COSMIC helper listing/focus/activation smoke passed; artifacts: %s\n' "$artifact_dir"
