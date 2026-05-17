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
enabled = false   # disabled to avoid duplicate computer-use server

[plugins."sky-cua@Heliasar"]
enabled = true

[marketplaces.Heliasar]
source = "/home/bex/projects/heliasar-marketplace"
source_type = "local"
```

The `computer-use@openai-bundled` compatibility entry is staged but
disabled so Codex does not see two active `computer-use` MCP servers.

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

Install-time (`scripts/install_plugin.py`,
`scripts/deploy_release_plugin.py`):

- Installs the bundle and runs browser preflight on Linux when the
  built bundle contains `resources/chrome_preflight.py`.
- The deploy scripts switch `sky-cua@debug` and `sky-cua@Heliasar`
  mutually so Codex never sees duplicate computer-use servers.
- Deploys preserve already-staged binaries for other platforms:
  rebuilding on Linux does not delete Windows `.exe` binaries from
  the local marketplace and vice versa.

Preflight (`resources/chrome_preflight.py`,
`bin/sky-cua-browser-preflight`):

- Syncs `chrome`, `browser-use`, and the `computer-use` compatibility
  plugin into the local Codex bundled marketplace cache.
- Writes exact native-host manifests for Google Chrome, Brave, and
  Chromium under `com.openai.codexextension`.
- Enables `chrome@openai-bundled` and `browser-use@openai-bundled`;
  stages `computer-use@openai-bundled` disabled.

The Linux native messaging host (`crates/sky-cua-chrome-host`) bridges
the official Codex Chrome extension and the sky-cua runtime: it
round-trips `getInfo` and `getTabs`, forwards extension heartbeat
`ping` messages, and emits `turnEnded` from a task-complete session
log so the extension's session lifecycle is honored.

## Source paths

- `scripts/build_plugin.py` — release bundle builder (stages
  bundled-plugin cache, embeds Chrome host).
- `scripts/install_plugin.py` — local install path.
- `scripts/deploy_debug_plugin.py`, `scripts/deploy_release_plugin.py`
  — debug-cache and Heliasar-marketplace deploys.
- `scripts/publish_marketplace_release.py` — pushes the marketplace
  checkout before upgrading the Codex Git marketplace source.
- `resources/chrome_preflight.py` — preflight that syncs bundled
  cache, writes native-host manifests, enables companion plugins.
- `crates/sky-cua-chrome-host/` — Linux native messaging host.
- `resources/chrome-extension/codex/1.1.4_0/` — extracted upstream
  Codex Chrome extension fallback payload.
- `bin/sky-cua-browser-preflight` — preflight wrapper.
- `.codex-plugin/plugin.json`, `.mcp.json` — plugin manifest and MCP
  server config.

## Verification

Focused tests:

```bash
uv run ruff format --check resources/chrome_preflight.py scripts/test_python_harness_helpers.py
uv run ruff check resources/chrome_preflight.py scripts/test_python_harness_helpers.py
uv run basedpyright resources/chrome_preflight.py scripts/test_python_harness_helpers.py
uv run pytest scripts/test_python_harness_helpers.py -k 'browser_preflight or update_codex_config'
cargo test -p sky-cua-chrome-host
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
- **First-class `browser_*` MCP tools are intentionally deferred.**
  Browser automation goes through the companion Browser Use plugin
  rather than `sky-cua`'s MCP surface. The Chrome host is a protocol
  bridge, not a command-specific browser MCP implementation.
- **No standalone Chrome extension smoke fixture.** The current
  fallback is the extracted upstream Codex extension under
  `resources/chrome-extension/codex/1.1.4_0/`. A
  `scripts/install_chrome_extension.py` automated temporary-profile
  loader does not exist yet.
- **Codex Desktop UI proof requires a fresh re-run after upstream
  Desktop refreshes** that change plugin descriptor or Settings
  behavior.

## Related

- Runtime contract: [`docs/runtime/mcp-boundary.md`](../runtime/mcp-boundary.md)
  describes the host-portable MCP entrypoint and the desktop env
  allowlist.
- Companion feature: [`docs/features/session-env-repair.md`](session-env-repair.md)
- ROADMAP entries: [`ROADMAP.md`](../../ROADMAP.md) under "Host
  portability".
- Originating ExecPlan (retired into this feature doc; see git history for `plans/1778463694899-nimble-knight.md`).
