---
name: phone-use
description: "Use for sky-cua phone MCP tools: Android discovery, wireless pairing, connect, observe, screenshots, taps/swipes, typing, keys, notifications, app control, and the companion backend."
---

# Phone Use

Controls a real Android phone over USB or wireless ADB. For the desktop, browser
chrome, or host windows, use the `computer-use` or `browser-use` skills instead;
those surfaces are not reachable through a phone session.

## Connecting

- Phone discovery is `list_resources(surface="phone", resource="devices")`,
  status is `status(component="phone")`, connect/disconnect/refresh are grouped
  under `phone_connection`, setup under `phone_setup`, and app launch/intent
  under `phone_app_action`.
- Start with `status(component="phone")` (adb path, server state, active
  sessions, default serial/backend) and
  `list_resources(surface="phone", resource="devices")` (the serial it returns
  is the value you feed to `phone_connection(operation="connect")`). Neither
  accepts a session/serial selector.
- Device states are distinct; only `device` is usable for connect. An
  `unauthorized` device needs the on-device "allow USB debugging" prompt
  accepted first. `offline`/`connecting`/`bootloader`/`recovery` cannot be
  driven.
- Android 11+ wireless: `phone_pair_wireless` with the host:port and pairing
  code shown under Wireless debugging, then
  `phone_connection(operation="connect")`. Pairing codes are single-use and
  never echoed back. The resolved serial it returns is what you then connect to.
- Connect before any observation or action — every other device-bound tool
  requires an active session and returns a no-session response
  (`PhoneNoSession`) otherwise. Connect mints the `session_id`; carry it on
  every later call (preferred over raw `serial`). Disconnect invalidates it.
- Connect targets the explicit `serial`, else the configured default, else the
  single attached device only when exactly one is in the authorized `device`
  state. An ambiguous multi-device set with no default, a missing serial, or a
  serial not in `device` state returns a host-status report (with
  `PhoneDeviceUnavailable`) and **no session** — it never optimistically mints a
  session for an unreachable serial. Confirm a `session_id` came back before
  acting. For wireless host:port targets, connect runs `adb connect` first and
  surfaces `PhoneConnectFailed` on failure.
- Reconnect for an already-connected serial refreshes that session in place
  (re-probes the profile, re-runs companion bootstrap) rather than duplicating.
- Missing `adb` disables phone-use entirely (no session can be minted); status
  and device listing still report the absence. A missing companion or scrcpy
  only degrades capability, not the session.

## Perception

- `observe(surface="phone")` is the default perception tool after connecting.
  One call returns a screenshot, a fresh `phone_snapshot_id`, current app,
  cursor state, the servicing `backend`, the `capability_profile_id` plus its
  `profile_refresh_state`, and the dynamic `available_actions` /
  `unavailable_actions` menu. Set `include_accessibility` /
  `include_notifications` to add those bounded companion-only sections.
- `capture_screen(surface="phone")`, `phone_accessibility_tree`, and
  `phone_notifications` remain available for focused work.
- Image delivery is **not** caller-controllable: an inline image block vs a
  `screenshot_path` on disk is decided server-side from the model's image
  capability, not by any input field. Capture auto-routes to the companion
  on-device screenshot first (its frame carries native-overlay metadata) then
  ADB `screencap`; forcing `backend=scrcpy` for a screenshot is unsupported and
  returns `PhoneBackendUnavailable`.

### Capability profile and the action menu

- `available_actions` (each tagged with the `backend` that would service it) and
  `unavailable_actions` (each with a structured `reason`) are the source of
  truth for what is possible right now. Read these structured lists; do not
  attempt an action that appears only in `unavailable_actions` — its reason
  names the gate (disabled permission, missing companion, wrong API level).
- `profile_refresh_state` is `detected` / `reused` / `refreshed` / `stale`. A
  `stale` profile means availability is no longer proven; companion and scrcpy
  are gated off and routing falls back to ADB-only.
- The profile is invalidated on reconnect, companion install/update, permission/
  orientation/display change, RPC failure, or wireless drop. The wireless
  re-probe fires on `observe(surface="phone")`, not on a bare action, so
  re-observe to surface a dropped link. Call
  `phone_connection(operation="refresh")` after any stale/drift signal, after a
  companion-to-ADB downgrade, or after a wireless reconnect, to re-prove
  companion/scrcpy before relying on them.

## Coordinates and snapshots

- `phone_pointer(operation="tap"|"swipe")` takes screenshot-pixel coordinates
  from a specific snapshot. A fresh `phone_snapshot_id` from
  `observe(surface="phone")` or `capture_screen(surface="phone")` is mandatory
  unless `use_device_coordinates=true`, in which case x/y are raw device pixels
  and no snapshot is needed (raw-point bounds are enforced only when the device
  display size is known). Omitting it without that flag returns
  `PhoneSnapshotRequired`.
- The runtime maps screenshot pixels to device pixels via the snapshot's
  mapping; do not pre-scale or reuse coordinates across snapshots, devices, or
  orientations. The mapping is validated before dispatch — an out-of-bounds
  point is rejected, never sent to the device.
- A snapshot id is rejected (action not dispatched) when it is unknown/evicted,
  stale (older than the cache TTL), from a different session or serial, or when
  the screen rotated or resized since capture (orientation/resolution mismatch).
  The registry keeps only the last 16 snapshots per session. On any rejection,
  and after any rotation or display-size change, **re-observe for a fresh
  snapshot before tapping** — `capture_screen(surface="phone")` and
  `observe(surface="phone")` themselves fail closed with
  `PhoneCapabilityProfileDrifted` when a fresh frame no longer matches the
  cached display size.
- **Prefer accessibility-tree bounds over visual estimation for tap targets on
  native UI.** `phone_accessibility_tree` reports each element's exact
  device-pixel `bounds`; tap the bounds center with `use_device_coordinates=true`
  (no snapshot, no TTL to race). The screenshot is a faithful 1:1 capture of the
  device, but estimating a target's pixel position by eye is unreliable on tall
  displays — a vertical misjudgement of 10-15% is easy and lands the tap on a
  neighboring element, with the error growing toward the bottom of the screen.
  The image is not the problem; the visual estimate is. Use the tree's bounds
  whenever the target is a native view.
- **Web content (Chrome and other WebViews) is not in the accessibility tree.**
  `phone_accessibility_tree` exposes only the app's native chrome (toolbar, URL
  bar, tabs) plus an opaque `WebView`/`FrameLayout` container — never the page's
  own links, buttons, or text. In-page targets must be located visually from the
  screenshot, where the tall-display estimation risk above applies. To place an
  in-page tap accurately, tap once, read the synthetic cursor's rendered position
  in the next screenshot (it marks exactly where the last action landed), and
  correct relative to it rather than re-estimating from scratch.

## Actions

- Routing is per tool family, not a single ladder:
  - `phone_pointer(operation="tap"|"swipe")` prefers the companion gesture path
    when it is proven (fresh profile, RPC reachable, gesture capability), else
    ADB.
  - `phone_keyboard(operation="type_text"|"press_key")` has **no** companion
    path in v1 and always route through ADB (`input text`, `input keyevent`).
    There is no
    "companion IME". Type text into the currently focused field — focus the
    target field first (e.g. tap it); whitespace is preserved and only an empty
    string is rejected. Press-key accepts a keycode name (`KEYCODE_BACK`), a
    bare alias (`home`), or a numeric keycode (`4`).
  - scrcpy never services tap/swipe/type/key dispatch or screenshots; control
    always reaches the device via ADB even while scrcpy mirrors. Its only runtime
    effect is the host-visible cursor-overlay plane.
- **Reading success/failure is structured, not prose.** Every action-bearing
  response (observe, screenshot, tap/swipe/type/press, and all notification ops)
  carries a `backend` field. `backend=none` means nothing ran — the op was
  rejected or never dispatched — and it flips `isError` even when the diagnostic
  code looks benign, so a rejected tap is never readable as success. A companion
  failure that fell back to ADB on a good result (`backend=adb`) is
  informational, not an error. The app-management family instead keys success on
  its `success` boolean. Read `backend`/`isError`, not the summary text. Tool
  success only means input was dispatched; verify consequential changes with a
  fresh `observe(surface="phone")`.
- **Expect first-run interstitials, especially in browsers.** A freshly
  installed or first-launched app commonly interposes consent sheets, permission
  prompts, or feature promos that steal focus before the screen you want — e.g.
  Chrome's notifications dialog, the Gboard handwriting promo (which can swallow
  typed text and the Enter key), and a locale-specific cookie-consent wall.
  Re-observe and confirm the foreground is the screen you expect before typing or
  submitting, and dismiss the interstitial first. When a search submit is
  swallowed, opening a result URL with
  `phone_app_action(operation="open_intent")` is a reliable fallback.

## Companion lifecycle

- The companion is optional; ADB is the control authority. Without a reachable
  companion the session is still fully usable for tap/swipe/type/key/screenshot
  and app management via ADB. The companion only adds richer native gestures,
  on-device screenshots with overlay metadata, the accessibility tree,
  notifications, and the native cursor overlay.
- `phone_accessibility_tree` and the entire `phone_notifications` family are
  **companion-only in v1 with no ADB fallback**: with no reachable companion
  they return `backend=none` with `PhoneCompanionRequired` and produce nothing.
  This is a hard gate, not a degrade — unlike gestures and screenshots, which do
  fall back to ADB.
- Recovery when companion-only actions are unavailable or a session has
  downgraded to ADB: `status(component="phone_companion")` reports the
  companion's installed version, signature match, permission grants, RPC
  reachability, and the latest bootstrap diagnostics — use it to diagnose *why*
  the companion is unreachable. `phone_setup(operation="install_companion")` forces an install/
  update plus a full re-bootstrap; reach for it when the companion is enabled
  but missing/unreachable (e.g. operator auto-install is off) or to force a
  reinstall (`force_reinstall`, or `allow_downgrade` to accept an older build).
- An install-bearing bootstrap (`phone_setup(operation="install_companion")`, or
  connect under operator auto-install / an explicit `install_companion`) also enables the
  companion's accessibility service and notification listener over ADB, so a
  freshly deployed companion is usable without a manual setup trip. It never
  clobbers the device's existing services. Watch the bootstrap diagnostics:
  `PhoneCompanionPermissionEnabled` confirms a grant; a
  `PhoneCompanion*ManualSetup` entry means the device gated the grant (e.g.
  Samsung "Restricted settings") and the operator must finish it by hand — the
  host best-effort opens the Accessibility screen in that case.
- `phone_setup(operation="open_settings")` drives the user to a specific Android screen
  (accessibility, notification_access, overlay_permission, app_details,
  wireless_debugging, battery_optimization) to grant a companion permission when
  the action menu reports it disabled or a `PhoneCompanion*ManualSetup`
  diagnostic asks for manual intervention. `app_details` requires a
  `package_name`.

## Cursor planes

Three distinct cursor planes, reported separately per session; do not conflate
them:

- Screenshot-synthetic cursor: a marker composited into the returned screenshot
  after a successful action. Available even on ADB-only sessions (gated by the
  `screenshot_cursor` config), and never double-composited when the captured
  frame already bakes in the native overlay.
- Host-visible overlay: the phone-native cursor mirrored into a host-mapped
  scrcpy window — the host does not draw it. Reported true only when a live
  scrcpy mirror is host-mapped **and** the companion RPC is reachable to draw it
  (and the visible-overlay config is on). A mapped mirror with no reachable
  companion reports false.
- Phone-native overlay: drawn on the device by the companion's accessibility
  service. Non-focusable and non-touchable; it does not intercept taps.

`PhoneCursorState` reports the post-action cursor in both device and screenshot
planes, tagged with the `snapshot_id` it was captured against — a screenshot
point is only valid relative to that snapshot. The cursor updates only after a
successful action.

## Notifications and apps

- Notification ids must come from a **fresh** `phone_notifications` or
  `observe(surface="phone", include_notifications=true)` call. Stale/handled
  ids are rejected (`PhoneNotificationOpRejected` / gone / expired). Re-list
  before acting. Structural id rules:
  - `phone_notification_action(operation="open"|"dismiss")` takes only
    `event_id`.
  - `phone_notification_action(operation="action")` takes `event_id` plus a
    matching `action_id` from that same event's `actions[]`.
  - `phone_notification_reply` takes `event_id` + `action_id` + `text`, and is valid **only** for an
    action whose `supports_inline_reply` is true; a non-reply target yields
    `reply_unavailable`. All non-inline-reply buttons go through
    notification-action.
  - Check `can_open` / `can_dismiss` before an op. Successful ops refetch and
    return the fresh notification list.
- App control: app/current-app listing uses
  `list_resources(surface="phone", resource="apps"|"current_app")`;
  launch/intent uses `phone_app_action`; force-stop uses
  `phone_app_force_stop`; install uses `phone_app_install`.
  Pass the exact `package_name` from app listing or current-app results, never a
  display label. Install reports the actual strategy it ran (single / multiple /
  multi_package) and takes host-side APK paths, not device paths. Intent open
  accepts an activity component, a deep link, or a full intent URI.

## Disconnecting

`phone_disconnect` ends the session — the `session_id` dies — and drops the
cached profile, snapshot/cursor, and companion runtime. It tears down only a
sky-cua-managed scrcpy mirror (adopted or operator-launched windows are never
killed) and never touches operator-launched adb/scrcpy processes. For wireless
serials it runs `adb disconnect` unless `keep_wireless=true`, which retains the
wireless adb link for a later reconnect.
