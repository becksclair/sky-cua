#!/usr/bin/env bash
set -euo pipefail

if [[ ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
  printf 'visible Wayland pointer profile requires a real Wayland session socket: %s/%s\n' "$XDG_RUNTIME_DIR" "$WAYLAND_DISPLAY" >&2
  exit 67
fi

exec python scripts/live_wayland_pointer_smoke.py
