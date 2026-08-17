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
- defensive Linux detached-session repair: client/service startup can recover
  missing desktop env and normalize `PATH`, with `doctor.session_env` and
  `SessionEnvRepaired` diagnostics showing what changed
- Wayland portal session reuse for ScreenCast metadata and RemoteDesktop input,
  including restore-token persistence under the per-user state directory
- in-process PipeWire frame capture from the active ScreenCast session, with
  Screenshot portal fallback when PipeWire capture fails
- explicit capture metadata:
  - `capture.backend` is the selected primary lane
  - `capture.image_backend` is the backend that produced the image
- a dedicated native Linux `appshot_capture` service/Node API for verified
  window crops with app identity, best-effort AT-SPI text, capture provenance,
  and service-owned temporary artifacts
- semantic AT-SPI actions for `click`, `perform_secondary_action`, `set_value`,
  and focus
- physical Wayland and X11 routing for `click`, `perform_secondary_action`,
  `scroll`, `drag`, `type_text`, and `press_key`
- crate-local Linux action execution boundary under
  `crates/sky-cua-linux/src/actions/`, keeping semantic/physical routing
  fakeable while `LinuxDesktopBackend::execute_action` remains the public
  backend entrypoint
- snapshotless physical actions for current-screen targeting, while
  element-index and semantic actions remain scoped to the snapshot that
  supplied their accessibility context
- X11/XWayland fallback discovery through window metadata and `xwininfo`
  descendant bounds
- app-action policy in `resources/app-instructions/index.json`, including the
  Kate-scoped `set_value` fallback
- host-portable workflow guidance in `skills/computer-use/` and `skills/browser-use/`
- agent-agnostic screenshot delivery: browser screenshots arrive as MCP image
  content blocks plus persisted capture paths in one CSS-pixel coordinate
  space, and `get_app_state` supports `screenshot_delivery: "inline"` for
  hosts that cannot read files by path
- Claude Code host support through `.claude-plugin/` (plugin + marketplace
  manifests) and `scripts/install_mcp_server.py --host claude-code`
- Windows UIA inspection and semantic actions through `sky-cua-windows`,
  including real app-shell child trees, semantic invoke/select/toggle
  routing, top-level window fallback snapshots, GDI screenshots, and
  SendInput actions
- Linux window targeting through a registry of KWin, X11, GNOME, COSMIC,
  Hyprland, and i3 backends, with terminal metadata selectors where available
- Linux compositor cursor overlay and hide/show support through X11/XFixes,
  GPU-backed layer-shell visuals plus the KWin cursor-hide/pointer-signal
  shim, the bundled GNOME Shell extension, Hyprland `cursor:invisible`, the
  COSMIC bridge prototype, and the dedicated no-patch
  `cosmic_transparent_xcursor` VM session mode
- `doctor`, `setup_accessibility`, and `setup_window_targeting` MCP tools with
  structured readiness reports
- Codex Desktop compatibility as a `computer-use` companion bundle: Linux
  installs can sync OpenAI-bundled `chrome` and `browser-use` resources beside
  a sky-cua-backed `computer-use` compatibility plugin
- Chrome/Brave/Chromium native-host manifest preflight on Linux through
  `resources/chrome_preflight.py` and the `bin/sky-cua-browser-preflight`
  wrapper

## Build and install

The standalone distribution has one build command and one install command:

```bash
python3 install.py build
python3 install.py install
```

The build owns its generated inputs and keeps them in durable checkout paths so
repeat builds reuse precompiled artifacts: `dist/plugin/sky-cua`,
`out/components/cua-node-linux-x64-glibc`, and
`dist/standalone/sky-cua-linux-x64-glibc`. It emits the checkout-free archive
`dist/sky-cua-linux-x64-glibc.tar.gz`. The payload contains only the latest
bundled Chrome extension version.

Install replaces the physical root `${XDG_DATA_HOME:-~/.local/share}/sky-cua`.
The stable consumer ABI is always `~/.local/share/sky-cua`; when custom XDG
storage differs, that path is a single rendezvous symlink to the physical tree.
Launchers, native-host manifests, skills, and detected consumer registrations
all use the stable path. There are no generations, `current` selector, rollback
operation, or hash-selected install arguments. Extracted artifacts use the same
`python3 install.py install` command. Details in
[`docs/features/one-shot-installer.md`](docs/features/one-shot-installer.md)
and [`docs/features/release-package.md`](docs/features/release-package.md).

For fastest Linux Wayland input, install the privileged uinput helper from a
runtime install with `scripts/install_mcp_server.py --input-helper`; it runs as
root and exposes `/run/sky-cua/input-helper.sock`.

## Development

From the repo root:

```bash
cargo build
cargo nextest run   # install once: cargo install cargo-nextest
uv sync --dev
uv run ruff format scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
```

`just verify` runs the full headless suite in one command (Rust + Python +
Kotlin-when-JDK-present).

Rust tests run under [`cargo-nextest`](https://nexte.st), not `cargo test`:
some `sky-cua-service` tests mutate process-global environment variables and
bind OS sockets, which race under `cargo test`'s single-process threading.
nextest isolates each test in its own process and serializes the heavy
integration tests via `.config/nextest.toml`. Doctests (none today) still run
with `cargo test --doc`.

Run the runtime pieces directly:

```bash
./bin/sky-cua-client mcp
./bin/sky-cua-service daemon
```

When touching Linux launch or environment repair, run the detached session
smokes as well:

```bash
python3 scripts/live_session_env_smoke.py
python3 scripts/live_app_server_session_env_smoke.py
python3 scripts/live_codex_exec_session_env_smoke.py
```

On Windows, use the `.exe` binaries:

```powershell
.\bin\sky-cua-client.exe mcp
.\bin\sky-cua-service.exe daemon
```

Build and install the local Codex payload:

```bash
python3 scripts/build_plugin.py
python3 scripts/install_plugin.py --bundle-root dist/plugin/sky-cua
```

On Linux, `install_plugin.py` also runs browser preflight when the built bundle
contains `resources/chrome_preflight.py`. That preflight syncs the local
OpenAI-bundled marketplace cache for `chrome`, `browser-use`, and the
`computer-use` compatibility entry, writes native-host manifests for Google
Chrome, Brave, and Chromium, and enables `chrome@openai-bundled` plus
`browser-use@openai-bundled` in `config.toml`. When the compat root points at
the installed sky-cua payload, `computer-use@openai-bundled` is the enabled
Computer Use plugin id and `sky-cua@local` stays a disabled payload carrier; if
the compat root cannot be materialized, the installer falls back to enabling
`sky-cua@local` directly.

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
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host hermes
```

On Linux, add `--input-helper` to install and start the root
`sky-cua-input-helper.service` for helper-backed `uinput` keyboard injection
and raw pointer observation.

During local sky-cua development, add `--restart-runtime` after rebuilding and
installing so OpenCode, Pi, or another MCP host respawns from the new binaries
on the next tool call. On Linux desktop sessions this also attempts to refresh
the user AT-SPI accessibility bus before stopping any already-running installed
`sky-cua-service`/helper processes:

```bash
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host opencode --bin-dir ~/.local/bin --restart-runtime
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host pi --bin-dir ~/.local/bin --restart-runtime
python3 scripts/install_mcp_server.py --target-dir ~/.local/share/sky-cua --host hermes --bin-dir ~/.local/bin --restart-runtime
```

If the host does not reconnect automatically after the runtime is stopped,
restart or reload the host session. For Pi, run `/reload` or restart Pi.
Hermes Agent receives both `sky_cua` and `node_repl` in
`${HERMES_HOME:-~/.hermes}/config.yaml`; start a fresh session or run
`/reload-mcp`, then validate with `python3 scripts/live_hermes_mcp_smoke.py`.
Setup also disables Hermes approval prompts for commands, memory/skill writes,
delegated workers, hook registration, MCP reload, and destructive slash
commands, including removal of command deny rules that override no-prompt mode.
The installer also merges Hermes-adapted Node REPL guidance into
`${HERMES_HOME:-~/.hermes}/AGENTS.md` without replacing unrelated instructions.
OpenClaw integration is owned by `python3 install.py install`: it installs the
native Codex compatibility plugins, registers global `node_repl`, projects the
three skills, and configures native Codex for no-prompt full-auto operation.
The retired standalone `sky_cua` MCP registration and
`~/.openclaw/workspace/skills` copy are not used.

For production-like Linux GUI and non-Codex harness proof, use the Arch
`testing-vm` path in `docs/operations/gui-desktop-test-harness.md`. The VM provisioner
installs OpenCode, and `scripts/testing-vm/sync-opencode-to-vm.sh` copies the
host OpenCode config/auth without copying the host OpenCode database, logs, or
snapshots.

Deploy and distribute Codex plugin builds:

```bash
python3 scripts/deploy_plugin.py          # compatibility-only local dev deploy
python3 install.py build                  # standalone tree + archive
# From the checkout or extracted standalone artifact:
python3 install.py install
# From a clean, synchronized main checkout (defaults to a minor bump):
python3 install.py release
```

`deploy_plugin.py` is the fast local lane: it installs the built bundle into
the local Codex payload as `sky-cua@local`, retargets the
`computer-use@openai-bundled` compat plugin at it, and refreshes the installed
MCP runtime - no git, no marketplace, no Codex `plugin/install`. In compat-first
mode `computer-use@openai-bundled` is the single enabled `computer-use` plugin
and `sky-cua@local` stays a disabled payload carrier; the active payload is
chosen by retargeting the compat root, so Codex never sees duplicate
`computer-use` MCP servers. Deploys preserve already-staged binaries for other
platforms, so rebuilding on Linux does not delete Windows `.exe` binaries from
the local payload and vice versa.

When the fast development loop also needs refreshed shared skill text, run
`python3 scripts/sync_agent_skills.py`. Its default mode copies the checkout
skills behind `~/.local/share/sky-cua/skills` and leaves the portable
`~/.agents/skills` links stable. Direct links from the shared catalog to a
checkout require the explicit `--checkout-links` development opt-in.

`python3 install.py build` emits one complete standalone payload and
`dist/sky-cua-linux-x64-glibc.tar.gz`. `python3 install.py install` replaces
the fixed install root and is idempotent when repeated. `release` accepts
`--patch`, `--minor`, `--major`, or an explicit stable `--version X.Y.Z`; it
runs `just verify`, commits only the standalone version, creates an annotated
`standalone-vX.Y.Z` tag, and atomically pushes `main` plus the tag to trigger
Gitea publication and automatic Saga deployment. See
[`docs/features/release-package.md`](docs/features/release-package.md).

If the local Codex config gets stale, the durable reset procedure is documented
in `docs/runtime/mcp-boundary.md` under "Codex deploy, packaging, and config
reset".

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
python3 scripts/live_agentic_loop_smoke.py
python3 scripts/live_wayland_pointer_smoke.py
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile computer-use
```

`scripts/run_gui_testing_vm_smoke.py` is the current Linux GUI matrix runner.
It targets the Arch `testing-vm` with real guest desktop sessions rather than
embedded X servers or container-nested compositors. It only copies selected
host Codex auth/config into the VM when `--sync-codex-settings` is set.

The substantive codex tool-use gate is the `codex-cua` profile: one `codex exec`
run that exercises the full computer-use and browser-use tool surface against
live fixtures (it brings up Chrome + the sky-cua extension + native host in the
VM), enforces a deterministic coverage/no-error gate, then runs a host-side
configured host judge that scores tool-use quality and emits a triage list. Harness model
defaults live in `scripts/model_profiles.yaml`:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile codex-cua --sync-codex-settings
```

Diagnostic or legacy lanes:

```bash
python3 scripts/live_kdialog_smoke.py
```

For the full installed-plugin ChatGPT-auth E2E investigation, see
`docs/research/2026-04-codex-plugin-chatgpt-auth-expedition.md`.

For the portable runtime boundary and host-adapter expectations, see
`docs/runtime/mcp-boundary.md`.

For the current Codex Desktop compatibility surface, including the Browser
Use / Chrome companion plugin coverage and the native messaging host, see
`docs/features/codex-desktop-compat.md`.

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
  tests. Agent-loop acceptance uses `scripts/live_agentic_loop_smoke.py`;
  `codex exec` and `codex app-server` remain diagnostic probes.
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
- Windows exposes UIA child trees and semantic actions (invoke, select,
  expand/collapse, toggle, focus), and element readback (`text`,
  `numeric_value`, `supports_editable_text`) is now populated natively from
  UIA ValuePattern/TextPattern/RangeValuePattern, live-proven against
  Notepad and a Mouse Properties slider (2026-07-07; see
  `docs/features/windows-uia-automation.md`). The top-level Win32 window
  fallback (used when UIA collection itself fails) still reports these
  fields as empty, since there is no UIA pattern to read from in that path.
