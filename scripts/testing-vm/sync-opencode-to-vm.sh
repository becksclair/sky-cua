#!/usr/bin/env bash
set -euo pipefail

# Sync only the portable OpenCode harness state into the Arch testing VM.
# This intentionally avoids ~/.local/share/opencode/{opencode.db,log,snapshot}
# so the VM stays a clean smoke environment instead of inheriting host history.

host="${SKY_CUA_TESTING_VM_HOST:-127.0.0.1}"
port="${SKY_CUA_TESTING_VM_PORT:-22222}"
user="${SKY_CUA_TESTING_VM_USER:-skycua}"
known_hosts="${SKY_CUA_TESTING_VM_KNOWN_HOSTS:-artifacts/testing-vm/known_hosts}"
opencode_config="${OPENCODE_CONFIG_HOME:-${HOME}/.config/opencode}"
opencode_auth="${OPENCODE_AUTH_JSON:-${HOME}/.local/share/opencode/auth.json}"
opencode_desktop_config="${OPENCODE_DESKTOP_CONFIG_HOME:-${HOME}/.config/@opencode-ai}"

ssh_options=(
	-p "${port}"
	-o BatchMode=yes
	-o ConnectTimeout=8
	-o StrictHostKeyChecking=no
	-o "UserKnownHostsFile=${known_hosts}"
)
rsync_ssh="ssh -p ${port} -o BatchMode=yes -o ConnectTimeout=8 -o StrictHostKeyChecking=no -o UserKnownHostsFile=${known_hosts}"
target="${user}@${host}"

if [[ ! -d "${opencode_config}" ]]; then
	printf 'OpenCode config directory not found: %s\n' "${opencode_config}" >&2
	exit 66
fi

if [[ ! -f "${opencode_auth}" ]]; then
	printf 'OpenCode auth file not found: %s\n' "${opencode_auth}" >&2
	exit 66
fi

ssh "${ssh_options[@]}" "${target}" \
	'set -euo pipefail; mkdir -p ~/.agents ~/.config ~/.local/share/opencode ~/.config/@opencode-ai'

rsync -aL --delete \
	--exclude node_modules \
	--exclude .ruff_cache \
	--exclude opencode-notifier-state.json \
	-e "${rsync_ssh}" \
	"${opencode_config}/" \
	"${target}:.agents/opencode/"

rsync -aL \
	-e "${rsync_ssh}" \
	"${opencode_auth}" \
	"${target}:.local/share/opencode/auth.json"

if [[ -d "${opencode_desktop_config}" ]]; then
	rsync -aL --delete \
		-e "${rsync_ssh}" \
		"${opencode_desktop_config}/" \
		"${target}:.config/@opencode-ai/"
fi

ssh "${ssh_options[@]}" "${target}" 'set -euo pipefail
export PATH="$HOME/.local/bin:$PATH"
rm -rf ~/.config/opencode
ln -s "$HOME/.agents/opencode" "$HOME/.config/opencode"
chmod 700 ~/.local/share/opencode ~/.config/@opencode-ai || true
chmod 600 ~/.local/share/opencode/auth.json
# Update OpenCode to latest. The provisioner installs it globally as root, so
# prefer passwordless sudo before falling back to a user-writable prefix.
sudo -n npm install -g opencode-ai@latest || npm install -g --prefix ~/.local opencode-ai@latest || printf "warning: opencode update failed; continuing with existing install\n" >&2

if [[ -f ~/.agents/opencode/package.json ]]; then
  cd ~/.agents/opencode
  npm install --omit=dev
fi
opencode --version
test -f ~/.config/opencode/opencode.jsonc
test -s ~/.local/share/opencode/auth.json
printf "OpenCode config and auth synced to %s\n" "$HOME"'
