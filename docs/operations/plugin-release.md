# Plugin Deploy and Release Runbook

Use this runbook for Codex plugin lifecycle work, not ordinary desktop or
browser automation. There are two normal lanes and no marketplace:

- `scripts/deploy_plugin.py` - compatibility-only local development deploy.
- `scripts/build_complete_release.py` plus the generated release-root
  `install.py install` - immutable distribution and complete activation.

The repository-root `install.py` and `scripts/package.py` are retained for
checkout bootstrap and legacy bundle compatibility. They are not complete
release activation entrypoints.

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

Keep exactly one enabled copy of each bundled sky-cua skill in Codex as well.
The canonical shared links under
`~/.agents/skills/{computer-use,browser-use,phone-use}` remain available to
other agents, while `update_codex_config` writes path-scoped
`[[skills.config]]` rules that disable those shared copies inside Codex. Codex
canonicalizes the configured paths, so the rules follow the symlinks when
`scripts/sync_agent_skills.py` repoints them to another checkout. The active
Codex copies are the plugin-namespaced skills from the enabled compat or local
plugin.

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

## Build a Complete Release

To ship sky-cua to a machine without a checkout or toolchain, build a
self-contained immutable release:

```bash
python3 scripts/build_complete_release.py
```

The command prints the exact `release_id`, `manifest_sha256`, `release_root`,
and `fat_archive`. Inspect those selected artifacts rather than choosing the
newest directory by name. The fat archive is
`dist/complete-release/sky-cua-<release-id>-linux-x64-glibc.tar.gz` and contains:

- `RELEASE.json` and `SHA256SUMS` - the content-addressed release contract.
- `components/` and `archives/` - the bound core, Browser, CUA Node, Codex,
  documentation, compliance, and installer payloads.
- `install.py` - the checkout-free complete activation controller.

Useful options:

- `--output-root`, `--core-source`, and `--cua-node-component` select inputs.
- `--producer-commit` binds provenance explicitly.
- `--no-fat-archive` builds only the unpacked release.

## Install on a Clean Machine

Copy the reported fat archive to the target machine, extract it, and use the
reported manifest hash:

```bash
tar xzf sky-cua-<release-id>-linux-x64-glibc.tar.gz
cd sky-cua-<release-id>
python3 install.py verify --manifest-sha256 <manifest-sha256>
python3 install.py install --manifest-sha256 <manifest-sha256>
python3 install.py verify-activation --manifest-sha256 <manifest-sha256>
```

The release-root `install` command owns the entire activation transaction:
generation promotion, exact native manifests, stable command links through
`current`, stale-process draining, receipt, and pruning. `ensure` is the
idempotent repair operation used by consumers. Never substitute raw
`scripts/release_generation.py install`; it is internal-only. The repository
root `install.py` and `scripts/package.py` remain compatibility/development
workflows, not the normal immutable release path. See
[`docs/features/release-package.md`](../features/release-package.md).

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

# BEGIN sky-cua managed shared-agent skill overrides
[[skills.config]]
path = "/home/<user>/.agents/skills/computer-use/SKILL.md"
enabled = false

[[skills.config]]
path = "/home/<user>/.agents/skills/browser-use/SKILL.md"
enabled = false

[[skills.config]]
path = "/home/<user>/.agents/skills/phone-use/SKILL.md"
enabled = false
# END sky-cua managed shared-agent skill overrides
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

For complete-release changes, prove the builder and checkout-free controller:

```bash
uv run pytest \
  scripts/test_build_complete_release.py \
  scripts/test_release_generation.py \
  scripts/test_install_complete_release.py \
  scripts/test_release_activation.py
python3 scripts/build_complete_release.py
# Then use the JSON-reported root and manifest:
python3 <release-root>/install.py verify --manifest-sha256 <manifest-sha256>
```

Use isolated temporary home/store roots for activation integration tests. Use
the GUI VM smokes for hard tool-list and desktop-control proof.
