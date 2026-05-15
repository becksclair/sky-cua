#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "--headed" ]]; then
  printf 'hyprland profile currently requires --headed so the nested Hyprland display is visible on the host\n' >&2
  exit 64
fi
shift

if [[ -z "${HOST_WAYLAND_DISPLAY:-}" ]]; then
  printf 'headed Hyprland profile requires HOST_WAYLAND_DISPLAY\n' >&2
  exit 64
fi

artifact_dir="/workspace/artifacts/gui-desktop-smoke/hyprland/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$artifact_dir"

mkdir -p "$XDG_RUNTIME_DIR" /tmp/host-wayland /tmp/.X11-unix
ln -sf "/tmp/host-wayland/$HOST_WAYLAND_DISPLAY" "$XDG_RUNTIME_DIR/wayland-0"

export HOME="$artifact_dir/home"
export XDG_CONFIG_HOME="$HOME/.config"
export XDG_CACHE_HOME="$HOME/.cache"
export XDG_STATE_HOME="$HOME/.local/state"
mkdir -p "$XDG_CONFIG_HOME/hypr" "$XDG_CACHE_HOME/hyprland" "$XDG_STATE_HOME"

export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
export XDG_SESSION_TYPE=wayland
export XDG_CURRENT_DESKTOP=Hyprland
export HYPRLAND_NO_SD_NOTIFY=1
export LIBGL_ALWAYS_SOFTWARE=1
export MESA_LOADER_DRIVER_OVERRIDE=llvmpipe

cat >"$artifact_dir/hyprland.conf" <<'EOF'
monitor=,1280x800@60,0x0,1
misc:disable_hyprland_logo=true
misc:disable_splash_rendering=true
debug:disable_logs=false
EOF

cleanup() {
  if [[ -n "${client_pid:-}" ]] && kill -0 "$client_pid" 2>/dev/null; then
    kill "$client_pid" 2>/dev/null || true
  fi
  if [[ -n "${hyprland_pid:-}" ]] && kill -0 "$hyprland_pid" 2>/dev/null; then
    kill "$hyprland_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

Hyprland -c "$artifact_dir/hyprland.conf" >"$artifact_dir/hyprland.log" 2>&1 &
hyprland_pid=$!
printf '%s\n' "$hyprland_pid" >"$artifact_dir/hyprland.pid"

nested_display=""
for _ in $(seq 1 120); do
  nested_display="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -printf '%f\n' | grep -E '^wayland-[0-9]+$' | grep -v '^wayland-0$' | head -1 || true)"
  if [[ -n "$nested_display" ]]; then
    break
  fi
  if ! kill -0 "$hyprland_pid" 2>/dev/null; then
    printf 'Hyprland exited before exposing a nested Wayland socket\n' >&2
    cat "$artifact_dir/hyprland.log" >&2
    exit 1
  fi
  sleep 0.25
done

if [[ -z "$nested_display" ]]; then
  printf 'Hyprland did not expose a nested Wayland socket\n' >&2
  cat "$artifact_dir/hyprland.log" >&2
  exit 1
fi

sleep 1
if ! kill -0 "$hyprland_pid" 2>/dev/null; then
  printf 'Hyprland crashed after exposing %s\n' "$nested_display" >&2
  cat "$artifact_dir"/home/.cache/hyprland/hyprlandCrashReport*.txt >"$artifact_dir/hyprland-crash-report.txt" 2>/dev/null || true
  cat "$artifact_dir/hyprland.log" >&2
  exit 1
fi

smoke_client="${SKY_CUA_WAYLAND_SMOKE_CLIENT:-weston-flower}"
printf '%s\n' "$smoke_client" >"$artifact_dir/wayland-client-command.txt"
XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WAYLAND_DISPLAY="$nested_display" \
  timeout 3s "$smoke_client" >"$artifact_dir/wayland-client.log" 2>&1 &
client_pid=$!
if wait "$client_pid"; then
  client_status=0
else
  client_status=$?
fi
unset client_pid

if [[ "$client_status" != 0 && "$client_status" != 124 ]]; then
  printf 'Hyprland nested Wayland client failed with status %s\n' "$client_status" >&2
  cat "$artifact_dir/wayland-client.log" >&2
  cat "$artifact_dir/hyprland.log" >&2
  exit "$client_status"
fi
if grep -Eiq 'support required.*exiting|exiting.*support required' "$artifact_dir/wayland-client.log"; then
  printf 'Hyprland nested Wayland client exited without required protocol support\n' >&2
  cat "$artifact_dir/wayland-client.log" >&2
  exit 1
fi

hyprctl -i 0 monitors >"$artifact_dir/hyprctl-monitors.txt" 2>&1 || true
pgrep -a Hyprland >"$artifact_dir/hyprland-process.txt" || true
{
  printf 'WAYLAND_DISPLAY=%s\n' "$nested_display"
  printf 'CLIENT_COMMAND=%s\n' "$smoke_client"
  printf 'CLIENT_STATUS=%s\n' "$client_status"
  printf 'XDG_RUNTIME_DIR=%s\n' "$XDG_RUNTIME_DIR"
} >"$artifact_dir/display-env.txt"

printf 'headed Hyprland Wayland compositor is active; sleeping for inspection\n' >"$artifact_dir/ready.txt"
cat "$artifact_dir/ready.txt"
sleep "${SKY_CUA_HEADED_SLEEP_SECONDS:-300}"
