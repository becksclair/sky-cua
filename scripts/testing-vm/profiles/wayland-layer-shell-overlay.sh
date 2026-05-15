#!/usr/bin/env bash
set -euo pipefail

if [[ ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
  printf 'Wayland layer-shell overlay profile requires a real Wayland session socket: %s/%s\n' "$XDG_RUNTIME_DIR" "$WAYLAND_DISPLAY" >&2
  exit 67
fi

export SKY_CUA_USE_PREBUILT_RUNTIMES=1
export SKY_CUA_OVERLAY_HOST_PATH="${SKY_CUA_OVERLAY_HOST_PATH:-/workspace/target/release/sky-cua-overlay-host}"

python scripts/live_wayland_layer_shell_overlay_smoke.py --wayland-display "$WAYLAND_DISPLAY"
