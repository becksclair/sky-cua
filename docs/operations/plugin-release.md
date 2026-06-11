# Plugin Release Runbook

Use this runbook for Codex plugin lifecycle work, not ordinary desktop or browser automation.

## Core Invariant

Keep the Heliasar marketplace registered, but enable only one `computer-use`
plugin id at a time.

- Debug work: `sky-cua@debug` enabled, `sky-cua@Heliasar` disabled.
- Release/published work: `sky-cua@Heliasar` enabled, `sky-cua@debug` disabled.

Do not remove the Heliasar marketplace just to work on debug builds. Toggle
plugin ids instead.

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

The `computer-use` server should point at a debug cache path, not the Heliasar cache.

## Publish Release

For the personal marketplace release path:

```bash
python3 scripts/publish_marketplace_release.py
```

This builds `dist/plugin/sky-cua`, stages it into
`~/projects/heliasar-marketplace/plugins/sky-cua`, writes the marketplace
manifest, commits marketplace changes, pushes `origin main`, upgrades the Codex
marketplace, installs/reloads `sky-cua@Heliasar`, and disables `sky-cua@debug`.

It then refreshes the local MCP-server install at `~/.local/share/sky-cua`
(claude-code host config) and restarts its runtime processes so the shared
daemon respawns from the new binaries. All plugin consumers share one daemon
socket, so publishing without this refresh would leave non-Codex hosts served
by stale daemon logic.

Useful options:

- `--no-build`: publish the existing `dist/plugin/sky-cua`. The local install
  copies binaries from `target/release`, so ensure that is current too.
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
installs `sky-cua`, enables `sky-cua@Heliasar`, disables `sky-cua@debug`, and
reloads MCP servers.

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
