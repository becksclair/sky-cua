#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "--headed" ]]; then
  printf 'gnome profile currently requires --headed so the nested GNOME Wayland display is visible on the host\n' >&2
  exit 64
fi
shift

if [[ -z "${HOST_WAYLAND_DISPLAY:-}" ]]; then
  printf 'headed GNOME profile requires HOST_WAYLAND_DISPLAY\n' >&2
  exit 64
fi

artifact_dir="/workspace/artifacts/gui-desktop-smoke/gnome/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$artifact_dir"

mkdir -p "$XDG_RUNTIME_DIR" /tmp/host-wayland
ln -sf "/tmp/host-wayland/$HOST_WAYLAND_DISPLAY" "$XDG_RUNTIME_DIR/wayland-0"

export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
export XDG_SESSION_TYPE=wayland
export XDG_CURRENT_DESKTOP=GNOME
export XDG_SESSION_DESKTOP=gnome
export GNOME_SHELL_SESSION_MODE=user
export GDK_BACKEND=wayland
export NO_AT_BRIDGE=1

export XDG_CONFIG_HOME="$artifact_dir/config"
export XDG_CACHE_HOME="$artifact_dir/cache"
export XDG_STATE_HOME="$artifact_dir/state"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"

system_bus_address="$(
  dbus-daemon \
    --system \
    --fork \
    --nopidfile \
    --address="unix:path=$XDG_RUNTIME_DIR/gnome-system-bus" \
    --print-address=1
)"
export DBUS_SYSTEM_BUS_ADDRESS="$system_bus_address"
printf '%s\n' "$system_bus_address" >"$artifact_dir/system-bus-address.txt"

cleanup() {
  if [[ -n "${client_pid:-}" ]] && kill -0 "$client_pid" 2>/dev/null; then
    kill "$client_pid" 2>/dev/null || true
  fi
  if [[ -n "${shell_pid:-}" ]] && kill -0 "$shell_pid" 2>/dev/null; then
    kill "$shell_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

dbus-run-session -- gnome-shell \
  --wayland \
  --wayland-display sky-cua-gnome \
  --devkit \
  >"$artifact_dir/gnome-shell.log" 2>&1 &
shell_pid=$!
printf '%s\n' "$shell_pid" >"$artifact_dir/gnome-shell.pid"

for _ in $(seq 1 120); do
  if [[ -S "$XDG_RUNTIME_DIR/sky-cua-gnome" ]]; then
    break
  fi
  if ! kill -0 "$shell_pid" 2>/dev/null; then
    printf 'gnome-shell exited before exposing the nested Wayland socket\n' >&2
    cat "$artifact_dir/gnome-shell.log" >&2
    exit 1
  fi
  sleep 0.25
done

if [[ ! -S "$XDG_RUNTIME_DIR/sky-cua-gnome" ]]; then
  printf 'gnome-shell did not expose %s\n' "$XDG_RUNTIME_DIR/sky-cua-gnome" >&2
  cat "$artifact_dir/gnome-shell.log" >&2
  exit 1
fi

smoke_client="${SKY_CUA_WAYLAND_SMOKE_CLIENT:-weston-flower}"
printf '%s\n' "$smoke_client" >"$artifact_dir/wayland-client-command.txt"
XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" WAYLAND_DISPLAY=sky-cua-gnome \
  timeout 3s "$smoke_client" >"$artifact_dir/wayland-client.log" 2>&1 &
client_pid=$!
if wait "$client_pid"; then
  client_status=0
else
  client_status=$?
fi
unset client_pid

if [[ "$client_status" != 0 && "$client_status" != 124 ]]; then
  printf 'GNOME nested Wayland client failed with status %s\n' "$client_status" >&2
  cat "$artifact_dir/wayland-client.log" >&2
  cat "$artifact_dir/gnome-shell.log" >&2
  exit "$client_status"
fi
if grep -Eiq 'support required.*exiting|exiting.*support required' "$artifact_dir/wayland-client.log"; then
  printf 'GNOME nested Wayland client exited without required protocol support\n' >&2
  cat "$artifact_dir/wayland-client.log" >&2
  exit 1
fi

if ! kill -0 "$shell_pid" 2>/dev/null; then
  printf 'gnome-shell exited after the client smoke\n' >&2
  cat "$artifact_dir/gnome-shell.log" >&2
  exit 1
fi

pgrep -a gnome-shell >"$artifact_dir/gnome-shell-process.txt" || true
pgrep -a Xwayland >"$artifact_dir/xwayland-process.txt" || true
{
  printf 'WAYLAND_DISPLAY=sky-cua-gnome\n'
  printf 'CLIENT_COMMAND=%s\n' "$smoke_client"
  printf 'CLIENT_STATUS=%s\n' "$client_status"
  printf 'DISPLAY=%s\n' "${DISPLAY:-}"
  printf 'XDG_RUNTIME_DIR=%s\n' "$XDG_RUNTIME_DIR"
} >"$artifact_dir/display-env.txt"

printf 'headed GNOME Wayland session is active; sleeping for inspection\n' >"$artifact_dir/ready.txt"
cat "$artifact_dir/ready.txt"
sleep "${SKY_CUA_HEADED_SLEEP_SECONDS:-300}"
