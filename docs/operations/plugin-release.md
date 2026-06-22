# Plugin Deploy and Release Runbook

Use this runbook for Codex plugin lifecycle work, not ordinary desktop or
browser automation. There are three distribution entrypoints and no
marketplace:

- `scripts/deploy_plugin.py` - fast local dev deploy.
- `scripts/package.py` - build a self-contained release tarball.
- `install.py` (delegating to `scripts/installer.py`) - install on a clean
  machine.

## Core Invariant

Keep exactly one active `computer-use` MCP server - provided by the compat
plugin id, not by a sky-cua channel id.

Codex Desktop detects Computer Use plugins by the built-in plugin name
`computer-use`, so `computer-use@openai-bundled` (the compat plugin root the
chrome preflight materializes under
`~/.codex/plugins/cache/openai-bundled/computer-use/`) is the single enabled
computer-use plugin on Linux. The single sky-cua channel id, `sky-cua@local`,
stays installed but disabled; it is a payload carrier, not the active plugin
id. The compat root's `.mcp.json` points at the `sky-cua@local` payload, so a
local deploy updates what runs by rematerializing the compat root.

Fallback: when a bundle ships without the openai-bundled resources (no compat
root can be materialized - minimal test bundles, non-Linux), the deploy falls
back to enabling `sky-cua@local` directly.

## Local Iteration

When changing Rust code, scripts, app guidance, skills, plugin metadata, or
local bundle contents, deploy the local plugin:

```bash
python3 scripts/deploy_plugin.py
```

This builds `dist/plugin/sky-cua`, installs the bundle as `sky-cua@local` into
`~/.codex/plugins/cache/local/sky-cua/local`, retargets the
`computer-use@openai-bundled` compat plugin at it via the bundled
`resources/chrome_preflight.py`, and refreshes the installed MCP runtime. It
touches no git history, no marketplace, and no Codex `plugin/install`.

Use `--no-build` only when `dist/plugin/sky-cua` is already current:

```bash
python3 scripts/deploy_plugin.py --no-build
```

Verify the active MCP server when needed:

```bash
codex mcp list --json
```

The `computer-use` server should point at the local cache path
(`~/.codex/plugins/cache/local/sky-cua/local`). The server is provided by
`computer-use@openai-bundled`; the command path shows which payload it runs.

## Build a Release Package

To ship sky-cua to a machine without a checkout or toolchain, build a
self-contained release tarball:

```bash
python3 scripts/package.py
```

This builds `dist/plugin/sky-cua` and assembles
`dist/release/sky-cua-<version>-<platform>.tar.gz`. The tarball contains:

- `plugin/sky-cua/` - the full plugin bundle (runtime binaries, resources,
  skills, plugin manifests).
- `scripts/` - the pure-Python installer subset (no cargo, no
  `build_plugin.py`).
- `skills/` - mirrored skills so the installer resolves them as in the repo.
- `install.py`, `VERSION` - the package-root installer and version stamp.

Useful options:

- `--no-build`: package the existing `dist/plugin/sky-cua` bundle as-is.
- `--platform`: target platform id (default: current host platform). Packaging
  fails loudly if the bundle lacks that platform's runtime binaries.
- `--version-from-tag [TAG]`: set the bundle version from a `vX.Y.Z` git tag
  before packaging. When `TAG` is omitted, use the current CI/git tag.
- `--release-dir`: output directory for the tarball (default `dist/release`).

## Install on a Clean Machine

Copy the tarball to the target machine, extract it, and run the package-root
installer:

```bash
tar xzf sky-cua-<version>-<platform>.tar.gz
cd sky-cua-<version>
python3 install.py
```

The package-root `install.py` pins bundle mode and the package's own payload
path, then delegates to `scripts/installer.py`. It installs the prebuilt
bundle with no build step and no cargo, materializes the
`computer-use@openai-bundled` compat plugin from the bundled preflight, and
registers the MCP server plus skills for every detected agent.

The same installer runs from a checkout with `python3 install.py` at the repo
root, where it builds the runtime from source first. See
[`docs/features/one-shot-installer.md`](../features/one-shot-installer.md) for
the mode and agent-detection details and
[`docs/features/release-package.md`](../features/release-package.md) for the
package layout and clean-machine flow.

## Config Reset

If `~/.codex/config.toml` has stale sky-cua state, clean only the plugin and
compatibility entries before redeploying. Keep unrelated project trust, auth,
model, and curated plugin settings intact. The compat-first post-deploy shape
on Linux is:

```toml
[plugins."computer-use@openai-bundled"]
enabled = true

[plugins."sky-cua@local"]
enabled = false
```

After cleanup, rerun `python3 scripts/deploy_plugin.py`. The cheap
control-plane proof is `codex mcp list --json` or `mcpServerStatus/list`
through `codex app-server`; it should show one `computer-use` server with tools
such as `doctor`, `status`, `list_resources`, `observe`, `capture_desktop`,
`desktop_pointer`, `desktop_keyboard`, `browser_input`, and `phone_connection`.
For an exact contract check, run:

```bash
python3 scripts/probe_mcp_tool_surface.py --installed
```

## Verification

Use the narrowest check that matches the work:

```bash
uv run ruff format --check scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
python3 scripts/build_plugin.py
codex mcp list --json
python3 scripts/live_agentic_loop_smoke.py
```

For pure script or metadata edits, the Python gates and `build_plugin.py` are
usually enough. For install or runtime registration changes, include
`codex mcp list --json`; for behavior acceptance, run `live_agentic_loop_smoke.py`.

For local deploy changes, prove the deploy lane itself:

```bash
python3 scripts/deploy_plugin.py --no-build
codex mcp list --json
```

The `computer-use` server command should point at
`~/.codex/plugins/cache/local/sky-cua/local`.

For release-package changes, prove the package lane and the clean-machine
installer path:

```bash
python3 scripts/package.py --no-build
docker/validate/run.sh --tarball dist/release/sky-cua-<version>-<platform>.tar.gz
```

The Docker validation is headless: it gates install/config/binary execution and
attempts the MCP handshake, but a skipped handshake is expected without a
desktop session. It does not mount host auth/config by default; pass
`--with-host-auth` only for trusted tarballs that need live host credentials.
Use the GUI VM smokes for hard tool-list and desktop-control proof.
