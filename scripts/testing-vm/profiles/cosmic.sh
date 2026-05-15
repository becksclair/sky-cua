#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" != "--headed" ]]; then
  printf 'cosmic profile currently requires --headed so the nested COSMIC Wayland display is visible on the host\n' >&2
  exit 64
fi
shift

if [[ -z "${HOST_WAYLAND_DISPLAY:-}" ]]; then
  printf 'headed COSMIC profile requires HOST_WAYLAND_DISPLAY\n' >&2
  exit 64
fi

artifact_dir="/workspace/artifacts/gui-desktop-smoke/cosmic/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$artifact_dir"

mkdir -p "$XDG_RUNTIME_DIR" /tmp/host-wayland /tmp/.X11-unix
ln -sf "/tmp/host-wayland/$HOST_WAYLAND_DISPLAY" "$XDG_RUNTIME_DIR/wayland-0"

export WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
export XDG_SESSION_TYPE=wayland
export XDG_CURRENT_DESKTOP=COSMIC
export XDG_CONFIG_HOME="$artifact_dir/config"
export XDG_CACHE_HOME="$artifact_dir/cache"
export XDG_STATE_HOME="$artifact_dir/state"
mkdir -p "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_STATE_HOME"

cleanup() {
  if [[ -n "${client_pid:-}" ]] && kill -0 "$client_pid" 2>/dev/null; then
    kill "$client_pid" 2>/dev/null || true
  fi
  if [[ -n "${cosmic_pid:-}" ]] && kill -0 "$cosmic_pid" 2>/dev/null; then
    kill "$cosmic_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

dbus-run-session -- cosmic-comp --no-xwayland >"$artifact_dir/cosmic-comp.log" 2>&1 &
cosmic_pid=$!
printf '%s\n' "$cosmic_pid" >"$artifact_dir/cosmic-comp.pid"

nested_display=""
for _ in $(seq 1 120); do
  nested_display="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -printf '%f\n' | grep -E '^wayland-[0-9]+$' | grep -v '^wayland-0$' | head -1 || true)"
  if [[ -n "$nested_display" ]]; then
    break
  fi
  if ! kill -0 "$cosmic_pid" 2>/dev/null; then
    printf 'cosmic-comp exited before exposing a nested Wayland socket\n' >&2
    cat "$artifact_dir/cosmic-comp.log" >&2
    exit 1
  fi
  sleep 0.25
done

if [[ -z "$nested_display" ]]; then
  printf 'cosmic-comp did not expose a nested Wayland socket\n' >&2
  cat "$artifact_dir/cosmic-comp.log" >&2
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
  printf 'COSMIC nested Wayland client failed with status %s\n' "$client_status" >&2
  cat "$artifact_dir/wayland-client.log" >&2
  cat "$artifact_dir/cosmic-comp.log" >&2
  exit "$client_status"
fi
if grep -Eiq 'support required.*exiting|exiting.*support required' "$artifact_dir/wayland-client.log"; then
  printf 'COSMIC nested Wayland client exited without required protocol support\n' >&2
  cat "$artifact_dir/wayland-client.log" >&2
  exit 1
fi

if ! kill -0 "$cosmic_pid" 2>/dev/null; then
  printf 'cosmic-comp exited after the client smoke\n' >&2
  cat "$artifact_dir/cosmic-comp.log" >&2
  exit 1
fi

pgrep -a cosmic-comp >"$artifact_dir/cosmic-comp-process.txt" || true
{
  printf 'WAYLAND_DISPLAY=%s\n' "$nested_display"
  printf 'CLIENT_COMMAND=%s\n' "$smoke_client"
  printf 'CLIENT_STATUS=%s\n' "$client_status"
  printf 'XDG_RUNTIME_DIR=%s\n' "$XDG_RUNTIME_DIR"
} >"$artifact_dir/display-env.txt"

printf 'headed COSMIC Wayland compositor is active; sleeping for inspection\n' >"$artifact_dir/ready.txt"
cat "$artifact_dir/ready.txt"
sleep "${SKY_CUA_HEADED_SLEEP_SECONDS:-300}"
