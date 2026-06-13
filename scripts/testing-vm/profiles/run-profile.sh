#!/usr/bin/env bash
set -euo pipefail

profile="${1:-kde-kwin-effect}"
shift || true
profile_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/sky-cua-runtime}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

copy_codex_path() {
	local relative_path="$1"
	local source_path="/mnt/host-codex/$relative_path"
	local target_path="$HOME/.codex/$relative_path"
	if [ -e "$source_path" ] || [ -L "$source_path" ]; then
		mkdir -p "$(dirname "$target_path")"
		cp -aL "$source_path" "$target_path"
	fi
}

if [ "${SKY_CUA_COPY_CODEX_SETTINGS:-0}" != "0" ] && [ -d /mnt/host-codex ]; then
	mkdir -p "$HOME/.codex"
	for relative_path in \
		auth.json \
		cap_sid \
		config.json \
		config.toml \
		.codex-global-state.json \
		installation_id \
		internal_storage.json \
		models_cache.json \
		state_5.sqlite \
		state_5.sqlite-shm \
		state_5.sqlite-wal \
		version.json \
		keybindings.json \
		browser/config.toml; do
		copy_codex_path "$relative_path"
	done
	for relative_dir in plugins skills; do
		if [ -d "/mnt/host-codex/$relative_dir" ]; then
			rm -rf "$HOME/.codex/$relative_dir"
			mkdir -p "$HOME/.codex"
			cp -aL "/mnt/host-codex/$relative_dir" "$HOME/.codex/$relative_dir"
		fi
	done
fi

case "$profile" in
all)
	exec "$profile_dir/all.sh" "$@"
	;;
kde | kde-kwin-effect)
	exec "$profile_dir/kde-kwin-effect.sh" "$@"
	;;
kde-kwin-effect-system-install)
	exec "$profile_dir/kde-kwin-effect-system-install.sh" "$@"
	;;
kde-plasma)
	exec "$profile_dir/kde-plasma.sh" "$@"
	;;
gnome)
	exec "$profile_dir/gnome.sh" "$@"
	;;
cosmic)
	exec "$profile_dir/cosmic.sh" "$@"
	;;
hyprland)
	exec "$profile_dir/hyprland.sh" "$@"
	;;
i3)
	exec "$profile_dir/i3.sh" "$@"
	;;
computer-use | wayland-pointer)
	exec "$profile_dir/wayland-pointer.sh" "$@"
	;;
targeted-screenshot)
	exec "$profile_dir/targeted-screenshot.sh" "$@"
	;;
display-screenshot)
	exec "$profile_dir/display-screenshot.sh" "$@"
	;;
wayland-pointer-scaled)
	exec "$profile_dir/wayland-pointer-scaled.sh" "$@"
	;;
session-env)
	exec "$profile_dir/session-env.sh" "$@"
	;;
desktop-smoke)
	exec "$profile_dir/desktop-smoke.sh" "$@"
	;;
text-readback)
	exec "$profile_dir/text-readback.sh" "$@"
	;;
codex-desktop)
	exec "$profile_dir/codex-desktop.sh" "$@"
	;;
cosmic-helper)
	exec "$profile_dir/cosmic-helper.sh" "$@"
	;;
wayland-layer-shell-overlay)
	exec "$profile_dir/wayland-layer-shell-overlay.sh" "$@"
	;;
opencode-mcp)
	exec "$profile_dir/opencode-mcp.sh" "$@"
	;;
pi-mcp)
	exec "$profile_dir/pi-mcp.sh" "$@"
	;;
*)
	printf 'unknown testing VM profile: %s\n' "$profile" >&2
	exit 64
	;;
esac
