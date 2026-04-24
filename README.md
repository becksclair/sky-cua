# sky-cua

Linux-first MCP Computer Use runtime, with Codex plugin packaging.

`sky-cua` is a Rust workspace plus Python harnesses for driving Linux desktop
apps from agent hosts that can talk to MCP. It exposes `sky-cua-client mcp`,
keeps a long-lived service behind a Unix socket, and routes app discovery,
screenshots, and input through Linux-native backends. The Codex plugin is a
packaged adapter around that runtime, not the runtime boundary itself.

## What Works

- portable MCP server entrypoint through `sky-cua-client mcp`
- Codex plugin packaging through `.codex-plugin/plugin.json` and `.mcp.json`
- split Rust client/service runtime with isolated socket support via
  `SKY_CUA_SERVICE_SOCKET_PATH`
- Wayland/X11 environment probing, AT-SPI app discovery, and targeted
  `get_app_state` selection by app identity or window title
- Wayland portal session reuse for ScreenCast metadata and RemoteDesktop input,
  including restore-token persistence under the per-user state directory
- in-process PipeWire frame capture from the active ScreenCast session, with
  Screenshot portal fallback when PipeWire capture fails
- explicit capture metadata:
  - `capture.backend` is the selected primary lane
  - `capture.image_backend` is the backend that produced the image
- semantic AT-SPI actions for `click`, `perform_secondary_action`, `set_value`,
  and focus
- physical Wayland and X11 routing for `click`, `perform_secondary_action`,
  `scroll`, `drag`, `type_text`, and `press_key`
- X11/XWayland fallback discovery through window metadata and `xwininfo`
  descendant bounds
- app-action policy in `resources/app-instructions/index.json`, including the
  Kate-scoped `set_value` fallback
- host-portable workflow guidance in `skills/computer-use-workflows/`

## Development

From the repo root:

```bash
cargo build
cargo test
uv sync --dev
uv run ruff format scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
```

Run the runtime pieces directly:

```bash
./bin/sky-cua-client mcp
./bin/sky-cua-service daemon
```

Build and install a Codex debug bundle:

```bash
python3 scripts/build_plugin.py
python3 scripts/install_plugin.py --bundle-root dist/plugin/sky-cua
```

Reset persisted portal restore tokens:

```bash
python3 scripts/reset_portal_tokens.py
```

## Live Smokes

These harnesses are operator-facing and may require a real desktop session,
portal approval, installed apps, or local Codex auth.

```bash
python3 scripts/live_desktop_smoke.py
python3 scripts/live_portal_downgrade_smoke.py
python3 scripts/live_x11_smoke.py
python3 scripts/live_kate_smoke.py
python3 scripts/live_krita_smoke.py
python3 scripts/live_app_server_smoke.py
python3 scripts/live_app_server_tidal_playlist.py
```

Diagnostic or legacy lanes:

```bash
python3 scripts/live_codex_exec_smoke.py
python3 scripts/live_codex_exec_tidal_playlist.py
python3 scripts/live_kdialog_smoke.py
```

For the full installed-plugin ChatGPT-auth E2E investigation, see
`docs/codex-plugin-e2e-expedition.md`.

For the portable runtime boundary and host-adapter expectations, see
`docs/mcp-runtime.md`.

## Current Limitations

- Wayland screenshot capture prefers PipeWire from the active ScreenCast session.
  Screenshot portal capture is a fallback, and downgrades are reported through
  `capture.image_backend` plus `CaptureBackendDowngraded`.
- Portal approval is operator-visible. If approval is not granted promptly,
  `get_app_state` reports `PortalApprovalPending` instead of hiding the wait
  behind a generic internal error.
- AT-SPI coverage varies sharply across Linux apps and toolkits. `zenity` is a
  reliable semantic smoke fixture; several real apps need hybrid tree plus
  screenshot-guided physical actions.
- X11/XWayland fallback trees are practical physical-targeting hints, not rich
  semantic accessibility trees.
- Pure X11 without a window manager may lack `_NET_CLIENT_LIST`; the backend
  falls back to `xwininfo -root -tree`.
- Kate replacement is proven only through the XWayland harness
  (`QT_QPA_PLATFORM=xcb kate --new --block <file>`), where keyboard-driven
  actions prefer X11/XTest once the focused app matches an X11 window.
- Krita is the best current native-Wayland graphical workflow proof on this
  machine. The reliable path is hybrid: AT-SPI for app/window anchors and
  screenshot-guided physical actions for dialog and canvas steps.
- Installed-plugin harnesses are opt-in acceptance tools, not default regression
  tests. The rich-client path uses `codex app-server`; `codex exec` remains a
  diagnostic probe.
