# ydotool vs direct /dev/uinput as the COSMIC pointer adapter

## Context

COSMIC Wayland sessions on the Arch testing-vm do not expose
`org.freedesktop.portal.RemoteDesktop`, so `sky-cua` needed a virtual
input adapter to drive the desktop. `ydotool` was the obvious first
candidate because it is widely packaged, has a per-user socket
(`/run/user/$UID/.ydotool_socket`), and the existing `sky-cua` doctor /
setup already probed for it.

This research records the live calibration that rejected `ydotool` as the
pointer adapter on COSMIC, kept it as the keyboard / text adapter, and
moved the pointer path to a direct absolute `/dev/uinput` adapter.

## Investigation

In the COSMIC VM, `ydotool` was confirmed installed and able to move /
click the pointer through `/run/user/1000/.ydotool_socket`. However:

- The ydotool virtual device appears as **relative-only** in
  `/proc/bus/input/devices`. It does not advertise an absolute axis.
- `ydotool mousemove --absolute X Y` landed at **accelerated / doubled
  coordinates**, not at the requested coordinate plane. Pointer
  acceleration interfered with absolute motion because ydotool's virtual
  device path is fundamentally relative.
- Click and drag using ydotool's relative path were workable for
  approximate motion but unsuitable for precise screenshot-pixel
  targeting.

The direct `/dev/uinput` adapter was tried as an alternative:

- It creates an absolute tablet-style device.
- It treats requested points as desktop logical coordinates within
  detected bounds.
- `cosmic-randr list` provides COSMIC's current output position and mode
  and is the preferred bounds source; `xrandr` works for X11-shaped
  sessions; `SKY_CUA_VIRTUAL_INPUT_*` env vars work as test overrides.
- COSMIC accepted this device for pointer motion, button events, and
  wheel events at 1x scale and at 125% display scale.
- At scale, the adapter must convert logical points to **physical
  absolute-device coordinates** by multiplying by output scale. The
  absolute device range is in physical output pixels, not logical.

Scrolling required both `REL_WHEEL_HI_RES` and `REL_WHEEL` events, with
the step sign inverted from the portal helper's discrete-scroll
direction. Sending only the ordinary wheel event did not satisfy the
visible smoke.

`ydotool` remained appropriate as the keyboard / text adapter. Its `type`
and `key` paths are not affected by the relative-pointer issue, and the
existing per-user socket and doctor probe make adoption cheap. The COSMIC
fixture proof for `type_text` and `press_key` came back clean once the
direct uinput path took over the pointer.

A separate gotcha discovered during ydotool argv construction: `ydotool`
treats arguments beginning with `-` as flags. Negative wheel deltas and
text that starts with a dash were mis-parsed until command construction
inserted `--` before payload arguments. The current command path always
inserts the separator and is covered by argv-shape unit tests rather than
shell-escaped strings.

## Conclusion

The COSMIC pointer path is **direct absolute `/dev/uinput`**, not
`ydotool`. Bounds come from `cosmic-randr` first, `xrandr` second, env
vars as test overrides; logical-to-physical conversion happens at the
uinput boundary using output scale; scrolling emits both
`REL_WHEEL_HI_RES` and `REL_WHEEL` with the sign inverted from portal
discrete scroll.

`ydotool` is **kept as the keyboard / text adapter** and as a sub-adapter
for Wayland sessions that have a usable ydotool socket. Its argv must
always insert `--` before payload arguments to avoid flag-parsing.

## Implications

- The shipped feature
  ([`docs/features/linux-virtual-input.md`](../features/linux-virtual-input.md))
  documents direct uinput as the primary pointer adapter and ydotool as
  the keyboard / text path.
- Tests cover argv construction (including the `--` separator), the
  COSMIC and XRandR bounds parsers, and the screenshot-to-desktop-logical
  conversion at 1x and at fractional scale.
- Future work on additional Wayland compositors should not adopt
  `ydotool` for pointer work without re-running this calibration on that
  compositor.
- The `wayland-pointer-scaled` VM profile must remain the regression gate
  for fractional scaling, because the smoke first proved fractional
  scaling and a missing scroll target due to oversized scaled fullscreen
  GTK allocation.
