#!/usr/bin/env bash
set -euo pipefail

export XDG_SESSION_TYPE=x11
export XDG_CURRENT_DESKTOP="${XDG_CURRENT_DESKTOP:-i3}"
export XDG_SESSION_DESKTOP="${XDG_SESSION_DESKTOP:-i3}"
export DESKTOP_SESSION="${DESKTOP_SESSION:-i3}"
unset WAYLAND_DISPLAY

if [[ -z "${DISPLAY:-}" ]] || ! xdpyinfo >/dev/null 2>&1; then
  xorg_args="$(ps -eo args | grep -E '/Xorg .* :[0-9]+' | grep -v grep | tail -1 || true)"
  DISPLAY="$(printf '%s\n' "$xorg_args" | sed -n 's/.*\(:[0-9][0-9]*\).*/\1/p')"
  server_auth="$(printf '%s\n' "$xorg_args" | sed -n 's/.* -auth \([^ ]*\).*/\1/p')"
  if [[ -n "$DISPLAY" && -f "$server_auth" ]]; then
    cookie="$(xauth -f "$server_auth" list | head -1 | sed 's/.*MIT-MAGIC-COOKIE-1  //')"
    export XAUTHORITY="${XDG_RUNTIME_DIR:-/tmp}/sky-cua-i3.Xauthority"
    rm -f "$XAUTHORITY"
    touch "$XAUTHORITY"
    host_name="$(cat /proc/sys/kernel/hostname 2>/dev/null || printf localhost)"
    xauth -f "$XAUTHORITY" add "${host_name}/unix${DISPLAY}" MIT-MAGIC-COOKIE-1 "$cookie" >/dev/null 2>&1 || true
    xauth -f "$XAUTHORITY" add "$DISPLAY" MIT-MAGIC-COOKIE-1 "$cookie" >/dev/null 2>&1 || true
  fi
  export DISPLAY
fi

if [[ -z "${DISPLAY:-}" ]] || ! xdpyinfo >/dev/null 2>&1; then
  printf 'i3/X11 profile requires a reachable real X11 session display; DISPLAY=%s XAUTHORITY=%s\n' "${DISPLAY:-}" "${XAUTHORITY:-}" >&2
  exit 67
fi

export SKY_CUA_USE_PREBUILT_RUNTIMES=1
export SKY_CUA_OVERLAY_HOST_PATH="${SKY_CUA_OVERLAY_HOST_PATH:-/workspace/target/release/sky-cua-overlay-host}"

python scripts/live_agent_cursor_x11_overlay_smoke.py --current-display
