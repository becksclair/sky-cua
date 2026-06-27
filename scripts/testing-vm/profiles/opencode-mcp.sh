#!/usr/bin/env bash
set -euo pipefail

export PATH="${HOME}/.local/bin:${PATH}"

if ! command -v opencode >/dev/null; then
	printf 'opencode is not installed in the testing VM\n' >&2
	exit 66
fi

remote_root="/workspace"
target_dir="${HOME}/.local/share/sky-cua"
install_policy_args=()
if [[ -n "${SKY_CUA_BROWSER_EVAL:-}" ]]; then
	install_policy_args+=(--browser-eval "${SKY_CUA_BROWSER_EVAL}")
fi
if [[ -n "${SKY_CUA_MODEL_SUPPORTS_IMAGES:-}" ]]; then
	install_policy_args+=(--model-supports-images "${SKY_CUA_MODEL_SUPPORTS_IMAGES}")
fi

# Forward API keys from host if available
export FIREWORKS_API_KEY="${FIREWORKS_API_KEY:-}"
export OPENAI_API_KEY="${OPENAI_API_KEY:-}"

# Install sky-cua MCP server for OpenCode
python3 "${remote_root}/scripts/install_mcp_server.py" \
	--host opencode \
	--target-dir "${target_dir}" \
	--restart-runtime \
	"${install_policy_args[@]}"

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

# Minimal wiring check: prove OpenCode sees the sky-cua schema and one read-only
# tool call succeeds (free model). Substantive tool-use coverage is the codex-cua profile.
python3 "${remote_root}/scripts/live_agent_mcp_smoke.py" --agent opencode --mode wiring || exit 1
