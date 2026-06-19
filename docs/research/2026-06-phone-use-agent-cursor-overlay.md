# Phone-use agent cursor overlay research

## Context

The question is whether `phone-use` can show an agent cursor like sky-cua does for desktop actions, and how to design it without confusing Android device coordinates, host desktop coordinates, scrcpy window geometry, and model-facing screenshots. This research extends `docs/research/2026-06-phone-use-architecture.md`, which recommends an ADB baseline backend plus optional scrcpy acceleration inside the existing sky-cua MCP server.

Update: `docs/research/2026-06-phone-use-android-helper-app.md` promotes a helper app for the personal/operator-owned deployment model. That does not replace this two-plane host/screenshot overlay design; it adds a third phone-native overlay plane implemented by the companion AccessibilityService.

## Investigation

sky-cua already has the right cursor architecture. `docs/features/agent-cursor-overlay.md` describes two independent planes: a user-visible native overlay drawn by `sky-cua-overlay-host`, and a synthetic cursor composited into the screenshot sent to the model. The public model lives in `crates/sky-cua-platform/src/model.rs`: `AgentCursorState` has `model_point`, `native_point`, `snapshot_id`, `source_action`, and an update timestamp; `AgentCursorPoint` has `x`, `y`, `CoordinateSpace`, and an optional `mapping_id`; `AgentCursorCapabilities` reports whether visible overlay and synthetic screenshot cursor are available. The service-side owner is `crates/sky-cua-service/src/overlay.rs`, where `OverlayController` updates state after successful actions, hides idle cursor state after 1.5 seconds, hides/restores the visible overlay around captures, and composes synthetic cursor pixels into captured screenshots.

For phone-use, the same two-plane model should be preserved. The model-facing phone screenshot can always include a synthetic cursor when there is a recent phone action and a phone screenshot image. A user-visible host overlay is only available when the phone has a host-rendered surface such as a scrcpy window or configured V4L2/preview surface. ADB-only screenshots have no host surface, so they can return a synthetic cursor in the image but cannot honestly claim a visible desktop overlay.

The hard part is coordinate mapping, not drawing. Phone actions naturally use Android display pixels. The existing desktop overlay expects a model point in screenshot pixels and a native point in desktop coordinates. A scrcpy-window session adds at least two mapping steps: Android device pixels map into the video content rectangle inside the scrcpy client window, then the client window maps into desktop coordinates. Letterboxing, window decorations, fractional scaling, Android rotation, and scrcpy options such as `--max-size`, `--crop`, `--orientation`, and `--display-id` can all affect that mapping. Therefore each `PhoneScreenshot` must return a stable mapping id and enough metadata to translate later `phone_tap`/`phone_swipe` actions only if they reference the matching phone snapshot.

Android-native overlays are possible but should not be first. Android's `SYSTEM_ALERT_WINDOW` permission allows apps to create `TYPE_APPLICATION_OVERLAY` windows shown above other apps, but Android documents that very few apps should use it and Android 11 tightened the grant flow to make it more intentional. Accessibility services can create `TYPE_ACCESSIBILITY_OVERLAY` windows and can dispatch gestures, but that requires a phone-side app, a user-enabled accessibility service, service capabilities, and a larger trust/install story. That is closer to a future semantic/control backend than to the first `phone-use` cursor. It would also create a new Android-side artifact to maintain and explain.

scrcpy's own "show touches" option is not a replacement for an agent cursor. The scrcpy documentation says `--show-touches` shows physical touches on the physical device, and Debian's current manpage explicitly says it does not show clicks from scrcpy. That option can help presentations, but it cannot show where the agent intends to click or where it clicked through ADB/scrcpy control.

## Conclusion

Implement a phone-scoped cursor by reusing sky-cua's existing overlay concepts instead of building a new overlay subsystem. Add phone-specific cursor state and mapping metadata, then bridge to the existing `OverlayController` for the host-visible plane when a scrcpy window or other host preview exists. Always support the screenshot-synthetic plane for phone screenshots.

The design should be:

1. `PhoneCursorState` or embedded `AgentCursorState` per `PhoneSession`, never one global phone cursor for every device.
2. `PhoneScreenshotResponse` includes `phone_snapshot_id`, `cursor`, `cursor_capabilities`, and `coordinate_mapping`.
3. `PhoneTap`/`PhoneSwipe` update the phone cursor after successful action dispatch.
4. ADB-only sessions return `visible_overlay=false` and `screenshot_synthetic_cursor=true` when screenshot synthesis is enabled.
5. scrcpy-window sessions translate phone cursor coordinates into host coordinates and call the existing `OverlayController` so the user sees the same desktop overlay used by normal computer-use actions.
6. Android-native overlay is deferred until there is a deliberate phone-side companion app milestone.

## Implications

The `phone-use` ExecPlan should explicitly add cursor work rather than treating it as a polish item. It should require unit tests for phone-to-host coordinate transforms, stale snapshot rejection, per-device cursor isolation, ADB-only synthetic cursor behavior, and scrcpy-window visible overlay behavior. The live smoke should capture before and after a benign phone tap and verify that the returned phone screenshot contains a synthetic cursor marker; when scrcpy is available, it should also verify visible overlay capabilities and cursor state from the service response.

## Update (2026-06-18): phone-native overlay milestone landed

Conclusion item 6 above deferred the Android-native overlay "until there is a deliberate phone-side companion app milestone." That milestone has now landed, so the deferral no longer holds.

The companion accessibility service now draws the agent cursor and a glowing screen-edge effect directly on the phone, from a single full-screen pass-through `TYPE_ACCESSIBILITY_OVERLAY` view (`android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/`). A persistent breathing edge glow signals "agent in control" while a phone session is held; the cursor animates per action (tap ripple, swipe/drag trail) with a brighter glow pulse for the action's duration. Overlay coordinates are Android device pixels — the same space gestures use — so for the phone-native plane there is no host/desktop coordinate mapping.

Three new/changed wire methods carry this (see `docs/runtime/phone-companion-protocol.md`):

- `overlay_active { active }` → `{ active, glow_supported }` toggles the persistent glow; the host calls it on session hold/release.
- `overlay_gesture { kind, points, duration_ms }` → `{ animated }` animates one action. It is visual only and never dispatches real input.
- `screenshot` with the include-overlay flag false now makes the companion hide the overlay for the capture and restore it afterward, so model-facing screenshots are clean; `contains_native_overlay` reflects what was captured.

As a direct consequence, the host-desktop draw of the phone cursor was removed: the `host_cursor_state` field and the `HostCursorDraw` bridge for phone actions no longer exist, along with the now-vestigial always-`None` `PhoneCursorState.host_point`. The desktop `OverlayController` / KWin effect remain for real desktop computer-use only. The host-visible overlay plane survives but is redefined: it is the companion's on-device overlay mirrored into a mapped scrcpy window, so `host_visible_overlay` is reported true only when the companion overlay is reachable AND a scrcpy mirror is mapped to display it — not merely when a scrcpy window exists. The screenshot-synthetic plane is still the fallback when no companion overlay is available (ADB-only sessions).
