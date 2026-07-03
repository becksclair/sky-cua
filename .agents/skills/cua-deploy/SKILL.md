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

Automates the sky-cua change-to-ship pipeline: build -> deploy/package -> sync agent skills -> commit -> push.
Determine the appropriate lane from context and task scope; never run more pipeline than was asked.

There is no marketplace and no publish flow. The two distribution lanes are:

- Local deploy (`scripts/deploy_plugin.py`): updates *what runs locally, immediately* - installs
  the built bundle as `sky-cua@local`, retargets the computer-use compat plugin at it, and
  refreshes the installed MCP runtime (no separate restart step). Does not touch git.
- Build + install a release package (`scripts/package.py`, then `python3 install.py` on the
  target): builds a self-contained tarball under `dist/release/`, which a clean machine extracts
  and installs in bundle mode (no build, no cargo). Materializes the compat plugin from the
  bundled preflight.

## Agent skills sync

For any deploy or publish lane, also link the repo-local sky-cua skills into
the global agent skill root:
`~/.agents/skills/{computer-use,browser-use,phone-use}` -> `skills/*`. This
keeps opencode/oracle/worker-style agents on the current repo skill text,
including `phone-use`. The links point at the checkout the sync ran from — a
deploy from a worktree leaves them at the worktree path, so rerun from the
main checkout to repoint them.

Run after the deploy/publish command succeeds:

```bash
python3 scripts/sync_agent_skills.py
```

Only the sky-cua-owned skill links above are replaced; unrelated skills in
`~/.agents/skills` are never touched. (The former OpenClaw workspace-skills
copy into `~/.openclaw/workspace/skills` was retired 2026-07-03 — OpenClaw no
longer reads bundled copies from there.)

## Lanes

### Local deploy (default)

Use when iterating on unreleased changes locally. Installs as `sky-cua@local` and refreshes the
MCP runtime in one command; in Linux desktop sessions this also attempts a
best-effort refresh of the user AT-SPI accessibility bus before sky-cua
reconnects so wedged semantic trees do not survive deploys. Does not touch the
marketplace.

A build-bearing deploy also rebuilds and stages the Android phone-companion APK
automatically (toolchain-gated, change-detected — see "Android phone companion"
below), so the bundled companion stays current without a manual Gradle step.

```bash
# Build + deploy locally (also refreshes the installed MCP runtime, and rebuilds
# the phone-companion APK when its sources changed and the Android toolchain is present)
python3 scripts/deploy_plugin.py
python3 scripts/sync_agent_skills.py

# Also rebuild and reload the KWin agent-cursor effect (Linux/KDE only)
python3 scripts/deploy_plugin.py --kwin-effect
python3 scripts/sync_agent_skills.py

# Force a companion rebuild, or skip it entirely
python3 scripts/deploy_plugin.py --force-companion
python3 scripts/deploy_plugin.py --no-companion

# Install existing bundle without rebuilding (skips the companion lane too)
python3 scripts/deploy_plugin.py --no-build
python3 scripts/sync_agent_skills.py
```

### Android phone companion

A build-bearing `deploy_plugin.py` runs the companion build/stage lane before
bundling. It rebuilds the APK with Gradle only when the companion sources
changed since the last staged APK (so a pure-Rust deploy is not slowed), stages
it to `resources/android/phone-companion.{apk,json}`, and lets `build_plugin.py`
bundle it. The lane is graceful: on a host without JDK 21 + the Android SDK it
logs a note and skips, reusing any previously staged APK (ADB-baseline phone-use
is unaffected). `--force-companion` forces a rebuild; `--no-companion` skips the
lane. Override the toolchain with `SKY_CUA_COMPANION_JAVA_HOME` /
`SKY_CUA_COMPANION_ANDROID_SDK_ROOT` when it is not auto-detected.

Installing the companion onto a device and enabling its required services is a
*runtime* concern, not a deploy-script one: `phone_install_companion` (and
`phone_connect` under auto-install) install the staged APK over ADB and enable
the accessibility + notification-listener services automatically. See
`docs/features/phone-use.md`.

To keep that handoff explicit rather than silent, a build-bearing deploy prints
a `[companion]` device-setup status: the staged version and the currently
connected adb devices. When you (the agent) see it after a deploy and a
companion is bundled, finish setup through the runtime tools — do not push the
APK from the shell (a raw `adb install` bypasses the service-enable logic, which
lives in the Rust tool path):

1. If devices are listed, ask the user which one(s) to set up (do not assume; do
   not auto-install on every connected device).
2. For each chosen device: `phone_connect(serial=…)` then
   `phone_install_companion`. That installs the staged APK and auto-enables the
   accessibility + notification-listener services; confirm with
   `phone_companion_status` (it reports installed-vs-expected version and the
   permission grants).

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
cargo fmt --check && cargo nextest run

# Python
uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest
```

State any live-smoke gates not run (desktop/portal/KDE/COSMIC/Hyprland/GNOME).

## Deploy before live tests

Live tests must run against current binaries, not a stale build. Always deploy
before a live smoke or an agent-driven device test. The harnesses enforce this
at the shared launch choke points: `deploy_freshness.py` fingerprints the Rust
runtime source, every build/deploy stamps the client binary it produced, and
every sky-cua MCP spawn (`_mcp_stdio.McpClient`) and agent launch
(`_agent_mcp_smoke.run_agent`) aborts with a redeploy hint when the binary it —
or the agent it drives — would use was not built from the current source. This
covers all `live_*_smoke.py` harnesses automatically.

```bash
# Standalone preflight (exits nonzero when the deployed runtime is stale):
python3 scripts/deploy_freshness.py            # checks the locally-deployed client
python3 scripts/deploy_freshness.py --client bin/sky-cua-client  # a specific binary

# Refresh, then test:
python3 scripts/deploy_plugin.py
```

Set `SKY_CUA_ALLOW_STALE_DEPLOY=1` only to intentionally bypass the gate.

## Common flags summary

| Script | Key flags |
|---|---|
| `deploy_plugin.py` | `--no-build`, `--symlink`, `--kwin-effect`, `--no-companion`, `--force-companion`, `--local-install-host` |
| `package.py` | `--no-build`, `--platform`, `--version-from-tag [TAG]`, `--release-dir` |
| `install.py` | `--agents`, `--mode {auto,repo,bundle}`, `--bundle-root`, `--target-dir`, `--kwin-effect`, `--skip-system-deps`, `--dry-run` |
| `install_mcp_server.py` | `--host`, `--restart-runtime`, `--kwin-effect` |
