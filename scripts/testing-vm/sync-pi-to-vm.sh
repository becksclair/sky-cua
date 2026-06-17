#!/usr/bin/env bash
set -euo pipefail

# Sync Pi harness state into the Arch testing VM and update to latest versions.
# This intentionally avoids runtime state (sessions, cache) so the VM stays
# a clean smoke environment instead of inheriting host history.

host="${SKY_CUA_TESTING_VM_HOST:-127.0.0.1}"
port="${SKY_CUA_TESTING_VM_PORT:-22222}"
user="${SKY_CUA_TESTING_VM_USER:-skycua}"
known_hosts="${SKY_CUA_TESTING_VM_KNOWN_HOSTS:-artifacts/testing-vm/known_hosts}"
pi_home="${PI_HOME:-${HOME}/.pi}"

ssh_options=(
	-p "${port}"
	-o BatchMode=yes
	-o ConnectTimeout=8
	-o StrictHostKeyChecking=no
	-o "UserKnownHostsFile=${known_hosts}"
)
rsync_ssh="ssh -p ${port} -o BatchMode=yes -o ConnectTimeout=8 -o StrictHostKeyChecking=no -o UserKnownHostsFile=${known_hosts}"
target="${user}@${host}"

if [[ ! -d "${pi_home}" ]]; then
	printf 'Pi home directory not found: %s\n' "${pi_home}" >&2
	exit 66
fi

ssh "${ssh_options[@]}" "${target}" \
	'set -euo pipefail; mkdir -p ~/.pi'

rsync -aL --delete \
	--exclude sessions \
	--exclude cache \
	--exclude memory \
	--exclude mcp-cache.json \
	--exclude '*.log' \
	--exclude node_modules \
	-e "${rsync_ssh}" \
	"${pi_home}/" \
	"${target}:.pi/"

ssh "${ssh_options[@]}" "${target}" 'set -euo pipefail
export PATH="${HOME}/.local/bin:${PATH}"
# Update Pi and its MCP adapter to latest. The provisioner installs global npm
# packages as root, so prefer passwordless sudo before using a local fallback.
sudo -n npm install -g @earendil-works/pi-coding-agent@latest || npm install -g --prefix ~/.local @earendil-works/pi-coding-agent@latest
sudo -n npm install -g pi-mcp-adapter@latest || npm install -g --prefix ~/.local pi-mcp-adapter@latest
if [[ -f ~/.pi/agent/git/github.com/BlackBeltTechnology/pi-flows/package.json ]]; then
    npm install --prefix ~/.pi/agent/git/github.com/BlackBeltTechnology/pi-flows 2>/dev/null || printf "warning: pi-flows dependency restore failed; continuing with existing install\n" >&2
fi
# Ensure binaries are discoverable even with local install
if command -v pi >/dev/null; then
    pi --version
else
    printf "warning: pi binary not on PATH after update; smoke tests may need PATH set\n" >&2
fi
test -d ~/.pi/agent
test -f ~/.pi/agent/mcp.json
printf "Pi config synced and updated to latest on %s\n" "$HOME"'
