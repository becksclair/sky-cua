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
  under `/usr` plus Plasma restart. See
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

## KWin agent-cursor effect

Behavior, install flow, ghost-cursor background, and the idle auto-hide
watchdog chain are documented in `docs/features/agent-cursor-overlay.md`
and `docs/features/compositor-cursor-hiding.md`. Tactical reminders:

- `SKY_CUA_PORTAL_EIS=never` isolates the legacy portal lane when debugging
  pointer mapping.
- A replaced effect `.so` never hot-reloads (no dlclose; verified 2026-06-10
  via BuildId) — after the user's session restart, confirm with
  `install_kwin_effect.py --status`; rerun the deploy after KWin updates.
- NEVER restart `plasma-kwin_wayland.service` from tooling: it can kill the
  whole session, or come back without re-claiming the `org.kde.KWin` DBus
  name (only `org.kde.KWinWrapper`), leaving effects DBus dead.
- Stale build hijacking the cursor:
  `qdbus6 org.kde.KWin /com/skycua/AgentCursor com.skycua.AgentCursor.Hide`.
- For live overlay visibility, query the overlay host `capabilities` reply —
  pre-fix effect builds report stale `StateJson`.

## Input adapters

- `ydotool` cannot be the precise pointer adapter on COSMIC (relative-only
  device; `--absolute` lands at accelerated coordinates). Use direct
  absolute `/dev/uinput` for pointer, ydotool for keyboard/text. See
  `docs/research/2026-05-ydotool-vs-direct-uinput.md`. Direct uinput scroll
  on COSMIC needs both `REL_WHEEL_HI_RES` and `REL_WHEEL`, sign inverted
  from the portal helper.
- `ydotool` argv must insert `--` before coordinate, wheel, and text payload
  arguments, or negative values and `-`-prefixed text parse as flags
  (unit-tested).
- `cosmic-randr list` is the preferred COSMIC bounds source; `xrandr` is the
  X11 fallback; `SKY_CUA_VIRTUAL_INPUT_X/Y/WIDTH/HEIGHT` are test overrides.
- At fractional scale, direct uinput must multiply desktop logical points by
  output scale, or clicks report success while the target never sees them.
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
- KDE background-window discovery without focus:
  `org.kde.KWin /WindowsRunner org.kde.krunner1.Match` for UUIDs plus
  `getWindowInfo` for metadata. Avoid `queryWindowInfo` (can `UserCancel`
  or hang under service environments).
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
- Marketplace entries keep the full scaffold (`source`, `policy.*`,
  `category`; Heliasar defaults `AVAILABLE`/`ON_INSTALL`/`Coding`). Release
  checkout is `~/projects/heliasar-marketplace` (`~/.agents/...` is legacy).
  Local release deploy expects `sky-cua@Heliasar` enabled, `sky-cua@debug`
  and `computer-use@openai-bundled` disabled; cheap proof is one
  `computer-use` server in `codex app-server` `mcpServerStatus/list`.
  History: `docs/research/2026-04-codex-plugin-chatgpt-auth-expedition.md`.

## Smoke harnesses

- The client Unix-socket read timeout must stay generous (60s) for real
  portal approval UX.
- For `codex exec` plugin tests, validate the JSONL transcript for an actual
  `mcp_tool_call` against server `computer-use`; the final JSON blob alone
  can describe shell-hack completions. The acceptance harness is
  `scripts/live_app_server_smoke.py`; `codex exec` is diagnostic. Close the
  `codex app-server` child before draining `stderr` or cleanup hangs.
- OpenClaw native-codex turns: `mcp.servers.<name>.codex.defaultToolsApprovalMode`
  must be `auto`. `approve` defers to `appServer.approvalPolicy`, and with
  `"never"` (unattended default) tools project but every call is blocked —
  `openclaw mcp probe` still passes. Post-deploy proof:
  `scripts/live_openclaw_mcp_smoke.py [--agent-turn]`; after config changes run
  `openclaw mcp reload` or the gateway keeps the cached runtime.
- `sky-cua-service` must handle SIGTERM through normal teardown. Process
  cleanup must match `sky-cua-service`, the full overlay-host argv, and the
  truncated comm name `sky-cua-overlay`. If the live KDE smoke fails
  mysteriously, kill stale `sky-cua-service` and re-run first.

## Portal state

- Persisted Wayland approval reuse lives in `portal-tokens.json` under
  `XDG_STATE_HOME/sky-cua` (or `~/.local/state/sky-cua`); the RemoteDesktop
  lane rotates the restore token on session start. Reset via
  `sky-cua-client clear-portal-tokens` or `scripts/reset_portal_tokens.py`.
