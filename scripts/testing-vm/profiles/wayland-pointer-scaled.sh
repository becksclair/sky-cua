#!/usr/bin/env bash
set -euo pipefail

if [[ ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
  printf 'scaled Wayland pointer profile requires a real Wayland session socket: %s/%s\n' "$XDG_RUNTIME_DIR" "$WAYLAND_DISPLAY" >&2
  exit 67
fi

if ! command -v cosmic-randr >/dev/null 2>&1; then
  printf 'scaled Wayland pointer profile currently requires cosmic-randr\n' >&2
  exit 69
fi

output="${SKY_CUA_SCALED_OUTPUT:-Virtual-1}"
scaled_width="${SKY_CUA_SCALED_WIDTH:-1600}"
scaled_height="${SKY_CUA_SCALED_HEIGHT:-1200}"
scaled_scale="${SKY_CUA_SCALED_SCALE:-1.25}"
restore_width="${SKY_CUA_RESTORE_WIDTH:-1280}"
restore_height="${SKY_CUA_RESTORE_HEIGHT:-800}"
restore_scale="${SKY_CUA_RESTORE_SCALE:-1.0}"

restore_display() {
  cosmic-randr mode --scale "$restore_scale" "$output" "$restore_width" "$restore_height" >/dev/null 2>&1 || true
}

trap restore_display EXIT

cosmic-randr mode --scale "$scaled_scale" "$output" "$scaled_width" "$scaled_height"
sleep 2
python scripts/live_wayland_pointer_smoke.py
