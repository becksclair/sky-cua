---
name: phone-use
description: "Use for sky-cua phone MCP tools: Android discovery, wireless pairing, connect, observe, screenshots, taps/swipes, typing, keys, notifications, app control, and the companion backend."
---

# Phone Use

Controls a real Android phone over USB or wireless ADB. For the desktop, browser
chrome, or host windows, use the `computer-use` or `browser-use` skills instead;
those surfaces are not reachable through a phone session.

## Connecting

- Start with `phone_status` (adb path, server state, active sessions) and
  `phone_list_devices`. Device states are distinct: USB, emulator, legacy
  TCP/IP, wireless debugging, unauthorized, offline, disconnected. Pass an
  explicit `serial` whenever more than one device is present.
- Android 11+ wireless: `phone_pair_wireless` with the host:port and pairing
  code shown under Wireless debugging, then `phone_connect`. Pairing codes are
  never stored or echoed; a code is single-use per pairing.
- `phone_connect` before any screenshot or action. It detects and caches a
  per-session capability profile and, when companion support is enabled,
  installs or updates the companion app and sets up the RPC forward. Reconnect
  is idempotent for an already-connected serial.
- Missing `adb` disables phone-use entirely with a structured diagnostic.
  Missing companion or scrcpy only degrades capability; ADB baseline still
  works.

## Perception

- `phone_observe` is the default perception tool after connecting. One call
  returns the screenshot, a fresh `phone_snapshot_id`, current app, screen
  size/orientation, cursor state, the backend used, the capability profile
  version, `available_actions`, `unavailable_actions`, and bounded
  accessibility and notification summaries when those are enabled.
- Raw tools (`phone_screenshot`, `phone_accessibility_tree`,
  `phone_notifications`) remain available for focused work.
- Trust `available_actions` / `unavailable_actions` from the cached profile
  over guessing: an action listed as unavailable carries a reason and will be
  rejected. Profiles can go stale on permission, orientation, display-size,
  companion, or wireless changes; `phone_refresh_capabilities` rebuilds one.

## Coordinates and snapshots

- Coordinate actions reference a `phone_snapshot_id`. Use a fresh one from the
  latest `phone_observe` or `phone_screenshot`; stale, cross-session, or
  cross-serial snapshot ids are rejected with a structured error, as are
  out-of-bounds coordinates.
- Snapshot coordinates are screenshot pixels for that capture. The runtime maps
  them to device pixels; do not pre-scale or reuse coordinates from a different
  snapshot, device, or orientation.

## Actions

- `phone_tap`, `phone_swipe`, `phone_type_text`, and `phone_press_key` route
  through the best available backend (companion gestures/IME, then scrcpy when
  active, then ADB). Tool success means input was dispatched; verify
  consequential changes with a fresh `phone_observe` or `phone_screenshot`.
- Prefer companion capabilities when the profile reports them: native gestures,
  accessibility tree, on-device screenshots, and notification events are richer
  and more reliable than ADB fallbacks.
- Every response names the backend that handled the action and the capability
  profile it used. Read those structured fields rather than inferring backend
  state from prose.

## Cursor planes

Three distinct cursor planes, reported separately per session; do not conflate
them:

- Screenshot-synthetic cursor: a marker composited into the returned screenshot
  after a successful action. Always available, including ADB-only sessions.
- Host-visible overlay: a desktop overlay marker, available only when a scrcpy
  or host preview window exists and host mapping is current.
- Phone-native overlay: drawn on the device by the companion's accessibility
  service. Non-focusable and non-touchable; it does not intercept taps.

When a companion screenshot already contains the native overlay, the response
says so and the synthetic cursor is not double-composited.

## Notifications and apps

- Notification operations (`phone_notification_open`,
  `phone_notification_dismiss`, `phone_notification_action`,
  `phone_notification_reply`) require explicit notification and action ids from
  a fresh observation. Ids that have expired, been redacted, or lost their
  pending intent return structured unavailable errors. Keep notification and
  accessibility output bounded; do not request unbounded dumps.
- App control: `phone_app_current`, `phone_app_list`, `phone_app_launch`,
  `phone_app_open_intent`, `phone_app_force_stop`, `phone_app_install`, and
  `phone_open_settings`. Use `phone_open_settings` to send the user to the
  accessibility, notification-access, or wireless-debugging screen when a
  capability is reported disabled.
