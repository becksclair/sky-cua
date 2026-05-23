#!/usr/bin/env bash
set -euo pipefail

export PATH="${HOME}/.local/bin:${PATH}"

if ! command -v pi >/dev/null; then
	printf 'pi is not installed in the testing VM\n' >&2
	exit 66
fi

remote_root="/workspace"
target_dir="${HOME}/.local/share/sky-cua"
pi_mcp="${HOME}/.pi/agent/mcp.json"

# Install sky-cua MCP server for Pi
python3 "${remote_root}/scripts/install_mcp_server.py" \
	--host pi \
	--target-dir "${target_dir}"

# Merge the generated pi_mcp.json into Pi's mcp.json
if [[ -f "${pi_mcp}" ]]; then
	# Use Python to merge JSON objects
	python3 -c "
import json
import sys

with open('${pi_mcp}') as f:
    existing = json.load(f)
with open('${target_dir}/pi_mcp.json') as f:
    new = json.load(f)

existing.setdefault('mcpServers', {})
existing['mcpServers']['sky_cua'] = new['mcpServers']['sky_cua']

with open('${pi_mcp}', 'w') as f:
    json.dump(existing, f, indent=2)
    f.write('\n')
"
else
	cp "${target_dir}/pi_mcp.json" "${pi_mcp}"
fi

# Deploy sky-cua skills for Pi
mkdir -p "${HOME}/.pi/agent/skills"
rm -rf "${HOME}/.pi/agent/skills/computer-use-workflows"
cp -r "${remote_root}/skills/computer-use-workflows" "${HOME}/.pi/agent/skills/"
rm -rf "${HOME}/.pi/agent/skills/sky-cua-isolated-daemon"
cp -r "${remote_root}/skills/sky-cua-isolated-daemon" "${HOME}/.pi/agent/skills/"

# Run smoke tests across available fixtures
for fixture in zenity kdialog; do
	if command -v "${fixture}" >/dev/null 2>&1; then
		python3 "${remote_root}/scripts/live_agent_mcp_smoke.py" --agent pi --fixture "${fixture}" || exit 1
	else
		printf 'Skipping %s fixture: not installed in this VM\n' "${fixture}"
	fi
done
