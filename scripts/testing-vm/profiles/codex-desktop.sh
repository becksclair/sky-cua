#!/usr/bin/env bash
set -euo pipefail

if ! command -v codex-desktop >/dev/null; then
  printf 'codex-desktop is not installed in the testing VM\n' >&2
  exit 66
fi

artifact_root="/workspace/artifacts/gui-desktop-smoke/codex-desktop/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$artifact_root"

if [[ -z "${WAYLAND_DISPLAY:-}" || ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
  printf 'codex-desktop profile requires a real Wayland session socket: %s/%s\n' "$XDG_RUNTIME_DIR" "${WAYLAND_DISPLAY:-}" >&2
  exit 67
fi

export DISPLAY="${DISPLAY:-}"
export ELECTRON_OZONE_PLATFORM_HINT=wayland
export XDG_CURRENT_DESKTOP="${XDG_CURRENT_DESKTOP:-COSMIC}"
export XDG_SESSION_DESKTOP="${XDG_SESSION_DESKTOP:-COSMIC}"
export DESKTOP_SESSION="${DESKTOP_SESSION:-COSMIC}"
export XDG_SESSION_TYPE=wayland

cleanup() {
  if [[ -n "${codex_pid:-}" ]] && kill -0 "$codex_pid" 2>/dev/null; then
    kill "$codex_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

sky_cua_client="${SKY_CUA_CLIENT:-/workspace/target/release/sky-cua-client}"
if [[ ! -x "$sky_cua_client" ]]; then
  printf 'missing host-built sky-cua client artifact: %s\n' "$sky_cua_client" >&2
  exit 66
fi

dbus-run-session -- codex-desktop \
  --no-sandbox \
  --disable-dev-shm-usage \
  --ozone-platform=wayland \
  >"$artifact_root/codex-desktop.log" 2>&1 &
codex_pid=$!

window_found=0
for _ in $(seq 1 120); do
  pgrep -a codex >"$artifact_root/processes.txt" 2>&1 || true
  "$sky_cua_client" list-windows >"$artifact_root/windows.json" 2>&1 || true
  if python3 - "$artifact_root/windows.json" >"$artifact_root/matched-window.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
try:
    payload = json.loads(path.read_text(encoding="utf-8"))
except json.JSONDecodeError:
    raise SystemExit(1)

for window in payload.get("windows", []):
    haystack = " ".join(
        str(window.get(key) or "") for key in ("title", "app_id", "wm_class", "name")
    ).lower()
    if "codex" in haystack:
        print(json.dumps(window, indent=2))
        raise SystemExit(0)

raise SystemExit(1)
PY
  then
    window_found=1
    break
  fi
  if [[ -n "${SKY_CUA_COSMIC_HELPER:-}" && -x "${SKY_CUA_COSMIC_HELPER:-}" ]]; then
    "$SKY_CUA_COSMIC_HELPER" list-windows >"$artifact_root/cosmic-windows.json" 2>&1 || true
  fi
  if ! kill -0 "$codex_pid" 2>/dev/null; then
    printf 'codex-desktop exited before opening a window\n' >&2
    cat "$artifact_root/codex-desktop.log" >&2
    exit 1
  fi
  sleep 0.5
done

if [[ "$window_found" != 1 ]]; then
  printf 'codex-desktop did not expose a visible window through sky-cua list-windows\n' >&2
  cat "$artifact_root/codex-desktop.log" >&2
  printf '\nlast list-windows output:\n' >&2
  cat "$artifact_root/windows.json" >&2 || true
  exit 1
fi

google-chrome --version >"$artifact_root/google-chrome-version.txt"
pacman -Q codex-desktop >"$artifact_root/codex-desktop-version.txt"

printf 'Codex Desktop launch smoke passed; artifacts: %s\n' "$artifact_root"
