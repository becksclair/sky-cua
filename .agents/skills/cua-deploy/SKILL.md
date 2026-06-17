---
name: cua-deploy
description: >-
  Use when asked to rebuild, deploy, package, install, restart, or push sky-cua
  changes - any combination of: build the plugin bundle, install it into the
  local Codex payload, build a release tarball, install on a clean machine,
  restart the MCP runtime, commit staged/unstaged changes semantically, and
  push. Also use when someone says "deploy it", "rebuild and push", "ship it",
  or equivalent shorthand.
---

# cua-deploy

Automates the sky-cua change-to-ship pipeline: build -> deploy/package -> sync OpenClaw workspace skills -> commit -> push.
Determine the appropriate lane from context and task scope; never run more pipeline than was asked.

There is no marketplace and no publish flow. The two distribution lanes are:

- Local deploy (`scripts/deploy_plugin.py`): updates *what runs locally, immediately* - installs
  the built bundle as `sky-cua@local`, retargets the computer-use compat plugin at it, and
  refreshes the installed MCP runtime (no separate restart step). Does not touch git.
- Build + install a release package (`scripts/package.py`, then `python3 install.py` on the
  target): builds a self-contained tarball under `dist/release/`, which a clean machine extracts
  and installs in bundle mode (no build, no cargo). Materializes the compat plugin from the
  bundled preflight.

## OpenClaw workspace skills sync

For any deploy or publish lane, also replace the bundled sky-cua skills in
OpenClaw's workspace skill root. This keeps OpenClaw agents on the same
browser/computer-use instructions as the deployed plugin.

Run after the deploy/publish command succeeds. The helper copies from the actual
bundle payload at `dist/plugin/sky-cua/skills`, so `--no-build` lanes sync the
existing bundle instead of the live source tree:

```bash
python3 scripts/sync_openclaw_workspace_skills.py
```

Only replace the sky-cua-owned skill folders above; do not delete unrelated
skills in `~/.openclaw/workspace/skills`. The helper stages both skills first
and rolls back prior destinations if replacement fails.

## Lanes

### Local deploy (default)

Use when iterating on unreleased changes locally. Installs as `sky-cua@local` and refreshes the
MCP runtime in one command; in Linux desktop sessions this also attempts a
best-effort refresh of the user AT-SPI accessibility bus before sky-cua
reconnects so wedged semantic trees do not survive deploys. Does not touch the
marketplace.

```bash
# Build + deploy locally (also refreshes the installed MCP runtime)
python3 scripts/deploy_plugin.py
python3 scripts/sync_openclaw_workspace_skills.py

# Also rebuild and reload the KWin agent-cursor effect (Linux/KDE only)
python3 scripts/deploy_plugin.py --kwin-effect
python3 scripts/sync_openclaw_workspace_skills.py

# Install existing bundle without rebuilding
python3 scripts/deploy_plugin.py --no-build
python3 scripts/sync_openclaw_workspace_skills.py
```

### Build + install a release package

Use when shipping to a machine without a checkout or toolchain. Builds a self-contained tarball;
the target extracts it and installs in bundle mode.

```bash
# Build the release tarball under dist/release/
python3 scripts/package.py

# Use the existing bundle (no Cargo rebuild)
python3 scripts/package.py --no-build

# On the target machine: extract and install (no build, no cargo)
tar xzf sky-cua-<version>-<platform>.tar.gz
cd sky-cua-<version>
python3 install.py
```

`package.py` flags: `--no-build`, `--platform`, `--version-from-tag [TAG]`,
`--release-dir`. The packaged `install.py` accepts the installer flags
(`--agents`, `--mode`, `--bundle-root`, `--target-dir`, `--kwin-effect`,
`--skip-system-deps`, `--dry-run`). See
`docs/features/release-package.md`.

## MCP runtime restart (standalone)

Refresh the local MCP server install and bounce the runtime without a full deploy. In Linux
desktop sessions this also attempts a best-effort refresh of the user AT-SPI
accessibility bus before sky-cua reconnects:

```bash
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime
```

For OpenClaw, reload after config changes:

```bash
openclaw mcp reload
```

## Commit and push

After the deploy succeeds, commit all relevant staged and unstaged changes with a semantic message and push. Use the `committer` subagent for this step - it writes diff-grounded commit messages and handles staging precisely.

Follow the commit, branch, and pre-push conventions in the repo root
[`AGENTS.md`](../../../AGENTS.md).

## Decision tree

1. **Local dev iteration / unreleased changes?** -> local deploy lane -> commit + push.
2. **Shipping to a clean machine?** -> build + install a release package (`package.py`, then `install.py` on the target) -> commit + push.
3. **Only need to restart the runtime?** -> standalone MCP restart, no deploy.

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
| `deploy_plugin.py` | `--no-build`, `--symlink`, `--kwin-effect`, `--local-install-host` |
| `package.py` | `--no-build`, `--platform`, `--version-from-tag [TAG]`, `--release-dir` |
| `install.py` | `--agents`, `--mode {auto,repo,bundle}`, `--bundle-root`, `--target-dir`, `--kwin-effect`, `--skip-system-deps`, `--dry-run` |
| `install_mcp_server.py` | `--host`, `--restart-runtime`, `--kwin-effect` |
