#!/usr/bin/env bash
set -euo pipefail

if [[ "${XDG_CURRENT_DESKTOP:-}" != *KDE* && "${DESKTOP_SESSION:-}" != *plasma* ]]; then
  printf 'kde-kwin-effect-system-install requires a real Plasma Wayland VM session\n' >&2
  exit 67
fi

export SKY_CUA_USE_PREBUILT_RUNTIMES=1
export SKY_CUA_OVERLAY_HOST_PATH="${SKY_CUA_OVERLAY_HOST_PATH:-/workspace/target/release/sky-cua-overlay-host}"
export KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1

python scripts/live_agent_cursor_kde_smoke.py \
  --mode kwin-effect-system-install \
  --allow-kwin-effect-system-install
