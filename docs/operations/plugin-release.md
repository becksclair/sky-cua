# Plugin deploy and standalone release runbook

Use this runbook for Codex plugin lifecycle work. There are two producer lanes
and no marketplace publish step:

- `python3 scripts/deploy_plugin.py` — fast local development deploy.
- `python3 install.py build` and `python3 install.py install` — standalone
  distribution and fixed-root machine install.

## Core invariant

Keep exactly one active `computer-use` MCP server. Local development retains
the compat-first `computer-use@openai-bundled` projection with
`sky-cua@local` as its disabled payload carrier. Standalone distribution ships
native `computer-use@openai-bundled` and `browser@openai-bundled` plugins and
lets consumer repositories own their final enablement and convergence. The
Browser plugin source is the fixed-root
`codex/openai-bundled/plugins/browser/` directory; do not add or project a
`browser-use` plugin alias.

Keep one enabled copy of each bundled sky-cua skill. The standalone installer
projects `computer-use`, `browser-use`, and `phone-use` from the fixed installed
payload rather than from a versioned generation.

## Local iteration

When changing runtime code, scripts, guidance, skills, or plugin metadata, use:

```bash
python3 scripts/deploy_plugin.py
```

This builds `dist/plugin/sky-cua`, installs it as `sky-cua@local`, retargets the
local `computer-use@openai-bundled` compat plugin, and refreshes the installed
MCP runtime. It is a development deploy, not the distributable install path.

Use `--no-build` only when `dist/plugin/sky-cua` is already current:

```bash
python3 scripts/deploy_plugin.py --no-build
```

## Build the standalone artifact

From the repository root:

```bash
python3 install.py build
```

The builder owns and refreshes these durable outputs:

```text
dist/plugin/sky-cua
out/components/cua-node-linux-x64-glibc
dist/standalone/sky-cua-linux-x64-glibc
dist/sky-cua-linux-x64-glibc.tar.gz
```

Do not redirect normal builds into a temporary directory. The stable checkout
paths preserve precompiled artifacts and make repeat builds incremental. The
standalone tree and archive carry exactly one Chrome extension: the latest
manifest version from `resources/chrome-extension/codex/`.

Inspect the archive directly:

```bash
tar tzf dist/sky-cua-linux-x64-glibc.tar.gz
```

It must expand under `sky-cua-linux-x64-glibc/` and include `RELEASE.json`,
`install.py`, `bin/`, `browser/`, `codex/`, `skills/`, and `docs/`. It must not
contain release generations, a `current` selector, activation receipts, or a
rollback controller.

## Install on a clean machine

Copy and extract the archive, then run the one install command:

```bash
tar xzf sky-cua-linux-x64-glibc.tar.gz
cd sky-cua-linux-x64-glibc
python3 install.py install
```

The destination is fixed:

```text
${XDG_DATA_HOME:-~/.local/share}/sky-cua
```

There is no generation id, manifest hash, rollback command, or selectable
install-root argument. Installation replaces the root, projects stable
launchers and native-host manifests, links packaged skills, and configures
detected Codex/OpenClaw integration. Repeating the command converges to the
same layout and removes stale files from the previous payload.

## Producer-safe validation

Never run producer tests against the live user install. Set disposable user
directories and omit consumer CLIs from `PATH` unless a fake integration is the
specific test fixture:

```bash
test_home="$(mktemp -d)"
HOME="$test_home" XDG_DATA_HOME="$test_home/data" python3 install.py install
```

For an extracted artifact, use the packaged `install.py` from its extraction
root with the same isolated environment. Validate:

- install root equals `$XDG_DATA_HOME/sky-cua`;
- `RELEASE.json` resolves `paths.codex_marketplace` and
  `paths.browser_client` to files under that root;
- the marketplace entries are exactly `computer-use` and `browser`, and
  `plugins/browser/scripts/browser-client.mjs` exports the shared
  `setupBrowserRuntime` adapter;
- all launcher symlinks resolve under that root;
- native-host manifests target the stable launcher;
- the payload has one `browser/extension/manifest.json` tree;
- reinstall removes an injected old-only marker;
- no consumer configuration contains a Browser trust hash.

Remove the disposable home after inspection. Do not install, deploy, publish,
or modify a consumer repository as part of producer validation.

## Local Codex config reset

If local-development `~/.codex/config.toml` state is stale, clean only the
sky-cua plugin/compat entries and preserve unrelated trust, auth, model, and
curated plugin settings. Then rerun:

```bash
python3 scripts/deploy_plugin.py
codex mcp list --json
```

The cheap proof is one `computer-use` server. For an exact tool contract:

```bash
python3 scripts/probe_mcp_tool_surface.py --installed
```

## Verification

For standalone producer changes:

```bash
uv run ruff format --check scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest scripts/test_standalone_release.py
python3 install.py build
tar tzf dist/sky-cua-linux-x64-glibc.tar.gz
```

Run broader Python, Rust, and CUA Node suites when the corresponding runtime or
shared contract changed. Consumer live acceptance belongs in the Codex Desktop
and OpenClaw repositories after the producer artifact is proven.

## Related

- [`docs/features/release-package.md`](../features/release-package.md)
- [`docs/features/one-shot-installer.md`](../features/one-shot-installer.md)
- [`docs/features/codex-desktop-compat.md`](../features/codex-desktop-compat.md)
