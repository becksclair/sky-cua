---
name: phone-use
description: "Use for sky-cua phone MCP tools: Android discovery, wireless pairing, connect, observe, screenshots, taps/swipes, typing, keys, notifications, app control, and the companion backend."
---

# Phone Use

Controls a real Android phone over USB or wireless ADB. For desktop windows,
browser chrome, or host UI, use `computer-use` or `browser-use`; those surfaces
are not reachable through a phone session.

## Connection

- Phone discovery is `list_resources(surface="phone", resource="devices")`;
  status is `status(component="phone")`; connect/disconnect/refresh are
  grouped under `phone_connection`; setup under `phone_setup`; app launch and
  intents under `phone_app_action`.
- Start with phone status and device listing. Neither accepts a session or
  serial selector. The discovered serial is the value used for connect.
- Only Android devices in `device` state are drivable. `unauthorized` requires
  the on-device USB-debugging prompt; `offline`, `connecting`, `bootloader`, and
  `recovery` cannot be driven.
- Android 11+ wireless pairing uses the host:port and single-use pairing code
  shown under Wireless debugging. The resolved serial from pairing is what you
  connect to.
- Connect before any observation or action. Connect mints `session_id`; carry
  that exact id on every device-bound call. Raw `serial` is discovery/connect
  only. Disconnect invalidates the session.
- Connect targets the explicit serial, else the configured default, else the
  single authorized attached device only when there is exactly one. Ambiguous,
  missing, or unusable targets return host status with `PhoneDeviceUnavailable`
  and no session. Confirm a session id exists before acting.
- Reconnect for an already-connected serial refreshes that session in place:
  it re-probes the capability profile and companion bootstrap rather than
  duplicating the session.
- Missing `adb` disables phone-use entirely. Missing companion or scrcpy only
  degrades capability.

## Perception and Capability

- `observe(surface="phone", session_id=...)` is the default perception tool. It
  returns a screenshot, fresh `phone_snapshot_id`, current app, cursor state,
  servicing `backend`, capability profile id/refresh state, and
  `available_actions` / `unavailable_actions`. Accessibility and notification
  sections are opt-in.
- `capture_screen(surface="phone", session_id=...)`,
  `phone_accessibility_tree`, and `phone_notifications` remain available for
  focused work.
- Image delivery is not caller-controllable. Inline image vs `screenshot_path`
  is decided server-side from model image capability. Capture routes through the
  companion screenshot path first, then ADB screencap; observe/capture do not
  accept caller-selected backends.
- `available_actions` and `unavailable_actions` are the source of truth for
  what can run now. Do not attempt an action that appears only in
  `unavailable_actions`; its reason names the gate.
- `profile_refresh_state=stale` means availability is no longer proven.
  Companion and scrcpy paths are gated off until re-observe or refresh proves
  them again. Coordinate actions fail closed while the companion gesture lane is
  unproven.
- The capability profile is invalidated on reconnect, companion install/update,
  orientation or display-size change, RPC failure, or wireless drop. Permission
  revocation is not auto-detected mid-session; refresh after permission state
  may have changed. Wireless re-probe happens on observe, not on a bare action.

## Coordinates and Snapshots

- `phone_pointer(operation="tap"|"swipe")` takes screenshot-pixel coordinates
  from a specific fresh `phone_snapshot_id`, unless `use_device_coordinates=true`
  is set. Raw device coordinates need no snapshot but are bounds-checked only
  when display size is known.
- The runtime maps screenshot pixels to device pixels via snapshot metadata. Do
  not pre-scale or reuse coordinates across snapshots, sessions, serials,
  devices, or orientations.
- A snapshot id is rejected, and the action is not dispatched, when it is
  unknown, evicted, older than the cache TTL, from a different session/serial,
  or mismatched after rotation/resolution change. The registry keeps only the
  last 16 snapshots per session. Re-observe after any rejection, rotation, or
  display-size change.
- Fresh observe/capture can fail closed with `PhoneCapabilityProfileDrifted`
  when the new frame no longer matches cached display size; refresh/re-observe
  before tapping.
- Prefer accessibility-tree bounds for native UI. Tree bounds are exact device
  pixels; tap the center with `use_device_coordinates=true` to avoid snapshot
  TTL and tall-display visual estimation errors.
- Web content in Chrome/WebViews is not exposed as page elements in the phone
  accessibility tree. The tree normally sees native chrome plus an opaque
  WebView/FrameLayout container. In-page taps must be placed visually from the
  screenshot; use the synthetic cursor after one tap to correct relative to the
  actual landed point.
- Keep `operation`, `session_id`, coordinates, and provenance as top-level tool
  fields. `phone_snapshot_id` is only the opaque id string; never concatenate
  JSON fields into it. If schema validation rejects a pointer action, fix the
  shape or re-observe. Do not fall back to `adb shell input tap`, because that
  bypasses companion routing and visual feedback.

## Actions and Failure Semantics

- Routing is per tool family, not a single fallback ladder.
- Coordinate gestures require the companion gesture path: fresh profile, RPC
  reachable, and gesture capability. If it is not proven, pointer actions return
  `backend=none` with `PhoneCompanionRequired`; they must not silently use ADB.
- Text/key input routes through ADB in v1. There is no companion IME. Focus the
  target field first, then type or press key. Whitespace is preserved; empty
  text is rejected.
- scrcpy never services tap/swipe/type/key dispatch or screenshots. Its runtime
  effect is the host-visible cursor-overlay plane.
- Read success/failure from structured fields, not prose. For observe,
  screenshot, tap/swipe/type/press, and notification ops, `backend=none` means
  nothing ran: the operation was rejected or never dispatched, even if the
  diagnostic code sounds benign. App-management tools instead key success on
  their `success` boolean.
- Tool success only means input was dispatched. Verify consequential changes
  with a fresh phone observe.
- Expect first-run interstitials in freshly installed or first-launched apps:
  consent sheets, permission prompts, feature promos, browser notification
  dialogs, keyboard promos, and cookie walls can steal focus. Re-observe before
  typing/submitting. If search submit is swallowed, opening a result URL via
  intent is a reliable fallback.

## Companion Lifecycle

- The companion is mandatory for visible coordinate control, accessibility tree,
  notifications, and coordinate gestures. Without a reachable companion, these
  return `backend=none` with `PhoneCompanionRequired` and produce nothing.
- ADB still handles text/key input, app management, setup, and screenshot
  fallback.
- Use companion status to diagnose installed version, signature match,
  permission grants, RPC reachability, and bootstrap diagnostics.
- Installing or reinstalling the companion also attempts to enable its
  accessibility service and notification listener over ADB without clobbering
  existing device services. Manual-setup diagnostics mean Android gated the
  grant and the operator must finish it on device.
- Use setup/open-settings for accessibility, notification access, overlay
  permission, app details, wireless debugging, or battery optimization when the
  action menu or diagnostics says a permission is disabled.

## Cursor Planes

Three distinct cursor planes are reported separately; do not conflate them.

- Screenshot-synthetic cursor: composited into returned screenshots after a
  successful action. It can exist on ADB-only sessions and is never
  double-composited when the captured frame already includes the native overlay.
- Host-visible overlay: the phone-native cursor mirrored into a host-mapped
  scrcpy window. The host does not draw it. It is true only when a live mirror is
  host-mapped and companion RPC can draw it.
- Phone-native overlay: drawn on device by the companion accessibility service.
  The full-screen overlay is non-focusable and non-touchable. Its active edge
  glow is visual activity feedback, not session validity.

`PhoneCursorState` reports the post-action cursor in device and screenshot
planes, tagged with the snapshot it was captured against. The cursor updates
only after a successful action.

## Notifications and Apps

- Notification ids must come from a fresh notification list or phone observe
  with notifications included. Stale, handled, gone, or expired ids are
  rejected. Re-list before acting.
- Notification open/dismiss use only the event id. Notification custom actions
  use event id plus a matching action id from that event. Inline reply requires
  an action that explicitly supports inline reply; all other buttons use the
  notification-action path. Check `can_open` and `can_dismiss` before an op.
- App and current-app listing use phone resources. Launch/intents use
  app-action; force-stop and install have their own tools. Pass exact
  `package_name`, never display labels. APK paths are host-side paths. Intent
  open accepts a deep link or full intent URI; there is no activity field.

## Disconnecting

Disconnect ends the session, drops cached profile/snapshots/cursor/companion
runtime, and tears down only a sky-cua-managed scrcpy mirror. Adopted or
operator-launched adb/scrcpy processes are not killed. For wireless serials,
`keep_wireless=true` retains the ADB link for later reconnect.
