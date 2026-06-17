#!/usr/bin/env bash
set -euo pipefail

export PATH="${HOME}/.local/bin:${PATH}"

if ! command -v opencode >/dev/null; then
	printf 'opencode is not installed in the testing VM\n' >&2
	exit 66
fi

remote_root="/workspace"
target_dir="${HOME}/.local/share/sky-cua"

# Forward API keys from host if available
export FIREWORKS_API_KEY="${FIREWORKS_API_KEY:-}"
export OPENAI_API_KEY="${OPENAI_API_KEY:-}"

# Install sky-cua MCP server for OpenCode
python3 "${remote_root}/scripts/install_mcp_server.py" \
	--host opencode \
	--target-dir "${target_dir}"

# Place the generated opencode.json in the workspace so OpenCode discovers it
# (OpenCode reads opencode.json from the current directory)
cp "${target_dir}/opencode.json" "${remote_root}/opencode.json"

# Ensure skills are discoverable by OpenCode
# The plugin install already bundles them; verify path exists
plugin_skills="${HOME}/.codex/plugins/cache/local/sky-cua/local/skills"
if [[ ! -d "${plugin_skills}" ]]; then
	# Fallback: copy skills directly into OpenCode skills path
	mkdir -p "${HOME}/.codex/skills"
	for skill in computer-use browser-use; do
		rm -rf "${HOME}/.codex/skills/${skill}"
		cp -r "${remote_root}/skills/${skill}" "${HOME}/.codex/skills/"
	done
fi

# Run smoke tests across available fixtures
for fixture in zenity kdialog; do
	if command -v "${fixture}" >/dev/null 2>&1; then
		python3 "${remote_root}/scripts/live_agent_mcp_smoke.py" --agent opencode --fixture "${fixture}" || exit 1
	else
		printf 'Skipping %s fixture: not installed in this VM\n' "${fixture}"
	fi
done
