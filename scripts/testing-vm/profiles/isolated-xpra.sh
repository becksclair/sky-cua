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
#      env recipe, confirm it appears on :N and on no host X display, assert its
#      /proc/<pid>/environ carries the sandbox markers, and (when GTK is
#      available) read its application/window root through the private registry,
#   4. tear the session down strictly by display number and confirm no
#      recorded AT-SPI owner, xpra session, state file, or X lock remains.
#
# Skips (exit 67) when xpra/openbox/xdotool/AT-SPI dependencies are unavailable.
# Self-contained and idempotent: it tears down its own session on every exit
# path and never uses pkill -f against the broad process table.

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
for dep in xpra openbox xdotool xdpyinfo gdbus; do
  if ! command -v "$dep" >/dev/null 2>&1; then
    missing+=("$dep")
  fi
done
for dep in /usr/lib/at-spi-bus-launcher /usr/lib/at-spi2-registryd; do
  if [[ ! -x "$dep" ]]; then
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

process_generation_is_alive() {
  local pid="$1"
  local expected_start_ticks="$2"
  local stat
  local fields
  [[ -r "/proc/${pid}/stat" ]] || return 1
  IFS= read -r stat <"/proc/${pid}/stat" || return 1
  # Strip pid + parenthesized comm first; field 22 (starttime) is then index 19.
  read -r -a fields <<<"${stat##*) }"
  [[ "${fields[19]:-}" == "$expected_start_ticks" ]]
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

# AT-SPI is part of ensure readiness. Prefer xpra's authoritative private bus
# field; sessions using the supported client-owned-bus fallback persist its
# address and process identity because xpra cannot report that address. Then
# require both accessibility names and a real registry method response before
# any application launch.
xpra_info="$(xpra info "$ISOLATED_DISPLAY")" || fail 'xpra info failed after ensure'
session_bus_address="$(printf '%s\n' "$xpra_info" |
  sed -n 's/^dbus\.env\.DBUS_SESSION_BUS_ADDRESS=//p' | head -n 1)"
sandbox_xauthority="$(printf '%s\n' "$xpra_info" |
  sed -n 's/^env\.XAUTHORITY=//p' | head -n 1)"
owned_bus_state_path="${XDG_RUNTIME_DIR}/sky-cua/isolated-bus-${DISPLAY_NUMBER}"
if [[ -z "$session_bus_address" ]]; then
  [[ -r "$owned_bus_state_path" ]] ||
    fail 'xpra reported no private session bus and no client-owned bus record exists'
  IFS= read -r session_bus_address <"$owned_bus_state_path" ||
    fail 'could not read the client-owned session bus address'
  owned_bus_pid="$(sed -n '2p' "$owned_bus_state_path")"
  [[ -n "$session_bus_address" && "$owned_bus_pid" =~ ^[0-9]+$ ]] ||
    fail 'client-owned session bus record is malformed'
  ((owned_bus_pid > 1)) || fail 'client-owned session bus record has an invalid pid'
  owned_bus_comm=""
  IFS= read -r owned_bus_comm <"/proc/${owned_bus_pid}/comm" ||
    fail 'recorded client-owned session bus process is not alive'
  [[ "$owned_bus_comm" == "dbus-daemon" ]] ||
    fail 'recorded client-owned session bus process is not dbus-daemon'
fi
[[ -n "$sandbox_xauthority" ]] || fail 'xpra did not report its Xauthority file'

a11y_address_raw="$(gdbus call --address "$session_bus_address" \
  --dest org.a11y.Bus \
  --object-path /org/a11y/bus \
  --method org.a11y.Bus.GetAddress)" || fail 'private org.a11y.Bus.GetAddress failed'
a11y_bus_address="$(printf '%s\n' "$a11y_address_raw" |
  sed -n "s/^('\\(.*\\)',)$/\\1/p")"
[[ -n "$a11y_bus_address" ]] || fail 'private org.a11y.Bus returned no accessibility bus'

registry_owned="$(gdbus call --address "$a11y_bus_address" \
  --dest org.freedesktop.DBus \
  --object-path /org/freedesktop/DBus \
  --method org.freedesktop.DBus.NameHasOwner org.a11y.atspi.Registry)" ||
  fail 'could not query the private AT-SPI registry owner'
[[ "$registry_owned" == "(true,)" ]] ||
  fail "private AT-SPI registry is not owned: ${registry_owned}"
gdbus call --address "$a11y_bus_address" \
  --dest org.a11y.atspi.Registry \
  --object-path /org/a11y/atspi/registry \
  --method org.a11y.atspi.Registry.GetRegisteredEvents >/dev/null ||
  fail 'private AT-SPI registry did not answer GetRegisteredEvents'
printf 'isolated-xpra: private AT-SPI registry ready on %s\n' "$a11y_bus_address"

# Idempotent reuse: a second ensure must land on the same display without
# replacing either exact AT-SPI owner generation or changing its persisted
# lifecycle classification.
atspi_state_path="${XDG_RUNTIME_DIR}/sky-cua/isolated-atspi-${DISPLAY_NUMBER}.json"
[[ -r "$atspi_state_path" ]] || fail 'ensure did not persist private AT-SPI ownership state'
atspi_state_before="$(<"$atspi_state_path")"
launcher_pid="$(printf '%s\n' "$atspi_state_before" |
  sed -n 's/.*"launcher":{"pid":\([0-9][0-9]*\).*/\1/p')"
registry_pid="$(printf '%s\n' "$atspi_state_before" |
  sed -n 's/.*"registry":{"pid":\([0-9][0-9]*\).*/\1/p')"
launcher_start_ticks="$(printf '%s\n' "$atspi_state_before" |
  sed -n 's/.*"launcher":{"pid":[0-9][0-9]*,"start_ticks":\([0-9][0-9]*\)}.*/\1/p')"
registry_start_ticks="$(printf '%s\n' "$atspi_state_before" |
  sed -n 's/.*"registry":{"pid":[0-9][0-9]*,"start_ticks":\([0-9][0-9]*\)}.*/\1/p')"
[[ -n "$launcher_pid" && -n "$registry_pid" && -n "$launcher_start_ticks" &&
  -n "$registry_start_ticks" ]] ||
  fail 'private AT-SPI ownership state did not contain both owner generations'
reuse_display="$("$client" isolated-desktop ensure | sed -n 's/^display=//p')" ||
  fail 'second isolated-desktop ensure failed'
[[ "$reuse_display" == "$ISOLATED_DISPLAY" ]] ||
  fail "ensure was not idempotent: reuse reported ${reuse_display}"
atspi_state_after="$(<"$atspi_state_path")"
[[ "$atspi_state_after" == "$atspi_state_before" ]] ||
  fail 'second ensure replaced or reclassified a private AT-SPI owner generation'

# Status must report the sandbox up with the expected display + geometry.
status_out="$("$client" isolated-desktop status)" || fail 'isolated-desktop status failed'
printf '%s\n' "$status_out"

status_up="$(printf '%s\n' "$status_out" | sed -n 's/^up=//p')"
status_display="$(printf '%s\n' "$status_out" | sed -n 's/^display=//p')"
status_geometry="$(printf '%s\n' "$status_out" | sed -n 's/^geometry=//p')"
status_bus_launcher="$(printf '%s\n' "$status_out" | sed -n 's/^dep_at_spi_bus_launcher=//p')"
status_registry="$(printf '%s\n' "$status_out" | sed -n 's/^dep_at_spi_registry=//p')"
[[ "$status_up" == "true" ]] || fail "status reported up=${status_up}, expected true"
[[ "$status_display" == "$ISOLATED_DISPLAY" ]] ||
  fail "status reported display=${status_display}, expected ${ISOLATED_DISPLAY}"
[[ "$status_geometry" == "$EXPECTED_GEOMETRY" ]] ||
  fail "status reported geometry=${status_geometry}, expected ${EXPECTED_GEOMETRY}"
[[ "$status_bus_launcher" == "true" ]] ||
  fail "status reported dep_at_spi_bus_launcher=${status_bus_launcher}, expected true"
[[ "$status_registry" == "true" ]] ||
  fail "status reported dep_at_spi_registry=${status_registry}, expected true"

# The sandbox X server must be reachable on :N.
DISPLAY="$ISOLATED_DISPLAY" xdpyinfo >/dev/null 2>&1 ||
  fail "sandbox display ${ISOLATED_DISPLAY} is not reachable via xdpyinfo"

# Pick an application to launch into the sandbox. Prefer the GTK app provisioned
# by the canonical VM so this profile also proves application-tree registration;
# retain pure-X fallbacks for older/minimal images.
app_cmd=()
accessible_app=0
app_title="sky-cua-isolated-smoke-$$"
if command -v zenity >/dev/null 2>&1; then
  app_cmd=(zenity --info --no-wrap --title "$app_title" --text "sky-cua isolated smoke")
  accessible_app=1
elif command -v xmessage >/dev/null 2>&1; then
  app_cmd=(xmessage -name "$app_title" -title "$app_title" "sky-cua isolated smoke")
elif command -v xterm >/dev/null 2>&1; then
  app_cmd=(xterm -name "$app_title" -title "$app_title" -e sleep 600)
elif command -v xclock >/dev/null 2>&1; then
  app_cmd=(xclock -name "$app_title")
else
  printf 'isolated-xpra profile requires one of: zenity, xmessage, xterm, xclock; skipping\n' >&2
  exit "$SKIP_ENVIRONMENT_MISSING"
fi

# Launch the app into the sandbox with the exact daemon sandbox env recipe
# (mirrors IsolatedDesktopHandle::spawn_env / removed_env): on :N, X11 session
# type, toolkit backends pinned to X, WAYLAND_DISPLAY removed. setsid detaches
# it so it outlives this shell, exactly as the daemon's launch_application does.
printf 'isolated-xpra: launching %s into the sandbox\n' "${app_cmd[0]}"
setsid env -u WAYLAND_DISPLAY -u AT_SPI_BUS_ADDRESS \
  DISPLAY="$ISOLATED_DISPLAY" \
  XAUTHORITY="$sandbox_xauthority" \
  DBUS_SESSION_BUS_ADDRESS="$session_bus_address" \
  XDG_SESSION_TYPE=x11 \
  QT_QPA_PLATFORM=xcb \
  GDK_BACKEND=x11 \
  NO_AT_BRIDGE=0 \
  ACCESSIBILITY_ENABLED=1 \
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

if [[ "$accessible_app" == "1" ]]; then
  app_bus=""
  app_window=""
  for _ in $(seq 1 40); do
    registry_children="$(gdbus call --address "$a11y_bus_address" \
      --dest org.a11y.atspi.Registry \
      --object-path /org/a11y/atspi/accessible/root \
      --method org.a11y.atspi.Accessible.GetChildren 2>/dev/null || true)"
    mapfile -t candidate_buses < <(
      printf '%s\n' "$registry_children" |
        grep -oE "':[0-9]+(\.[0-9]+)*'" | tr -d "'" | sort -u || true
    )
    for candidate_bus in "${candidate_buses[@]:-}"; do
      [[ -n "$candidate_bus" ]] || continue
      candidate_role="$(gdbus call --address "$a11y_bus_address" \
        --dest "$candidate_bus" \
        --object-path /org/a11y/atspi/accessible/root \
        --method org.a11y.atspi.Accessible.GetRoleName 2>/dev/null || true)"
      [[ "$candidate_role" == "('application',)" ]] || continue
      candidate_children="$(gdbus call --address "$a11y_bus_address" \
        --dest "$candidate_bus" \
        --object-path /org/a11y/atspi/accessible/root \
        --method org.a11y.atspi.Accessible.GetChildren 2>/dev/null || true)"
      mapfile -t candidate_paths < <(
        printf '%s\n' "$candidate_children" |
          grep -oE "objectpath '[^']+'" | sed "s/^objectpath '//; s/'$//" || true
      )
      for candidate_path in "${candidate_paths[@]:-}"; do
        [[ -n "$candidate_path" ]] || continue
        candidate_name="$(gdbus call --address "$a11y_bus_address" \
          --dest "$candidate_bus" \
          --object-path "$candidate_path" \
          --method org.freedesktop.DBus.Properties.Get \
          org.a11y.atspi.Accessible Name 2>/dev/null || true)"
        if [[ "$candidate_name" == "(<'${app_title}'>,)" ]]; then
          app_bus="$candidate_bus"
          app_window="$candidate_path"
          break 2
        fi
      done
    done
    [[ -n "$app_window" ]] && break
    sleep 0.25
  done
  [[ -n "$app_bus" && -n "$app_window" ]] ||
    fail 'launched GTK window never registered with the private AT-SPI registry'
  window_role="$(gdbus call --address "$a11y_bus_address" \
    --dest "$app_bus" \
    --object-path "$app_window" \
    --method org.a11y.atspi.Accessible.GetRoleName)" ||
    fail 'registered application window did not answer GetRoleName'
  [[ "$window_role" == "('window',)" || "$window_role" == "('frame',)" ||
    "$window_role" == "('dialog',)" || "$window_role" == "('alert',)" ]] ||
    fail "registered AT-SPI child has unexpected role: ${window_role}"
  printf 'isolated-xpra: private AT-SPI registry exposes application + window roots\n'
else
  printf 'isolated-xpra: selected pure-X app; application-tree proof skipped\n'
fi

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
  has_xauthority=0
  has_session_type=0
  has_wayland=0
  has_session_bus=0
  has_no_at_bridge=0
  has_accessibility_enabled=0
  has_direct_atspi_bus=0
  for entry in "${environ_entries[@]}"; do
    case "$entry" in
    "DISPLAY=${ISOLATED_DISPLAY}") has_display=1 ;;
    "XAUTHORITY=${sandbox_xauthority}") has_xauthority=1 ;;
    "XDG_SESSION_TYPE=x11") has_session_type=1 ;;
    "DBUS_SESSION_BUS_ADDRESS=${session_bus_address}") has_session_bus=1 ;;
    "NO_AT_BRIDGE=0") has_no_at_bridge=1 ;;
    "ACCESSIBILITY_ENABLED=1") has_accessibility_enabled=1 ;;
    WAYLAND_DISPLAY=*) has_wayland=1 ;;
    AT_SPI_BUS_ADDRESS=*) has_direct_atspi_bus=1 ;;
    esac
  done
  [[ "$has_display" == "1" ]] ||
    fail "launched app environ lacks DISPLAY=${ISOLATED_DISPLAY}"
  [[ "$has_xauthority" == "1" ]] ||
    fail 'launched app environ lacks the private XAUTHORITY'
  [[ "$has_session_type" == "1" ]] ||
    fail 'launched app environ lacks XDG_SESSION_TYPE=x11'
  [[ "$has_wayland" == "0" ]] ||
    fail 'launched app environ still carries WAYLAND_DISPLAY'
  [[ "$has_session_bus" == "1" ]] ||
    fail 'launched app environ lacks the private DBUS_SESSION_BUS_ADDRESS'
  [[ "$has_no_at_bridge" == "1" ]] || fail 'launched app environ lacks NO_AT_BRIDGE=0'
  [[ "$has_accessibility_enabled" == "1" ]] ||
    fail 'launched app environ lacks ACCESSIBILITY_ENABLED=1'
  [[ "$has_direct_atspi_bus" == "0" ]] ||
    fail 'launched app environ carries a stale AT_SPI_BUS_ADDRESS override'
  printf 'isolated-xpra: launched app environ confirms private X11/AT-SPI identity\n'
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
if [[ -e "${XDG_RUNTIME_DIR}/sky-cua/isolated-atspi-${DISPLAY_NUMBER}.json" ]]; then
  fail 'isolated AT-SPI ownership state survived desktop teardown'
fi
for _ in $(seq 1 40); do
  if ! process_generation_is_alive "$registry_pid" "$registry_start_ticks" &&
    ! process_generation_is_alive "$launcher_pid" "$launcher_start_ticks"; then
    break
  fi
  sleep 0.25
done
if process_generation_is_alive "$registry_pid" "$registry_start_ticks"; then
  fail "private AT-SPI registry generation ${registry_pid}/${registry_start_ticks} survived desktop teardown"
fi
if process_generation_is_alive "$launcher_pid" "$launcher_start_ticks"; then
  fail "private AT-SPI launcher generation ${launcher_pid}/${launcher_start_ticks} survived desktop teardown"
fi

printf 'isolated-xpra: PASS (sandbox up, app isolated to %s, clean teardown)\n' "$ISOLATED_DISPLAY"
