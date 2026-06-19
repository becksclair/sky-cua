# Phone-use Android helper app research

## Context

The baseline `phone-use` design uses ADB for device discovery, wireless
pair/connect, diagnostics, screenshots, and fallback input, with scrcpy as an
optional low-latency visual/control acceleration backend. The open question is
whether sky-cua should also ship a helper Android app that can draw the agent
cursor on-device and forward richer phone state such as notifications or screen
content.

## Investigation

An Android helper app is feasible and useful. Under the intended deployment
model, this is an operator-owned personal device with privileged/full-control
expectations, not a consumer-safe app distributed broadly. That changes the
priority: the companion should become the rich/native backend after ADB
bootstrap, not a distant polish item. It can assume sideloading, ADB install,
manual permission enablement, and optional privileged helpers such as root,
Shizuku, or device-owner flows when available.

ADB still remains necessary for bootstrapping, wireless pairing, fallback
control, install/update, port forwarding, and operation before the helper is
enabled. scrcpy remains valuable for low-latency visual mirroring and as a
fallback on devices where the helper is not installed, but a helper app is the
cleanest path for native overlay, semantic screen content, notification events,
and gesture dispatch.

The strongest helper shape is an AccessibilityService plus companion app UI. An
accessibility service can retrieve window content through `getRootInActiveWindow`
and `getWindows` when it declares the required capability to retrieve window
content. It can dispatch gestures with `dispatchGesture` when configured with
`canPerformGestures`, which gives a native tap/swipe path that does not depend on
ADB shell input or host-window coordinate translation. It can also take display
screenshots on API 30+ with `takeScreenshot`, and API 34+ adds
`takeScreenshotOfWindow`, which is specifically useful when an accessibility
overlay might otherwise cover the captured display.

For the agent cursor, the best phone-side overlay is an accessibility overlay
owned by the helper service. This keeps the cursor tied to the same service that
knows accessibility windows, gestures, and screenshot capability. A generic
`TYPE_APPLICATION_OVERLAY` is also possible with the special
`SYSTEM_ALERT_WINDOW` permission, but Android documents that application
overlays appear above app windows and below critical system windows, can have
their position or visibility changed by the system, and require special
permission. Android 12 also lets apps hide application overlays on sensitive
screens. Therefore `TYPE_APPLICATION_OVERLAY` is a fallback, not the preferred
cursor path.

Notifications are also feasible. `NotificationListenerService` receives system
callbacks when notifications are posted, removed, or ranking changes, and
`onNotificationPosted` exposes the notification object plus source package
metadata. It should be modeled as a separate opt-in capability because the user
must enable notification access and Android may redact sensitive notification
content. Android 15 specifically redacts OTP content from untrusted notification
listeners.

Raw screen capture through MediaProjection should not be the first helper screen
path. It can capture the display, but Android 14 requires user consent for each
MediaProjection capture session for apps targeting API 34+, and a media
projection foreground service type is required. Accessibility screenshots are a
better fit for an accessibility-enabled companion, with ADB/scrcpy fallback when
the API is unavailable.

The transport should be explicit and authenticated, but it can be optimized for
one trusted operator rather than broad consumer distribution. The simplest first
transport is host-managed ADB port forwarding to a localhost-only HTTP/WebSocket
endpoint in the helper. That keeps pairing inside the existing ADB trust model
and works for USB and wireless ADB sessions where forwarding is available. A
later no-ADB LAN mode can use QR/code pairing, TLS, and a short session token.

For privileged personal use, there should be an "operator mode" in the host
tools:

- Install or update the helper with `adb install -r`.
- Open the relevant Android settings screens for Accessibility and notification
  access.
- Detect whether the helper can use Accessibility screenshots, gesture dispatch,
  accessibility overlays, notification listener callbacks, and MediaProjection.
- When root, Shizuku, or device-owner style authority is present, optionally
  automate deeper setup and report exactly which privileged path was used.

## Conclusion

Build a helper app as the first rich/native backend after the ADB bootstrap
contract is stable. The helper should be called something like
`Sky Phone Companion` and live inside the sky-cua repo at first, for example
under `android/phone-companion/`, because its protocol, schemas, MCP tools,
smoke tests, and release packaging are tightly coupled to `phone-use`. If it
grows into a general Android agent runtime, it can be split later.

Recommended helper capabilities:

1. Native agent cursor overlay through an accessibility overlay.
2. Accessibility window/tree snapshots for semantic UI understanding.
3. Native gesture dispatch for taps, swipes, and multi-touch.
4. Native screenshots through AccessibilityService APIs when available.
5. Notification event forwarding through NotificationListenerService.
6. Helper health and permission state reporting.

Do not make the helper required for basic `phone-use`, because ADB is still the
bootstrap and rescue path. But for this personal privileged use case, treat the
helper as the preferred backend for native overlay, semantic UI, notifications,
and gesture dispatch once installed and enabled.

## Implications

The ExecPlan should add a `companion` backend immediately after the ADB baseline,
before treating scrcpy polish as complete. The public MCP contract should expose
companion capabilities as structured fields on `phone_status`, `phone_connect`,
and `phone_screenshot`, not as a separate tool universe. Possible companion
tools include `phone_install_companion`, `phone_companion_status`,
`phone_notifications`, and `phone_accessibility_tree`, gated by explicit
capabilities and permissions.

The helper app must have a clear operator trust model:

- Sideloaded/operator-controlled install and update.
- Explicit per-capability permission state.
- Pairing token or ADB tunnel authentication.
- Optional root/Shizuku/device-owner setup paths where available.
- No silent persistence of notification text, screenshots, or accessibility
  trees into committed artifacts.
- Clear distinction between host-visible overlay, screenshot-synthetic cursor,
  and phone-native overlay.
