# NOTES

Durable tactical memory: proven commands, pitfalls, patterns, invariants,
environment quirks. Not transcripts, not stale TODO lists, not artifact
dumps. Per-feature artifact paths live in
[`docs/features/<slug>.md`](docs/features/) Verification sections.

## Environment quirks (host)

- `XDG_SESSION_TYPE` lies on Asgard: the remote shell reports `tty` while
  the real stack is KDE 6 Wayland. Corroborate with live compositor and
  portal processes, not the env var alone.
- SSH/TTY automation can present a Wayland session as `XDG_SESSION_TYPE=tty`
  plus a valid `WAYLAND_DISPLAY` and a stale `DISPLAY=:0`. Backend
  detection must prefer the live Wayland display in that shape.
- When diffing `virsh screenshot` PNGs, convert to RGB before comparing.
  RGBA `getbbox()` can hide real changes because the screenshot alpha
  channel is not a useful proof signal.

## VM session management

- Do not launch Plasma / COSMIC / Hyprland under `dbus-run-session` in
  the testing-vm. That puts compositor services on a private bus while
  SSH talks to `/run/user/<uid>/bus`, breaking KWin DBus discovery. Set
  `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/<uid>/bus`, import the
  desktop env, then exec the session.
- Disable the KDE screen locker and PowerDevil in the test image for
  framebuffer cursor proofs. A blanked guest looks like a fullscreen-
  black overlay failure.
- Launch `virt-viewer` detached:
  `setsid -f virt-viewer --connect qemu:///session testing-vm >/tmp/sky-cua-virt-viewer.log 2>&1`.
- VM session switching must stop the inactive display manager and clean
  stale compositor / sky-cua processes before restarting the selected
  one. Use `scripts/testing-vm/select-session.sh <session>`. Otherwise
  GNOME and Plasma can run side by side, or `kwin_wayland` survives a
  Plasma→Hyprland switch on `wayland-0` while Hyprland uses `wayland-1`.
- After switching, refresh user portal state before preauthorization /
  profile startup. The runner imports the target desktop env and stops
  portal services; virtual input scopes `cosmic-randr` to COSMIC and
  timeout-protects bounds helpers.

## Compositor and capture gotchas

- Real-session KWin `ScreenShot2` is not a reliable SSH smoke capture
  path: it returns `NoAuthorized` even with
  `KWIN_SCREENSHOT_NO_PERMISSION_CHECKS=1`. Use host-side libvirt
  framebuffer capture for production KWin pixel proof; nested KWin can
  still use `ScreenShot2`.
- Hyprland is stricter than KWin about layer-shell configure state. An
  overlay host must draw only to layer surfaces that have received a
  configure event.
- Hyprland `grim` capture must name the active nonzero output (`grim -o
  Virtual-1`). `grim` without `-o` fails with `failed to create buffer`
  when a zero-sized output is present. Smokes derive
  `HYPRLAND_INSTANCE_SIGNATURE` and pick the focused nonzero monitor
  from `hyprctl monitors -j`.
- KWin user-level compiled-effect discovery is blocked on Plasma 6 even
  with explicit `loadEffect` and reconfigure. Production proof is system
  install under `/usr` plus Plasma restart. See
  `docs/research/2026-05-kwin-effect-discovery.md`.
- Fullscreen GTK windows can report a logical width larger than the
  portal stream width. Keep explicit-coordinate smoke targets
  comfortably inside the monitor.
- GNOME RemoteDesktop accepts smooth portal scroll calls without moving
  the GTK scroller. Send discrete wheel steps with the sign inverted
  from the XTest helper.
- GNOME GTK fullscreen allocations can be taller than the framebuffer
  (e.g. 1280x973 on 1280x800) with centered / clipped vertical content.
  Keep that adjustment GNOME-scoped; KDE allocations behave differently.
- KDE Screenshot portal returns a local `file://` URI; copy it into
  `/run/user/<uid>/sky-cua/captures/`.

## Input adapters

- `ydotool` is unusable as the precise pointer adapter on COSMIC: its
  virtual device is relative-only, and `mousemove --absolute` lands at
  accelerated coordinates. Use direct absolute `/dev/uinput` for
  pointer, ydotool for keyboard / text. See
  `docs/research/2026-05-ydotool-vs-direct-uinput.md`.
- Direct uinput scroll on COSMIC needs both `REL_WHEEL_HI_RES` and
  `REL_WHEEL`, sign inverted from the portal helper.
- `ydotool` argv must insert `--` before coordinate, wheel, and text
  payload arguments. Otherwise negative wheel values and text starting
  with `-` get parsed as flags. argv has unit-test coverage.
- `cosmic-randr list` is the preferred COSMIC bounds source for the
  direct uinput device. `xrandr` is the X11-shaped fallback;
  `SKY_CUA_VIRTUAL_INPUT_X/Y/WIDTH/HEIGHT` are test overrides.
- At fractional scale, direct uinput must multiply desktop logical
  points by output scale before emitting absolute uinput values.
  Otherwise click success is reported while the target never receives
  the event.
- The i3/X11 VM can start Xorg on `:1` even though the user systemd
  env still says `DISPLAY=:0` and `WAYLAND_DISPLAY=wayland-0` from a
  previous Plasma session. The i3 profile reconstructs `DISPLAY` and
  `XAUTHORITY` from the active Xorg command, synthesizes a temporary
  Xauthority, and unsets Wayland before running the smoke.

## AT-SPI and selectors

- Cache the AT-SPI connection in the Linux backend. Reopening per
  request can wedge under portal-driven loops.
- Selector matching is score-based, not first-match-wins. PID dominates;
  class name, instance name, executable name, desktop-file stem, exact
  title, and focused-window status all help rank candidates. No
  title-only correlation, ever — KDE service roots like `ksmserver` and
  `kaccess` will steal title matches.
- A naive AT-SPI focus heuristic happily picks session services like
  `ksmserver`. Penalize obvious service executables; prefer roots with
  real window titles.
- KDE background-window discovery without focus uses
  `org.kde.KWin /WindowsRunner org.kde.krunner1.Match` for UUIDs plus
  `org.kde.KWin.getWindowInfo` for metadata. Do not depend on
  `org.kde.KWin.queryWindowInfo`; under Codex-launched service
  environments it can return `UserCancel` or hang.
- On KDE 6 Wayland, `zenity` is a reliable semantic smoke fixture.
  `kdialog` can be visibly present without surfacing through AT-SPI from
  a remote harness.
- A fullscreen GTK fixture can be a useful pointer smoke target even
  when it does not appear in `list_apps`.
- X11-only windows appear in `list_apps` and expose a synthetic root
  element with bounds, but may have no semantic children. Physical
  targeting is not semantic parity. The X11 fallback tree's blunt roles
  (`x11_container`, `x11_leaf_region`, `x11_action_region`) plus real
  bounds are honest; inventing widget semantics from geometry is not.
- For an `xmessage` smoke, set `-title`. Without it, `WM_NAME` stays
  the default `xmessage` and title-based matching looks more broken than
  it is.

## Plugin packaging and Codex loading

- The full plugin-loading recipe and historical investigation
  (ChatGPT-auth + `apps=false` + bypass flag, host-tool-surface
  contamination, etc.) lives in
  `docs/research/2026-04-codex-plugin-chatgpt-auth-expedition.md`.
- `.mcp.json` keeps the MCP server name `computer-use`. Do not launch
  it via `/bin/sh -lc`; login-shell startup can emit junk like
  `not a tty` and corrupt the JSON-RPC stream. Use
  `/bin/sh -c "exec ./bin/sky-cua-client mcp"`.
- Codex's stdio MCP transport uses newline-delimited JSON-RPC (`rmcp`),
  not `Content-Length` framing. `sky-cua-client mcp` accepts both and
  mirrors the framing it saw on input.
- Codex-launched plugin servers do not inherit the desktop session
  environment unless `.mcp.json` lists it in `env_vars`. Keep at least
  `DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR`, `WAYLAND_DISPLAY`,
  `DISPLAY`, `XDG_SESSION_TYPE`, `XDG_CURRENT_DESKTOP`,
  `DESKTOP_SESSION`. (Runtime repair exists as a fallback; see
  `docs/features/session-env-repair.md`.)
- Marketplace plugin entries keep the full scaffold: `source`,
  `policy.installation`, `policy.authentication`, `category`. For the
  Heliasar `sky-cua` release marketplace, default to `AVAILABLE`,
  `ON_INSTALL`, `Coding`.
- Release marketplace checkout is `~/projects/heliasar-marketplace`.
  `~/.agents/sky-cua-marketplace` is legacy; do not publish there.
- Local release deploy expects `sky-cua@Heliasar` enabled, `sky-cua@debug`
  disabled, `computer-use@openai-bundled` disabled. Cheap proof:
  `codex app-server` `mcpServerStatus/list` shows one `computer-use`
  server with the sky-cua tool set.

## Smoke harnesses

- The client Unix-socket read timeout must be generous enough for real
  portal approval UX. Sixty seconds is the current setting; tighten
  only with a clearer operator-facing diagnostic when the prompt is
  missed.
- For `codex exec` plugin tests, validate the JSONL transcript for an
  actual `mcp_tool_call` against server `computer-use`. The final JSON
  blob alone can describe a workflow the model completed with shell
  hacks.
- The installed-plugin acceptance harness is
  `scripts/live_app_server_smoke.py` against `codex app-server`.
  `codex exec` is a diagnostic probe.
- When driving `codex app-server` directly, close the child process
  before draining `stderr`. The reverse order hangs Python harness
  cleanup forever.
- `sky-cua-service` must handle SIGTERM through normal teardown.
  Cleanup must match `sky-cua-service`, the full overlay-host argv (on
  `sky-cua-overlay-host`), and the truncated Linux comm name
  `sky-cua-overlay`.
- If the live KDE smoke starts failing mysteriously, kill stale
  `sky-cua-service` and re-run before inventing a theory.

## Portal state

- Persisted Wayland approval reuse lives in `portal-tokens.json` under
  `XDG_STATE_HOME/sky-cua` (or `~/.local/state/sky-cua`). The
  RemoteDesktop lane rotates the restore token on successful session
  start. Reset via `sky-cua-client clear-portal-tokens` or
  `python3 scripts/reset_portal_tokens.py`.
