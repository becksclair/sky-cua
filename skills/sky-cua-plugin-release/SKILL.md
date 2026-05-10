---
name: sky-cua-plugin-release
description: "Use when working on sky-cua's Codex plugin lifecycle: switching between the local debug plugin and the Heliasar marketplace release, publishing a new release bundle, setting up the Heliasar marketplace on a machine, or diagnosing duplicate computer-use MCP servers caused by debug and release plugin ids."
---

# Sky CUA Plugin Release

Use this skill for the repo-local plugin lifecycle, not for ordinary desktop automation.

## Core invariant

Keep the Heliasar marketplace registered, but enable only one `computer-use` plugin id at a time:

- Debug work: `sky-cua@debug` enabled, `sky-cua@Heliasar` disabled.
- Release/published work: `sky-cua@Heliasar` enabled, `sky-cua@debug` disabled.

Do not remove the Heliasar marketplace just to work on debug builds. Toggle plugin ids instead.

## Debug Iteration

When changing Rust code, scripts, app guidance, skills, plugin metadata, or local bundle contents, deploy the debug plugin:

```bash
python3 scripts/deploy_debug_plugin.py
```

Use `--no-build` only when `dist/plugin/sky-cua` is already current:

```bash
python3 scripts/deploy_debug_plugin.py --no-build
```

`deploy_debug_plugin.py` builds the bundle by default, installs it into the debug plugin cache, enables `sky-cua@debug`, and disables `sky-cua@Heliasar`.

Verify the active MCP server when needed:

```bash
codex mcp list --json
```

The `computer-use` server should point at a debug cache path, not the Heliasar cache.

## Publish Release

When the debug build is ready to publish through the personal marketplace, run:

```bash
python3 scripts/publish_marketplace_release.py
```

This is the release path. It builds `dist/plugin/sky-cua`, stages it into `~/projects/heliasar-marketplace/plugins/sky-cua`, writes the marketplace manifest, commits marketplace changes, pushes `origin main`, upgrades the Codex marketplace, installs/reloads `sky-cua@Heliasar`, and disables `sky-cua@debug`.

Use these options deliberately:

- `--no-build`: publish the existing `dist/plugin/sky-cua`.
- `--no-push`: commit locally but leave the marketplace push to the operator.
- `--skip-codex-install`: update the marketplace repo without changing the local Codex install.

Before publishing, the Heliasar checkout must already be an initialized Git repo with `HEAD`; first-time repo creation belongs to `gitea-repo`, not this script.

## First-Time Setup

On a machine that should consume Heliasar:

```bash
python3 scripts/setup_heliasar_marketplace.py
```

This clones or fast-forwards `~/projects/heliasar-marketplace`, runs the public Codex marketplace add/upgrade flow for `becksclair/heliasar-marketplace`, installs `sky-cua`, enables `sky-cua@Heliasar`, disables `sky-cua@debug`, and reloads MCP servers.

## Local Release Staging

`deploy_release_plugin.py` is a local release staging/install helper. It stages a release bundle into the Heliasar checkout and enables `sky-cua@Heliasar`, but it must not rewrite the configured Heliasar marketplace source from Git to local.

Use `publish_marketplace_release.py` for real publishing.

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

For pure script or metadata edits, the Python gates and `build_plugin.py` are usually enough. For install or runtime registration changes, include `codex mcp list --json`; for behavior acceptance, run `live_app_server_smoke.py`.
