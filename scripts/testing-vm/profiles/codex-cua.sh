#!/usr/bin/env bash
set -euo pipefail

# Heavy single-run codex tool-use profile: drives the full computer-use and
# browser-use surface in one codex exec run, then writes a deterministic
# coverage summary the host judge consumes. Chrome + the sky-cua extension +
# the native-messaging host are brought up by the Python smoke itself.
#
# Auth: codex exec authenticates from the VM's own ~/.codex/auth.json (copied
# from the host via SKY_CUA_COPY_CODEX_SETTINGS). This profile is intentionally
# absent from AGENT_AUTH_PROFILES in run_gui_testing_vm_smoke.py, so no host
# AGENT_AUTH_ENV_KEYS are forwarded for it (it needs none). Re-scp ~/.codex to
# the VM when that auth is stale; see the testing-vm codex auth runbook.

export PATH="/opt/codex-desktop/resources:${HOME}/.local/bin:${PATH}"

if ! command -v codex >/dev/null; then
	printf 'codex CLI not found on PATH (expected /opt/codex-desktop/resources/codex)\n' >&2
	exit 66
fi
if ! command -v google-chrome >/dev/null; then
	printf 'google-chrome is not installed in the testing VM\n' >&2
	exit 66
fi
if [[ -z "${WAYLAND_DISPLAY:-}" || ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then
	printf 'codex-cua profile requires a real Wayland session socket: %s/%s\n' \
		"$XDG_RUNTIME_DIR" "${WAYLAND_DISPLAY:-}" >&2
	exit 67
fi

remote_root="/workspace"

# Build the native-messaging host binary up front; the smoke registers it in the
# Chrome native-host manifest and waits for its socket before invoking codex.
cargo build --release --bin sky-cua-chrome-host --manifest-path "${remote_root}/Cargo.toml"

export SKY_CUA_BROWSER=chrome
# SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS can extend the browser request budget past the
# 12s default for genuinely slow/remote bridges. It is intentionally NOT set here:
# the VM's browser failures are CDP debugger detaches, not budget exhaustion, and a
# longer budget only makes each failed op block longer and waste the agent's steps.

# Use the production openai-bundled compat surface (computer-use@openai-bundled)
# when its marketplace was staged into the VM by the runner; otherwise the build
# falls back to the sky-cua@local dev surface. The tool surface is identical.
bundled_root="${HOME}/.cache/sky-cua/openai-bundled"
if [[ -f "${bundled_root}/plugins/openai-bundled/.agents/plugins/marketplace.json" ]]; then
	export SKY_CUA_OPENAI_BUNDLED_RESOURCE_ROOT="${bundled_root}"
	printf 'codex-cua: openai-bundled compat surface staged at %s\n' "${bundled_root}"
else
	printf 'codex-cua: openai-bundled resources not staged; using sky-cua@local surface\n' >&2
fi

python3 "${remote_root}/scripts/live_codex_cua_smoke.py" "$@"
