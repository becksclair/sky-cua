---
name: cua-deploy
description: >-
  Use when asked to rebuild, deploy, restart, publish, or push sky-cua changes
  — any combination of: build the plugin bundle, install it into Codex debug
  cache or as a local release, publish to the Heliasar marketplace, restart the
  MCP runtime, commit staged/unstaged changes semantically, and push. Also use
  when someone says "deploy it", "rebuild and push", "ship it", or equivalent
  shorthand.
---

# cua-deploy

Automates the sky-cua change-to-ship pipeline: build → deploy/publish → sync OpenClaw workspace skills → restart → commit → push.
Determine the appropriate lane from context and task scope; never run more pipeline than was asked.

## OpenClaw workspace skills sync

For any deploy or publish lane, also replace the bundled sky-cua skills in
OpenClaw's workspace skill root. This keeps OpenClaw agents on the same
browser/computer-use instructions as the deployed plugin.

Run after the bundle build/deploy command succeeds and before any standalone
MCP restart/reload command you invoke. The helper copies from the actual bundle
payload at `dist/plugin/sky-cua/skills`, so `--no-build` lanes sync the
existing bundle instead of the live source tree:

```bash
python3 scripts/sync_openclaw_workspace_skills.py
```

Only replace the sky-cua-owned skill folders above; do not delete unrelated
skills in `~/.openclaw/workspace/skills`. The helper stages both skills first
and rolls back prior destinations if replacement fails.

## Lanes

### Debug deploy (local Codex cache)

Use when iterating on unreleased changes locally. Installs into `~/.codex` debug cache; does not touch the marketplace.
After any command in this lane succeeds, run the OpenClaw workspace skills sync
block before restarting the MCP runtime.

```bash
# Build + deploy debug bundle, then restart stale MCP runtime processes
python3 scripts/deploy_debug_plugin.py
python3 scripts/sync_openclaw_workspace_skills.py
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime

# Also rebuild and reload the KWin agent-cursor effect (Linux/KDE only)
python3 scripts/deploy_debug_plugin.py --kwin-effect
python3 scripts/sync_openclaw_workspace_skills.py
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime

# Install existing bundle without rebuilding, then restart
python3 scripts/deploy_debug_plugin.py --no-build
python3 scripts/sync_openclaw_workspace_skills.py
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime
```

### Release deploy (local marketplace)

Use when testing the full release install path against the local Heliasar checkout at `~/projects/heliasar-marketplace`.
After any command in this lane succeeds, run the OpenClaw workspace skills sync
block. If you also run a standalone MCP runtime restart, run the sync first.

```bash
# Deploy, then restart stale MCP runtime processes
python3 scripts/deploy_release_plugin.py
python3 scripts/sync_openclaw_workspace_skills.py
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime

# Skip calling codex app-server plugin/install (just stage marketplace + config)
python3 scripts/deploy_release_plugin.py --skip-codex-install
python3 scripts/sync_openclaw_workspace_skills.py
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime
```

### Publish (full release to Heliasar marketplace + local MCP install)

Use when shipping a version. Builds, writes marketplace entries, commits/pushes the marketplace repo, and refreshes the local MCP install.
After any command in this lane succeeds, run the OpenClaw workspace skills sync
block.

```bash
# Full publish including local Claude Code MCP install restart
python3 scripts/publish_marketplace_release.py --local-install-host claude-code
python3 scripts/sync_openclaw_workspace_skills.py

# Skip local MCP install (marketplace push only)
python3 scripts/publish_marketplace_release.py --skip-local-install
python3 scripts/sync_openclaw_workspace_skills.py

# Use existing bundle (no Cargo rebuild)
python3 scripts/publish_marketplace_release.py --local-install-host claude-code --no-build
python3 scripts/sync_openclaw_workspace_skills.py
```

## MCP runtime restart (standalone)

Refresh the local MCP server install and bounce the runtime without a full deploy:

```bash
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime
```

For OpenClaw, reload after config changes:

```bash
openclaw mcp reload
```

## Commit and push

After the deploy succeeds, commit all relevant staged and unstaged changes with a semantic message and push. Use the `committer` subagent for this step — it writes diff-grounded commit messages and handles staging precisely.

Key conventions:
- Commit message prefix: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`, `test:` etc.
- One logical unit per commit; do not bundle unrelated changes.
- Do not push until Rust and Python checks pass (`cargo fmt --check && cargo test`, `uv run ruff check scripts && uv run basedpyright && uv run pytest`).
- Branch names use `bex/` unless otherwise specified.

## Decision tree

1. **Is this a local dev iteration?** → debug deploy lane + MCP restart → commit + push.
2. **Is this a local release test?** → release deploy lane → commit + push.
3. **Is this shipping to the marketplace?** → publish lane (includes MCP restart) → commit + push.
4. **Only need to restart the runtime?** → standalone MCP restart, no deploy.

## Checks before pushing

Run the narrowest relevant check set. Only run root checks when shared contracts changed.

```bash
# Rust
cargo fmt --check && cargo test

# Python
uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest
```

State any live-smoke gates not run (desktop/portal/KDE/COSMIC/Hyprland/GNOME).

## Common flags summary

| Script | Key flags |
|---|---|
| `deploy_debug_plugin.py` | `--no-build`, `--symlink`, `--kwin-effect` |
| `deploy_release_plugin.py` | `--no-build`, `--skip-codex-install`, `--codex-bin` |
| `publish_marketplace_release.py` | `--no-build`, `--local-install-host`, `--skip-local-install`, `--skip-codex-install`, `--version-from-tag`, `--no-push` |
| `install_mcp_server.py` | `--host`, `--restart-runtime`, `--kwin-effect` |
