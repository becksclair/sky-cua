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
install_policy_args=()
if [[ -n "${SKY_CUA_BROWSER_EVAL:-}" ]]; then
	install_policy_args+=(--browser-eval "${SKY_CUA_BROWSER_EVAL}")
fi
if [[ -n "${SKY_CUA_MODEL_SUPPORTS_IMAGES:-}" ]]; then
	install_policy_args+=(--model-supports-images "${SKY_CUA_MODEL_SUPPORTS_IMAGES}")
fi

# Install sky-cua MCP server for Pi
python3 "${remote_root}/scripts/install_mcp_server.py" \
	--host pi \
	--target-dir "${target_dir}" \
	--restart-runtime \
	"${install_policy_args[@]}"

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
rm -f "${HOME}/.pi/agent/mcp-cache.json"

# Deploy sky-cua skills to the shared agent skill root imported by Pi
mkdir -p "${HOME}/.agents/skills"
for skill in computer-use browser-use; do
	rm -rf "${HOME}/.agents/skills/${skill}"
	cp -r "${remote_root}/skills/${skill}" "${HOME}/.agents/skills/"
done

# Minimal wiring check: prove Pi sees the sky-cua schema and one read-only tool
# call succeeds (free model). Substantive tool-use coverage is the codex-cua profile.
python3 "${remote_root}/scripts/live_agent_mcp_smoke.py" --agent pi --mode wiring || exit 1
