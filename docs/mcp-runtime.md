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

The service can also be run directly for debugging:

```bash
./bin/sky-cua-service daemon
```

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
runtime. It syncs the OpenAI-bundled `chrome` and `browser-use` plugins plus a
disabled `computer-use` compatibility entry into the local Codex bundled
marketplace cache and writes Chromium-family native-host manifests.

## Required host responsibilities

A host adapter should provide:

- an MCP server registration that launches `sky-cua-client mcp`
- a working directory or absolute binary path that resolves `bin/sky-cua-client`
- the desktop/session environment needed by the Linux backend, including
  `DBUS_SESSION_BUS_ADDRESS`, `DISPLAY`, `WAYLAND_DISPLAY`,
  `XDG_CURRENT_DESKTOP`, `XDG_RUNTIME_DIR`, and `XDG_SESSION_TYPE` when present
- on Windows, a direct `.exe` launch is preferred; `SKY_CUA_SERVICE_TCP_ADDR`
  may be set to isolate the service loopback endpoint for tests
- access to the workflow guidance in `skills/computer-use-workflows/`
- access to app-specific guidance packaged from `resources/app-instructions/`

Codex satisfies this through `.codex-plugin/plugin.json` plus `.mcp.json`.
Other hosts, including OpenCode, should map their own install and instruction
mechanisms onto the same pieces.

For Codex Desktop compatibility, the shipped plugin presents one active
`computer-use` MCP server while still launching the `sky-cua-client mcp`
runtime. Browser Use remains a companion integration: the adapter syncs
OpenAI-bundled `chrome` and `browser-use` resources and enables their plugin
ids. It stages `computer-use@openai-bundled` for marketplace completeness but
keeps it disabled so Codex does not see duplicate active `computer-use`
servers. Other MCP hosts do not need these Codex bundled plugin cache steps.

## Codex release deploy and config reset

The current local release lane is the Heliasar marketplace install, not direct
cache editing:

```bash
python3 scripts/deploy_release_plugin.py
```

That command builds `dist/plugin/sky-cua`, stages the bundle into
`~/projects/heliasar-marketplace/plugins/sky-cua`, writes
`~/projects/heliasar-marketplace/.agents/plugins/marketplace.json`, asks
`codex app-server` to install `sky-cua`, enables `sky-cua@Heliasar`, disables
`sky-cua@debug`, and reloads MCP servers. The installed cache is an output of
Codex's plugin install path:

```text
~/.codex/plugins/cache/Heliasar/sky-cua/<version>/
```

If `~/.codex/config.toml` has stale sky-cua state, clean only the plugin and
compatibility entries before redeploying. Keep unrelated project trust, auth,
model, and curated plugin settings intact. The stale entries to remove or
normalize are:

```toml
notify = ["/Users/rebecca/.codex/reverse-engineering/computer-use-...", "turn-ended"]

[plugins."sky-cua@debug"]
enabled = false

[plugins."sky-cua@Heliasar"]
enabled = true

[marketplaces.Heliasar]
...

[plugins."computer-use@openai-bundled"]
enabled = true
```

After cleanup, rerun `python3 scripts/deploy_release_plugin.py`. If no
`[marketplaces.Heliasar]` stanza exists, the deploy script configures the local
marketplace source. If one already exists, the script preserves it so a
Git-backed marketplace source is not silently replaced.

The expected post-deploy config shape is:

```toml
[plugins."computer-use@openai-bundled"]
enabled = false

[plugins."sky-cua@Heliasar"]
enabled = true

[plugins."sky-cua@debug"]
enabled = false

[marketplaces.Heliasar]
source = "/home/bex/projects/heliasar-marketplace"
source_type = "local"
last_updated = "..."
```

The cheap control-plane proof is `mcpServerStatus/list` through `codex
app-server`; it should show one `computer-use` server with tools such as
`list_apps`, `get_app_state`, `click`, `scroll`, `type_text`, and `doctor`.
If both `sky-cua@Heliasar` and `computer-use@openai-bundled` are enabled, Codex
can skip one duplicate `computer-use` server, so fix the config before chasing
runtime bugs.

For plain MCP hosts, build release binaries and emit a host-specific config:

```bash
cargo build --release
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host generic
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host opencode
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host claude-desktop
```

The generated generic config keeps the MCP server name `computer-use`, uses
absolute paths, and preserves the same desktop-session environment allowlist as
the Codex plugin config.

## MCP tool surface

The host-facing tools are the portable product contract. Current tools:

- readiness/setup: `doctor`, `setup_accessibility`, `setup_window_targeting`
- app/window discovery: `list_apps`, `list_windows`, `focused_window`,
  `activate_window`
- state capture: `get_app_state`, with `detail: "full"` by default and
  `detail: "compact"` for repeated screenshot-first loops
- semantic element actions: `focus_element`, `activate_element`,
  `select_element`, `expand_element`, `collapse_element`, `toggle_element`,
  and `perform_action`
- physical or hybrid actions: `click`, `perform_secondary_action`, `scroll`,
  `drag`, `type_text`, `press_key`, and `set_value`

Action tools accept `snapshot_id` from the latest `get_app_state` result. With
`snapshot_id`, explicit coordinates are screenshot pixel coordinates from that
snapshot image. Without `snapshot_id`, supported coordinate actions use the
current screen coordinate space exposed by the active input backend.

`list_windows`, `focused_window`, and `activate_window` use native window
metadata when available. Linux currently probes GNOME Shell extension, GNOME
Shell Introspect, COSMIC helper, KWin/Plasma, Hyprland, i3, and X11 metadata
backends. Window payloads may include bounds, workspace, PID, client type, and
terminal metadata depending on what the backend can prove.

## Adapter split

Use these lanes when validating changes:

- Runtime lane: direct MCP and desktop smokes such as
  `scripts/live_desktop_smoke.py`, `scripts/live_portal_downgrade_smoke.py`,
  `scripts/live_wayland_pointer_smoke.py`, `scripts/live_kate_smoke.py`, and
  `scripts/live_krita_smoke.py`.
- Codex adapter lane: bundle/install checks and app-server smokes such as
  `scripts/build_plugin.py`, `scripts/install_plugin.py`, and
  `scripts/live_app_server_smoke.py`.

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
uv run pytest scripts/test_python_harness_helpers.py -k 'browser_preflight or update_codex_config'
```

The corresponding CodexDesktop-Rebuild patch checks live in that project:

```bash
bun test ./scripts/patch-computer-use.test.ts
bun scripts/patch-computer-use.ts --check
bun scripts/patch/apply.ts --check
```

## OpenCode adapter

The repo-local `opencode.json` registers `sky_cua` as a local MCP server for
OpenCode:

```bash
opencode mcp list
opencode run --dir /home/bex/projects/sky-cua \
  "Use the sky_cua MCP tool list_apps directly."
```

For VM-based non-Codex harness work, use the Arch testing VM documented in
`docs/gui-desktop-test-harness.md`. The provisioner installs OpenCode from npm
with `OPENCODE_NPM_SPEC` defaulting to `opencode-ai@1.14.51`; then sync host
OpenCode config/auth into the VM without copying the host DB, logs, snapshots,
or tool-output history:

```bash
scripts/testing-vm/sync-opencode-to-vm.sh
ssh -p 22222 \
  -o StrictHostKeyChecking=no \
  -o UserKnownHostsFile=artifacts/testing-vm/known_hosts \
  skycua@127.0.0.1 'opencode --version && opencode models openai | head'
```

The current live VM proof is `opencode 1.14.51` with copied
`~/.config/opencode` and `~/.local/share/opencode/auth.json`; `opencode models
openai` succeeds in the guest. This proves the OpenCode auth/config surface,
not sky-cua MCP behavior under OpenCode yet. For that, install/register the MCP
runtime with `scripts/install_mcp_server.py --host opencode` and then run an
OpenCode MCP tool smoke.

For the LAN server at `https://opencode.heliasar.com`, restart the
`opencode-lan.service` user service after changing repo-local OpenCode config:

```bash
systemctl --user restart opencode-lan.service
opencode run --attach https://opencode.heliasar.com \
  --dir /home/bex/projects/sky-cua \
  "Use the sky_cua MCP tool list_apps directly."
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

Do not make Chrome/Browser Use behavior a dependency of the core MCP runtime.
The native-host and bundled-plugin cache work is an adapter layer for Codex
Desktop's existing Browser Use flow. First-class `browser_*` MCP tools should
wait for an isolated host/client or extension smoke that proves the protocol
outside Codex Desktop.
