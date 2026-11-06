# Compat plugin materialization contract

`sky-cua` owns its computer-use and external-browser tool surfaces. Packagers
may materialize compatibility roots such as `computer-use@openai-bundled`.
The native Linux ChatGPT app owns its in-app Browser plugin, client, service,
bootstrap instructions, and runtime. Sky must not replace that plugin with
its native-pipe Browser client. This document describes the payload and
materialization boundary.

The standalone fixed-root distribution owns ready-to-install roots for those
two identities at
`${XDG_DATA_HOME:-~/.local/share}/sky-cua/codex/openai-bundled/plugins/`.
Its marketplace is
`codex/openai-bundled/.agents/plugins/marketplace.json`. Consumers should
project `plugins/computer-use/`. When the native ChatGPT Browser is installed
at `/usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/browser`,
standalone installation and browser preflight materialize that complete
baseline for `browser@openai-bundled`, preserving its version and bytes.
The separate Codex `sky-browser-enhancements` skill explicitly attaches
`tab.sky.expectNavigation` to an existing native tab using the helper at
`~/.local/share/sky-cua/browser/sky-enhancements.mjs`. Both remain outside
the app-owned Browser cache. The Sky `plugins/browser/` root remains a
fallback for hosts without that native plugin; `plugins/browser-use/` is not
a compatibility alias. Existing `browser-use@openai-bundled` config entries
are disabled rather than re-enabled by installation.

Ownership split, corrected for the native Linux app on 2026-11-06:

- sky-cua owns behavior and producer bytes: the MCP server, shared Browser
  client, tool surfaces, skills, runtime binaries, fixed-root
  `computer-use`/`browser` plugin directories, and this payload contract.
- Sky's standalone installer and preflight own their filesystem projections
  and config reconciliation. The retired `codex-desktop` rebuild repository
  is not the native app's consumer or installation mechanism.
- The native app keeps its Browser client paired with its service RPC and
  packaged `cua_node`; Sky adds a separate helper and supplemental skill.
  Sky's external client keeps its trusted native-pipe
  lane; changing trust hashes or forcing a different runtime is not part of
  native Browser materialization.

## Payload layout

The released bundle (staged by `scripts/build_plugin.py`, shipped through the
marketplace) is the wrapper-friendly payload. Stable top-level shape:

```
sky-cua/
├── .mcp.json                  # MCP server definitions (see below)
├── bin/
│   ├── sky-cua-client         # POSIX launcher (relocatable, symlink-safe)
│   ├── sky-cua-service        # POSIX launcher
│   ├── sky-cua-overlay-host   # POSIX launcher
│   ├── sky-cua-browser-preflight
│   ├── sky-cua-client.exe     # Windows launchers
│   ├── sky-cua-service.exe
│   └── runtimes/<platform>/   # real binaries: client, service,
│                              # overlay-host, chrome-host, cosmic-helper
├── resources/
│   ├── app-instructions/      # per-app guidance index + docs
│   └── chrome_preflight.py
├── skills/
│   ├── computer-use/SKILL.md
│   └── browser-use/SKILL.md
└── docs/
```

`<platform>` is `linux-x64`, `linux-arm64`, or `windows-x64`.

## Invocation rules

Wrappers may copy or symlink the payload, and must get identical behavior.
Guaranteed by this contract and proven by tests:

- `bin/` launchers resolve their own symlinks before locating
  `bin/runtimes/<platform>/`, so a materialized plugin root may symlink the
  entry point instead of copying the whole bundle
  (`scripts/test_plugin_bundle.py::test_unix_launcher_runs_bundled_runtime_from_relocated_bundle`).
- The client locates `sky-cua-service` as a sibling of the real client binary
  inside `runtimes/<platform>/`; no working-directory or checkout assumption
  (`crates/sky-cua-client/src/service_launcher.rs::service_path`). The same
  sibling rule covers the overlay host and chrome host.
- App-instruction resources are found from the runtime binary's ancestors
  when the launch cwd is unrelated to the payload
  (`crates/sky-cua-platform/src/app_instructions.rs::repo_root`).
- Explicit overrides always win: `SKY_CUA_SERVICE_PATH`,
  `SKY_CUA_OVERLAY_HOST_PATH`, `SKY_CUA_REPO_ROOT`.

The host-facing entrypoint is `./bin/sky-cua-client mcp` (stdio MCP), as
documented in [`mcp-boundary.md`](mcp-boundary.md). A generated wrapper
plugin should point its MCP server command at the packaged launcher by
absolute or plugin-root-relative path.

## .mcp.json server definition

The bundle root `.mcp.json` defines one MCP server named `computer-use` with
`command: ./bin/sky-cua-client`, `args: ["mcp"]`, and an `env_vars` allowlist
naming the environment keys the server consumes. Wrapper generators should:

- copy the server definition, rewriting `command` to the packaged payload
  location;
- forward the full `env_vars` allowlist — these include desktop session keys
  (`DISPLAY`, `WAYLAND_DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`,
  `XDG_RUNTIME_DIR`, ...) and `SKY_CUA_*` tuning keys; dropping session keys
  breaks detached-launch repair;
- not rename the server: `computer-use` is the stable server identity that
  installers and docs reference. A compat plugin may present its own plugin
  ID, but the MCP server name inside it should stay `computer-use`.

## Stable names

Wrapper generators may rely on these staying stable; renames are
contract-breaking and require a major release note:

- Launcher paths: `bin/sky-cua-client`, `bin/sky-cua-service`,
  `bin/sky-cua-overlay-host`, `bin/sky-cua-browser-preflight`.
- MCP server name: `computer-use`.
- Desktop tool names: `get_app_state`, `list_apps`, `list_windows`,
  `focused_window`, `perform_action`, semantic element actions, input tools
  (`click`, `type_text`, `press_key`, `scroll`, `drag`, ...), `doctor`,
  setup tools.
- Browser tool names: `browser_status`, `browser_list_tabs`, `browser_open`,
  `browser_claim_tab`, `browser_move_mouse`, `browser_navigate`,
  `browser_snapshot`, `browser_screenshot`, `browser_click`,
  `browser_type_text`, `browser_press_key`, `browser_scroll`
  (plus opt-in `browser_eval`).
- Skill roots: `skills/computer-use/`, `skills/browser-use/`.

## Generating a compat plugin root

A materialized compat plugin is an ordinary Codex plugin directory:

```
<plugin-cache-root>/openai-bundled/plugins/<built-in-id>/
├── .codex-plugin/plugin.json   # name matches the built-in plugin ID
├── .mcp.json                   # server definition pointing at the payload
└── skills/                     # copied or symlinked from sky-cua skills/
```

For the fallback `browser@openai-bundled`, the fixed-root producer directory contains
`scripts/browser-client.mjs`. The adapter resolves the shared
`RELEASE.json` `paths.browser_client` file and exports
`setupBrowserRuntime`. Its routing skill selects only an entry with
`type === "iab"` and `transport === "host_provided_iab"` for Codex Desktop.
The shared client retains its `extension_native_host` lane for OpenClaw and
other external Chrome-family consumers. These fallback instructions and the
`transport` field are not the native app's Browser bootstrap contract; use
the native plugin's own skill and returned documentation there.

Rules for materialization:

- `plugin.json` `name` carries the impersonated identity; everything it
  points at resolves into the packaged sky-cua payload.
- Only one active `computer-use` MCP server per host. For the codex host the
  compat root (`computer-use@openai-bundled`) is the enabled plugin id because
  Codex Desktop detects Computer Use plugins by that built-in name. The direct
  sky-cua channel id (`sky-cua@local`) stays installed but disabled as a payload
  carrier. Payload selection retargets the compat root's `.mcp.json` at the
  installed payload (see `docs/operations/plugin-release.md`).
- The materialized root must remain a complete, CLI-readable plugin after
  sync. Native Browser projection preserves the entire installed plugin,
  including its original manifest, client, service and docs. Sky's helper and
  activation skill live outside that cache; ordinary native setup is unchanged.

## Verification

- `uv run pytest scripts/test_plugin_bundle.py` — payload shape and
  relocatable launcher proof.
- `cargo nextest run --workspace -p sky-cua-platform app_instructions` — packaged resource-root
  resolution.
- `python3 scripts/build_plugin.py` then inspect `dist/plugin/sky-cua/` for
  the staged layout above.
- `uv run pytest scripts/test_standalone_release.py scripts/test_plugin_bundle.py`
  — native Browser projection, retired identity reconciliation, and isolated
  fallback fixtures. Native package updates remain owned by the distribution.
