#!/usr/bin/env bash
set -euo pipefail

# Isolated-xpra desktop end-to-end smoke.
#
# Exercises the [isolated_desktop] lane fully inside the testing VM, where
# launching GUI applications is safe and isolated from any real session:
#   1. bring the private xpra desktop up via the built client subcommand
#      (no viewer window, so nothing is drawn outside the sandbox),
#   2. confirm `isolated-desktop status` reports up=true with the expected
#      display and geometry,
#   3. launch an application into the sandbox with the exact daemon sandbox
#      env recipe, confirm it appears on :N and on no host X display, and
#      assert its /proc/<pid>/environ carries the sandbox markers (no leak),
#   4. tear the session down strictly by display number and confirm no
#      leftover xpra session or /tmp/.X<N>-lock remains.
#
# Skips (exit 67) when xpra/openbox/xdotool are unavailable. Self-contained and
# idempotent: it tears down its own session on every exit path and never uses
# pkill -f against the broad process table.

readonly SKIP_ENVIRONMENT_MISSING=67

# The display this profile owns. Kept off the common :100 default so a stray
# developer session on :100 cannot collide with the smoke.
readonly ISOLATED_DISPLAY=":131"
readonly DISPLAY_NUMBER="131"
readonly EXPECTED_RESOLUTION="1920x1080"
readonly EXPECTED_GEOMETRY="1920x1080"

# Force the resolver onto our owned, viewerless display regardless of any
# host config file, so the smoke is hermetic and never opens a viewer window.
export SKY_CUA_ISOLATED_DESKTOP=1
export SKY_CUA_ISOLATED_DESKTOP_DISPLAY="$ISOLATED_DISPLAY"
export SKY_CUA_ISOLATED_DESKTOP_RESOLUTION="$EXPECTED_RESOLUTION"
export SKY_CUA_ISOLATED_DESKTOP_VIEWER=none
export SKY_CUA_ISOLATED_DESKTOP_LIFECYCLE=persistent

export SKY_CUA_USE_PREBUILT_RUNTIMES=1

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
client="${repo_root}/bin/sky-cua-client"

if [[ ! -x "$client" ]]; then
  printf 'isolated-xpra profile requires the built sky-cua-client at %s\n' "$client" >&2
  exit "$SKIP_ENVIRONMENT_MISSING"
fi

missing=()
for dep in xpra openbox xdotool xdpyinfo; do
  if ! command -v "$dep" >/dev/null 2>&1; then
    missing+=("$dep")
  fi
done
if ((${#missing[@]} > 0)); then
  printf 'isolated-xpra profile requires: %s; skipping\n' "${missing[*]}" >&2
  exit "$SKIP_ENVIRONMENT_MISSING"
fi

# Record the launched sandbox app PID so cleanup can reap it strictly by PID.
app_pid=""

cleanup() {
  local status=$?
  set +e
  if [[ -n "$app_pid" ]] && kill -0 "$app_pid" 2>/dev/null; then
    kill "$app_pid" 2>/dev/null
  fi
  # Strict-by-display-number teardown via the client subcommand; never pkill -f.
  "$client" isolated-desktop stop >/dev/null 2>&1 || true
  # Belt-and-braces: if a stale lock survived, remove only our display's lock.
  rm -f "/tmp/.X${DISPLAY_NUMBER}-lock" 2>/dev/null || true
  return "$status"
}
trap cleanup EXIT

fail() {
  printf 'isolated-xpra: %s\n' "$1" >&2
  exit 1
}

# Capture every X display socket the host already owns BEFORE we bring up the
# sandbox, so the host-leak check below can confirm the launched app never
# appears on any of them.
host_displays=()
if [[ -d /tmp/.X11-unix ]]; then
  for sock in /tmp/.X11-unix/X*; do
    [[ -e "$sock" ]] || continue
    num="${sock##*/X}"
    [[ "$num" == "$DISPLAY_NUMBER" ]] && continue
    host_displays+=(":${num}")
  done
fi
# A live $DISPLAY (real X session) counts as a host display too.
if [[ -n "${DISPLAY:-}" && "${DISPLAY}" != "$ISOLATED_DISPLAY" ]]; then
  host_displays+=("${DISPLAY}")
fi

printf 'isolated-xpra: bringing up %s (viewer=none)\n' "$ISOLATED_DISPLAY"
ensure_out="$("$client" isolated-desktop ensure)" || fail 'isolated-desktop ensure failed'
printf '%s\n' "$ensure_out"

ensure_display="$(printf '%s\n' "$ensure_out" | sed -n 's/^display=//p')"
[[ "$ensure_display" == "$ISOLATED_DISPLAY" ]] ||
  fail "ensure reported display=${ensure_display}, expected ${ISOLATED_DISPLAY}"

# Idempotent reuse: a second ensure must land on the same display.
reuse_display="$("$client" isolated-desktop ensure | sed -n 's/^display=//p')" ||
  fail 'second isolated-desktop ensure failed'
[[ "$reuse_display" == "$ISOLATED_DISPLAY" ]] ||
  fail "ensure was not idempotent: reuse reported ${reuse_display}"

# Status must report the sandbox up with the expected display + geometry.
status_out="$("$client" isolated-desktop status)" || fail 'isolated-desktop status failed'
printf '%s\n' "$status_out"

status_up="$(printf '%s\n' "$status_out" | sed -n 's/^up=//p')"
status_display="$(printf '%s\n' "$status_out" | sed -n 's/^display=//p')"
status_geometry="$(printf '%s\n' "$status_out" | sed -n 's/^geometry=//p')"
[[ "$status_up" == "true" ]] || fail "status reported up=${status_up}, expected true"
[[ "$status_display" == "$ISOLATED_DISPLAY" ]] ||
  fail "status reported display=${status_display}, expected ${ISOLATED_DISPLAY}"
[[ "$status_geometry" == "$EXPECTED_GEOMETRY" ]] ||
  fail "status reported geometry=${status_geometry}, expected ${EXPECTED_GEOMETRY}"

# The sandbox X server must be reachable on :N.
DISPLAY="$ISOLATED_DISPLAY" xdpyinfo >/dev/null 2>&1 ||
  fail "sandbox display ${ISOLATED_DISPLAY} is not reachable via xdpyinfo"

# Pick an application to launch into the sandbox. Prefer a pure-Xlib app so the
# launch is robust even on a minimal VM; fall back to a terminal if present.
app_cmd=""
app_title="sky-cua-isolated-smoke-$$"
if command -v xmessage >/dev/null 2>&1; then
  app_cmd=(xmessage -name "$app_title" -title "$app_title" "sky-cua isolated smoke")
elif command -v xterm >/dev/null 2>&1; then
  app_cmd=(xterm -name "$app_title" -title "$app_title" -e sleep 600)
elif command -v xclock >/dev/null 2>&1; then
  app_cmd=(xclock -name "$app_title")
else
  printf 'isolated-xpra profile requires one of: xmessage, xterm, xclock; skipping\n' >&2
  exit "$SKIP_ENVIRONMENT_MISSING"
fi

# Launch the app into the sandbox with the exact daemon sandbox env recipe
# (mirrors IsolatedDesktopHandle::spawn_env / removed_env): on :N, X11 session
# type, toolkit backends pinned to X, WAYLAND_DISPLAY removed. setsid detaches
# it so it outlives this shell, exactly as the daemon's launch_application does.
printf 'isolated-xpra: launching %s into the sandbox\n' "${app_cmd[0]}"
setsid env -u WAYLAND_DISPLAY \
  DISPLAY="$ISOLATED_DISPLAY" \
  XDG_SESSION_TYPE=x11 \
  QT_QPA_PLATFORM=xcb \
  GDK_BACKEND=x11 \
  "${app_cmd[@]}" >/dev/null 2>&1 &
app_pid=$!

# Wait (bounded) for the window to register on :N.
found_on_sandbox=0
for _ in $(seq 1 60); do
  if DISPLAY="$ISOLATED_DISPLAY" xdotool search --name "$app_title" >/dev/null 2>&1; then
    found_on_sandbox=1
    break
  fi
  if ! kill -0 "$app_pid" 2>/dev/null; then
    fail "launched app exited before registering on ${ISOLATED_DISPLAY}"
  fi
  sleep 0.25
done
[[ "$found_on_sandbox" == "1" ]] ||
  fail "launched app never appeared on ${ISOLATED_DISPLAY}"
printf 'isolated-xpra: app present on %s\n' "$ISOLATED_DISPLAY"

# Host-leak guard: the window must NOT appear on any pre-existing host X display.
for host_display in "${host_displays[@]:-}"; do
  [[ -n "$host_display" ]] || continue
  if DISPLAY="$host_display" xdotool search --name "$app_title" >/dev/null 2>&1; then
    fail "HOST LEAK: launched app appeared on host display ${host_display}"
  fi
done
if ((${#host_displays[@]} > 0)); then
  printf 'isolated-xpra: app absent from host displays: %s\n' "${host_displays[*]}"
else
  # On a headless VM with no real X session the absent-on-host assertion is
  # vacuous, so a green run here proves present-on-sandbox plus the environ
  # markers but not the full no-visible-leak guarantee. The host-leak assertion
  # is only meaningful when a real host display exists (e.g. a headed VM run).
  printf 'isolated-xpra: no host X displays present to leak onto\n'
fi

# Environ guard: the launched process must carry the sandbox markers and must
# not carry WAYLAND_DISPLAY (the toolkit Wayland-escape the spike found).
environ_path="/proc/${app_pid}/environ"
if [[ -r "$environ_path" ]]; then
  mapfile -d '' -t environ_entries <"$environ_path"
  has_display=0
  has_wayland=0
  for entry in "${environ_entries[@]}"; do
    case "$entry" in
    "DISPLAY=${ISOLATED_DISPLAY}") has_display=1 ;;
    WAYLAND_DISPLAY=*) has_wayland=1 ;;
    esac
  done
  [[ "$has_display" == "1" ]] ||
    fail "launched app environ lacks DISPLAY=${ISOLATED_DISPLAY}"
  [[ "$has_wayland" == "0" ]] ||
    fail 'launched app environ still carries WAYLAND_DISPLAY'
  printf 'isolated-xpra: launched app environ confirms DISPLAY=%s and no WAYLAND_DISPLAY\n' \
    "$ISOLATED_DISPLAY"
else
  printf 'isolated-xpra: /proc environ not readable; skipping environ marker check\n' >&2
fi

# Tear the launched app down before stopping the desktop, strictly by PID.
if kill -0 "$app_pid" 2>/dev/null; then
  kill "$app_pid" 2>/dev/null || true
fi
app_pid=""

# Teardown: strictly by display number via the client subcommand.
printf 'isolated-xpra: stopping %s\n' "$ISOLATED_DISPLAY"
stop_out="$("$client" isolated-desktop stop)" || fail 'isolated-desktop stop failed'
printf '%s\n' "$stop_out"

# After stop, the sandbox display must be gone and its lock removed.
for _ in $(seq 1 40); do
  DISPLAY="$ISOLATED_DISPLAY" xdpyinfo >/dev/null 2>&1 || break
  sleep 0.25
done
if DISPLAY="$ISOLATED_DISPLAY" xdpyinfo >/dev/null 2>&1; then
  fail "sandbox display ${ISOLATED_DISPLAY} still reachable after stop"
fi
if [[ -e "/tmp/.X${DISPLAY_NUMBER}-lock" ]]; then
  fail "stale /tmp/.X${DISPLAY_NUMBER}-lock left behind after stop"
fi

printf 'isolated-xpra: PASS (sandbox up, app isolated to %s, clean teardown)\n' "$ISOLATED_DISPLAY"
