#!/usr/bin/env bash
set -euo pipefail

profile_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# Self-contained isolated X11 sandbox proof: brings up its own headless xpra
# desktop, so it runs regardless of which real session the VM is booted into.
# Skips cleanly (exit 67) when xpra/openbox/xdotool are unavailable; tolerate
# that skip so a VM without xpra still runs the rest of the suite (xpra/openbox/
# xdotool are new prerequisites this profile introduced, unlike the always-
# provisioned deps the other lanes assume).
"$profile_dir/isolated-xpra.sh" || { rc=$?; [[ "$rc" -eq 67 ]] || exit "$rc"; }
"$profile_dir/wayland-pointer.sh"
"$profile_dir/targeted-screenshot.sh"
"$profile_dir/display-screenshot.sh"
"$profile_dir/session-env.sh"
"$profile_dir/text-readback.sh"
"$profile_dir/codex-desktop.sh"
"$profile_dir/opencode-mcp.sh"
"$profile_dir/pi-mcp.sh"
# Heavy single-run codex tool-use coverage (deterministic gate only here; the
# host-side performance judge runs via `run_gui_testing_vm_smoke.py --profile
# codex-cua`, which has the host Codex auth the VM lacks).
"$profile_dir/codex-cua.sh"
"$profile_dir/kde-kwin-effect.sh"

if [[ -n "${HOST_WAYLAND_DISPLAY:-}" ]]; then
	for headed in kde-plasma gnome cosmic hyprland; do
		"$profile_dir/$headed.sh" --headed
	done
else
	printf 'Skipping legacy nested visual-debug compositor profiles; run the VM in the target desktop session for real-session proof, or pass --headed for nested debug profiles.\n'
fi
