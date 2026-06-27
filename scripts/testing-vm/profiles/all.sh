#!/usr/bin/env bash
set -euo pipefail

profile_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

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
# codex-cua`, which has the host gpt-5.5 auth the VM lacks).
"$profile_dir/codex-cua.sh"
"$profile_dir/kde-kwin-effect.sh"

if [[ -n "${HOST_WAYLAND_DISPLAY:-}" ]]; then
	for headed in kde-plasma gnome cosmic hyprland; do
		"$profile_dir/$headed.sh" --headed
	done
else
	printf 'Skipping legacy nested visual-debug compositor profiles; run the VM in the target desktop session for real-session proof, or pass --headed for nested debug profiles.\n'
fi
