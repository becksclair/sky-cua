#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "--headed" ]]; then
  printf 'kde-plasma profile currently requires --headed so the full Plasma Wayland desktop is visible on the host\n' >&2
  exit 64
fi
shift

if [[ -z "${HOST_WAYLAND_DISPLAY:-}" ]]; then
  printf 'headed KDE Plasma profile requires HOST_WAYLAND_DISPLAY\n' >&2
  exit 64
fi

mkdir -p "$XDG_RUNTIME_DIR" /tmp/host-wayland
ln -sf "/tmp/host-wayland/$HOST_WAYLAND_DISPLAY" "$XDG_RUNTIME_DIR/wayland-0"
export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
export QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-wayland}"
export XDG_SESSION_TYPE=wayland
export XDG_CURRENT_DESKTOP=KDE
export KDE_FULL_SESSION=true
export KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1

artifact_dir="/workspace/artifacts/gui-desktop-smoke/kde-plasma/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$artifact_dir"
install_prefix="$artifact_dir/kwin-effect-prefix"
export INSTALL_PREFIX="$install_prefix"
export SHOT_DIR="$artifact_dir"
export KWIN_EFFECT_ID="sky-cua-agent-cursor"

cmake \
  -S /workspace/resources/kwin/effects/sky-cua-agent-cursor \
  -B "$artifact_dir/kwin-effect-build" \
  -G Ninja \
  -DCMAKE_INSTALL_PREFIX="$install_prefix"
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

plasma_session_script="$artifact_dir/kde-plasma-headed-session.sh"
cat >"$plasma_session_script" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

cleanup() {
  if [[ -n "${plasmashell_pid:-}" ]] && kill -0 "$plasmashell_pid" 2>/dev/null; then
    kill "$plasmashell_pid" 2>/dev/null || true
  fi
  if [[ -n "${kded_pid:-}" ]] && kill -0 "$kded_pid" 2>/dev/null; then
    kill "$kded_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

export XDG_CONFIG_HOME="$SHOT_DIR/config"
export XDG_CACHE_HOME="$SHOT_DIR/cache"
export XDG_STATE_HOME="$SHOT_DIR/state"
export WAYLAND_DISPLAY=sky-cua-plasma
export QT_QPA_PLATFORM=wayland
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"

for _ in $(seq 1 240); do
  if qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.listOfEffects >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done

qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.listOfEffects >"$SHOT_DIR/plasma-effects-list.txt"
qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.loadEffect "$KWIN_EFFECT_ID" >"$SHOT_DIR/plasma-effect-load.txt"
sleep 0.5
qdbus6 org.kde.KWin /Effects org.kde.kwin.Effects.isEffectLoaded "$KWIN_EFFECT_ID" >"$SHOT_DIR/plasma-effect-loaded.txt" || true

kded6 >"$SHOT_DIR/kded6.log" 2>&1 &
kded_pid=$!
printf '%s\n' "$kded_pid" >"$SHOT_DIR/kded6.pid"

plasmashell >"$SHOT_DIR/plasmashell.log" 2>&1 &
plasmashell_pid=$!
printf '%s\n' "$plasmashell_pid" >"$SHOT_DIR/plasmashell.pid"

for _ in $(seq 1 120); do
  if pgrep -x plasmashell >/dev/null && qdbus6 org.kde.plasmashell /PlasmaShell >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$plasmashell_pid" 2>/dev/null; then
    printf 'plasmashell exited before exposing its DBus service\n' >&2
    cat "$SHOT_DIR/plasmashell.log" >&2
    exit 1
  fi
  sleep 0.25
done

pgrep -a plasmashell >"$SHOT_DIR/plasmashell-process.txt"
pgrep -a kwin_wayland >"$SHOT_DIR/kwin-wayland-process.txt"
pgrep -a Xwayland >"$SHOT_DIR/xwayland-process.txt" || true
{
  printf 'WAYLAND_DISPLAY=%s\n' "${WAYLAND_DISPLAY:-}"
  printf 'DISPLAY=%s\n' "${DISPLAY:-}"
  printf 'QT_QPA_PLATFORM=%s\n' "${QT_QPA_PLATFORM:-}"
  printf 'XDG_RUNTIME_DIR=%s\n' "${XDG_RUNTIME_DIR:-}"
} >"$SHOT_DIR/display-env.txt"

for point in "420 260" "240 160" "500 330" "320 240"; do
  set -- $point
  python3 - "$1" "$2" <<'PY' | "$SKY_CUA_OVERLAY_HOST_PATH" serve >>"$SHOT_DIR/plasma-overlay-replies.jsonl"
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

qdbus6 org.kde.KWin /com/skycua/AgentCursor com.skycua.AgentCursor.StateJson >"$SHOT_DIR/plasma-effect-state.json"
printf 'headed KDE Plasma layer-shell cursor overlay with KWin cursor-hide shim is active; sleeping for inspection\n' >"$SHOT_DIR/plasma-ready.txt"
cat "$SHOT_DIR/plasma-ready.txt"
sleep "${SKY_CUA_HEADED_SLEEP_SECONDS:-300}"
EOF
chmod +x "$plasma_session_script"

dbus-run-session -- kwin_wayland \
  --xwayland \
  --wayland-display "$WAYLAND_DISPLAY" \
  --width 1280 \
  --height 800 \
  --no-lockscreen \
  --no-global-shortcuts \
  --socket sky-cua-plasma \
  --exit-with-session "$plasma_session_script"
