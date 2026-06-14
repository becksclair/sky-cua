# Linux virtual input backend

## Status

Shipped on Linux. Last verified: 2026-05-15 on the Arch `testing-vm`
COSMIC Wayland session at 1x and at 1600x1200 with 125% display scale.

## Summary

`InputBackendKind::LinuxVirtualInput` is the Linux fallback input backend
that lets `sky-cua` drive Wayland desktops without
`org.freedesktop.portal.RemoteDesktop`, including COSMIC and Hyprland.
The MCP tool surface and coordinate contract do not change: agents still
request clicks, drags, scrolls, typing, and key presses in the same
screenshot-pixel coordinate system. The runtime detects the best available
adapter, translates coordinates correctly for display scale and monitor
layout, and chooses the right backend itself.

## Contract surface

Public model in `crates/sky-cua-platform/src/model.rs`:

- `InputBackendKind::LinuxVirtualInput` — top-level public backend kind.
  Adapter detail (`ydotool` vs direct `/dev/uinput`) is internal; not part
  of action routing.

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
- `SKY_CUA_VIRTUAL_INPUT_X` / `Y` / `WIDTH` / `HEIGHT` — desktop bounds
  override for the direct uinput adapter.

## Behavior

Backend auto-detection:

- X11 with XTest available → `XTest`.
- Wayland with RemoteDesktop portal available → `PortalRemoteDesktop`.
- Wayland without RemoteDesktop but with virtual input available →
  `LinuxVirtualInput`.
- Otherwise → `None` with structured diagnostics.

The runtime does not silently bypass an explicit portal denial. There is a
difference between "the compositor does not offer RemoteDesktop" and "a
human denied a RemoteDesktop permission prompt"; only the first falls back
to virtual input.

Inside `LinuxVirtualInput`:

- **Pointer adapter**: prefers direct absolute `/dev/uinput` when
  `/dev/uinput` is writable and desktop bounds are detected.
  `cosmic-randr list` is the preferred COSMIC bounds source; `xrandr` is a
  fallback for X11-shaped sessions; `SKY_CUA_VIRTUAL_INPUT_*` env vars are
  test overrides. Direct uinput creates an absolute tablet-style device,
  treats requested points as desktop logical coordinates within detected
  bounds, and converts logical to physical absolute-device coordinates at
  the uinput boundary using output scale.
- **Keyboard / text adapter**: `ydotool` through the per-user socket
  (`$YDOTOOL_SOCKET` or `/run/user/$UID/.ydotool_socket`). `ydotool`
  command construction inserts `--` before coordinate, wheel, and text
  payload arguments to keep negative wheel values and text beginning with
  a dash from being interpreted as ydotool flags.
- **Scrolling**: direct uinput scroll emits both `REL_WHEEL_HI_RES` and
  `REL_WHEEL`, with the sign inverted from the portal helper's discrete
  scroll direction.

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
cargo test -p sky-cua-linux virtual_input
cargo test -p sky-cua-linux env_probe
cargo test -p sky-cua-linux coords
```

Live VM acceptance via `scripts/run_gui_testing_vm_smoke.py`:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --profile wayland-pointer --desktop-env COSMIC --wayland-display wayland-1
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
