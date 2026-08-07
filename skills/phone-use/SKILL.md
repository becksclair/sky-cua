---
name: phone-use
description: "Use for sky-cua MCP phone control through an ADB or ADB-independent direct Companion session: AppShots, Android input, clipboard/editor, camera, storage, notifications, and apps."
---

# Phone Use

Use this skill for real Android phone sessions through sky-cua MCP tools.
Desktop windows, host browser chrome, and other host UI require the desktop or
browser surface when that surface is enabled; otherwise report that the task
requires a disabled surface.

## Mandatory execution contract

Every phone plan and execution must name this chain explicitly:

1. Call `status(component="phone")` and discover devices without a session selector. Direct Companion devices use `device_id`; ADB devices use `serial`.
2. Connect, retain the returned exact `session_id`, and pass it on every device-bound call.
3. Call `observe(surface="phone", session_id=...)` immediately after connect and retain its exact `appshot_id`. If `profile_refresh_state=stale`, refresh and re-observe.
4. Use only `available_actions`, and pass the current `appshot_id` to every state-changing call. For pointer work, also specify either screenshot-local x/y with the exact producing `phone_snapshot_id`, or accessibility/device x/y with `use_device_coordinates=true`; never pre-scale.
5. Inspect the operation's structured result: app management requires `success=true`; other operations require a real `backend`, and `backend=none` means no operation occurred. Preserve diagnostics when reporting refusal or failure.
6. Re-observe after the operation and prove the requested final state.

For notifications, also check `can_open` and `can_dismiss`, use current event/action ids, inspect each operation's structured backend (`none` means no operation), and re-list after every open, dismiss, or action.

## Connect before acting

- Discover devices with `list_resources(surface="phone", resource="devices")`.
- Check host status with `status(component="phone")`.
- Group connect, disconnect, and refresh under `phone_connection`, setup under `phone_setup`, and app launch/intents under `phone_app_action`.
- Call neither discovery nor status with a session or serial selector.
- Use the discovered `device_id` for Companion Direct or `serial` for ADB; never invent one from the other.
- Drive only Android devices in `device` state.
- Treat `unauthorized` as requiring the on-device USB-debugging prompt.
- Treat `offline`, `connecting`, `bootloader`, and `recovery` as undrivable.
- Pair Android 11+ wireless devices with the Wireless debugging host:port and single-use pairing code.
- Connect the serial resolved by pairing.
- Connect before every observation or action.
- Carry the exact `session_id` minted by connect on every device-bound call.
- Use raw `serial` only for discovery and connect.
- Treat disconnect as invalidating the session.
- Resolve connect targets as explicit serial, configured default, or the single authorized attached device.
- Treat an ambiguous, missing, or unusable target as host status with `PhoneDeviceUnavailable` and no session.
- Confirm that a session id exists before acting.
- Reconnect an already-connected serial to refresh its session in place and re-probe its profile and companion bootstrap.
- Missing `adb` disables only ADB sessions. An authenticated Companion Direct device remains fully usable through its advertised capability routes.
- Treat missing companion or scrcpy as a capability reduction rather than a total failure.

## Perception, profile, and routing

Use `observe(surface="phone", session_id=...)` as the default perception call.
It returns a canonical AppShot containing a clean screenshot, foreground app,
interactive accessibility windows/tree, consistency and coverage state, fresh
`phone_snapshot_id`, current app, cursor state,
servicing `backend`, capability-profile id and refresh state, and
`available_actions` / `unavailable_actions`; accessibility and notifications
are opt-in. Use `capture_screen`, `phone_accessibility_tree`, and
`phone_notifications` for focused calls.

| Work | Runtime route and stopping rule |
| --- | --- |
| Observe or screenshot | Capture through the companion first, then ADB screencap; the server chooses inline image versus `screenshot_path`; do not choose the backend. |
| Accessibility tree or notifications | Use the companion; without a reachable companion, return `backend=none` with `PhoneCompanionRequired` and produce nothing. |
| Tap or swipe | Use the companion gesture path only when a fresh profile, reachable RPC, and gesture capability are proven; otherwise return `backend=none` with `PhoneCompanionRequired` and never fall back to ADB. |
| Text or key input | Direct sessions use accessibility/editor input, with optional Sky IME capabilities advertised separately; ADB remains a compatibility provider. Focus the intended editor first. |
| Clipboard/editor, camera, content, or storage | Use the grouped direct-Companion tools and only operations whose capability routes are ready. IME, visible camera activity, all-files access, and SAF roots remain explicit prerequisites. |
| App management or setup | Direct sessions support launch/intents/settings; force-stop and silent install remain ADB-only in v1. App-management success is the structured `success` boolean. |
| scrcpy | Use only its host-visible cursor-overlay plane; it never dispatches touch, text, or key input and never supplies screenshots. |

- Fail coordinate actions closed while the companion gesture lane is unproven.
- Re-probe the wireless path on observe rather than on a bare action.
- Treat the profile as invalid after reconnect, companion install/update, orientation or display-size change, RPC failure, or wireless drop.
- Refresh after permission state may have changed because permission revocation is not auto-detected mid-session.
- Treat a successful dispatch as input delivery only.
- `AppShotRequired` means the requested mutation did not execute. Continue from the fresh recovery AppShot returned with the error; never blindly replay a non-idempotent action after reconnect.
- Re-observe before typing or submitting when a first-run consent sheet, permission prompt, feature promo, browser notification dialog, keyboard promo, or cookie wall may have stolen focus.
- Open a result URL through intent if search submission is swallowed.

## Coordinates and snapshot provenance

- Use `phone_pointer(operation="tap"|"swipe")` for gestures.
- Supply screenshot-pixel coordinates with the exact fresh `phone_snapshot_id` that produced the target.
- Set `use_device_coordinates=true` when using raw device pixels instead.
- Use raw device coordinates without a snapshot only when display-size bounds are known for validation.
- In every pointer plan, name one provenance mode explicitly: screenshot x/y plus the exact `phone_snapshot_id`, or accessibility/device pixels plus `use_device_coordinates=true`.
- Let runtime snapshot metadata map screenshot pixels to device pixels.
- Never pre-scale coordinates.
- Never reuse coordinates across snapshots, sessions, serials, devices, or orientations.
- Treat `phone_snapshot_id` as an opaque id string rather than concatenated JSON fields.
- Keep `operation`, `session_id`, coordinates, and provenance as top-level tool fields.
- Treat an unknown, evicted, expired, cross-session/serial, or rotation/resolution-mismatched snapshot as rejected without dispatch.
- Assume only the last 16 snapshots per session remain cached.
- Re-observe after snapshot rejection, rotation, or display-size change.
- Treat fresh observe/capture with `PhoneCapabilityProfileDrifted` as failed closed and refresh or re-observe before tapping.
- Prefer native accessibility bounds for native UI.
- Treat accessibility bounds as exact device pixels and tap their center with `use_device_coordinates=true` to avoid snapshot TTL and tall-display visual-estimation errors.
- Place Chrome/WebView in-page taps visually from the screenshot because the accessibility tree exposes the page as an opaque WebView/FrameLayout.
- Use the synthetic cursor after one visual tap to correct relative to the landed point.
- Fix pointer schema shape or re-observe after validation rejection.
- Never substitute `adb shell input tap`, which bypasses companion routing and visual feedback.

## Notifications and apps

- `phone_notifications(session_id=...)` is list-only. Obtain notification ids from a fresh list or an observe result with notifications included.
- Treat stale, handled, gone, or expired notification ids as rejected and re-list before acting.
- Check `can_open` and `can_dismiss` before those operations.
- Require a unique metadata match for the requested notification and action; never choose the first openable event or first custom action when several match, and stop for clarification on material ambiguity.
- Open or dismiss through `phone_notification_action(operation="open"|"dismiss", session_id=..., event_id=...)`.
- Invoke a custom action through `phone_notification_action(operation="action", session_id=..., event_id=..., action_id=...)` using the matching current ids.
- Re-list after an open, dismiss, or custom action before attempting another notification operation; the prior event/action ids may already be invalid.
- Use inline reply only for an action that explicitly supports inline reply.
- Use the notification-action path for other notification buttons.
- List apps and the current app through phone resources.
- Launch apps and intents through `phone_app_action`.
- Use dedicated tools for force-stop and install.
- Use exact `package_name` values rather than display labels.
- Treat APK paths as host-side paths.
- Pass a deep link or full intent URI for intent open; there is no activity field.

## Cursor, setup, and disconnect

- Keep the screenshot-synthetic, host-visible overlay, and phone-native overlay as three distinct cursor planes.
- Read [references/cursor-planes.md](references/cursor-planes.md) when interpreting cursor fields, mirroring, or overlay behavior.
- Treat `PhoneCursorState` as the post-action cursor in device and screenshot planes tagged to its snapshot.
- Update cursor state only after a successful action.
- Read [references/companion-lifecycle.md](references/companion-lifecycle.md) when diagnosing companion installation, permissions, bootstrap, or setup.
- Disconnect to end the session and drop cached profile, snapshots, cursor, and companion runtime.
- Disconnect only when the user requests ending the session or the task explicitly requires teardown; otherwise stop after final verification and preserve the session.
- Tear down only a sky-cua-managed scrcpy mirror.
- Preserve adopted or operator-launched ADB/scrcpy processes.
- Set `keep_wireless=true` to retain a wireless ADB link for later reconnect.
