# NOTES

Durable tactical memory: proven commands, pitfalls, patterns, invariants,
environment quirks. Not transcripts, not stale TODO lists, not artifact
dumps. Per-feature artifact paths live in
[`docs/features/<slug>.md`](docs/features/) Verification sections.

## Environment quirks (host)

- `XDG_SESSION_TYPE` lies on Asgard: remote shells report `tty` while the
  real stack is KDE 6 Wayland. Corroborate with live compositor and portal
  processes; backend detection must prefer a valid `WAYLAND_DISPLAY` over
  the env var and a stale `DISPLAY=:0`.
- When diffing `virsh screenshot` PNGs, convert to RGB first. RGBA
  `getbbox()` can hide real changes; the alpha channel is not proof signal.

## VM session management

- Never launch Plasma / COSMIC / Hyprland under `dbus-run-session` in the
  testing-vm: compositor services land on a private bus while SSH talks to
  `/run/user/<uid>/bus`, breaking KWin DBus discovery. Set
  `DBUS_SESSION_BUS_ADDRESS` to the real bus, import the desktop env, then
  exec the session.
- Disable the KDE screen locker and PowerDevil in the test image (a blanked
  guest looks like a fullscreen-black overlay failure). Launch `virt-viewer`
  detached: `setsid -f virt-viewer --connect qemu:///session testing-vm`.
- Switch VM sessions with `scripts/testing-vm/select-session.sh <session>`:
  it stops the inactive display manager and cleans stale compositor and
  sky-cua processes (otherwise GNOME and Plasma run side by side, or
  `kwin_wayland` survives on `wayland-0` while Hyprland uses `wayland-1`).
  After switching, refresh user portal state before preauthorization;
  virtual input scopes `cosmic-randr` to COSMIC and timeout-protects bounds
  helpers.

## Compositor and capture gotchas

- Real-session KWin `ScreenShot2` returns `NoAuthorized` over SSH even with
  `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1`. Use host-side libvirt
  framebuffer capture for production KWin pixel proof; nested KWin can use
  `ScreenShot2`.
- Hyprland: layer surfaces must receive a configure event before an overlay
  host may draw (stricter than KWin); `grim` must name the active nonzero
  output (`grim -o Virtual-1`) or it fails when a zero-sized output exists —
  smokes derive `HYPRLAND_INSTANCE_SIGNATURE` and pick the focused monitor
  from `hyprctl monitors -j`.
- KWin user-level compiled-effect discovery is blocked on Plasma 6 even with
  explicit `loadEffect` + reconfigure; production proof is a system install
  under `/usr`. See
  `docs/research/2026-05-kwin-effect-discovery.md`.
- Fullscreen GTK windows can report a logical width larger than the portal
  stream width — keep explicit-coordinate smoke targets well inside the
  monitor. On GNOME they can also exceed the framebuffer height (clipped,
  centered content); keep that adjustment GNOME-scoped.
- GNOME RemoteDesktop scroll: smooth portal calls are accepted but move
  nothing — send discrete wheel steps, sign inverted from the XTest helper.
  The EIS lane uses libei semantics (positive Y scrolls down); invert at the
  EIS boundary so negative `delta_y` still scrolls down (`wayland-pointer`
  smoke: `scroll_delta_y=180` for `delta_y=-180`).
- GNOME RemoteDesktop keyboard injection must use EIS, resolving keysyms via
  the compositor-provided XKB keymap; hard-coded evdev positions regress
  non-US layouts and uppercase keys. The `wayland-pointer` smoke must
  require `PortalEisInputUsed` (no fallback) for every action or it can pass
  through the unproved legacy path.
- KDE Screenshot portal returns a local `file://` URI; copy it into
  `/run/user/<uid>/sky-cua/captures/`.

## KWin cursor shim

Behavior, install flow, pointer-position telemetry, ghost-cursor background,
and the idle auto-hide watchdog chain are documented in
`docs/features/agent-cursor-overlay.md` and
`docs/features/compositor-cursor-hiding.md`. Tactical reminders:

- `SKY_CUA_PORTAL_EIS=never` isolates the legacy portal lane when debugging
  pointer mapping.
- A replaced effect `.so` never hot-reloads under the same id (no dlclose;
  verified 2026-06-10 via BuildId). The installer avoids this with rotating
  ids and keeps only the active generated id after a successful deploy; confirm
  with `install_kwin_effect.py --status`; rerun the deploy after KWin updates.
- NEVER restart `plasma-kwin_wayland.service` from tooling: it can kill the
  whole session, or come back without re-claiming the `org.kde.KWin` DBus
  name (only `org.kde.KWinWrapper`), leaving effects DBus dead.
- Stale shim hiding the cursor:
  `qdbus6 org.kde.KWin /com/skycua/AgentCursor com.skycua.AgentCursor.Hide`.
- For live overlay visibility, query the overlay host `capabilities` reply —
  pre-fix effect builds report stale `StateJson`.

## Input adapters

- Direct `/dev/uinput` pointer injection was retired after live compositor
  testing showed it was not reliable enough to keep as a production or
  fallback pointer path. The privileged helper still owns `/dev/uinput` for
  keyboard injection and raw pointer observation; Linux virtual pointer
  actions use ydotool when selected.
- `ydotool` argv must insert `--` before coordinate, wheel, and text payload
  arguments, or negative values and `-`-prefixed text parse as flags
  (unit-tested).
- The i3/X11 VM can start Xorg on `:1` while the user systemd env still says
  `DISPLAY=:0`/`wayland-0`. The i3 profile reconstructs `DISPLAY` and
  `XAUTHORITY` from the active Xorg command and unsets Wayland.

## AT-SPI and selectors

- Cache the AT-SPI connection in the Linux backend; reopening per request
  can wedge under portal-driven loops.
- Selector matching is score-based: PID dominates; class/instance/executable
  names, desktop-file stem, exact title, and focus all rank. Never
  title-only correlation — KDE service roots (`ksmserver`, `kaccess`) steal
  title matches; penalize service executables in focus heuristics.
- KDE background-window discovery:
  `org.kde.KWin /WindowsRunner org.kde.krunner1.Match` for UUIDs plus
  `getWindowInfo` for metadata. Never call `queryWindowInfo` — it is an
  interactive window picker (blocks on a click, `UserCancel` otherwise).
  Active-window readback and verified activation go through KWin scripting
  with a `callDBus` result callback to the daemon's unique bus name
  (`kwin_script.rs`, kdotool pattern); KWin exposes no foreign-toplevel
  Wayland protocol and no active-window DBus getter.
- On KDE 6 Wayland, `zenity` is a reliable semantic smoke fixture; `kdialog`
  can be visible without surfacing through AT-SPI remotely. A fullscreen GTK
  fixture works as a pointer target even when absent from `list_apps`. For
  `xmessage`, always set `-title`.
- X11-only windows expose a synthetic root with bounds but possibly no
  semantic children. The fallback tree's blunt roles plus real bounds are
  honest; inventing widget semantics from geometry is not.

## Plugin packaging and host loading

- `.mcp.json` keeps the MCP server name `computer-use` (Codex contract);
  Claude Code reserves that name, so its lane registers user-scope as
  `sky-cua` via `install_mcp_server.py --host claude-code`.
- Never launch the client via `/bin/sh -lc` (login-shell junk corrupts
  JSON-RPC); use `/bin/sh -c "exec ./bin/sky-cua-client mcp"`. Codex stdio
  MCP is newline-delimited JSON-RPC; the client accepts both framings and
  mirrors what it saw.
- Codex-launched servers do not inherit the desktop session env unless
  `.mcp.json` lists it in `env_vars` (at least `DBUS_SESSION_BUS_ADDRESS`,
  `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`, `DISPLAY`, `XDG_SESSION_TYPE`,
  `XDG_CURRENT_DESKTOP`, `DESKTOP_SESSION`); runtime repair is the fallback
  (`docs/features/session-env-repair.md`).
- Advertised tool `inputSchema` must never carry top-level `allOf`/`oneOf`/
  `anyOf`/`not`: the Anthropic Messages API rejects them and Claude Code then
  silently drops the tool (it never surfaces in `ToolSearch`), so under
  Claude Code the grouped desktop verbs vanish and the agent reaches for the
  banned built-in `computer-use`. The registry advertises flattened schemas and
  keeps the rich exact-branch schemas in `validation_schemas` for runtime
  enforcement (`definitions.rs`). Codex accepts top-level `allOf`; Claude Code/
  the API do not — guarded by `advertised_schemas_have_no_top_level_composition`.
- Compact MCP surface closeout: direct desktop/browser/phone tools stay
  removed; live smoke harnesses call the grouped tools
  (`observe`, `list_resources`, `capture_screen`, `desktop_pointer`,
  `desktop_keyboard`, `desktop_scroll`, `desktop_set_value`). The focused
  pre-install gate is `cargo fmt --check && cargo nextest run -p sky-cua-client &&
  uv run ruff format --check scripts && uv run ruff check scripts && uv run
  basedpyright && uv run pytest scripts/test_probe_mcp_tool_surface.py
  scripts/test_live_smoke_helpers.py scripts/test_gui_testing_vm.py`, plus
  `git diff --check`.
- Single channel id is `sky-cua@local`; the Heliasar marketplace and publish
  flow were retired. On Linux `computer-use@openai-bundled` (the compat plugin)
  is the enabled id and `sky-cua@local` stays a disabled payload carrier (the
  compat root's `.mcp.json` points at it). The off-compat/Windows fallback
  enables `sky-cua@local` directly. Cheap proof is one `computer-use` server in
  `codex app-server` `mcpServerStatus/list`. Local dev deploy:
  `scripts/deploy_plugin.py`; clean-machine install: `scripts/package.py` then
  `python3 install.py` on the target.
  History: `docs/research/2026-04-codex-plugin-chatgpt-auth-expedition.md`.

## Smoke harnesses

- `codex-cua` browser-phase wedge (`Page.enable`/`Page.navigate` timeouts,
  "Detached"): it is the **Codex extension's `chrome.debugger` relay**, not
  Chrome, not our Rust, not the VM. Raw CDP and direct `chrome.debugger` both
  work on the same Chrome; only the SW relay wedges. Do NOT tune timeouts (makes
  it worse), reboot (does nothing), or blame our code (reverts fail identically).
  Each run now leaves `chrome-debug.log`/`chrome-stderr.log` in the judge dir.
  Full evidence, probes, ruled-out list, and next steps:
  [`docs/research/2026-06-chrome-debugger-relay-wedge.md`](docs/research/2026-06-chrome-debugger-relay-wedge.md).
- The client Unix-socket read timeout must stay generous (60s) for real
  portal approval UX.
- For `codex exec` plugin tests, validate the JSONL transcript for an actual
  `mcp_tool_call` against server `computer-use`; the final JSON blob alone
  can describe shell-hack completions. The acceptance harness is
  `scripts/live_agentic_loop_smoke.py`; `codex exec` is diagnostic. For
  app-server diagnostic harnesses, close the `codex app-server` child before
  draining `stderr` or cleanup hangs.
- OpenClaw native-codex turns: `mcp.servers.<name>.codex.defaultToolsApprovalMode`
  must be `approve` (codex semantics: always approved, no user interaction).
  `auto` prompts on every sky-cua call because codex treats unannotated MCP
  tools as destructive + open-world. Post-deploy proof:
  `scripts/live_openclaw_mcp_smoke.py [--agent-turn]`; after config changes run
  `openclaw mcp reload` or the gateway keeps the cached runtime. The installer
  also pins `[mcp_servers.sky_cua]` (with `default_tools_approval_mode =
  "approve"`) into each agent's `codex-home/config.toml`, which codex
  app-server applies process-wide. Agent-turn smokes must use a fresh session
  key per run; a reused key resumes a codex thread with stale MCP state.
- Startup health must never require cross-host equality of per-host env:
  each MCP host spawns sky-cua-client with a different PATH and browser
  env, so exact-equality checks let the first spawning host starve every
  other host under the daemon singleton. Health equality is scoped to
  `GRAPHICAL_SESSION_ENV_KEYS` (never PATH); browser keys reject only when
  both sides pin different values. Accepted tradeoffs: a daemon spawned
  with a genuinely broken PATH stays "healthy", and a daemon pinned to one
  browser serves clients with no pin. The startup failure message includes
  the last per-poll health error — read it before strace.
- Machine-level settings live in `~/.config/sky-cua/sky-cua.toml`
  (`%APPDATA%\sky-cua\sky-cua.toml` on Windows), starting with `browser`
  selection. Env (`SKY_CUA_BROWSER`) overrides the file per process;
  `SKY_CUA_CONFIG_PATH` overrides the file location for tests. Prefer the
  file over baking selection env into per-host MCP registrations.
  Decision: per-request browser selection is rejected — the daemon uses
  its machine config, full stop; do not add per-call selection plumbing.
- `sky-cua-service` must handle SIGTERM through normal teardown. Process
  cleanup must match `sky-cua-service`, the full overlay-host argv, and the
  truncated comm name `sky-cua-overlay`. If the live KDE smoke fails
  mysteriously, kill stale `sky-cua-service` and re-run first.
- Desktop cursor preview/testing: `sky-cua-overlay-host playground [--backdrop
  transparent|grid|dark|light]` opens an interactive Wayland layer-shell surface
  that hides the system cursor and draws the real agent cursor at the pointer
  (Ctrl-C to quit). Wayland-only; previews the production layer-shell visual
  path; KWin supplies the optional cursor-hide and pointer-position shim.
  Bounded capture:
  wrap in `timeout -k 2 4 ...`, shoot with
  `spectacle -b -n -f -o` (grim has no wlr-screencopy on KWin). Full notes:
  `docs/features/agent-cursor-overlay.md` → Pointer playground.

## Portal state

- Persisted Wayland approval reuse lives in `portal-tokens.json` under
  `XDG_STATE_HOME/sky-cua` (or `~/.local/state/sky-cua`); the RemoteDesktop
  lane rotates the restore token on session start. Reset via
  `sky-cua-client clear-portal-tokens` or `scripts/reset_portal_tokens.py`.

## Phone companion (Android)

- Companion has no runtime permissions; setup = two service enablements the
  install-bearing bootstrap does over ADB. Accessibility: read-merge-write
  `settings put secure enabled_accessibility_services <merged>` + `settings put
  secure accessibility_enabled 1` — binds immediately. Verify with `dumpsys
  accessibility` ("Bound services" line; `capabilities=161` = retrieve + perform
  gestures + screenshot).
- Notification listener: a bare `settings put secure
  enabled_notification_listeners` sets the list but may NOT bind until the next
  reconcile (health then spuriously reads it off). Use `cmd notification
  allow_listener <pkg>/<cls>` — additive, binds immediately. Verify a live
  `INotificationListener$Stub$Proxy` in `dumpsys notification`.
- Always read-merge these `:`-lists; never blind-`put` (the emulator ships
  Google notification listeners). Samsung One UI may gate sideloaded
  accessibility behind a manual "Restricted settings" confirmation the ADB
  write cannot satisfy — the host then emits `PhoneCompanion*ManualSetup` and
  opens the on-device Accessibility screen.
- RPC token delivery: a host file pushed to `/sdcard/Android/data/<pkg>/cache/`
  is UNREADABLE by the app on Android 11+ (per-app storage mount namespaces;
  `run-as <pkg> cat` of the shell-pushed file gives "Permission denied"), so the
  companion's `SetupActivity` never started the RPC server. Deliver the token as
  an `am start --es sky_cua_rpc_token <tok>` intent extra instead.
- Installed signing-cert SHA-256 is NOT in `dumpsys package` on API 28+ (only a
  short `signatures:[<hash>]`), so the host can't verify the installed cert. The
  signature gate refuses only a *readable* mismatch; an unreadable cert proceeds
  with `signature_matches_expected=false`. Expected cert/sha/version come from
  the bundled `resources/android/phone-companion.json` sidecar (env overrides).
- Verify companion RPC up: `cat /proc/net/tcp /proc/net/tcp6 | grep -i BA43`
  (47683=0xBA43) on the device. Host forward: `adb forward --list | grep 47683`.
