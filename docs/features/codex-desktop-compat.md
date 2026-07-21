# Codex Desktop compatibility

## Status

Shipped on Linux. Last verified: 2026-05-14 with the official Codex
Chrome extension reaching the sky-cua native host through a temporary
Brave native-host manifest, plus the patched Codex Desktop Settings
proof from 2026-05-13.

## Summary

`sky-cua` presents to Codex Desktop as the bundled `computer-use`
plugin while keeping `sky-cua-client mcp` as the actual runtime
boundary. Browser Use and Chrome remain companion OpenAI-bundled
plugins; the build and preflight steps stage them beside the
`computer-use` compatibility entry, write Chromium-family native-host
manifests, and embed the Linux native messaging host so the official
Codex Chrome extension can reach `sky-cua` directly.

Codex Desktop itself is patched separately (in `CodexDesktop-Rebuild`)
to widen Linux `computerUse` feature availability and to keep Browser
Use and Chrome companion descriptors visible when `computerUse` is
enabled. Other MCP hosts do not need any of the bundled-plugin cache
steps.

## Contract surface

The portable runtime entrypoint remains:

```bash
./bin/sky-cua-client mcp
```

Codex-specific compatibility files:

- `.codex-plugin/plugin.json` — plugin manifest.
- `.mcp.json` — single cross-platform MCP server config; names the
  server `computer-use` and uses absolute paths plus the desktop env
  allowlist.
- `bin/sky-cua-browser-preflight` — plugin-local browser preflight
  wrapper.

Bundled-marketplace cache layout (output of preflight, not committed
state):

```
~/.codex/plugins/openai-bundled/marketplace.json   # adjusted in place
~/.codex/plugins/openai-bundled/plugins/chrome/...
~/.codex/plugins/openai-bundled/plugins/browser-use/...
~/.codex/plugins/openai-bundled/plugins/computer-use/...   # disabled
```

`config.toml` post-deploy shape:

```toml
[plugins."chrome@openai-bundled"]
enabled = true

[plugins."browser-use@openai-bundled"]
enabled = true

[plugins."computer-use@openai-bundled"]
enabled = true    # the single enabled computer-use plugin id

[plugins."sky-cua@local"]
enabled = false   # payload carrier; the compat root points at its cache payload
```

Codex Desktop detects Computer Use plugins by the built-in plugin name
`computer-use`, so the `computer-use@openai-bundled` compat entry is the
single enabled computer-use plugin; the `sky-cua@local` channel id stays
disabled so Codex never sees two active `computer-use` MCP servers.

## Behavior

Build-time (`scripts/build_plugin.py`):

- Stages OpenAI-bundled `chrome` and `browser-use` resources from
  `SKY_CUA_UPSTREAM_CODEX_RESOURCES`,
  `SKY_CUA_OPENAI_BUNDLED_RESOURCE_ROOT`, or the sibling
  `codex-desktop-linux` resource path.
- Removes macOS sidecars from copied bundles.
- Ensures marketplace entries for `chrome`, `browser-use`, and
  `computer-use` (the last as a disabled compatibility entry).
- Embeds `sky-cua-chrome-host` under
  `resources/plugins/openai-bundled/plugins/chrome/extension-host/linux/<arch>/extension-host`.

Install-time (`scripts/install_plugin.py`, `scripts/deploy_plugin.py`,
`scripts/installer.py`):

- Installs the bundle and runs browser preflight on Linux when the
  built bundle contains `resources/chrome_preflight.py`.
- The compat root's `.mcp.json` is retargeted at the installed
  `sky-cua@local` payload, which stays disabled so Codex never sees
  duplicate computer-use servers.
- Deploys preserve already-staged binaries for other platforms:
  rebuilding on Linux does not delete Windows `.exe` binaries from
  the local payload and vice versa.

Preflight (`resources/chrome_preflight.py`,
`bin/sky-cua-browser-preflight`):

- Syncs `chrome`, `browser-use`, and the `computer-use` compatibility
  plugin into the local Codex bundled marketplace cache.
- Writes exact native-host manifests for Google Chrome, Brave, and
  Chromium under `com.openai.codexextension`.
- Enables `chrome@openai-bundled`, `browser-use@openai-bundled`, and the
  `computer-use@openai-bundled` compat plugin (compat-first enablement).

The Linux native messaging host (`crates/sky-cua-chrome-host`) bridges
the upstream ChatGPT Chrome extension and the sky-cua runtime. It handles
`codexRuntime/hello`, `codexRuntime/ensure`, and `codexRuntime/restart`, selects
a compatible local Codex app-server entry, and owns the loopback WebSocket
proxy that validates the extension origin. The legacy `ensureCodexAppServer`
response remains compatible. The host also round-trips `getInfo` and `getTabs`,
forwards extension heartbeat `ping` messages, and emits `turnEnded` from a
task-complete session log so the extension's session lifecycle is honored.

## Source paths

- `scripts/build_plugin.py` — release bundle builder (stages
  bundled-plugin cache, embeds Chrome host).
- `scripts/install_plugin.py` — local install path.
- `scripts/deploy_plugin.py` - fast local deploy (`sky-cua@local`,
  compat root retargeted at it).
- `scripts/build_complete_release.py` - builds the immutable complete release
  and fat archive under `dist/complete-release/`.
- `scripts/complete_release_cli.py` / generated release-root `install.py` -
  complete machine activation, idempotent ensure, and activation proof.
- `scripts/installer.py`, repository-root `install.py`, and
  `scripts/package.py` - checkout/legacy compatibility workflows, not complete
  release activation.
- `resources/chrome_preflight.py` - preflight that syncs bundled
  cache, writes native-host manifests, enables companion plugins.
- `crates/sky-cua-chrome-host/` — Linux native messaging host.
- `resources/chrome-extension/codex/1.2.27203.26575_0/` — extracted upstream
  ChatGPT Chrome extension fallback payload. Local extraction keeps upstream
  source maps and Chrome `_metadata`; plugin staging excludes both because they
  are not runtime assets.
- `bin/sky-cua-browser-preflight` — preflight wrapper.
- `.codex-plugin/plugin.json`, `.mcp.json` — plugin manifest and MCP
  server config.

## Verification

Focused tests:

```bash
uv run ruff format --check resources/chrome_preflight.py scripts/test_plugin_bundle.py
uv run ruff check resources/chrome_preflight.py scripts/test_plugin_bundle.py
uv run basedpyright resources/chrome_preflight.py scripts/test_plugin_bundle.py
uv run pytest scripts/test_plugin_bundle.py -k 'browser_preflight or update_codex_config'
cargo nextest run -p sky-cua-chrome-host
```

Live host smoke:

```bash
python3 scripts/live_chrome_host_client_smoke.py --browser brave --install-temp-native-manifest --host-path target/debug/sky-cua-chrome-host
```

Latest accepted artifacts:

- Live Chrome native-host bridge: `artifacts/chrome-host-smoke/20260514T154125Z/result.json`
  proves the official Codex extension can reach the sky-cua native host
  binary through a temporary Brave native-host manifest, bridge
  client-to-extension `getInfo` and `getTabs`, bridge extension-to-client
  heartbeat `ping`, observe a session/turn request, emit `turnEnded`
  from a task-complete session log, receive the official extension's
  `turnEnded` response, and restore the original Brave manifest.
- Patched Codex Desktop Settings proof:
  `artifacts/desktop-ui-proof/20260513T100515Z-browser-settings-patched-profile/`
  shows `Computer Use` with Google Chrome management and `Browser Use`
  with the Chrome plugin rather than pruning the browser companions.

## Known limitations

- **Codex Desktop side patches live in a separate repository**
  (`/home/bex/projects/sky/CodexDesktop-Rebuild`) and are not part of
  the `sky-cua` source tree. The patches are intentionally small
  (widen Linux `computerUse` availability, strip external Browser Use
  build-flavor gates, keep Browser Use / Chrome companion descriptors
  available when `computerUse` is enabled).
- **Codex Desktop still presents the upstream Browser Use plugin as its
  browser surface.** The full first-class `browser_*` MCP action surface is
  shipped, but Codex Desktop's adoption of it goes through compat
  materialization owned by the codex-desktop repo: that repo generates
  plugin cache roots under the OpenAI built-in IDs that point at the
  packaged sky-cua implementation. The sky-cua side of that contract is
  documented in
  [`docs/runtime/compat-plugin-contract.md`](../runtime/compat-plugin-contract.md).
- **No standalone Chrome extension smoke fixture.** The current
  fallback is the extracted upstream ChatGPT extension under
  `resources/chrome-extension/codex/1.2.27203.26575_0/`. A
  `scripts/install_chrome_extension.py` automated temporary-profile
  loader does not exist yet.
- **Codex Desktop UI proof is release-bound.** Release
  `a1b86ed8c07f88e09e3607065f26ee7583c1faa394f3d809b173a095f7a2891d`
  was installed through the normal Codex Desktop installer on 2026-07-21.
  Fresh task `019f8499-1bcd-7f13-8966-a392574a81d5` selected
  `host_provided_iab`, navigated, captured a screenshot, emitted it back to
  Codex for inspection, and returned the page marker without generation or
  module-path environment overrides. Repeat this proof after an upstream
  Desktop refresh changes the plugin descriptor, Settings, or Browser runtime.

## Related

- Runtime contract: [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
  describes the host-portable MCP entrypoint and the desktop env
  allowlist.
- Companion feature: [`docs/features/session-env-repair.md`](session-env-repair.md)
- ROADMAP entries: [`ROADMAP.md`](../../ROADMAP.md) under "Host
  portability".
- Originating ExecPlan (retired into this feature doc; see git history for `plans/1778463694899-nimble-knight.md`).
