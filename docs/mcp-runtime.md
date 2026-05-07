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

## Adapter split

Use these lanes when validating changes:

- Runtime lane: direct MCP and desktop smokes such as
  `scripts/live_desktop_smoke.py`, `scripts/live_portal_downgrade_smoke.py`,
  `scripts/live_x11_smoke.py`, `scripts/live_kate_smoke.py`, and
  `scripts/live_krita_smoke.py`.
- Codex adapter lane: bundle/install checks and app-server smokes such as
  `scripts/build_plugin.py`, `scripts/install_plugin.py`, and
  `scripts/live_app_server_smoke.py`.

Runtime changes should pass the narrowest relevant runtime lane first. Codex
plugin checks prove that the Codex adapter still packages and activates the
same runtime correctly.

## OpenCode adapter

The repo-local `opencode.json` registers `sky_cua` as a local MCP server for
OpenCode:

```bash
opencode mcp list
opencode run --dir /home/bex/projects/sky-cua \
  "Use the sky_cua MCP tool list_apps directly."
```

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
