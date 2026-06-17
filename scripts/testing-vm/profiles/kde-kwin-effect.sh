#!/usr/bin/env bash
set -euo pipefail

cleanup_nested_portals() {
  systemctl --user stop \
    xdg-desktop-portal.service \
    xdg-desktop-portal-kde.service \
    xdg-desktop-portal-gtk.service \
    xdg-document-portal.service \
    xdg-permission-store.service >/dev/null 2>&1 || true
  pkill -u "$(id -u)" -f '/usr/lib/xdg-desktop-portal-kde' >/dev/null 2>&1 || true
  pkill -u "$(id -u)" -f '/usr/lib/xdg-desktop-portal-gtk' >/dev/null 2>&1 || true
  pkill -u "$(id -u)" -f '/usr/lib/xdg-desktop-portal($| )' >/dev/null 2>&1 || true
}

if [[ "${1:-}" == "--headed" ]]; then
  shift

  if [[ -z "${HOST_WAYLAND_DISPLAY:-}" ]]; then
    printf 'headed KDE/KWin profile requires HOST_WAYLAND_DISPLAY\n' >&2
    exit 64
  fi

  mkdir -p "$XDG_RUNTIME_DIR" /tmp/host-wayland
  ln -sf "/tmp/host-wayland/$HOST_WAYLAND_DISPLAY" "$XDG_RUNTIME_DIR/wayland-0"
  export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
  export QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-wayland}"

  artifact_dir="/workspace/artifacts/codex-e2e/agent-cursor-kde/$(date -u +%m%d%H%M%S%N)-kwin-headed"
  mkdir -p "$artifact_dir"
  install_prefix="$artifact_dir/kwin-effect-prefix"
  export INSTALL_PREFIX="$install_prefix"
  export SHOT_DIR="$artifact_dir"
  export KWIN_EFFECT_ID="sky-cua-agent-cursor"
  export KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1

  cmake \
    -S /workspace/resources/kwin/effects/sky-cua-agent-cursor \
    -B "$artifact_dir/kwin-effect-build" \
    -G Ninja \
    -DCMAKE_INSTALL_PREFIX="$install_prefix" \
    -DSKY_CUA_CURSOR_ASSET=/workspace/crates/sky-cua-overlay-host/assets/cursor-chat.png
  cmake --build "$artifact_dir/kwin-effect-build"
  cmake --install "$artifact_dir/kwin-effect-build"
  export SKY_CUA_OVERLAY_HOST_PATH="${SKY_CUA_OVERLAY_HOST_PATH:-/workspace/target/release/sky-cua-overlay-host}"
  if [[ ! -x "$SKY_CUA_OVERLAY_HOST_PATH" ]]; then
    printf 'missing host-built overlay host: %s\n' "$SKY_CUA_OVERLAY_HOST_PATH" >&2
    printf 'run scripts/run_gui_testing_vm_smoke.py without --skip-host-build, or build the release artifact on the host first\n' >&2
    exit 66
  fi
  export QT_PLUGIN_PATH="$install_prefix/lib/qt6/plugins${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}"
  export XDG_DATA_DIRS="$install_prefix/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

  session_script="$artifact_dir/kwin-headed-session.sh"
  cat >"$session_script" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

for _ in $(seq 1 80); do
  if qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.listOfEffects >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.listOfEffects >"$SHOT_DIR/headed-effects-list.txt"
qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.loadEffect "$KWIN_EFFECT_ID" >"$SHOT_DIR/headed-effect-load.txt"
sleep 0.2

weston-simple-shm >"$SHOT_DIR/weston-simple-shm.log" 2>&1 &
weston_pid=$!
printf '%s\n' "$weston_pid" >"$SHOT_DIR/weston-simple-shm.pid"
trap 'kill "$weston_pid" >/dev/null 2>&1 || true' EXIT
sleep 0.5

for point in "420 260" "240 160" "500 330" "320 240"; do
  set -- $point
  python3 - "$1" "$2" <<'PY' | "$SKY_CUA_OVERLAY_HOST_PATH" serve >>"$SHOT_DIR/headed-overlay-replies.jsonl"
import json
import sys
import time

x = float(sys.argv[1])
y = float(sys.argv[2])
state = {
    "visible": True,
    "sequence": int(time.time() * 1000),
    "native_point": {"x": x, "y": y, "coordinate_space": "desktop_logical"},
    "model_point": {"x": x, "y": y, "coordinate_space": "stream_pixels"},
    "updated_at_ms": int(time.time() * 1000),
}
print(json.dumps({"version": 1, "kind": "set_cursor", "state": state}, separators=(",", ":")))
PY
  sleep 1.2
done

qdbus6 org.kde.KWin /com/skycua/AgentCursor com.skycua.AgentCursor.StateJson >"$SHOT_DIR/headed-effect-state.json"
qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.isEffectLoaded "$KWIN_EFFECT_ID" >"$SHOT_DIR/headed-effect-loaded.txt" || true
printf 'headed KWin cursor overlay is active; sleeping for inspection\n' >"$SHOT_DIR/headed-ready.txt"
cat "$SHOT_DIR/headed-ready.txt"
sleep "${SKY_CUA_HEADED_SLEEP_SECONDS:-300}"
EOF
  chmod +x "$session_script"

  dbus-run-session -- kwin_wayland \
    --wayland-display "$WAYLAND_DISPLAY" \
    --width 640 \
    --height 480 \
    --no-lockscreen \
    --no-global-shortcuts \
    --socket sky-cua-headed \
    --exit-with-session "$session_script"
  exit $?
fi

cleanup_nested_portals
SKY_CUA_KWIN_NESTED_ACCEPT_IPC_ONLY=1 python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested
python scripts/live_agent_cursor_kde_smoke.py --mode kwin-effect-nested-user-install
cleanup_nested_portals
