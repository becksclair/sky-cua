# Plugin deploy and standalone release runbook

Use this runbook for Codex plugin lifecycle work. There are two producer lanes
and one release pipeline; there is no marketplace publish step:

- `python3 scripts/deploy_plugin.py` — legacy fast deploy for older packaged
  consumer resources; current native ChatGPT installations use the fixed-root
  install command below.
- `python3 install.py build` and `python3 install.py install` — standalone
  distribution and fixed-root machine install.
- `python3 install.py release` — guarded standalone version commit and tag push.
- `scripts/release_pipeline.py validate|smoke-install` — check a built archive's
  safety and identity, then install it into an isolated temporary home.

## Core invariant

Keep exactly one active `computer-use` MCP server. Local development retains
the compat-first `computer-use@openai-bundled` projection with
`sky-cua@local` as its disabled payload carrier. Standalone distribution ships
native `computer-use@openai-bundled` and `browser@openai-bundled` plugins and
enables the selected surfaces in detected hosts. The
Browser plugin source is the fixed-root
`codex/openai-bundled/plugins/browser/` directory; do not add or project a
`browser-use` plugin alias. When the native ChatGPT Browser is present, its
unchanged package takes precedence over that fallback source.

Keep one enabled copy of each bundled sky-cua skill. The standalone installer
projects `computer-use`, `browser-use`, and `phone-use` from the fixed installed
payload rather than from a versioned generation. ChatGPT loads the three skills
from the installed `computer-use` plugin; duplicate shared skill entries are
disabled. The installer uses the native Codex `plugin/install` API to refresh the
cache, including same-version skill changes. Other hosts receive relative links
to the fixed payload; old installer-owned copied skills and generation routers
are migrated without replacing unowned skills.

With native Browser enabled, Codex also receives the focused
`sky-browser-enhancements` skill under its own skill root. It loads the helper
from the fixed install tree and decorates an existing native tab explicitly.
It does not replace native Browser instructions, fork the reserved plugin
version, or rewrite the app's trusted-service mappings. Native startup
reconciliation can therefore retain its packaged baseline.

## Local iteration

When changing runtime code, scripts, guidance, skills, or plugin metadata, use:

```bash
python3 install.py install
```

This incrementally refreshes the durable build outputs and installs the fixed
tree, host configuration and skills. The native `plugin/install` API owns
Browser cache materialization. Native Browser keeps its original identity;
Sky's helper and supplemental skill remain outside the app-owned cache.

The development fallback includes its own assembled runtime when the native
Browser is absent. Its cache identity covers that runtime and the generated
adapter/configuration. Standalone production selects
`--browser-runtime-mode standalone` internally, and carries one runtime at the
fixed root.

Codex registration stages its source under
`CODEX_HOME/.tmp/bundled-marketplaces/openai-bundled` before `plugin/install`.
The official 0.149 client and the current native client both accept this layout;
there is no version gate or silent filesystem fallback. A registration failure
reports its phase and completed/unfinished plugins while preserving the error.

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
detected host integration. ChatGPT gets the bundled plugins, Claude Code and
existing Claude Desktop configuration get `sky-cua` and `node_repl`, and OpenCode
gets native `mcp.servers` entries. Existing host settings and permissions are
preserved. OpenCode uses phase-specific startup/catalog/execution timeouts;
its tool catalog becomes available asynchronously after startup. Repeating the
command converges to the
same layout and removes stale files from the previous payload.

Existing host processes may retain their MCP catalog. Validate in a fresh host
session after installation. Claude Desktop Chat can cache its configuration; on
builds that expose it, `Developer > Reload MCP Configuration` forces a reread.
If that menu is unavailable, its next normal app restart is the reload boundary.
The embedded Claude Code engine reads user-scope MCP settings independently.

Every state-changing desktop/browser call must carry the current observation's
`appshot_id`; a `snapshot_id` alone is insufficient. An action rejection with a
recovery AppShot requires inspecting that state before retrying. Browser open or
navigation can itself return the AppShot for the next action.

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

## Cut a release

From a clean `main` checkout synchronized with its configured upstream, run:

```bash
python3 install.py release                 # default: next minor
python3 install.py release --patch
python3 install.py release --minor
python3 install.py release --major
python3 install.py release --version 2.3.4
```

The command accepts stable `X.Y.Z` versions only and requires the explicit
version to increase. It updates only `scripts/standalone_release.py`, runs
`just verify`, creates a scoped version commit and annotated
`standalone-vX.Y.Z` tag, then atomically pushes exactly `main` and that tag. It
requires exactly one configured push destination and does not publish or deploy
locally. Do not use it with unrelated staged, unstaged, or untracked files.

The pushed tag no longer triggers anything. Release publication and the Saga
deployment ran as Gitea workflows, and Gitea is retired. Build and check the
archive locally:

```bash
python3 install.py build
python3 scripts/release_pipeline.py validate --archive dist/sky-cua-linux-x64-glibc.tar.gz --tag standalone-vX.Y.Z
python3 scripts/release_pipeline.py smoke-install --archive dist/sky-cua-linux-x64-glibc.tar.gz --tag standalone-vX.Y.Z
```

Saga's `/opt/homelab/scripts/deploy-sky-cua` hook downloaded a published
release, so automated Saga deployment has no working path until a new
publication target replaces Gitea.

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
uv run pytest scripts/test_standalone_release.py scripts/test_standalone_release_command.py
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
