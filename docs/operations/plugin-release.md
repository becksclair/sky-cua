# Plugin Release Runbook

Use this runbook for Codex plugin lifecycle work, not ordinary desktop or browser automation.

## Core Invariant

Keep the Heliasar marketplace registered, and keep exactly one active
`computer-use` MCP server — provided by the compat plugin id, not by a
sky-cua channel id.

Codex Desktop detects Computer Use plugins by the built-in plugin name
`computer-use`, so `computer-use@openai-bundled` (the compat plugin root the
chrome preflight materializes under
`~/.codex/plugins/cache/openai-bundled/computer-use/`) is the single enabled
computer-use plugin. Both `sky-cua@debug` and `sky-cua@Heliasar` stay
installed but disabled; they are payload carriers, not the active plugin id.

Debug-versus-release selection happens by retargeting the compat root, not by
toggling channel ids:

- Debug work: `deploy_debug_plugin.py` reruns the preflight from the debug
  install, so the compat root's `.mcp.json` launches the debug payload.
- Release work: `publish_marketplace_release.py` / `deploy_release_plugin.py`
  rerun the preflight from the installed Heliasar cache payload
  (`~/.codex/plugins/cache/Heliasar/sky-cua/<version>`), so the compat root
  launches the published release.

Fallback: when a bundle ships without the openai-bundled resources (no compat
root can be materialized — minimal test bundles, non-Linux), the deploy
scripts fall back to enabling the matching sky-cua channel id directly.

Do not remove the Heliasar marketplace just to work on debug builds.

## Debug Iteration

When changing Rust code, scripts, app guidance, skills, plugin metadata, or
local bundle contents, deploy the debug plugin:

```bash
python3 scripts/deploy_debug_plugin.py
```

Use `--no-build` only when `dist/plugin/sky-cua` is already current:

```bash
python3 scripts/deploy_debug_plugin.py --no-build
```

Verify the active MCP server when needed:

```bash
codex mcp list --json
```

The `computer-use` server should point at a debug cache path, not the
Heliasar cache. The server is provided by `computer-use@openai-bundled`;
whether it runs debug or release bits is visible in the server command path.

## Publish Release

For the personal marketplace release path:

```bash
python3 scripts/publish_marketplace_release.py
```

This builds `dist/plugin/sky-cua`, stages it into
`~/projects/heliasar-marketplace/plugins/sky-cua`, writes the marketplace
manifest, commits marketplace changes, pushes `origin main`, upgrades the
Codex marketplace, installs the plugin, regenerates the `computer-use` compat
plugin root against the installed Heliasar cache payload, enables
`computer-use@openai-bundled`, disables both sky-cua channel ids, and reloads
MCP servers.

It then refreshes the local MCP-server install at `~/.local/share/sky-cua`
(claude-code host config) and restarts its runtime processes so the shared
daemon respawns from the new binaries. All plugin consumers share one daemon
socket, so publishing without this refresh would leave non-Codex hosts served
by stale daemon logic. The local install copies binaries from the published
bundle, so every channel ships identical bits.

Useful options:

- `--no-build`: publish the existing `dist/plugin/sky-cua`. The publish
  aborts if the bundle's binaries differ from a present `target/release`
  (a rebuilt workspace with an unrebuilt bundle would silently ship old
  code); pass `--allow-stale-bundle` to publish the bundle as-is.
- `--no-push`: commit locally but leave the marketplace push to the operator.
- `--skip-codex-install`: update the marketplace repo without changing the
  local Codex install. Also skips the local MCP-server refresh (repo-only
  runs, e.g. CI).
- `--skip-local-install`: publish and install in Codex without touching the
  local MCP-server install.
- `--local-install-dir`, `--local-install-host`: override the local
  MCP-server install location and host config format.

Before publishing, the Heliasar checkout must already be an initialized Git repo
with `HEAD`; first-time repo creation belongs to `gitea-repo`, not this script.

## First-Time Setup

```bash
python3 scripts/setup_heliasar_marketplace.py
```

This clones or fast-forwards `~/projects/heliasar-marketplace`, runs the public
Codex marketplace add/upgrade flow for `becksclair/heliasar-marketplace`,
installs `sky-cua`, regenerates the compat plugin root from the installed
cache payload, applies the compat-first enablement, and reloads MCP servers.

## Verification

Use the narrowest check that matches the work:

```bash
uv run ruff format --check scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
python3 scripts/build_plugin.py
codex mcp list --json
python3 scripts/live_app_server_smoke.py
```

For pure script or metadata edits, the Python gates and `build_plugin.py` are
usually enough. For install or runtime registration changes, include
`codex mcp list --json`; for behavior acceptance, run `live_app_server_smoke.py`.
