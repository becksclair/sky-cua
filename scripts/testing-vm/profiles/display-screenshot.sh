#!/usr/bin/env bash
set -euo pipefail

desktop_hint="${XDG_CURRENT_DESKTOP:-} ${XDG_SESSION_DESKTOP:-} ${DESKTOP_SESSION:-}"
if [[ "${desktop_hint,,}" == *i3* ]]; then
  export XDG_SESSION_TYPE=x11
  unset WAYLAND_DISPLAY
fi

if [[ -n "${WAYLAND_DISPLAY:-}" && -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
  export XDG_SESSION_TYPE=wayland
elif [[ -n "${DISPLAY:-}" ]] && xdpyinfo >/dev/null 2>&1; then
  export XDG_SESSION_TYPE=x11
  unset WAYLAND_DISPLAY
else
  xorg_args="$(ps -eo args | grep -E '/Xorg .* :[0-9]+' | grep -v grep | tail -1 || true)"
  DISPLAY="$(printf '%s\n' "$xorg_args" | sed -n 's/.*\(:[0-9][0-9]*\).*/\1/p')"
  server_auth="$(printf '%s\n' "$xorg_args" | sed -n 's/.* -auth \([^ ]*\).*/\1/p')"
  if [[ -n "$DISPLAY" && -f "$server_auth" ]]; then
    cookie="$(xauth -f "$server_auth" list | head -1 | sed 's/.*MIT-MAGIC-COOKIE-1  //')"
    export XAUTHORITY="${XDG_RUNTIME_DIR:-/tmp}/sky-cua-display-screenshot.Xauthority"
    rm -f "$XAUTHORITY"
    touch "$XAUTHORITY"
    host_name="$(cat /proc/sys/kernel/hostname 2>/dev/null || printf localhost)"
    xauth -f "$XAUTHORITY" add "${host_name}/unix${DISPLAY}" MIT-MAGIC-COOKIE-1 "$cookie" >/dev/null 2>&1 || true
    xauth -f "$XAUTHORITY" add "$DISPLAY" MIT-MAGIC-COOKIE-1 "$cookie" >/dev/null 2>&1 || true
    export DISPLAY XDG_SESSION_TYPE=x11
    unset WAYLAND_DISPLAY
  fi
fi

if [[ "${XDG_SESSION_TYPE:-}" != "wayland" && "${XDG_SESSION_TYPE:-}" != "x11" ]]; then
  printf 'display screenshot profile requires a real Wayland or X11 session; WAYLAND_DISPLAY=%s DISPLAY=%s XAUTHORITY=%s\n' "${WAYLAND_DISPLAY:-}" "${DISPLAY:-}" "${XAUTHORITY:-}" >&2
  exit 67
fi

export SKY_CUA_DISPLAY_SCREENSHOT_REQUIRE_SECONDARY="${SKY_CUA_DISPLAY_SCREENSHOT_REQUIRE_SECONDARY:-0}"
exec python scripts/live_display_screenshot_smoke.py
