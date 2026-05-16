# sky-cua

Cross-platform MCP Computer Use runtime, with Codex plugin packaging.

`sky-cua` is a Rust workspace plus Python harnesses for driving desktop
apps from agent hosts that can talk to MCP. It exposes `sky-cua-client mcp`,
keeps a long-lived service behind platform IPC, and routes app discovery,
screenshots, and input through native backends. Linux remains the most proven
backend; Windows now has a native v1 using Win32 window discovery, GDI capture,
and SendInput physical actions. The Codex plugin is a
packaged adapter around that runtime, not the runtime boundary itself.

## What Works

- portable MCP server entrypoint through `sky-cua-client mcp`
- Codex plugin packaging through `.codex-plugin/plugin.json` and one
  cross-platform `.mcp.json`
- split Rust client/service runtime with isolated Linux socket support via
  `SKY_CUA_SERVICE_SOCKET_PATH` and Windows loopback endpoint support via
  `SKY_CUA_SERVICE_TCP_ADDR`
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
- snapshotless physical actions for current-screen targeting, while
  element-index and semantic actions remain scoped to the snapshot that
  supplied their accessibility context
- X11/XWayland fallback discovery through window metadata and `xwininfo`
  descendant bounds
- app-action policy in `resources/app-instructions/index.json`, including the
  Kate-scoped `set_value` fallback
- host-portable workflow guidance in `skills/computer-use-workflows/`
- Windows compile/runtime foundation through `sky-cua-windows`, including
  top-level window fallback snapshots, GDI screenshots, and SendInput actions
- Linux window targeting through a registry of KWin, X11, GNOME, COSMIC,
  Hyprland, and i3 backends, with terminal metadata selectors where available
- Linux compositor cursor overlay and hide/show support through X11/XFixes, the
  KWin effect, the bundled GNOME Shell extension, Hyprland
  `cursor:invisible`, the COSMIC bridge prototype, and the dedicated no-patch
  `cosmic_transparent_xcursor` VM session mode
- `doctor`, `setup_accessibility`, and `setup_window_targeting` MCP tools with
  structured readiness reports
- Codex Desktop compatibility as a `computer-use` companion bundle: Linux
  installs can sync OpenAI-bundled `chrome` and `browser-use` resources beside
  a sky-cua-backed `computer-use` compatibility plugin
- Chrome/Brave/Chromium native-host manifest preflight on Linux through
  `resources/chrome_preflight.py` and the `bin/sky-cua-browser-preflight`
  wrapper

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

On Windows, use the `.exe` binaries:

```powershell
.\bin\sky-cua-client.exe mcp
.\bin\sky-cua-service.exe daemon
```

Build and install a Codex debug bundle:

```bash
python3 scripts/build_plugin.py
python3 scripts/install_plugin.py --bundle-root dist/plugin/sky-cua
```

On Linux, `install_plugin.py` also runs browser preflight when the built bundle
contains `resources/chrome_preflight.py`. That preflight syncs the local
OpenAI-bundled marketplace cache for `chrome`, `browser-use`, and a
`computer-use` compatibility entry, writes native-host manifests for Google
Chrome, Brave, and Chromium, and enables `chrome@openai-bundled` plus
`browser-use@openai-bundled` in `config.toml`. The compatibility
`computer-use@openai-bundled` entry is staged but disabled so it does not
collide with the active `sky-cua` MCP server.

Run the browser preflight directly when debugging Codex Desktop browser
integration:

```bash
bin/sky-cua-browser-preflight --codex-home ~/.codex
```

To stage OpenAI bundled Chrome/Browser Use resources during build, point the
builder at an upstream Codex Desktop resource root:

```bash
SKY_CUA_UPSTREAM_CODEX_RESOURCES=/path/to/codex/resources \
  python3 scripts/build_plugin.py
```

`SKY_CUA_OPENAI_BUNDLED_RESOURCE_ROOT` may also point directly at an
`openai-bundled` marketplace root. If neither variable is set, the builder
checks the sibling `../codex-desktop-linux/codex-app/resources/plugins/openai-bundled`
path and skips Chrome/Browser Use staging with a warning when it is absent.

Install the runtime as a plain MCP server for non-Codex hosts:

```bash
cargo build --release
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host generic
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host opencode
```

For production-like Linux GUI and non-Codex harness proof, use the Arch
`testing-vm` path in `docs/gui-desktop-test-harness.md`. The VM provisioner
installs OpenCode, and `scripts/testing-vm/sync-opencode-to-vm.sh` copies the
host OpenCode config/auth without copying the host OpenCode database, logs, or
snapshots.

Deploy local Codex plugin builds:

```bash
python3 scripts/deploy_debug_plugin.py
python3 scripts/deploy_release_plugin.py
```

`deploy_debug_plugin.py` keeps the direct debug-cache install as
`sky-cua@debug`. `deploy_release_plugin.py` stages a release bundle through the
local Heliasar marketplace checkout under `~/projects/heliasar-marketplace`,
installs it through Codex, and enables `sky-cua@Heliasar`. If
`~/.codex/config.toml` already has a Heliasar marketplace source, release deploy
preserves it; otherwise it configures the local checkout as
`[marketplaces.Heliasar]`. The deploy scripts switch debug and release plugin
ids mutually so Codex does not see duplicate `computer-use` MCP servers, and
`computer-use@openai-bundled` should remain disabled when `sky-cua@Heliasar` is
active. Deploys preserve already-staged binaries for other platforms, so
rebuilding on Linux does not delete Windows `.exe` binaries from the local
marketplace and vice versa.

If the local Codex config gets stale, the durable reset procedure is documented
in `docs/mcp-runtime.md` under "Codex release deploy and config reset".
`publish_marketplace_release.py` commits and pushes the marketplace checkout
before upgrading the Codex Git marketplace.

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
python3 scripts/live_kate_smoke.py
python3 scripts/live_krita_smoke.py
python3 scripts/live_app_server_smoke.py
python3 scripts/live_wayland_pointer_smoke.py
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile computer-use
```

`scripts/run_gui_testing_vm_smoke.py` is the current Linux GUI matrix runner.
It targets the Arch `testing-vm` with real guest desktop sessions rather than
embedded X servers or container-nested compositors. It only copies selected
host Codex auth/config into the VM when `--sync-codex-settings` is set.

Diagnostic or legacy lanes:

```bash
python3 scripts/live_codex_exec_smoke.py
python3 scripts/live_kdialog_smoke.py
```

For the full installed-plugin ChatGPT-auth E2E investigation, see
`docs/codex-plugin-e2e-expedition.md`.

For the portable runtime boundary and host-adapter expectations, see
`docs/mcp-runtime.md`.

For the current Codex Desktop compatibility plan, including the exact Browser
Use/Chrome proof artifacts, see
`plans/1778463694899-nimble-knight.md`.

## Current Limitations

- Wayland screenshot capture prefers PipeWire from the active ScreenCast session.
  Screenshot portal capture is a fallback, and downgrades are reported through
  `capture.image_backend` plus `CaptureBackendDowngraded`.
- Portal approval is operator-visible. If approval is not granted promptly,
  `get_app_state` reports `PortalApprovalPending` instead of hiding the wait
  behind a generic internal error.
- AT-SPI coverage varies sharply across Linux accessibility stacks and toolkits. `zenity` is a
  reliable semantic smoke fixture; several real apps need hybrid tree plus
  screenshot-guided physical actions.
- X11/XWayland fallback trees are practical physical-targeting hints, not rich
  semantic accessibility trees.
- Coordinate actions with a `snapshot_id` use screenshot pixel coordinates;
  coordinate actions without a `snapshot_id` use the current screen coordinate
  space exposed by the active input backend.
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
- Browser Use support currently relies on Codex Desktop's bundled `chrome` and
  `browser-use` plugins plus the Linux native-host preflight. `sky-cua` does not
  yet expose first-class `browser_*` MCP tools of its own.
- GNOME Shell extension setup installs files and asks GNOME to enable the
  extension, but GNOME may still require a Shell reload or login restart before
  the extension DBus backend appears.
- Cross-desktop cursor proof is live on the Arch `testing-vm` for KDE/KWin,
  GNOME, Hyprland, i3/X11, and COSMIC helper/cursor modes. The remaining
  desktop-matrix work is narrower: broader registry/list/focus re-smokes on
  some desktops and routine reruns after harness or compositor changes.
- Normal unpatched COSMIC still has no dynamic compositor cursor hide/show API.
  The no-patch `cosmic_transparent_xcursor` mode keeps the native cursor
  transparent for the full session instead of restoring it when the overlay
  hides.
- Windows v1 is intentionally conservative: it exposes real top-level window
  bounds and physical actions, but does not yet provide rich UI Automation
  child trees or semantic invoke/value routing.
