# Linux virtual input backend

## Status

Partial on Linux. Direct uinput pointer injection was retired after live KDE
testing showed it was not a reliable compositor input path.

## Summary

`InputBackendKind::LinuxVirtualInput` is the Linux virtual-device input
backend. It uses ydotool for pointer actions and either the privileged helper
or ydotool for keyboard/text actions, avoiding RemoteDesktop/EIS input startup
when a usable virtual input path is available.
The MCP tool surface and coordinate contract do not change: agents still
request clicks, drags, scrolls, typing, and key presses in the same
screenshot-pixel coordinate system. The runtime detects the best available
adapter, translates coordinates correctly for display scale and monitor
layout, and chooses the right backend itself.

## Contract surface

Public model in `crates/sky-cua-platform/src/model.rs`:

- `InputBackendKind::LinuxVirtualInput` — top-level public backend kind.
  Adapter detail (`privileged_helper` for keyboard-only helper mode or
  `ydotool`) is reported through doctor diagnostics, not a separate public
  backend enum.

The agent-facing coordinate contract is unchanged:

1. With a captured `snapshot_id`, coordinates are screenshot pixels for that
   snapshot; structure-only snapshot ids still scope element indexes.
2. Without `snapshot_id`, coordinates are interpreted as current desktop
   input coordinates.
3. Tool schemas do not expose backend, compositor, monitor scale, or
   adapter parameters.
4. Tool results and diagnostics may report which backend was selected, but
   that is observation, not an input requirement.

Operator override (diagnostics and tests only):

- `SKY_CUA_INPUT_BACKEND` — `auto` (default), `portal`, `x11`,
  `linux-virtual`, or `none`.
- `SKY_CUA_INPUT_HELPER_SOCKET` — privileged helper socket override
  (default `/run/sky-cua/input-helper.sock`).
- `SKY_CUA_XKB_RULES` / `MODEL` / `LAYOUT` / `VARIANT` / `OPTIONS` —
  explicit XKB keymap inputs for helper-backed text and key resolution.

## Behavior

Backend auto-detection:

- X11 with XTest available → `XTest`.
- Wayland with virtual input available → `LinuxVirtualInput`.
- Wayland without virtual input but with RemoteDesktop portal available →
  `PortalRemoteDesktop`.
- Otherwise → `None` with structured diagnostics.

The runtime does not silently bypass an explicit portal denial. There is a
difference between "the compositor does not offer RemoteDesktop" and "a
human denied a RemoteDesktop permission prompt"; only the first falls back
to virtual input.

Inside `LinuxVirtualInput`, adapter probe order is:

- **Ydotool**: the only Linux virtual pointer adapter, through the per-user
  socket (`$YDOTOOL_SOCKET` or `/run/user/$UID/.ydotool_socket`).
- **Privileged helper**: keyboard-only injection fallback. A root
  `sky-cua-input-helper` service owns `/dev/uinput`, exposes
  `/run/sky-cua/input-helper.sock`, creates a persistent virtual keyboard
  device, and accepts JSON-lines commands carrying already-resolved evdev
  key events. Its `observe_pointer` stream remains available for non-exact
  pointer observation; it does not inject pointer events.
- **Keyboard / text resolution**: helper-backed keyboard actions compile an
  XKB keymap from `SKY_CUA_XKB_*`, then `XKB_DEFAULT_*`, then
  `setxkbmap -query` / `localectl status`, then defaults, and send evdev
  press/release events to the helper. The helper does not parse characters
  or key names.

Snapshot-based actions map screenshot pixels to desktop logical
coordinates through `capture.pixel_size` and `capture.logical_rect`,
including monitor offsets. The mapping fails closed if either value is
missing rather than pretending screenshot pixels are desktop coordinates.

Session detection: SSH/TTY automation with a valid `WAYLAND_DISPLAY` is
treated as Wayland even when `XDG_SESSION_TYPE=tty` and `DISPLAY=:0` are
both present. Without that fix, COSMIC smoke runs would be misrouted
toward X11.

## Source paths

- `crates/sky-cua-platform/src/model.rs` — `InputBackendKind::LinuxVirtualInput`
- `crates/sky-cua-input-helper/` — privileged helper protocol, uinput
  device code, and helper server
- `crates/sky-cua-linux/src/virtual_input.rs` — adapter probing, selection,
  command construction
- `crates/sky-cua-linux/src/env_probe.rs` — backend auto-detection
- `crates/sky-cua-linux/src/coords.rs` — coordinate plane conversions
- `crates/sky-cua-linux/src/actions/targeting.rs` — screenshot-pixel to
  desktop-logical action coordinate mapping
- `crates/sky-cua-linux/src/actions/mod.rs` — Linux click, secondary-click,
  scroll, drag, type_text, press_key, and set_value fallback routing
- `crates/sky-cua-linux/src/backend.rs` — Linux backend orchestration and
  runtime adapter for the action executor
- `scripts/live_wayland_pointer_smoke.py` — fullscreen GTK pointer fixture
- `scripts/testing-vm/profiles/wayland-pointer.sh` —
  `wayland-pointer-scaled` repeatable VM profile

## Verification

Focused tests:

```bash
cargo test -p sky-cua-platform
cargo test -p sky-cua-input-helper
cargo test -p sky-cua-linux virtual_input
cargo test -p sky-cua-linux env_probe
cargo test -p sky-cua-linux coords
```

Live VM acceptance via `scripts/run_gui_testing_vm_smoke.py`:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --profile wayland-pointer --desktop-env COSMIC --wayland-display wayland-1
python3 scripts/run_gui_testing_vm_smoke.py --profile text-readback --desktop-env KDE --wayland-display wayland-0
python3 scripts/run_gui_testing_vm_smoke.py --profile wayland-pointer-scaled --desktop-env COSMIC --wayland-display wayland-1
```

Latest accepted artifacts:

- COSMIC 1x click / drag / scroll / `type_text` / `press_key`:
  `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T092606Z`
- COSMIC scaled (1600x1200, 125%): `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260515T093737Z`

The `wayland-pointer-scaled` profile sets COSMIC scale, runs the full
input smoke, and restores 1280x800 at 100% afterward.

## Known limitations

- COSMIC's `xdg-desktop-portal-cosmic` does not currently advertise
  RemoteDesktop. ScreenCast and Screenshot are present, so portal capture
  works while portal input injection does not. This is a compositor /
  portal capability gap, not a permission issue.
- `ydotool` is unsuitable as the precise pointer adapter on COSMIC: its
  virtual device is relative-only in `/proc/bus/input/devices`, and
  `ydotool mousemove --absolute` lands at accelerated/doubled coordinates.
  See [`docs/research/2026-05-ydotool-vs-direct-uinput.md`](../research/2026-05-ydotool-vs-direct-uinput.md).
- The fixture's center scroller can be too low in oversized scaled
  fullscreen GTK allocations; the fixture exposes a `scroll_safe` upper
  point for portable scaled-scroll proof.
- X11 without XTest but with virtual input available is not part of the
  first acceptance gate. It may be added as a fallback once tests prove
  coordinate behavior under X11.

## Related

- Research: [`docs/research/2026-05-ydotool-vs-direct-uinput.md`](../research/2026-05-ydotool-vs-direct-uinput.md)
- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Originating ExecPlan (retired into this feature doc; see git history for `plans/linux_virtual_input_backend.md`).
