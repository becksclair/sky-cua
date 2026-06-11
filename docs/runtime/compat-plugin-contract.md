# Compat plugin materialization contract

`sky-cua` is the canonical implementation of its computer-use and browser
tool surfaces. Downstream packagers — primarily the codex-desktop repo — may
materialize compatibility plugin roots that present sky-cua under other
plugin identities (for example the OpenAI built-in IDs
`computer-use@openai-bundled` and `browser-use@openai-bundled`). This
document is the contract those wrappers can rely on.

Ownership split, decided 2026-06-11:

- sky-cua owns behavior: the MCP server, tool surfaces, skills, runtime
  binaries, and this payload contract. sky-cua is tested as sky-cua and never
  ships under another plugin identity from this repo.
- codex-desktop owns impersonation/materialization: bundling the sky-cua
  payload, generating plugin cache roots whose `.codex-plugin/plugin.json`
  matches the built-in plugin IDs Codex Desktop expects, enabling those IDs
  in `~/.codex/config.toml`, and keeping materialized roots valid for an
  unpatched Codex CLI.

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

Rules for the generator (lives in codex-desktop, not here):

- `plugin.json` `name` carries the impersonated identity; everything it
  points at resolves into the packaged sky-cua payload.
- Only one active `computer-use` MCP server per host: when a compat root is
  enabled, the direct `sky-cua@<marketplace>` plugin entry must be disabled,
  or vice versa, to avoid duplicate tool surfaces.
- The materialized root must remain a complete, CLI-readable plugin after
  sync; repair belongs to the desktop-side cache-sync/resync flow.

## Verification

- `uv run pytest scripts/test_plugin_bundle.py` — payload shape and
  relocatable launcher proof.
- `cargo test -p sky-cua-platform app_instructions` — packaged resource-root
  resolution.
- `python3 scripts/build_plugin.py` then inspect `dist/plugin/sky-cua/` for
  the staged layout above.
- Desktop-side materialization smokes (stock cache layout, config
  enablement, Desktop settings visibility) live in the codex-desktop repo.
