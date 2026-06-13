#!/usr/bin/env bash
set -euo pipefail

profile_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

"$profile_dir/kde-kwin-effect.sh"
"$profile_dir/wayland-pointer.sh"
"$profile_dir/targeted-screenshot.sh"
"$profile_dir/display-screenshot.sh"
"$profile_dir/session-env.sh"
"$profile_dir/text-readback.sh"
"$profile_dir/codex-desktop.sh"
"$profile_dir/opencode-mcp.sh"
"$profile_dir/pi-mcp.sh"

if [[ -n "${HOST_WAYLAND_DISPLAY:-}" ]]; then
	for headed in kde-plasma gnome cosmic hyprland; do
		"$profile_dir/$headed.sh" --headed
	done
else
	printf 'Skipping legacy nested visual-debug compositor profiles; run the VM in the target desktop session for real-session proof, or pass --headed for nested debug profiles.\n'
fi
