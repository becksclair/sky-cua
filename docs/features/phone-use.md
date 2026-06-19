# Phone Use (Android control)

## Status

Partial. Source-complete and green across the Rust workspace; live device proof
and installed-MCP/packaging proof are pending. Last verified: 2026-06-17
(`cargo test` green for the phone platform model, service backends, and client
MCP surface; `python3 scripts/build_plugin.py` stages `skills/phone-use`).

## Summary

Lets a sky-cua agent control a real Android phone from the existing
`sky-cua-client mcp` process over USB or wireless ADB: discover, pair, connect,
observe, screenshot, tap/swipe/type, press keys, read and act on notifications,
manage apps, and render an agent cursor. `phone_observe` is the default
per-turn perception tool; ADB is the required baseline, with an optional
companion app and optional scrcpy acceleration layered on top.

## Contract surface

A static `phone_*` MCP tool family inside the single `computer-use` MCP server
(no new server). Tools:

- Session and discovery: `phone_status`, `phone_list_devices`,
  `phone_pair_wireless`, `phone_connect`, `phone_disconnect`,
  `phone_refresh_capabilities`.
- Perception: `phone_observe` (default), `phone_screenshot`,
  `phone_accessibility_tree`, `phone_notifications`.
- Input: `phone_tap`, `phone_swipe`, `phone_type_text`, `phone_press_key`.
- Companion: `phone_install_companion`, `phone_companion_status`.
- Notifications (explicit ids required): `phone_notification_open`,
  `phone_notification_dismiss`, `phone_notification_action`,
  `phone_notification_reply`.
- Apps: `phone_app_current`, `phone_app_list`, `phone_app_launch`,
  `phone_app_open_intent`, `phone_app_force_stop`, `phone_app_install`,
  `phone_open_settings`.

Callers can rely on: structured responses carrying the truth (backend used,
capability profile id/version, snapshot ids, cursor capabilities, permission
state, diagnostics) — prose summaries are secondary. Coordinate actions require
a fresh `phone_snapshot_id`. `available_actions` / `unavailable_actions` come
from the cached capability profile, not a static schema. The tool list is
static (compatibility), action affordances are dynamic.

Config lives in the `[phone]` table of the machine config and is mirrored by
environment overrides, all allowlisted in `.mcp.json`:
`SKY_CUA_PHONE`, `SKY_CUA_PHONE_SERIAL`, `SKY_CUA_PHONE_BACKEND`,
`SKY_CUA_ADB`, `SKY_CUA_SCRCPY`, `SKY_CUA_PHONE_WIRELESS_AUTO_CONNECT`,
`SKY_CUA_PHONE_VISIBLE_OVERLAY`, `SKY_CUA_PHONE_SCREENSHOT_CURSOR`,
`SKY_CUA_PHONE_V4L2_SINK`, `SKY_CUA_PHONE_COMPANION`,
`SKY_CUA_PHONE_COMPANION_AUTO_INSTALL`,
`SKY_CUA_PHONE_COMPANION_OPERATOR_MODE`, `SKY_CUA_PHONE_COMPANION_PACKAGE`,
`SKY_CUA_PHONE_COMPANION_APK`, `SKY_CUA_PHONE_COMPANION_CERT_SHA256`,
`SKY_CUA_PHONE_COMPANION_ALLOW_DOWNGRADE`,
`SKY_CUA_PHONE_COMPANION_RPC_PORT`,
`SKY_CUA_PHONE_COMPANION_RPC_TOKEN_TTL_MS`,
`SKY_CUA_PHONE_CAPABILITY_CACHE_TTL_MS`, `SKY_CUA_PHONE_TARGET_MODELS`.

Intentionally not stable: the action affordance lists vary by device and
session; the companion APK and its identity sidecar are optional packaging
inputs that may be absent.

## Behavior

`phone_connect` resolves `adb` (config, then environment, then `PATH`), probes
device identity and display metrics, detects and caches a per-session
`PhoneCapabilityProfile`, and — when companion support is enabled — verifies,
installs, or updates the companion APK and establishes the RPC forward before
finalizing the profile. The cache is per session and is invalidated on
reconnect, companion install/update, permission change, orientation or
display-size change, RPC failure, wireless disconnect, and explicit
`phone_refresh_capabilities`.

Freshness contract (as built): the tools that act on or perceive through the
profile — the action tools (`phone_tap`, `phone_swipe`, `phone_type_text`,
`phone_press_key`, the notification and app operations), `phone_observe`, and
`phone_screenshot` — carry the profile id/version used and a
`profile_refresh_state` of `refreshed`, `reused`, or `stale`. `reused` is
emitted on a within-TTL cache hit, and the `profile_refresh_state` is kept in
lockstep with the `stale` boolean. The pure-status tools
(`phone_companion_status`, `phone_accessibility_tree`, `phone_notifications`, and
the `phone_app_*` reads) do not act on the profile and intentionally omit the
freshness field.

Three backends, deterministically routed from the cached profile:

- ADB baseline (required): discovery, USB and Android 11+ wireless pairing,
  connect/disconnect, device property probing, fallback screenshots, fallback
  input, app inventory/management, companion install/update primitives, and all
  recovery flows. Missing `adb` disables phone-use with a structured
  diagnostic.
- Companion (preferred when enabled and healthy): native gesture dispatch,
  accessibility window tree, on-device screenshots, notification events and
  actions, and the phone-native agent overlay (animated cursor plus the
  persistent "agent in control" edge glow), reached over a localhost-only RPC
  through a host-managed ADB forward with an ephemeral session token.
- scrcpy (optional acceleration): host-rendered mirror/control window with
  host-visible overlay support and codec-failure retry. When `[phone] max_size`
  is unset, the mirror is capped to a phone-scale size derived from the host
  display topology (≈55% of the primary display's logical height, clamped) so a
  hi-res device renders phone-sized instead of filling the desktop; an explicit
  `max_size` always overrides. The daemon probes the topology
  (`DesktopBackend::list_displays`) only for a connect that launches a mirror,
  and an unknown topology leaves the configured value untouched. Missing scrcpy
  degrades capability without disabling baseline phone-use.

Three cursor planes, reported separately per session:

- Screenshot-synthetic cursor: composited into the returned screenshot after a
  successful action; always available, including ADB-only mode.
- Host-visible overlay: the phone-native cursor made visible on the host through
  a mapped scrcpy mirror window. The host no longer draws this cursor itself — it
  is the device overlay (below), mirrored into the host window by scrcpy.
  Reported true only when the companion overlay is reachable to draw it AND a
  scrcpy mirror is currently mapped to display it; a mapped mirror with no
  reachable companion reports false, because nothing is drawn into it.
- Phone-native overlay: the agent cursor and a glowing screen-edge effect drawn
  directly on the device by the companion accessibility service. This is the
  primary "agent in control" indicator for phone actions (see below). When a
  companion screenshot already contains the native overlay, the response reports
  that and the synthetic cursor is not double-composited.

### Phone-side agent overlay (as built)

The agent cursor for phone actions is drawn on the phone, not on the host
desktop. The companion accessibility service renders a single full-screen,
pass-through `TYPE_ACCESSIBILITY_OVERLAY` view (non-focusable, non-touchable),
sized to the full display bounds (status- and navigation-bar regions and any
cutout, via `currentWindowMetrics` and `LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS`)
so the glow reaches the true screen edges. It drives every overlay element from
that one view:

- A pronounced pink screen-edge glow signals that the agent is in control while a
  phone session is held: a gently breathing border with a bright wave of light
  travelling around the perimeter (a rotating sweep gradient). The host turns it
  on when a session establishes with a reachable companion and off on
  disconnect/release. All geometry is sized in dp from the display density so the
  glow reads at a consistent physical size rather than vanishing on a high-DPI
  panel.
- The cursor is the same `cursor-chat` pointer the desktop agent uses, scaled up
  and seated in a soft pink halo. It is parked at screen centre the moment the
  session connects so the pointer is visible immediately, and it — like the glow
  — persists for the whole connected session and never auto-hides; only
  disconnect removes the overlay.
- The cursor animates per action: a tap shows an expanding, fading ripple; a
  swipe or drag moves the cursor along the path with a fading trail. The edge
  glow pulses brighter for the action's duration, then returns to the breathing
  baseline. Animations are visual only — they never dispatch input (the real
  tap/swipe is dispatched separately through the companion `gesture` method or
  ADB `input`).
- All overlay coordinates are Android device pixels (post-rotation display
  pixels), the same space gestures use. No host/desktop coordinate mapping is
  involved for the phone-native plane.
- Model-facing screenshots stay clean: when a screenshot request does not ask to
  include the overlay, the companion synchronously hides every overlay pixel
  (cursor, ripple, trail, and glow) for the capture and restores the prior state
  afterward. The result's `contains_native_overlay` reflects what was actually
  captured.

The host bridges this plane best-effort: the `overlay_active`/`overlay_gesture`
calls never fail a connect, disconnect, or action. A transport failure drops the
companion runtime and routes subsequent operations through ADB; a per-method
failure (for example, the accessibility service unavailable, reported via
`glow_supported=false`) is swallowed because the action itself already succeeded
and only the cosmetic overlay is unavailable. Sessions with no reachable
companion are a no-op for this plane and fall back to the screenshot-synthetic
cursor. The host no longer draws the phone cursor on the desktop; the
`host_cursor_state`/`HostCursorDraw` desktop bridge for phone actions was
removed.

Capability profiles capture device identity (manufacturer/brand/model/codename,
Android SDK/release, HyperOS version when detected), display metrics,
connection kind, companion capabilities, scrcpy capabilities, and detectable
privileged state (root, Shizuku, device/profile owner). Privileged state is
reported and used only for low-risk setup; baseline operation never depends on
it.

As-built behavior notes:

- Snapshot guard: coordinate actions reject a snapshot whose display geometry no
  longer matches the live device. An orientation change (swapped width/height)
  returns `PhoneSnapshotOrientationMismatch`; a resolution change returns
  `PhoneSnapshotResolutionMismatch`. Both are structured rejections, not silent
  remaps.
- Cache invalidation triggers: a capture observing display/orientation drift, or
  an observe seeing a wireless drop, invalidates the cached profile. The
  `profile_refresh_state`/`stale` pair reports the result. Permission re-probe is
  a documented TODO and is not yet wired as an invalidation trigger.
- scrcpy resilience: a 2-second watchdog detects a mid-session scrcpy crash,
  downgrades the scrcpy capability, and hides the host overlay; the host overlay
  is remapped when the mirror window is resized; and an explicit-serial connect
  can adopt an already-open mirror window. The host overlay marker is
  native-point-only, which closes the cross-surface synthetic-cursor leak.
- Orientation probe: a live `dumpsys` read feeds `profile.orientation` so the
  cached profile tracks the device's current rotation.
- Notification affordances: events carry real `can_open`/`can_dismiss`/`ongoing`
  plus `ranking`. Immutable `PendingIntent`s are accepted for plain open/action
  sends and return a structured `immutable` error only when fill-in data is
  required, such as inline reply. A `full`-redacted event reports
  `can_open=false`.
- ADB diagnostics: `force_stop` verifies the stop via a foreground re-check, a
  wireless `adb connect` failure surfaces a `PhoneConnectFailed` diagnostic, and
  `phone_app_install` reports an `install_strategy` of `single`, `multiple`, or
  `multi_package`.
- Companion identity reporting: the reachable report includes the installed
  signing-cert SHA-256, and the expected `apk_sha256` is surfaced (report-only)
  from config `companion_apk_sha256` (env override
  `SKY_CUA_PHONE_COMPANION_APK_SHA256`, allowlisted).

### Configuration knobs (wired)

These `[phone]` keys are resolved into `ResolvedPhoneSelection`
(`crates/sky-cua-platform/src/config.rs`) with per-process environment overrides
and consumed by the service. Each is a no-op at its default value, so default
behavior is unchanged.

- `visible_overlay` (default `true`; env `SKY_CUA_PHONE_VISIBLE_OVERLAY`):
  whether the on-phone agent overlay (edge glow, cursor, per-action gesture
  animation) is shown. When `false`, the host suppresses every companion
  visible-overlay call — `overlay_active` on connect/disconnect/refresh and
  `overlay_gesture` after an action — so the device draws no agent overlay, while
  the real input still dispatches. The session's reported cursor capabilities and
  backend capabilities then carry `host_visible_overlay=false` and
  `phone_native_overlay=false` with a config-grounded `visible_overlay_reason`.
  The screenshot-synthetic cursor is a separate plane driven by
  `screenshot_cursor` and is unaffected. ADB-only sessions continue to report
  `visible_overlay=false` regardless, preserving the Phase 3 contract.
- `wireless_auto_connect` (default `false`; env
  `SKY_CUA_PHONE_WIRELESS_AUTO_CONNECT`): when `true` and the configured
  `default_serial` is a wireless `host:port` target, `phone_connect` runs `adb
  connect <host:port>` for that default before serial resolution, so a
  pre-configured wireless link is brought up and the device becomes present for
  targeting. Best-effort and idempotent; a failed link resurfaces as the normal
  device-unavailable diagnostic. A USB/emulator default or an empty default is a
  no-op.
- `companion_operator_mode` (default `true`; env
  `SKY_CUA_PHONE_COMPANION_OPERATOR_MODE`): gates the host-side privileged
  operator conveniences. As wired, it gates the *silent* companion auto-install
  convenience (`adb install -r`): the automatic install/update on connect or
  `phone_refresh_capabilities` runs only when operator mode AND
  `companion_auto_install` are both on. An explicit `phone_install_companion`
  (or `install_companion: true` on connect) is the operator acting deliberately
  and still installs regardless of operator mode. With operator mode off, an
  already-installed companion still connects (forward + token + probe); only the
  unattended install convenience is suppressed. This is the minimal honest gate;
  no settings-screen automation or deeper privileged setup exists yet to gate.
- `primary_target_models` (default empty; env `SKY_CUA_PHONE_TARGET_MODELS`,
  comma-separated): the operator's known device models. In the
  `phone_list_devices` path, each device whose reported `model` matches a
  configured target (case-insensitive, trimmed) is marked `primary=true` and
  stably sorted ahead of the rest, preserving adb's order within each group. The
  `PhoneDevice.primary` field is omitted from the wire when `false`. An empty
  target list leaves the listing byte-identical to adb's order.

## Source paths

- Platform model and contracts: `crates/sky-cua-platform/src/model/phone.rs`,
  `crates/sky-cua-platform/src/model/service.rs`,
  `crates/sky-cua-platform/src/config.rs`.
- Service backends and routing:
  `crates/sky-cua-service/src/phone/adb.rs`, `command.rs`, `device.rs`,
  `snapshot.rs`, `mapping.rs`, `cursor.rs`, `scrcpy.rs`,
  `crates/sky-cua-service/src/phone/companion/` (`protocol.rs`, `client.rs`,
  `identity.rs`), and the `manager/` and `device/` submodules.
- MCP client tools: `crates/sky-cua-client/src/mcp_tools/`.
- Companion wire contract: `docs/runtime/phone-companion-protocol.md`
  (authoritative).
- Packaging: `scripts/build_plugin.py`, `scripts/_plugin_bundle.py`.
- Bundled skill: `skills/phone-use/`.
- Optional companion artifacts: `resources/android/phone-companion.apk` and
  `resources/android/phone-companion.json` (conditional; produced by the
  Android lane).

## Verification

Source proof (run from the repo root):

```bash
cargo fmt --check && cargo test
python3 scripts/build_plugin.py
```

`cargo test` covers the platform phone model, service ADB/snapshot/mapping/
cursor/scrcpy backends, the companion RPC client (success, timeout, malformed
payload, version mismatch, token mismatch, fallback), and the client MCP
surface (tool list, schemas, response shaping). `build_plugin.py` stages
`skills/phone-use` into the bundle and conditionally stages the companion APK.

Live and installed proof (pending hardware/packaging run):

```bash
uv run python scripts/live_phone_use_smoke.py --profile adb-usb --serial <serial>
uv run python scripts/live_phone_use_smoke.py --profile full --serial <serial>
```

The full live smoke must run from the installed MCP surface after packaging and
record adb version, Android version, companion version when installed, scrcpy
version when used, connection kind, cursor planes proven, and skipped profiles.

## Known limitations

- Live device proof is pending: no installed-MCP `tools/list` proof and no
  full live-smoke run is recorded yet. Treat all backends as source-complete,
  not field-proven.
- Redmi/API-36 tablet lane is blocked until that device is connected. The
  Redmi-family target is complete only when a real device reports HyperOS 3.1
  (or equivalent), Android release 16, and SDK 36 through ADB/capability
  evidence; API 35 is only a documented launch-baseline fallback.
- The companion APK and its identity sidecar may not be built yet. Packaging
  stages them conditionally and never hard-requires them; without them only the
  ADB baseline and (if present) scrcpy paths are exercised. Building the
  companion requires JDK 21 (the host default `java` is unsupported by AGP)
  plus SDK build-tools/platforms 35–37; that toolchain note lives with the
  Android build survey research.
- Companion signature-mismatch recovery is intentionally manual: a same-named
  package whose certificate differs from packaged metadata is refused, not
  silently replaced.
- ADB `input tap`/`swipe` dispatch coordinates in the device's displayed
  (rotated) frame — the same frame the agent's screenshot uses — so no rotation
  transform is applied on the ADB input path. Validated 2026-06-17 on a Samsung
  Galaxy S24 (SM-S948B) via a dialer-keypad A/B in forced landscape: tapping a
  key at its displayed-frame position entered the digit; the natural-frame
  transform did not. The `device_point_to_natural`/`build_mapping` helpers
  remain as unused scaffolding for any device that proves to need natural-frame
  input. Re-validation note: rotation reads from `dumpsys display`
  (`mCurrentOrientation=N`, N in 0..3) because this Samsung's `dumpsys input`
  exposes no `SurfaceOrientation` line.
- The companion's exported `SetupActivity` lets a malicious co-resident app
  inject its own RPC token (local privilege escalation). Accepted as risk on
  2026-06-17 for trusted local use; the logcat-readback handshake redesign is
  the documented future hardening fix. See the "Security and threat model"
  section of `docs/runtime/phone-companion-protocol.md`.
- Clearing app data/cache is deferred. Direct scrcpy protocol consumption and
  Appium/WebDriver-style semantic automation are out of scope for v1.
- Permission re-probe is not yet an invalidation trigger: a runtime permission
  change (for example, the accessibility or notification-listener grant being
  revoked mid-session) does not currently invalidate the cached profile. It is a
  documented TODO; the existing reconnect/refresh paths are the workaround.
- Overlay-free window capture (`takeScreenshotOfWindow`) is not implemented. The
  companion captures the full display via the accessibility-service screenshot
  API; the companion KDoc and config reflect this display-wide capture honestly,
  and the `oem_policy` screenshot error code is reserved/unreachable on that
  route.

## Related

- Originating ExecPlan: `plans/phone-use.md` (Phase 6 packaging/skill/docs).
- Companion protocol: `docs/runtime/phone-companion-protocol.md`.
- Research: `docs/research/2026-06-phone-use-architecture.md`,
  `docs/research/2026-06-phone-use-agent-cursor-overlay.md`,
  `docs/research/2026-06-phone-use-android-helper-app.md`,
  `docs/research/2026-06-phone-use-capabilities-and-target-devices.md`,
  `docs/research/2026-06-phone-use-android-build-survey.md`.
- ROADMAP: "Phase: Android phone control (phone-use)".
