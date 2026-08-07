# MCP runtime boundary

`sky-cua` is primarily an MCP desktop-control runtime. Host-specific packages
such as the Codex plugin should adapt to this boundary rather than own it.

## Runtime entrypoint

The stable host-facing entrypoint is:

```bash
./bin/sky-cua-client mcp
```

That process speaks MCP over stdio and delegates desktop work to the
long-lived service process over platform IPC: a Unix socket on Linux and a
loopback TCP endpoint on Windows. Hosts may launch the client
directly, use a generated MCP config, or wrap it in their own package format.
The stdio reader accepts newline-delimited JSON-RPC and `Content-Length`
framing with one 64 MiB frame limit. Oversized lines, headers, and declared
payloads are rejected before unbounded accumulation or payload allocation.

The service can also be run directly for debugging:

```bash
./bin/sky-cua-service daemon
```

**Daemon logs.** When the client spawns the daemon, it captures the daemon's
stderr into a per-endpoint log under the sky-cua state dir
(`~/.local/state/sky-cua/daemon-<socket-stem>.log` on Linux; the isolated
desktop daemon gets its own file) and passes the same path via
`SKY_CUA_DAEMON_LOG_PATH` so the daemon routes tracing output through a
self-rotating writer (rotated past 8 MiB to one `.log.old` generation, both
at spawn and at runtime). Stderr capture catches pre-init output; on unix the
daemon re-points fd 2 at each fresh log so panics follow the live log across
rotations (on Windows panic capture ends at the first runtime rotation). An operator-run `sky-cua-service daemon` has no log path env set and
logs to plain stderr. These files are the primary forensic record for
daemon-side incidents — agent transcripts are the only other trace.

For local operator triage without an MCP host, `sky-cua-client` also exposes a
JSON-first CLI surface over the same service requests:

```bash
./bin/sky-cua-client health
./bin/sky-cua-client doctor
./bin/sky-cua-client list-apps
./bin/sky-cua-client list-windows
./bin/sky-cua-client focused-window
./bin/sky-cua-client get-app-state --detail compact --capture-screen if-changed
./bin/sky-cua-client get-app-state --app-id org.kde.kate --detail full --capture-screen never
```

These commands pretty-print JSON to stdout and exit non-zero for service
errors, degraded discovery responses, or setup reports that are still not
ready.

Linux bundles may also include a browser preflight wrapper:

```bash
./bin/sky-cua-browser-preflight --codex-home ~/.codex
```

That wrapper is a Codex Desktop adapter helper, not part of the portable MCP
runtime. It syncs the OpenAI-bundled `chrome` and `browser-use` plugins plus the
`computer-use` compatibility entry into the local Codex bundled marketplace
cache and writes Chromium-family native-host manifests.

## Required host responsibilities

A host adapter should provide:

- an MCP server registration that launches `sky-cua-client mcp`
- a working directory or absolute binary path that resolves `bin/sky-cua-client`
- the desktop/session environment needed by the Linux backend, including
  `DBUS_SESSION_BUS_ADDRESS`, `DISPLAY`, `WAYLAND_DISPLAY`,
  `XDG_CURRENT_DESKTOP`, `XDG_RUNTIME_DIR`, and `XDG_SESSION_TYPE` when present
- on Windows, a direct `.exe` launch is preferred; `SKY_CUA_SERVICE_TCP_ADDR`
  may be set to isolate the service loopback endpoint for tests
- access to the workflow guidance in `skills/computer-use/` and `skills/browser-use/`
- access to app-specific guidance packaged from `resources/app-instructions/`

Codex satisfies this through `.codex-plugin/plugin.json` plus `.mcp.json`.
Other hosts, including OpenCode, should map their own install and instruction
mechanisms onto the same pieces.

Linux detached launches are now repaired defensively at runtime, so a host that
fails to forward those variables is no longer automatically fatal. The client
normalizes `PATH`, probes `/run/user/<uid>`, X11 sockets, logind, and the
systemd user manager before spawning `sky-cua-service`; the Linux service then
hydrates the same session state again before probing portals, AT-SPI, KWin, and
other desktop backends. The repair is observable through `doctor.session_env`,
the `SessionEnvRepaired` diagnostic, and desktop resource diagnostics. This is a
recovery path, not a reason for host adapters to omit the environment allowlist.

For Codex Desktop compatibility, the shipped plugin presents one active
`computer-use` MCP server while still launching the `sky-cua-client mcp`
runtime. Browser Use remains a companion integration: the adapter syncs
OpenAI-bundled `chrome` and `browser-use` resources and enables their plugin
ids. On Linux, the adapter also enables `computer-use@openai-bundled` as the
single Computer Use plugin id when its `.mcp.json` points at the installed
sky-cua payload; `sky-cua@local` stays disabled as a payload carrier. Other MCP
hosts do not need these Codex bundled plugin cache steps.

## Codex deploy, packaging, and config reset

Local development uses a direct cache deploy, not a marketplace install:

```bash
python3 scripts/deploy_plugin.py
```

That command builds `dist/plugin/sky-cua`, installs the bundle as
`sky-cua@local` into the local Codex cache, retargets the
`computer-use@openai-bundled` compat plugin at it through the bundled
`resources/chrome_preflight.py`, and refreshes the installed MCP runtime. No
git, no marketplace, no Codex `plugin/install`. The installed cache lives at:

```text
~/.codex/plugins/cache/local/sky-cua/local/
```

For a machine without a checkout or toolchain, build the standalone artifact:

```bash
python3 install.py build
# Copy dist/sky-cua-linux-x64-glibc.tar.gz to the target, then:
tar xzf sky-cua-linux-x64-glibc.tar.gz
cd sky-cua-linux-x64-glibc
python3 install.py install
```

The installer recoverably replaces
`${XDG_DATA_HOME:-~/.local/share}/sky-cua`, writes exact native-host manifests,
and projects stable launchers, skills, and detected consumer registrations.
There are no generations, `current` selector, rollback operation, hash-selected
install arguments, or consumer Browser trust hashes. The artifact carries only
the latest bundled Chrome extension. See
[`docs/features/release-package.md`](../features/release-package.md) and
[`docs/operations/plugin-release.md`](../operations/plugin-release.md).

If `~/.codex/config.toml` has stale sky-cua state, clean only the plugin and
compatibility entries before redeploying; the exact reset stanza and
post-deploy verification are documented in
[`docs/operations/plugin-release.md`](../operations/plugin-release.md).

For plain MCP hosts, build release binaries and emit a host-specific config:

```bash
cargo build --release
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host generic
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host opencode
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host claude-code
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host claude-desktop
```

The generated generic config keeps the MCP server name `computer-use`, uses
absolute paths, and preserves the same desktop-session environment allowlist as
the Codex plugin config.

For local development updates, use the installer's opt-in runtime restart after
building and copying fresh binaries:

```bash
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host opencode --bin-dir ~/.local/bin --restart-runtime
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host pi --bin-dir ~/.local/bin --restart-runtime
```

`--restart-runtime` attempts to refresh the user AT-SPI accessibility bus on Linux
desktop sessions, then stops installed sky-cua runtime processes rooted under the target
directory, including `sky-cua-client`, `sky-cua-service`,
`sky-cua-overlay-host`, `sky-cua-chrome-host`, and `sky-cua-cosmic-helper`. This
is deliberately opt-in so an install does not interrupt an active MCP session by
surprise. OpenCode and Pi usually respawn the lazy MCP process on the next tool
call; if they do not, reload the host session. For Pi, run `/reload` or restart
Pi.

OpenClaw is not a `scripts/install_mcp_server.py` host. The standalone
`python3 install.py install` path installs the native Computer Use and Browser
Codex plugins, registers global `node_repl`, and projects the shared skills.
It also sets OpenClaw's native Codex app-server to `mode: yolo`,
`approvalPolicy: never`, and `sandbox: danger-full-access`, then converges all
existing agent Codex homes to `approval_policy = "never"` and
`sandbox_mode = "danger-full-access"`. This deliberately covers the whole
Codex runtime so desktop input, Browser input, phone actions, commands, file
changes, and permission requests do not prompt. The global OpenClaw policy
covers agents created after installation; per-agent files make existing
runtimes converge immediately.

Verify deployment with a fresh OpenClaw native-Codex turn that exercises both
Computer Use and `node_repl`. A resumed thread can retain pre-install runtime
state, so it is not sufficient acceptance evidence.

## Machine configuration

Per-machine runtime settings live in one TOML file rather than in every MCP
host registration's environment:

- Linux/macOS: `$XDG_CONFIG_HOME/sky-cua/sky-cua.toml`
  (default `~/.config/sky-cua/sky-cua.toml`)
- Windows: `%APPDATA%\sky-cua\sky-cua.toml`

Supported keys:

```toml
# Chrome-family browser selection: brave, chrome, chromium, or all.
# Unset probes every Chrome-family browser.
browser = "brave"

# Agent-facing MCP surfaces. Each field defaults to true when omitted.
[surfaces]
desktop = true
browser = true
phone = true
```

The surface policy is frozen when an MCP session initializes. The `[surfaces]`
table is strict: misspelled or unknown surface keys are configuration errors
rather than silently falling back to enabled. A disabled surface is removed from
`tools/list`; shared tools such as `status`,
`list_resources`, `observe`, and `capture_screen` also remove that surface's
branches and fields. `capture_screen` disappears entirely when both browser and
phone are disabled. `[phone].enabled = false` also disables the phone MCP
surface.

Environment variables stay per-process overrides on top of the file:
`SKY_CUA_BROWSER` beats the file's `browser`, `SKY_CUA_SURFACES` is an exact
comma-separated temporary surface allowlist (for example `browser` or
`desktop,phone`), and `SKY_CUA_CONFIG_PATH` relocates the file itself (tests,
fixtures). Provisioning deliberately ignores `SKY_CUA_SURFACES` and projects
skills only from the durable machine file. Browser selection is read on use,
so browser changes apply without restarting the daemon. Longer-lived subsystem
managers can resolve their own config at construction time; for example,
phone-use changes apply after reconnecting/restarting the runtime that owns the
phone manager. Unknown keys are tolerated for forward compatibility; an
unparseable file surfaces as a
`MachineConfigInvalid` diagnostic instead of being silently ignored.

## MCP tool surface

The host-facing tools are the portable product contract. Current tools are
canonical and grouped by target/intent:

- readiness/setup: `doctor`, `status(component=...)`, and
  `setup_desktop(operation="accessibility"|"window_targeting")`
- session presence: `session_presence(operation="hold"|"unlock"|"release")`
  plus `status(component="session_presence")`
- app/window discovery: `list_resources(surface="desktop",
  resource="apps"|"windows"|"focused_window")` and `activate_window`
- desktop state: `observe(surface="desktop")`, with `detail: "compact"` by
  default and `detail: "full"` as the exhaustive inspection mode. Responses
  default to 200 returned elements while preserving `element_count`/
  `filtered_element_count` metadata; use `element_query`, `element_offset`,
  and `element_limit` to page or narrow dense accessibility trees. Use
  `screenshot_delivery: "inline"` to attach the captured screenshot as an MCP
  image content block for hosts that cannot read `inspection_image_path` files
- desktop visual capture: `capture_desktop`, which captures exactly one screen
  and defaults to the main (primary) display, accepts the same window target
  fields as `activate_window`, and accepts
  `display_id`/`display_name`/`display_index` from `environment.displays` to
  target a specific non-main monitor; it exposes no whole-virtual-desktop option
- desktop semantic actions: `desktop_semantic`, `desktop_toggle`,
  `desktop_action`, and `desktop_set_value`
- desktop physical or hybrid actions: `desktop_pointer`, `desktop_scroll`, and
  `desktop_keyboard`
- browser readiness, tab lifecycle, page state, screenshots, and tab-scoped
  actions: `status(component="browser")`,
  `list_resources(surface="browser", resource="tabs")`, `browser_open`,
  `browser_claim_tab`, `browser_move_mouse`, `browser_navigate`,
  `observe(surface="browser")`, `capture_screen(surface="browser")`,
  `browser_input`, and `browser_scroll`
- phone discovery, connection, perception, input, notifications, and app
  control: `status(component="phone"|"phone_companion")`,
  `list_resources(surface="phone", resource=...)`, `phone_connection`,
  `phone_pair_wireless`, `phone_setup`, `observe(surface="phone")`,
  `capture_screen(surface="phone")`, `phone_pointer`, `phone_keyboard`,
  `phone_notification_action`, `phone_notification_reply`, `phone_app_action`,
  `phone_app_force_stop`, `phone_app_install`, `phone_accessibility_tree`, and
  `phone_notifications`

Browser tools need no host-specific opt-in because the browser surface defaults
to enabled; `[surfaces].browser = false` removes them. `browser_eval` is the
additional security gate and is advertised only when the browser surface exists
and `SKY_CUA_BROWSER_EVAL` is `on`, `1`, or `true`.
Codex Desktop may still use the companion Browser Use/Chrome plugin path until
its adapter delegates to this shared browser surface, while host-specific
configs emitted by `scripts/install_mcp_server.py` can pass browser-selection
environment such as
`SKY_CUA_BROWSER`.

Every tool definition carries MCP `ToolAnnotations` (`readOnlyHint`,
`destructiveHint`, `idempotentHint`, `openWorldHint`). Hosts use these for
graduated approval: Codex's `auto` approval mode silently approves read-only
tools and prompts for destructive or open-world ones, and treats unannotated
tools as both. The hints are honest by policy, not flattering: observation
tools (`doctor`, `status`, `list_resources`, `observe`, `capture_screen`,
`phone_accessibility_tree`, and `phone_notifications`) are read-only;
focus/selection/expansion moves, session hold/release/unlock requests, tab
claims, and desktop captures are non-destructive and idempotent; arbitrary
input (`desktop_pointer`, `desktop_keyboard`, `desktop_action`,
`desktop_set_value`, `browser_input`, `phone_pointer`, `phone_keyboard`,
notification/app actions, and enabled `browser_eval`) stays destructive because
it can trigger any in-app action. Live-web actions are additionally open-world;
`browser_scroll` is
non-destructive but still open-world because it mutates a real web page's
viewport or scrollable DOM state. The full table is pinned by
`mcp_tools::annotation_tests`; changing a row changes what hosts auto-approve
and must be deliberate.

Browser tools are documented in
[`docs/features/browser-mcp-tools.md`](../features/browser-mcp-tools.md); the
runtime exposes the same contract to every host.

`doctor` includes Linux `session_env` repair details when the runtime had to
recover detached desktop state. `repaired` records which keys were filled and
from which source, `path_changed` reports whether `PATH` was normalized, and
`final_path` records the effective path after repair. Tool text summaries and
snapshot/list diagnostics may include `SessionEnvRepaired`; treat that as
useful context that the runtime recovered, not as an error by itself.

Action tools accept `snapshot_id` from the latest `observe(surface="desktop")`
or `capture_desktop` result. With a captured snapshot that includes capture metadata,
explicit coordinates are screenshot pixels from that image. A structure-only
desktop observation snapshot id still scopes `element_index` lookups, but
cannot translate screenshot pixels. Without capture metadata, supported
coordinate actions use the current screen coordinate space exposed by the active
input backend.
Desktop snapshots may be cropped to a window or one display; callers should
always pass the matching `snapshot_id` so the backend can translate screenshot
pixels through `capture.logical_rect` and the backend-specific source rect.

`observe(surface="desktop")` elements may include readback fields when the
backend can prove them. On Linux, focused or editable AT-SPI Text controls can populate
`ElementNode.value` and `text.content` with the current text, including a known
empty string. AT-SPI Value controls can populate `numeric_value` and use a
short value summary. Password/protected controls suppress content and leave
`value` absent even if AT-SPI exposes data. Compact snapshots intentionally
preserve `value`, `text`, `numeric_value`, and `supports_editable_text` so
agents can verify text entry without switching back to full detail.

Desktop window resources and `activate_window` use native window metadata when
available. Linux currently probes GNOME Shell extension, GNOME
Shell Introspect, COSMIC helper, KWin/Plasma, Hyprland, i3, and X11 metadata
backends. Window payloads may include bounds, workspace, PID, client type,
display assignment, spanning-display intersections, and terminal metadata
depending on what the backend can prove. Desktop observations and captures
surface `environment.displays`; agents should use those display IDs instead of
guessing monitor names.

## Adapter split

Use these lanes when validating changes:

- Runtime lane: direct MCP and desktop smokes such as
  `scripts/live_desktop_smoke.py`, `scripts/live_portal_downgrade_smoke.py`,
  `scripts/live_wayland_pointer_smoke.py`, `scripts/live_kate_smoke.py`, and
  `scripts/live_krita_smoke.py`.
- Installed-agent lane: bundle/install checks and agentic-loop smokes such as
  `scripts/build_plugin.py`, `scripts/install_plugin.py`, and
  `scripts/live_agentic_loop_smoke.py`.
- Detached Linux session-env lane: `scripts/live_session_env_smoke.py` proves
  direct MCP recovery from a stripped desktop environment, while
  `scripts/live_codex_exec_session_env_smoke.py` and
  `scripts/live_app_server_session_env_smoke.py` prove that agent harnesses can
  see `SessionEnvRepaired` or `session_env` before operating the desktop.
- Text-readback lane: `scripts/live_desktop_smoke.py` covers direct `zenity`
  readback before and after `set_value` / `type_text`; `scripts/live_codex_exec_text_readback_smoke.py`
  and `scripts/live_app_server_text_readback_smoke.py` prove agents consume
  stale and replacement values from desktop observation transcripts before
  submitting.

Runtime changes should pass the narrowest relevant runtime lane first. Codex
plugin checks prove that the Codex adapter still packages and activates the
same runtime correctly.

`scripts/run_gui_testing_vm_smoke.py` is the current Linux GUI matrix runner.
It builds runtime artifacts on the host, syncs the checkout into the Arch
`testing-vm`, copies selected Codex state, and runs profiles over SSH against
real guest desktop sessions. Keep non-Codex host proof in the same VM: OpenCode
is installed there by `scripts/testing-vm/provision-arch-testing-vm.sh`, and
`scripts/testing-vm/sync-opencode-to-vm.sh` syncs host OpenCode config/auth
without copying the host DB/log/snapshot history.

Codex Desktop browser compatibility has an additional adapter lane:

```bash
python3 scripts/build_plugin.py
python3 scripts/install_plugin.py --bundle-root dist/plugin/sky-cua
uv run pytest scripts/test_plugin_bundle.py -k 'browser_preflight or update_codex_config'
```

The corresponding CodexDesktop-Rebuild patch checks live in that project:

```bash
bun test ./scripts/patch-computer-use.test.ts
bun scripts/patch-computer-use.ts --check
bun scripts/patch/apply.ts --check
```

## OpenCode adapter

The standalone installer registers `sky_cua` in the user's global OpenCode
configuration. Run OpenCode from any project without adding a repository-local
MCP override:

```bash
opencode mcp list
opencode run --dir /home/bex/projects/sky-cua \
  "Use the sky_cua MCP tool list_resources with surface=desktop and resource=apps."
```

For VM-based non-Codex harness work, use the Arch testing VM documented in
`docs/operations/gui-desktop-test-harness.md`. It installs OpenCode from npm and
syncs host OpenCode config/auth into the VM without copying the host DB, logs,
snapshots, or tool-output history, then verifies `opencode --version` and
`opencode models openai` in the guest. That proves the OpenCode auth/config
surface, not sky-cua MCP behavior under OpenCode yet. For that, install/register
the MCP runtime with `scripts/install_mcp_server.py --host opencode` and then run an
OpenCode MCP tool smoke.

For the LAN server at `https://opencode.heliasar.com`, restart the
`opencode-lan.service` user service after changing repo-local OpenCode config:

```bash
systemctl --user restart opencode-lan.service
opencode run --attach https://opencode.heliasar.com \
  --dir /home/bex/projects/sky-cua \
  "Use the sky_cua MCP tool list_resources with surface=desktop and resource=apps."
```

OpenCode validates `tools/list.nextCursor` strictly: omit `nextCursor` when
there is no cursor instead of sending `null`. OpenCode also exposes the text
content of tool results prominently, so keep `content[].text` useful even when
the full structured payload is present.

## Portability notes

Keep new behavior behind the MCP tool contract and shared resources whenever
possible. Avoid putting core behavior in Codex-only prompts, app-server
harnesses, or plugin metadata. If host-specific wording is needed, put it under
an adapter-specific file and keep the shared workflow guidance neutral.

Do not make Chrome/Browser Use packaging behavior a dependency of the core MCP
runtime. The native-host and bundled-plugin cache work is an adapter layer for
Chrome-family browser access. First-class browser MCP behavior is exposed
through the shared browser tool contract; `user_chrome` is the only browser
target, and the managed/isolated browser lifecycle was retired and removed from
the wire contract.
