# Phone-use Android control with wireless, companion app, and agent cursor

This ExecPlan is a living document. The sections `Progress`,
`Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must
be kept up to date as work proceeds.

This plan follows `~/.agents/PLANS.md` and `plans/AGENTS.md`. It is written so a
stateless implementer can start from this file and deliver a working feature
without relying on conversation history.

## Purpose / Big Picture

After this work, an agent using sky-cua can control a real Android phone from
the existing sky-cua MCP server. The phone can be discovered, paired, connected,
inspected, and controlled over USB or wireless ADB. The agent's normal entry
point is `phone_observe`, which returns a fresh device-specific observation:
screenshot, snapshot id, current app, recent notification summary, accessibility
summary when available, cursor state, backend capability profile, and a dynamic
list of available actions. The agent can then tap or swipe phone-screen
coordinates, type text, press Android keys, launch or manage apps, act on
notifications by explicit IDs, and verify the result through a new phone
snapshot.

The product surface is `phone-use`. Agents should reason in terms of phones,
sessions, screens, snapshots, cursor state, capabilities, and Android actions.
ADB, the Android companion app, and scrcpy are backend implementations behind
that contract.

The design has three runtime layers:

- ADB baseline: required for discovery, USB and wireless pairing/connect,
  device property probing, companion install/update, diagnostics, fallback
  screenshots, fallback input, and all recovery flows. ADB means Android Debug
  Bridge, the Android host/device transport used by Android developer tools.
- Android companion backend: the preferred rich/native path for this personal
  full-privilege setup once installed and enabled. It provides phone-native
  cursor overlay, accessibility screen content, native gestures, screenshots,
  notification forwarding, and companion health/capability reporting.
- scrcpy acceleration: optional visual mirror/control path when scrcpy is
  installed and a host-rendered phone surface is useful or the companion is not
  available. scrcpy mirrors and controls Android over USB or TCP/IP without root
  or a phone-side app.

The cursor contract has three planes:

- Screenshot-synthetic cursor: a marker composited into returned phone
  screenshots after successful phone actions, including ADB-only sessions.
- Host-visible cursor overlay: a desktop overlay marker drawn through the
  existing sky-cua overlay host when a scrcpy window or other host preview
  surface exists.
- Phone-native cursor overlay: a transparent overlay drawn directly on the
  Android device by the companion app's AccessibilityService.

As-built cursor contract: the phone-native overlay is the plane that draws the
agent cursor for phone actions, and it now also carries a persistent "agent in
control" breathing screen-edge glow plus per-action animations (tap ripple,
swipe/drag trail). The companion renders all of it from a single full-screen
pass-through `TYPE_ACCESSIBILITY_OVERLAY` view, addressed in Android device
pixels with no host/desktop mapping. The host drives it through new companion RPCs
(`overlay_active` on session hold/release, `overlay_gesture` per action, both
best-effort and visual-only) and hides the overlay around model-facing captures.
The earlier host-desktop draw of the phone cursor was removed (`host_cursor_state`
and the `HostCursorDraw` bridge for phone actions are gone); the host-visible
overlay plane and desktop `OverlayController` now serve only the scrcpy/preview
mirror and real desktop computer-use. See `docs/runtime/phone-companion-protocol.md`
and the 2026-06-18 update in
`docs/research/2026-06-phone-use-agent-cursor-overlay.md`.

The feature is complete only when source tests, adversarial tests, installed MCP
proof, and full live-smoke profiles all demonstrate the specified behavior. A
session is not considered ready until the service has detected and cached the
device's current capability profile and tailored the available action list from
that profile.

## Progress

- [x] (2026-06-17 08:35Z) Read the root project guide, docs guide, plan guide,
  MCP boundary, compat plugin contract, current tool schema layout, service
  request model, overlay model, and supplied scrcpy recommendation.
- [x] (2026-06-17 08:45Z) Verified current official scrcpy evidence from
  Genymobile sources: scrcpy 4.0 was current as of 2026-05-12; official docs
  cover USB/TCP/IP selection, video options, V4L2 output, control sockets, and
  the internal/version-specific nature of the direct protocol.
- [x] (2026-06-17 09:00Z) Chose the product boundary: `phone-use` is a first
  class tool family and bundled skill served by the existing
  `sky-cua-client mcp` process.
- [x] (2026-06-17 09:40Z) Recorded broader architecture research in
  `docs/research/2026-06-phone-use-architecture.md`: keep `phone-use` in
  sky-cua and the same MCP process, make ADB the connection/baseline backend,
  and make scrcpy an acceleration backend.
- [x] (2026-06-17 10:15Z) Recorded cursor/overlay research in
  `docs/research/2026-06-phone-use-agent-cursor-overlay.md`: reuse sky-cua's
  overlay model, synthesize cursor markers into phone screenshots, and bridge to
  the host-visible overlay when a scrcpy or preview surface exists.
- [x] (2026-06-17 10:25Z) Rewrote this plan as an ExecPlan that links research,
  official docs, existing sky-cua runtime code, packaging seams, and validation
  gates.
- [x] (2026-06-17 10:55Z) Added helper Android app research in
  `docs/research/2026-06-phone-use-android-helper-app.md`: a companion app can
  provide native overlay, accessibility tree, gesture, screenshot, and
  notification capabilities.
- [x] (2026-06-17 11:05Z) Updated the plan for personal/operator privileged use:
  the helper app is promoted to the preferred rich backend after ADB bootstrap,
  with scrcpy retained as visual acceleration/fallback.
- [x] (2026-06-17 11:40Z) Reorganized this plan into dependency-ordered phases
  with per-phase proof gates, adversarial testing objectives, and full live-smoke
  acceptance.
- [x] (2026-06-17 12:10Z) Added capability and target-device research in
  `docs/research/2026-06-phone-use-capabilities-and-target-devices.md`: session
  start should detect/cache device capabilities, `phone_observe` should become
  the primary perception tool, companion install/update should happen during
  session start, notifications should be actionable by explicit IDs, and app
  management should be included in v1.
- [x] (2026-06-17 12:45Z) Completed a second-pass consistency review and a
  third-pass parallelization rewrite: resolved the duplicate app-launch tool,
  moved config/env/companion identity into Phase 1, strengthened API 36 target
  acceptance, added Android API failure-mode requirements, and reorganized
  implementation around a contract spine plus parallel worker lanes.
- [x] (2026-06-17 13:20Z) Phase 0: surveyed repo-local Android build
  conventions and host dependencies in
  `docs/research/2026-06-phone-use-android-build-survey.md`. No pre-existing
  Android project; `phone-use` mirrors the existing `browser` family. `adb`
  1.0.41 and `scrcpy` 4.0 present. Android build viable with JDK 21 (default
  `java` is 26, unsupported by AGP) + SDK build-tools/platforms 35-37, so
  `compileSdk`/`targetSdk` 36 is buildable at `android/phone-companion/`. A
  real Galaxy `SM-S948B` is attached over wireless ADB on Android 16 / API 36
  (`172.16.255.58:38781`), enabling live API-36 proof; the Redmi tablet lane
  stays blocked until that device is connected.
- [x] (2026-06-19) Phase 1 source landed: phone platform contracts, service
  routing, config/env selection, MCP schemas, fake backends, capability profiles,
  action availability, and `phone_observe`.
- [x] (2026-06-19) Phase 2 source landed: ADB discovery, wireless pair/connect,
  fallback screenshots/input, device property probing, and baseline smoke
  harness coverage.
- [x] (2026-06-19) Phase 3 source landed: phone snapshot ids, coordinate
  mappings, stale snapshot rejection, and screenshot-synthetic cursor behavior.
- [x] (2026-06-19) Phase 4 source landed: Android companion app, ADB
  install/update, RPC tunnel, native overlay, accessibility tree, gestures,
  screenshots, notifications, and companion smoke harness coverage.
- [x] (2026-06-19) Phase 5 source landed: scrcpy acceleration, host-window
  mapping, host-visible overlay gating, crash/codec fallback, adoption, and
  scrcpy smoke harness coverage.
- [ ] Phase 6: bundled `skills/phone-use`, packaging, docs, and staged bundle
  support have source-landed; installed MCP proof remains open until the package
  is built, installed, and read back from the live MCP `tools/list` surface.
- [ ] Phase 7: run adversarial tests for permissions, stale state, wireless
  drops, multi-device routing, malformed inputs, process crashes, and sensitive
  data bounding.
- [ ] Phase 8: run the full live-smoke matrix and record evidence in this plan's
  `Outcomes & Retrospective`.

## Surprises & Discoveries

- Observation: The current sky-cua bundle contract intentionally exposes one MCP
  server named `computer-use`; browser tools are already a tool family inside
  the same `sky-cua-client mcp` process.
  Evidence: `.mcp.json`, `docs/runtime/mcp-boundary.md`, and
  `docs/runtime/compat-plugin-contract.md`.

- Observation: ADB, not scrcpy, is the authority for wireless pairing,
  reconnect, serial selection, authorization state, and mDNS diagnostics.
  Evidence: Android adb docs describe Android 11+ wireless debugging with
  `adb pair`, same-network requirements, one-time pairing, `adb devices -l`,
  `ANDROID_SERIAL`, legacy `adb tcpip` / `adb connect`, and mDNS diagnostics.

- Observation: scrcpy is still the best optional host visual mirror/control
  implementation.
  Evidence: scrcpy docs describe Android mirroring and control over USB or
  TCP/IP without root or a phone-side app, including video size, frame rate,
  codec, V4L2, and control modes.

- Observation: Directly consuming scrcpy's raw stream/control protocol is a bad
  first seam.
  Evidence: scrcpy developer docs describe video/audio/control sockets, but
  state the protocol is internal, may change between versions, and requires
  exact client/server version matching.

- Observation: sky-cua already has the right host/screenshot cursor
  architecture.
  Evidence: `docs/features/agent-cursor-overlay.md`,
  `crates/sky-cua-platform/src/model.rs`, and
  `crates/sky-cua-service/src/overlay.rs` model separate `UserVisible` and
  `ScreenshotSynthetic` planes.

- Observation: scrcpy's `--show-touches` option is not a substitute for an
  agent cursor.
  Evidence: scrcpy documentation describes show-touches as showing physical
  touches, and downstream manpage text says scrcpy clicks are not shown.

- Observation: Android-native overlays are possible, and the right first native
  path is the companion AccessibilityService.
  Evidence: Generic `TYPE_APPLICATION_OVERLAY` windows require
  `SYSTEM_ALERT_WINDOW`, while `TYPE_ACCESSIBILITY_OVERLAY` belongs to an
  enabled AccessibilityService with the right service metadata/capabilities. The
  accessibility overlay keeps the cursor tied to the same service that owns
  semantic screen content and gesture dispatch.

- Observation: A helper Android app can do more than draw a cursor.
  Evidence: Android AccessibilityService supports retrieving active window
  content, dispatching gestures, and taking screenshots on supported API levels;
  NotificationListenerService receives posted/removed notification events;
  MediaProjection can capture screens but has stricter consent and foreground
  service requirements.

- Observation: This feature is for personal, full-privilege operator use.
  Evidence: The target deployment can assume sideloading, ADB install/update,
  explicit permission enablement, and optional root/Shizuku/device-owner paths
  where available, rather than optimizing for public consumer-app distribution.

- Observation: The primary target devices should be treated as runtime-discovered
  devices, not hard-coded spec profiles.
  Evidence: Samsung has official Galaxy S26 Ultra pages, but Xiaomi official
  sources did not surface an exact "Redmi Pad 15 Pro" page during research.
  Both devices must therefore be identified by ADB properties and companion
  capability reports at session start.

- Observation: The Android APIs needed for richer requirements are available but
  capability-gated.
  Evidence: AccessibilityService provides active-window access, gesture
  dispatch, accessibility overlays, and screenshot APIs on supported API levels;
  NotificationListenerService provides notification posted/removed callbacks;
  PackageManager supports launchable app discovery and launch intents; adb
  supports install/update through `adb install -r`.

- Observation: The first plan draft over-serialized independent work.
  Evidence: Snapshot/mapping tests can run against fake sessions before real ADB
  lands; MCP schema tests can run against fake phone responses; scrcpy process
  tests do not need the companion; Android app scaffolding only needs the
  protocol shape after the contract spine.

- Observation: Companion auto-install/update needs a package identity contract
  before backend implementation.
  Evidence: `phone_connect` cannot safely compare installed version/signature or
  decide downgrade behavior unless Phase 1 defines package id, APK path,
  signing fingerprint metadata, and config/env overrides.

- Observation: Accessibility overlay and generic application overlay have
  different Android permission and touch behavior.
  Evidence: `TYPE_APPLICATION_OVERLAY` requires `SYSTEM_ALERT_WINDOW`.
  `TYPE_ACCESSIBILITY_OVERLAY` is owned by an enabled AccessibilityService and
  should be smoke-tested as non-focusable, non-touchable, pass-through cursor
  UI.

## Decision Log

- Decision: Implement `phone-use` in this repository and expose it from the
  existing `sky-cua-client mcp` process as a new `phone_*` tool family.
  Rationale: sky-cua already owns the agent-facing CUA runtime, plugin
  packaging, installed proof, screenshot delivery conventions, tool annotations,
  service daemon, and Codex Desktop compatibility contract. A second enabled MCP
  server would duplicate those seams and compete for shared desktop/session
  state.
  Date/Author: 2026-06-17 / Codex

- Decision: Keep the implementation internally separable.
  Rationale: `phone-use` should be easy to extract later if a standalone MCP
  becomes valuable. Start with a distinct platform model module and
  `crates/sky-cua-service/src/phone/`; promote to a dedicated
  `crates/sky-cua-phone/` crate only after the boundary is proven.
  Date/Author: 2026-06-17 / Codex

- Decision: Make ADB required.
  Rationale: Wireless support, install/update, fallback control, fallback
  screenshots, recovery, and companion bootstrapping all depend on ADB. Missing
  ADB should disable phone-use with structured diagnostics. Missing companion or
  missing scrcpy should degrade capability, not disable baseline phone-use.
  Date/Author: 2026-06-17 / Codex

- Decision: Treat wireless as a first-class workflow in the public contract.
  Rationale: The user specifically wants wireless use. The tool surface must
  expose pairing, connect/reconnect, mDNS diagnostics, connection kind, selected
  serial, and explicit disconnect ownership instead of hiding wireless behind
  scrcpy flags.
  Date/Author: 2026-06-17 / Codex

- Decision: Reuse sky-cua's existing overlay model for host-visible and
  screenshot-synthetic cursor behavior.
  Rationale: Existing code already separates host-visible overlays from
  screenshot-synthetic cursor composition. Phone-use needs device-to-host
  coordinate mapping and per-session state, not a parallel host overlay
  architecture.
  Date/Author: 2026-06-17 / Codex

- Decision: Implement phone-native overlay through the companion
  AccessibilityService; defer generic application-overlay and direct scrcpy
  protocol approaches.
  Rationale: Accessibility overlay keeps the cursor tied to the helper service
  that owns semantic screen content and gestures. Generic application overlays
  require `SYSTEM_ALERT_WINDOW` and are less precise for this backend. Direct
  scrcpy protocol use binds sky-cua to an explicitly internal protocol.
  Date/Author: 2026-06-17 / Codex

- Decision: Add the Android companion backend as the preferred rich backend
  after the ADB bootstrap, but do not make it required for basic phone-use.
  Rationale: A helper app can provide a native cursor overlay, accessibility
  tree, notification events, native screenshots, and gesture dispatch. For this
  personal operator use case, install and permission setup are acceptable. ADB
  remains the bootstrap/rescue layer; scrcpy remains a useful visual
  mirror/fallback.
  Date/Author: 2026-06-17 / Codex

- Decision: Treat live-smoke and adversarial testing as release objectives, not
  optional QA follow-up.
  Rationale: The feature spans host tools, a long-lived daemon, ADB, wireless
  network state, an Android app, Android permissions, overlays, screenshots, and
  installed plugin packaging. Unit tests alone cannot prove the product works.
  Date/Author: 2026-06-17 / Codex

- Decision: Detect and cache a `PhoneCapabilityProfile` when a session starts,
  and tailor available actions from that profile.
  Rationale: The Galaxy S26 Ultra and Redmi Pad 15 Pro may differ by Android
  version, OEM policy, screen geometry, companion permissions, notification
  access, root/Shizuku/device-owner state, scrcpy support, and wireless state.
  Caching a session profile makes backend routing deterministic while still
  allowing explicit refresh after permission or device-state changes.
  Date/Author: 2026-06-17 / Codex

- Decision: Keep the MCP tool list static but make action affordances dynamic.
  Rationale: Static tools preserve installed MCP compatibility and tool-list
  proof. `phone_observe`, `phone_status`, and action responses should expose
  `available_actions` and `unavailable_actions` with reasons so the agent sees a
  device-tailored action menu without requiring session-scoped dynamic MCP
  registration.
  Date/Author: 2026-06-17 / Codex

- Decision: `phone_connect` auto-installs or updates the companion app when
  companion support is enabled.
  Rationale: For personal/operator use, a session should arrive ready to use the
  rich backend. ADB can check installed package version/signature, install with
  `adb install -r`, set up port forwarding, and then run capability detection.
  Date/Author: 2026-06-17 / Codex

- Decision: Add `phone_observe` as the primary perception tool.
  Rationale: Agents should not have to stitch together screenshot, current app,
  accessibility summary, notifications, cursor state, and backend capability
  state manually every turn. Raw tools remain available for focused work.
  Date/Author: 2026-06-17 / Codex

- Decision: Notifications are actionable in v1, but only through explicit IDs.
  Rationale: Opening, dismissing, invoking action buttons, and inline replies are
  valuable. Requiring fresh notification/action IDs prevents ambiguous low-level
  commands like "reply to the latest message" from becoming unsafe primitives.
  Date/Author: 2026-06-17 / Codex

- Decision: Include a small app-management family in v1.
  Rationale: Launch-only app control is too thin for real phone use. V1 should
  include current foreground app, list launchable apps, launch package,
  launch/deep-link intent URI, force-stop, install/update APK, and open setup
  settings screens. Clearing app data/cache is deferred.
  Date/Author: 2026-06-17 / Codex

- Decision: Use `phone_app_launch` as the single public app-launch tool.
  Rationale: The draft tool list had a standalone launch-app tool and an
  app-management launch tool. Keeping both would create duplicate schemas and
  ambiguous guidance. App launch belongs with the app-management family, so
  `phone_app_launch` is canonical.
  Date/Author: 2026-06-17 / Codex

- Decision: Implement with a contract-first barrier followed by parallel
  backend lanes.
  Rationale: ADB, companion, scrcpy, smoke scripts, adversarial tests, and
  bundled skill work can proceed in parallel only after the public model,
  request/response enums, config keys, and tool names are stable. Shared model
  files should have one owner; backend workers should write disjoint modules and
  integrate through the frozen contract.
  Date/Author: 2026-06-17 / Codex

- Decision: Treat companion package identity, permissions, and RPC tokens as
  first-class product requirements.
  Rationale: Auto-install/update, Accessibility Service, Notification Access,
  and ADB-forwarded RPC are the parts most likely to fail partially. The plan
  must require explicit package signing checks, structured disabled-permission
  diagnostics, settings launch helpers, and per-session token validation instead
  of letting the implementation rely on implicit local trust.
  Date/Author: 2026-06-17 / Codex

- Decision: The companion setup-token delivery path (`SetupActivity`) keeps the
  documented accepted-risk posture for v1. Do not add a `getLaunchedFromUid()`
  caller-UID gate or a signature-level custom permission to gate token install.
  Rationale: `getLaunchedFromUid()` is identity-share gated and only available on
  API 34+, while the companion's `minSdk` is 30; for an `adb shell am start`
  launch it returns `INVALID_UID`, so a UID gate would break 100% of token
  delivery without closing the local-privilege-escalation window. A signature
  permission is likewise undeliverable over adb. The mitigation of record is the
  logcat-readback handshake documented in
  `docs/runtime/phone-companion-protocol.md` (the companion mints the token,
  emits it to logcat, and the host reads it back via `adb logcat`); it remains
  planned future hardening, not a v1 blocker, because this is a personal,
  full-privilege operator deployment where a local actor with adb already holds
  broad control. An earlier ultra-review fix that added the UID gate was reverted
  as a regression.
  Date/Author: 2026-06-19 / Operator decision

- Decision: `companion_operator_mode` gates only the silent companion
  auto-install/update path, not companion use as a whole.
  Rationale: When `companion_operator_mode` is false, `phone_connect` still
  detects and reports companion state and explicit `phone_install_companion`
  remains available; it only suppresses the silent APK push during connect. When
  true (the default), the operator-trust auto-install during `phone_connect`
  proceeds. This keeps the no-companion and explicit-install flows usable while
  scoping the "act without asking" behavior to an opt-in operator-trust signal.
  Date/Author: 2026-06-19 / Operator decision

- Decision: `primary_target_models` marks devices, it does not filter them.
  Rationale: A configured model match adds a `PhoneDevice.primary` boolean
  (serialized skip-if-false) and stable-sorts matching devices ahead of the rest
  in `phone_list_devices`; it never hides non-matching devices. Filtering would
  silently drop a connected-but-unlisted device (e.g. a new test handset) and
  contradict the runtime-discovery requirement that devices be identified by ADB
  properties at session start rather than by a hard-coded allowlist.
  Date/Author: 2026-06-19 / Operator decision

## Outcomes & Retrospective

Source implementation is now present for Phases 1-5 and partial Phase 6:
contracts, MCP tool definitions, service routing, ADB control, wireless
pair/connect, snapshot mapping, cursor planes, Android companion, notification
affordances, scrcpy acceleration, bundled skill, feature docs, runtime protocol
docs, and smoke/test harnesses. The open completion gates are live-device proof
on the target Galaxy/Redmi devices, adversarial testing across the Phase 7
matrix, staged package inspection, and installed MCP `tools/list` readback from
the deployed bundle.

Current automated proof recorded during review:

- `cargo test -p sky-cua-service phone::manager::tests::`
- `cargo fmt --check && cargo check -p sky-cua-service -p sky-cua-client -p sky-cua-platform`
- `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest scripts/test_live_phone_use_smoke.py`
- `JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT=$HOME/Android/Sdk ./gradlew :app:testDebugUnitTest` in `android/phone-companion/`

At final completion, record the real device or emulator used for proof, Android
version, adb version, scrcpy version, companion app version, connection modes
exercised, permission state, cursor proof for all three planes, adversarial
failures tested, staged bundle shape, and installed MCP proof.

## Context and Orientation

sky-cua is a Rust workspace plus Python harnesses. The MCP server entrypoint is
`./bin/sky-cua-client mcp`. MCP means Model Context Protocol: it is the tool
server process that exposes actions such as screenshots and clicks to an agent.
The MCP boundary is documented in `docs/runtime/mcp-boundary.md`.

The MCP client crate exposes tools, talks to the long-lived `sky-cua-service`
daemon, and uses structured request/response values from
`crates/sky-cua-platform/src/model/service.rs`. A daemon is a background service
that owns runtime state and serializes operations that must not race, such as
capture, input, overlays, and phone sessions.

Browser tools are already a tool family inside the same MCP process. `phone-use`
must follow that pattern. The new platform model lives in
`crates/sky-cua-platform/src/model/phone.rs`, routes through
`ServiceRequest::Phone { request: PhoneRequest }`, and returns
`ServiceResponse::Phone { response: PhoneResponse }`.

The service daemon in `crates/sky-cua-service/src/daemon.rs` owns runtime
ordering. It already has a service-owned overlay controller in
`crates/sky-cua-service/src/overlay.rs`. The phone manager must be another
service-owned runtime lane, not a loose MCP helper that shells out independently
of daemon state.

Important existing files to reuse or adapt:

- `crates/sky-cua-platform/src/model/service.rs` for `ServiceRequest` and
  `ServiceResponse` extension points.
- `crates/sky-cua-platform/src/model.rs` for existing cursor, coordinate,
  rectangle, screenshot, and diagnostic model patterns.
- `crates/sky-cua-platform/src/config.rs` for machine config extension.
- `crates/sky-cua-service/src/daemon.rs` for service dispatch and runtime state
  ownership.
- `crates/sky-cua-service/src/overlay.rs` for visible and synthetic cursor
  behavior.
- `crates/sky-cua-client/src/mcp_tools/` for tool registration, schemas,
  annotations, dispatch, and tests.
- `scripts/build_plugin.py` for bundled skill and package shape.
- `.mcp.json` for MCP server name and environment allowlist.
- `.codex-plugin/plugin.json` for plugin identity and payload contract.

Terms used in this plan:

- ADB: Android Debug Bridge, the host/device transport used for discovery,
  shell commands, pairing, screenshots, input, installs, and port forwarding.
- Wireless debugging: Android 11+ pairing with `adb pair host:port` and a
  pairing code shown on the phone, followed by same-network wireless ADB.
- Legacy TCP/IP: the older wireless flow where USB enables `adb tcpip 5555`,
  then the host connects to `device_ip:5555`.
- Device serial: the target string accepted by ADB and scrcpy, including USB
  serials, emulator serials, and TCP/IP `host:port` serials.
- Phone snapshot: a captured phone screenshot plus metadata, cursor state,
  device dimensions, backend, and coordinate mapping id.
- Coordinate mapping: the data needed to translate a point from Android display
  pixels into screenshot pixels and, when possible, host desktop pixels.
- AccessibilityService: an Android service the user enables in Accessibility
  settings. It can inspect screen/window structure, dispatch gestures, create an
  accessibility overlay, and on supported API levels take screenshots.
- NotificationListenerService: an Android service the user enables in
  Notification Access settings. It receives notification posted/removed events.
- RPC: remote procedure call. In this plan it means a small typed HTTP/WebSocket
  protocol between the host service and the companion app through ADB port
  forwarding.
- PhoneCapabilityProfile: a structured, cached description of what this device
  and session can do right now. It includes device properties, Android API
  level, display metrics, ADB state, companion version/permissions, notification
  support, screenshot support, gesture support, scrcpy support, privileged setup
  state, and available/unavailable actions.

## Research and Documentation

Local research:

- `docs/research/2026-06-phone-use-architecture.md` records the ADB baseline
  plus scrcpy acceleration architecture and the same-MCP recommendation.
- `docs/research/2026-06-phone-use-agent-cursor-overlay.md` records the
  host-visible, screenshot-synthetic, and phone-native cursor planes.
- `docs/research/2026-06-phone-use-android-helper-app.md` records the
  operator-mode Android companion design for native overlay, accessibility
  snapshots, gestures, screenshots, and notifications.
- `docs/research/2026-06-phone-use-capabilities-and-target-devices.md` records
  capability detection, backend routing, auto-install/update, target-device
  assumptions, notification actions, and app-management recommendations.

Existing sky-cua docs:

- `docs/runtime/mcp-boundary.md` documents the host-facing MCP entrypoint and
  installed proof expectations.
- `docs/runtime/compat-plugin-contract.md` documents the single `computer-use`
  MCP server compatibility contract.
- `docs/features/agent-cursor-overlay.md` documents the current overlay
  architecture.
- `docs/features/compositor-cursor-hiding.md` documents compositor limitations
  that matter when scrcpy is shown in a host window.

Primary external docs checked on 2026-06-17:

- Android adb docs: https://developer.android.com/tools/adb
- Android `SYSTEM_ALERT_WINDOW`: https://developer.android.com/reference/android/Manifest.permission#SYSTEM_ALERT_WINDOW
- Android `TYPE_APPLICATION_OVERLAY`: https://developer.android.com/reference/android/view/WindowManager.LayoutParams#TYPE_APPLICATION_OVERLAY
- Android `TYPE_ACCESSIBILITY_OVERLAY`: https://developer.android.com/reference/android/view/WindowManager.LayoutParams#TYPE_ACCESSIBILITY_OVERLAY
- Android `AccessibilityService`: https://developer.android.com/reference/android/accessibilityservice/AccessibilityService
- Android `NotificationListenerService`: https://developer.android.com/reference/android/service/notification/NotificationListenerService
- Android `PackageManager`: https://developer.android.com/reference/android/content/pm/PackageManager
- Android MediaProjection: https://developer.android.com/media/grow/media-projection
- Android SDK platform release notes / Android 16 API 36: https://developer.android.com/tools/releases/platforms
- Android 15 SDK setup / API 35: https://developer.android.com/about/versions/15/setup-sdk
- Android Play target API requirement: https://developer.android.com/google/play/requirements/target-sdk
- adb manpage: https://android.googlesource.com/platform/packages/modules/adb/+/refs/heads/master/docs/user/adb.1.md
- Xiaomi Redmi Pad 2 Pro FAQ: https://www.mi.com/global/support/faq/details/KA-608151/
- Xiaomi Redmi Pad 2 Pro 5G FAQ: https://www.mi.com/global/support/faq/details/KA-611514/
- Xiaomi HyperOS 3 global page: https://www.mi.com/global/event/2025/hyperos-2025/
- Third-party Xiaomi ROM tracker corroborating Redmi Pad 2 Pro 5G HyperOS 3.1 /
  Android 16 builds: https://mirom.ezbox.idv.tw/en/phone/organ/
- scrcpy repository and release notes: https://github.com/Genymobile/scrcpy
- scrcpy connection docs: https://github.com/Genymobile/scrcpy/blob/master/doc/connection.md
- scrcpy video docs: https://github.com/Genymobile/scrcpy/blob/master/doc/video.md
- scrcpy control docs: https://github.com/Genymobile/scrcpy/blob/master/doc/control.md
- scrcpy developer docs: https://github.com/Genymobile/scrcpy/blob/master/doc/develop.md
- scrcpy V4L2 docs: https://github.com/Genymobile/scrcpy/blob/master/doc/v4l2.md
- scrcpy device docs: https://github.com/Genymobile/scrcpy/blob/master/doc/device.md

Reference projects checked:

- `JuanCF/scrcpy-mcp`: useful evidence that practical mobile MCP control tends
  to combine scrcpy fast paths with ADB fallback.
- `appium/appium-mcp` and WebdriverIO MCP: useful future semantic/testing
  references, but too heavy for the first personal-phone CUA layer.

## Milestone Dependency Map

The work is ordered around a short contract spine and several backend lanes.
Later lanes must not delete or weaken earlier proof. Once the contract spine is
merged, subagents can work in parallel as long as their write scopes stay
disjoint and any requested model/schema change returns to the contract owner
first.

Phase 0 is a read-only survey of existing build conventions and host tools. It
must run before adding the Android app, because the repo may already have an
accepted Android toolchain pattern.

Phase 1 is the contract spine. It creates the Rust model, service request and
response variants, config table and environment allowlist, daemon phone lane,
MCP tool names, fake backends, and tests for `PhoneCapabilityProfile`,
`phone_observe`, dynamic action affordances, notification actions, and
app-management schemas. It has no device dependency and must be merged before
backend lanes change shared contracts.

After Phase 1, the ADB lane, snapshot/cursor lane, MCP/client shaping lane,
companion protocol lane, Android companion lane, scrcpy lane, smoke-harness
lane, and packaging/skill/docs lane can proceed in parallel. The ADB lane proves
USB/wireless discovery, pairing, device property probing, screenshots, input,
companion install/update primitives, and recovery. The snapshot/cursor lane can
start against fake screenshots and fake sessions; it does not need real ADB for
most stale-snapshot, mapping, and cursor tests. The companion protocol and
Android app lanes share the versioned JSON protocol but should otherwise avoid
editing each other's files. The scrcpy lane depends on mapping interfaces, not on
the companion. The smoke-harness lane can add profiles early and leave backend
profiles skipped until their prerequisites land.

Phase 6 packaging and docs can start after tool names stabilize and at least the
ADB lane has source proof. It finishes only after companion and scrcpy packaging
decisions are known, because installed MCP proof must exercise the staged
package, companion APK metadata, and bundled skill.

Phase 7 adversarial testing is not one late cleanup task. Each backend lane owns
its own adversarial tests from the start, and Phase 7 closes the matrix by
verifying that every category is passed, skipped with a reason, or blocked with a
concrete follow-up.

Phase 8 runs the full live-smoke matrix from the installed MCP surface and
records proof. Live profiles that target the same physical phone must not run
concurrently unless the smoke harness implements explicit serial/session
isolation.

## Parallel Implementation Strategy

Use these lanes when assigning parallel subagents. The `contract-spine` lane must
merge first. Any lane that discovers a contract gap should stop and propose the
model/schema edit instead of patching shared files independently.

- `contract-spine`: owns `crates/sky-cua-platform/src/model.rs`,
  `crates/sky-cua-platform/src/model/phone.rs`,
  `crates/sky-cua-platform/src/model/service.rs`,
  `crates/sky-cua-platform/src/config.rs`, minimal daemon phone dispatch, and
  fake manager traits. It validates with platform and daemon routing tests.
- `service-adb`: owns `crates/sky-cua-service/src/phone/adb.rs`,
  `command.rs`, `device.rs`, and parser/fake-runner tests. It depends on the
  contract spine and validates with service ADB tests plus live ADB smoke when a
  target device is available.
- `snapshot-cursor`: owns `crates/sky-cua-service/src/phone/snapshot.rs`,
  `mapping.rs`, `cursor.rs`, and image-composition helpers if needed. It uses
  fake sessions first and validates mapping, stale snapshot rejection,
  per-device isolation, and pixel marker tests.
- `client-mcp`: owns `crates/sky-cua-client/src/mcp_tools/phone.rs`, a phone
  tool definitions module, response shaping tests, and a local tools-list
  fixture. It depends on the contract spine and fake service responses, not on
  ADB.
- `companion-protocol`: owns `crates/sky-cua-service/src/phone/companion.rs`
  and protocol/RPC tests. It depends on the shared DTOs and validates timeout,
  malformed payload, version mismatch, token mismatch, and fallback behavior
  with a fake server.
- `android-companion`: owns `android/phone-companion/**` and Android-side tests.
  It depends on the Phase 0 build survey and initial protocol shape. It should
  not edit Rust service routing.
- `scrcpy`: owns `crates/sky-cua-service/src/phone/scrcpy.rs` and any
  scrcpy-only content-rect helpers. It validates command construction, codec
  retry, process ownership, and host-window mapping without requiring the
  companion.
- `smoke-harness`: owns `scripts/live_phone_use_smoke.py` and focused script
  tests. It can create profile scaffolding early; each backend profile remains
  skipped with an explicit prerequisite until that backend lands.
- `packaging-skill-docs`: owns `skills/phone-use/**`, `scripts/build_plugin.py`,
  `.mcp.json` allowlist packaging, `ROADMAP.md`, and later
  `docs/features/phone-use.md`. It starts after tool names stabilize and
  finishes after installed MCP proof.

The preferred merge order is: survey, contract spine, client MCP fake tools,
ADB baseline, snapshot/cursor, smoke harness ADB profile, companion protocol,
Android companion, host companion integration, scrcpy, packaging/skill/docs,
adversarial closure, installed full smoke.

## Requirements Matrix

The requirements in this section are the product contract. Implementation phases
below exist to deliver and prove these behaviors.

Primary observation requirement: `phone_observe` is the default perception tool.
It returns a screenshot or screenshot reference, `phone_snapshot_id`, current
foreground app, screen size/orientation, cursor state, backend used, cached
capability profile version, `available_actions`, `unavailable_actions`, a
bounded accessibility summary when available, and a bounded recent-notification
summary when enabled. Raw tools such as `phone_screenshot`,
`phone_accessibility_tree`, and `phone_notifications` remain available.

Capability requirement: `phone_connect` detects and caches a
`PhoneCapabilityProfile`. The cache is per session, not global. It is invalidated
on reconnect, companion install/update, permission-state change, orientation or
display-size change, companion RPC failure, wireless disconnect, and explicit
`phone_refresh_capabilities`. Every action response includes the capability
profile id or version it used.

Companion install/update requirement: when companion support is enabled,
`phone_connect` checks whether `Sky Phone Companion` is installed, compares
version and expected signature/hash, installs or updates with `adb install -r`
when missing or stale, sets up ADB port forwarding, and only then finalizes the
session profile. If install/update fails, the session still starts with ADB
baseline capabilities and a structured companion diagnostic.

Backend routing requirement: actions use deterministic routing from the cached
capability profile. `phone_observe` prefers companion semantic state, uses scrcpy
for low-latency visual frames when active or requested, and uses ADB fallback.
Screenshots prefer companion screenshot APIs when available, then scrcpy when
requested/active and mapped, then ADB. Coordinate actions translate from
snapshot coordinates to device coordinates, then dispatch through companion
gestures when available, scrcpy when the snapshot came from scrcpy and host
mapping is current, then ADB. Text input prefers companion input/IME path when
available, then scrcpy keyboard when active, then ADB text fallback. Android
keys prefer companion, then ADB keyevent, then scrcpy only where clearly better.
Every response states which backend actually handled the action.

Accessibility and screenshot API requirement: companion gestures require an
enabled AccessibilityService with `canPerformGestures`; accessibility tree
access requires `canRetrieveWindowContent`; screenshot APIs require the service
screenshot capability and the relevant Android API level. The plan treats
`dispatchGesture` as API 24+, `takeScreenshot` as API 30+, and
`takeScreenshotOfWindow` as API 34+. Screenshot responses must model secure
window failures, service capability failures, OEM policy failures, and throttling
as structured diagnostics.

Native overlay requirement: the companion cursor overlay must be non-focusable
and non-touchable, and live smoke must prove it does not intercept the taps it
visualizes. `TYPE_ACCESSIBILITY_OVERLAY` is preferred because it belongs to the
enabled AccessibilityService and is a trusted accessibility window. Generic
`TYPE_APPLICATION_OVERLAY` remains deferred unless a future requirement needs
it, because it requires `SYSTEM_ALERT_WINDOW` and has different touch-pass
through behavior on modern Android.

Notification requirement: notifications are observable and actionable.
`phone_notifications` and `phone_observe` return bounded notification events with
stable event ids, package, channel, title/body when available, redaction state,
ranking metadata, action ids, and inline-reply capability. V1 supports opening a
notification, dismissing a notification, invoking a notification action, and
sending an inline reply only when the caller supplies explicit notification and
action ids from a fresh observation. Opening a notification sends its
content-intent PendingIntent, invoking an action sends the selected
Notification.Action PendingIntent, and inline reply attaches RemoteInput results
before sending that PendingIntent. Null, canceled, expired, immutable,
redacted, or OEM-filtered PendingIntents must return structured unavailable
errors.

App-management requirement: v1 includes current foreground app, list launchable
apps, launch app by package, launch activity/deep link/intent URI, force-stop
app, install/update APK through ADB, and open relevant Android setup/settings
screens. Companion launch should use `getLaunchIntentForPackage` when available
and may use `getLaunchIntentSenderForPackage` on API 33+; full app inventory may
fall back to ADB `pm list packages` because Android package visibility can hide
apps from normal PackageManager queries. General app install must support single
APK install/update and leave room for split APKs through `install-multiple` and
multi-package installs through `install-multi-package`. Downgrade, test APK, and
runtime-permission grant flags must be explicit options. Clearing app data/cache
is explicitly deferred.

Privileged-mode requirement: v1 detects root, Shizuku, and device-owner or
profile-owner state where possible. It reports those as capabilities and may use
them for low-risk setup automation and diagnostics. V1 does not depend on those
privileged paths for baseline operation.

Target-device requirement: the Galaxy S26 Ultra and Redmi Pad 15 Pro are the
primary target devices. The implementation must not hard-code their specs. It
must detect device identity, Android API level, display metrics, permissions,
and backend capabilities at runtime because OEM firmware and user settings can
change capability availability. Public official Xiaomi sources did not expose
an exact "Redmi Pad 15 Pro" product page; the closest official Redmi tablet
match found during research is Redmi Pad 2 Pro / Redmi Pad 2 Pro 5G, whose
first-batch OS is documented by Xiaomi as HyperOS 2.2 based on Android 15.
The actual target tablet is expected to already have HyperOS 3.1, and HyperOS
3.x / 3.1 public rollout evidence points to the Android 16 generation. Therefore
the tablet compatibility lane must explicitly cover Android 16 / API 36, with
Android 15 / API 35 retained as the documented first-batch baseline, and the
real connected device must confirm `ro.product.model`, `ro.build.version.release`,
and `ro.build.version.sdk` at session start.

Android platform compatibility requirement: the companion app should compile
against Android 16 / API 36 when available, and target API 36 if the chosen
Android Gradle tooling supports it cleanly. Individual companion features must
still be runtime-gated by `Build.VERSION.SDK_INT`, granted permissions, enabled
services, and OEM policy. If the first repo-local Android toolchain only supports
API 35, that is acceptable for an initial sideloaded prototype only when smoke
tests pass on the HyperOS 3.1 tablet and an explicit API 36 upgrade task remains
open. Keep API 35 as a compatibility baseline, not as the final tablet target.

Companion identity requirement: the companion must have a stable package name,
version code/name scheme, signing certificate fingerprint, and packaged APK path
recorded in config or build metadata. `phone_connect` must never silently trust
an APK merely because the package name matches. It must compare the installed
package version and signing certificate fingerprint against the expected
metadata, update a stale trusted package, and refuse or require explicit
recovery for a signature mismatch.

Permission setup requirement: normal Android builds do not let a sideloaded app
silently enable Accessibility Service or Notification Access. `phone_connect`
and `phone_companion_status` must report disabled permissions as structured
unavailable capabilities. `phone_open_settings` must open the most relevant
settings screen for accessibility service setup, notification access, overlay
permission if ever needed, app details, wireless debugging, and companion
battery/background restrictions. Root, Shizuku, or device-owner helpers may
automate setup only when detected and explicitly reported.

Companion RPC requirement: the companion RPC endpoint is reachable only through
host-managed ADB forwarding and must require an ephemeral session token. The
host generates the token for each session, passes it to the companion through an
ADB-launched setup intent or equivalent explicit bootstrap path, and sends it
with every RPC request. Wrong, missing, expired, or replayed tokens produce
structured authentication diagnostics and must not fall through to ADB shell
actions that pretend the companion succeeded.

Capture and overlay requirement: phone-native overlays and screenshot-synthetic
cursor markers are separate outputs. If Android screenshot APIs include the
native overlay in a captured image on a device, the response must report that
fact and avoid double-compositing a screenshot-synthetic cursor. If the native
overlay is hidden for capture, the screenshot response must still report cursor
state separately so the agent can reason about where the cursor is.

Capability freshness requirement: `capability_cache_ttl_ms` is a soft refresh
hint, not permission to act on stale state. `phone_observe` may refresh an
expired profile opportunistically. Action tools must either use a fresh profile
or mark the old profile as `stale=true` and reject actions whose backend
availability is no longer proven.

As-built freshness contract: the tools that act on or perceive through the
capability profile — the action tools, `phone_observe`, and `phone_screenshot` —
include the profile id used and a `profile_refresh_state` of `refreshed`,
`reused`, or `stale` (kept in lockstep with the `stale` boolean). The
pure-status tools (`phone_companion_status`, `phone_accessibility_tree`,
`phone_notifications`, and the `phone_app_*` reads) do not act on the profile and
intentionally omit the freshness field.

## Plan of Work

### Phase 0: Survey and proof-of-feasibility setup

Start by checking the real repo state. Run `git status --short` and preserve any
unrelated user changes. Search for Android, Gradle, Kotlin, Java, ADB, scrcpy,
and existing smoke-test conventions using `rg` and `rg --files`. Check whether
`adb` and `scrcpy` are installed with `command -v adb` and `command -v scrcpy`.
If an Android build system already exists, reuse it. If not, create
`android/phone-companion/` later with the smallest standard Android Gradle
project that builds an APK, but do not introduce that dependency before this
survey is complete.

The proof gate for Phase 0 is a short note in `Progress` and, if useful,
`docs/research/2026-06-phone-use-android-build-survey.md` naming the chosen
Android app location and build command. Add an open `ROADMAP.md` entry pointing
to this ExecPlan so the active workstream is visible from the curated project
index. No source behavior is expected yet.

### Phase 1: Contracts, config, service routing, and fake backends

Create `crates/sky-cua-platform/src/model/phone.rs` and export it from
`crates/sky-cua-platform/src/model.rs`. Add `ServiceRequest::Phone` and
`ServiceResponse::Phone` in `crates/sky-cua-platform/src/model/service.rs`.
Add phone configuration to `crates/sky-cua-platform/src/config.rs`.

The Phase 1 config work is part of the contract, not packaging polish. Extend
`crates/sky-cua-platform/src/config.rs` with a `[phone]` table before ADB or
companion backends are implemented:

    [phone]
    enabled = true
    adb_path = "/path/to/adb"
    scrcpy_path = "/path/to/scrcpy"
    default_serial = "optional"
    default_backend = "auto"
    wireless_auto_connect = false
    window_width = 540
    window_height = 1200
    max_size = 1440
    max_fps = 60
    video_codec = "h265"
    v4l2_sink = "/dev/video10"
    visible_overlay = true
    screenshot_cursor = true
    companion_enabled = true
    companion_auto_install = true
    companion_operator_mode = true
    companion_package = "com.skycua.phonecompanion"
    companion_apk_path = "resources/android/phone-companion.apk"
    companion_expected_cert_sha256 = "from packaged companion metadata"
    companion_allow_downgrade = false
    companion_rpc_port = 47683
    companion_rpc_token_ttl_ms = 900000
    capability_cache_ttl_ms = 30000
    primary_target_models = ["Galaxy S26 Ultra", "Redmi Pad 15 Pro", "Redmi Pad 2 Pro", "Redmi Pad 2 Pro 5G"]

Add environment overrides and allowlist them in `.mcp.json` during Phase 1, so
all later tests use the same configuration surface:

- `SKY_CUA_PHONE`
- `SKY_CUA_PHONE_SERIAL`
- `SKY_CUA_PHONE_BACKEND`
- `SKY_CUA_ADB`
- `SKY_CUA_SCRCPY`
- `SKY_CUA_PHONE_WIRELESS_AUTO_CONNECT`
- `SKY_CUA_PHONE_VISIBLE_OVERLAY`
- `SKY_CUA_PHONE_SCREENSHOT_CURSOR`
- `SKY_CUA_PHONE_V4L2_SINK`
- `SKY_CUA_PHONE_COMPANION`
- `SKY_CUA_PHONE_COMPANION_AUTO_INSTALL`
- `SKY_CUA_PHONE_COMPANION_OPERATOR_MODE`
- `SKY_CUA_PHONE_COMPANION_PACKAGE`
- `SKY_CUA_PHONE_COMPANION_APK`
- `SKY_CUA_PHONE_COMPANION_CERT_SHA256`
- `SKY_CUA_PHONE_COMPANION_ALLOW_DOWNGRADE`
- `SKY_CUA_PHONE_COMPANION_RPC_PORT`
- `SKY_CUA_PHONE_COMPANION_RPC_TOKEN_TTL_MS`
- `SKY_CUA_PHONE_CAPABILITY_CACHE_TTL_MS`
- `SKY_CUA_PHONE_TARGET_MODELS`

Create `crates/sky-cua-service/src/phone/` with fake backend support before real
ADB calls. A fake backend is an in-memory test implementation that returns
deterministic devices, sessions, screenshots, cursor capabilities, companion
capabilities, and diagnostics. It lets service and MCP tests prove contracts
without a phone.

Reserve and register the initial MCP phone tool family in
`crates/sky-cua-client/src/mcp_tools/` against fake service responses. Keep
phone schemas in a dedicated module such as `phone.rs` or
`definitions_phone.rs` so the existing central definitions builder does not
become a merge hotspot. If work is parallelized, the contract spine should
stabilize the tool names and the `client-mcp` lane can finish detailed schema
shaping and response tests:

- `phone_observe`
- `phone_status`
- `phone_list_devices`
- `phone_refresh_capabilities`
- `phone_pair_wireless`
- `phone_connect`
- `phone_disconnect`
- `phone_screenshot`
- `phone_tap`
- `phone_swipe`
- `phone_type_text`
- `phone_press_key`
- `phone_install_companion`
- `phone_companion_status`
- `phone_accessibility_tree`
- `phone_notifications`
- `phone_notification_open`
- `phone_notification_dismiss`
- `phone_notification_action`
- `phone_notification_reply`
- `phone_app_current`
- `phone_app_list`
- `phone_app_launch`
- `phone_app_open_intent`
- `phone_app_force_stop`
- `phone_app_install`
- `phone_open_settings`

The proof gate for Phase 1 is:

- `cargo fmt --check`
- `cargo test -p sky-cua-platform phone`
- `cargo test -p sky-cua-service phone`
- `cargo test -p sky-cua-client phone`
- a local MCP tool-list test or fixture showing all `phone_*` tools exist
- config tests proving the `[phone]` table, defaults, environment overrides,
  and `.mcp.json` allowlist entries exist before ADB/companion work begins
- fake backend tests proving schemas, annotations, request routing, structured
  diagnostics, capability fields, available/unavailable action lists, and
  backend routing decisions
- fake companion identity tests proving package id, APK path, version,
  signing-fingerprint metadata, downgrade policy, and auto-install diagnostics
  are represented before the real APK exists

### Phase 2: ADB baseline and wireless control

Implement `crates/sky-cua-service/src/phone/adb.rs`. Resolve `adb` from config,
then environment, then `PATH`. Missing `adb` returns structured diagnostics and
disables phone-use. Parse stable command outputs with small test-covered
parsers: `adb devices -l`, `adb server-status`, `adb mdns ...`, `wm size`, and
known screenshot/input command results.

Implement:

- `phone_status` reporting adb path/version/server status, mDNS readiness,
  selected serial, configured defaults, active sessions, and diagnostics.
- `phone_list_devices` distinguishing USB, emulator, legacy TCP/IP, wireless
  debugging, unauthorized, offline, and disconnected states where ADB exposes
  them.
- `phone_pair_wireless` for Android 11+ pairing-code flow. Do not store pairing
  codes in logs, config, artifacts, or responses.
- `phone_connect` for USB serials, emulator serials, and `host:port` wireless
  targets.
- `phone_disconnect` scoped to the target serial or host:port.
- `phone_screenshot` through ADB as a PNG snapshot.
- `phone_tap`, `phone_swipe`, `phone_type_text`, `phone_press_key`, and
  `phone_app_launch` through ADB primitives.
- ADB-backed app inventory and app management fallbacks where practical:
  foreground app discovery, launchable app list, package launch, intent URI
  launch, force-stop, APK install/update, and opening setup settings screens.
- general install primitives that distinguish single APK install/update,
  split-APK `install-multiple`, multi-package `install-multi-package`,
  downgrade attempts, test APKs, and best-effort runtime permission grants.
- companion install/update primitives such as `adb install -r` and ADB port
  forwarding, but only expose them fully in Phase 4.
- session-start device probes for `ro.product.manufacturer`, `ro.product.brand`,
  `ro.product.model`, `ro.product.device`, `ro.build.version.sdk`,
  `ro.build.version.release`, display size, density, current orientation,
  connection kind, root availability, Shizuku availability when detectable, and
  device-owner/profile-owner state when detectable.
- explicit target-device compatibility classification: Galaxy S26 Ultra,
  Redmi-family tablet, emulator, or unknown Android device. For the Redmi tablet
  lane, treat HyperOS 3.1 as the expected practical state and Android 16 / API
  36 as the compatibility proof target, while accepting Android 15 / API 35 only
  as a documented first-batch baseline or fallback test lane.

The proof gate for Phase 2 is:

- parser unit tests with normal, unauthorized, offline, malformed, empty, and
  multi-device outputs
- fake command-runner tests for success and failure of pair/connect/screenshot
  and input
- fake command-runner tests proving capability profile construction from device
  properties and app/privileged-state probes
- `scripts/live_phone_use_smoke.py --backend adb --serial <serial>` on a USB or
  emulator target
- `scripts/live_phone_use_smoke.py --backend adb --wireless-host <host:port>` on
  an already paired wireless target
- `scripts/live_phone_use_smoke.py --pair-wireless <host:port>` for the manual
  Android 11+ pairing flow when a physical phone is available

The live smoke should print concise evidence like:

    PASS phone_status adb_path=/.../adb devices=1
    PASS phone_connect serial=<serial> connection_kind=wireless
    PASS capability_profile model=<model> api=<sdk> actions=<n>
    PASS target_device kind=redmi_tablet release=16 sdk=36 hyperos=3.1
    PASS phone_screenshot snapshot_id=<id> size=<width>x<height>
    PASS phone_tap snapshot_id=<id> x=<x> y=<y>
    PASS phone_screenshot_after_action snapshot_id=<id2>

### Phase 3: Snapshots, mappings, and cursor invariants

Implement `crates/sky-cua-service/src/phone/mapping.rs` and
`crates/sky-cua-service/src/phone/cursor.rs`. A phone snapshot id uniquely
identifies a captured image, backend, session, serial, device size, orientation,
coordinate mapping id, and capture timestamp. Coordinate actions must reference
a fresh `phone_snapshot_id` unless the caller explicitly opts into active
session device coordinates.

Implement screenshot-synthetic cursor rendering for ADB-only mode by adapting
the existing composition approach in `crates/sky-cua-service/src/overlay.rs`.
ADB-only mode must report `visible_overlay=false`,
`screenshot_synthetic_cursor=true`, and `phone_native_overlay=false` when the
companion is unavailable.

The proof gate for Phase 3 is:

- unit tests for device-to-screenshot coordinate transforms
- unit tests for rotation-aware mapping
- unit tests for stale snapshot rejection
- unit tests for mismatched session/serial rejection
- unit tests for per-device cursor isolation
- image or pixel-level test proving a synthetic cursor appears after a
  successful action and does not appear for stale or unrelated sessions
- live ADB smoke proving before/after screenshots contain expected cursor
  metadata and, where practical, a detectable marker in the returned image

### Phase 4: Android companion backend

Create the Android app under the path chosen in Phase 0, expected to be
`android/phone-companion/`. The companion is for personal/operator use. It may
assume sideloading, ADB install/update, manual permission enablement, and
optional root/Shizuku/device-owner paths where available.

The companion package id is `com.skycua.phonecompanion` unless Phase 0 discovers
a strong repo-local naming convention that requires a different id. The Android
build must produce a single-APK artifact for the companion auto-install path and
must publish build metadata containing package id, version code, version name,
APK relative path, APK SHA-256, and signing certificate SHA-256. Debug/dev builds
may use the local debug signing certificate for personal sideloading, but
`phone_connect` must compare the installed package signing certificate with the
packaged metadata and refuse silent replacement when the certificate differs.
Downgrades are denied unless `companion_allow_downgrade=true` and the APK is
debuggable or Android otherwise accepts the downgrade.

The app should include:

- a minimal operator UI showing version, connection state, permission state, and
  enabled capabilities
- an AccessibilityService for phone-native cursor overlay, accessibility window
  snapshots, gesture dispatch, and supported screenshot APIs
- a NotificationListenerService for notification event forwarding
- a localhost-only HTTP/WebSocket RPC endpoint reachable through
  host-managed `adb forward`
- a small versioned JSON protocol shared with the Rust host model

Host behavior:

- `phone_connect` automatically checks whether the companion package is
  installed when companion support is enabled. It compares installed version and
  expected signature/hash, installs or updates with `adb install -r` when
  missing or stale, sets up ADB forwarding, and then finalizes the cached
  `PhoneCapabilityProfile`.
- `phone_install_companion` remains available for explicit reinstall/update,
  recovery, and diagnostics.
- `phone_companion_status` reports installed version, RPC reachability,
  accessibility service enabled state, overlay capability, gesture capability,
  `canPerformGestures`, `canRetrieveWindowContent`, screenshot capability,
  notification listener state, MediaProjection state if implemented, and
  privileged setup status.
- `phone_accessibility_tree` returns a bounded, structured active-window tree
  with package name, class name, text/content descriptions where available,
  bounds, focusability, enabled state, and redaction metadata.
- `phone_notifications` returns bounded notification events with sequence ids,
  package, channel, title/body when available, ranking metadata, and redaction
  metadata.
- `phone_notification_open`, `phone_notification_dismiss`,
  `phone_notification_action`, and `phone_notification_reply` require explicit
  notification/action ids from a fresh observation and return structured
  unavailable errors when the notification disappeared, content was redacted, a
  PendingIntent is missing/canceled/expired/filtered, RemoteInput is no longer
  available, or the action is no longer valid.
- `phone_app_current`, `phone_app_list`, `phone_app_launch`,
  `phone_app_open_intent`, `phone_app_force_stop`, `phone_app_install`, and
  `phone_open_settings` expose the app-management slice using companion APIs
  where available and ADB fallbacks where appropriate.
- `phone_tap` and `phone_swipe` prefer native gesture dispatch when the
  companion reports it available.
- `phone_screenshot` prefers companion screenshot APIs when available and falls
  back to ADB or scrcpy. Screenshot diagnostics must distinguish unsupported API
  level, disabled service metadata, secure-window failure, throttling, OEM
  policy, and transient service errors where Android exposes that information.
- `phone_native_overlay` cursor updates are sent to the companion only after
  successful action dispatch. The overlay window must be non-focusable and
  non-touchable, and smoke tests must prove taps pass through it.

The proof gate for Phase 4 is:

- Android unit/instrumentation tests for protocol serialization, overlay state,
  overlay pass-through, bounded accessibility tree serialization, notification
  event serialization, notification action/reply validation, app-management
  request validation, screenshot failure classification, and gesture request
  validation
- Rust tests for companion RPC client success, timeout, malformed payload,
  version mismatch, token mismatch, sequence ordering, and fallback
- live companion smoke proving install/update, ADB forward, status, native
  overlay cursor, accessibility tree retrieval, gesture dispatch, screenshot
  capability when supported, notification listener diagnostics, notification
  open/dismiss/action/reply when available, current app detection, launchable app
  listing, app launch, intent launch, and force-stop of a benign test package
- live failure smoke where accessibility or notification access is disabled and
  the MCP responses report disabled capability instead of pretending success

### Phase 5: scrcpy acceleration and host-visible overlay

Implement `crates/sky-cua-service/src/phone/scrcpy.rs`. Resolve scrcpy from
config, then environment, then `PATH`. Missing scrcpy degrades to ADB and
companion capabilities. Managed scrcpy sessions use deterministic window titles
with sanitized serials.

The first supported scrcpy path uses documented CLI/window behavior:

    scrcpy --serial=<serial> --window-title=sky-cua-phone-<safe-serial> --window-width=540 --window-height=1200 --max-size=1440 --max-fps=60 --video-codec=h265 --no-audio --keyboard=uhid --always-on-top

If H.265 fails, retry with H.264 or no explicit codec:

    scrcpy --serial=<serial> --window-title=sky-cua-phone-<safe-serial> --window-width=540 --window-height=1200 --max-size=1440 --max-fps=60 --no-audio --keyboard=uhid --always-on-top

Default mouse mode should remain scrcpy's SDK mouse mode. Do not default to
`--mouse=uhid`, because UHID mouse mode moves/captures the host pointer and is
less clean for agent coordinate injection.

The manager must compute the phone video content rectangle inside the host
window. Letterboxing, window decorations, host scaling, device rotation, and
aspect ratio must not corrupt tap coordinates. Host-visible cursor overlay is
enabled only when mapping to host coordinates is current.

The proof gate for Phase 5 is:

- unit tests for content-rect mapping with letterboxing, rotation, resized
  windows, and fractional host scale
- tests for scrcpy launch command construction and retry after codec failure
- tests for managed/adopted/external process ownership
- tests proving capture hides/restores host-visible overlay
- live scrcpy smoke proving launch/adopt, screenshot, tap, host-visible overlay
  capability, screenshot-synthetic cursor, and cleanup
- failure smoke proving scrcpy crash or missing codec degrades to ADB/companion
  diagnostics without losing the phone session

Direct scrcpy protocol consumption, broad shell/file/clipboard/admin tools, and
Appium/WebDriver semantic automation are explicitly out of this first feature.

### Phase 6: Packaging, skills, docs, and installed MCP proof

Phase 6 packages and proves the contract created in Phase 1. Do not introduce
new config keys here except for packaging-specific paths discovered during the
Android build survey. Verify that `.mcp.json` allows every Phase 1 phone
environment override, that the companion APK path exists in the staged package,
and that the expected companion signing fingerprint/hash metadata is packaged
next to the APK.

`skills/phone-use/SKILL.md` now exists with guidance for agents:

- start with `phone_status` and `phone_list_devices`
- use `phone_pair_wireless` for Android 11+ pairing-code flows
- use `phone_connect` before screenshots or actions
- use `phone_observe` as the default perception tool after connection
- use fresh `phone_snapshot_id` for coordinate actions
- follow `available_actions` and `unavailable_actions` from the cached
  capability profile
- prefer companion capabilities when available
- distinguish host-visible overlay, screenshot-synthetic cursor, and
  phone-native overlay
- keep notification/accessibility responses bounded and structured
- use explicit notification/action ids for notification operations

`scripts/build_plugin.py` now bundles `skills/phone-use` with
`skills/computer-use` and `skills/browser-use`, includes the phone runtime
protocol documentation, and conditionally stages the companion APK when present.
Keep the `ROADMAP.md` entry linked to this ExecPlan while live-device and
installed-MCP proof remain open. `docs/features/phone-use.md` exists as the
source-landed feature doc; retire this plan only after the remaining Phase 6-8
proof gates are recorded.

The proof gate for Phase 6 is:

- `python3 scripts/build_plugin.py`
- inspection of the staged bundle shape showing `skills/phone-use`
- installed MCP `tools/list` proof showing the `phone_*` tools from the
  installed bundle, not just source
- docs updated with exact commands and known limitations

### Phase 7: Adversarial testing

Adversarial tests deliberately try to break assumptions. Add unit, integration,
and live-smoke cases where practical.

Required adversarial categories:

- Device routing: no devices, one device, multiple devices, default serial set,
  serial omitted with multiple devices, wrong serial, emulator plus physical
  phone, USB and wireless instances of the same phone, Galaxy S26 Ultra and
  Redmi Pad 15 Pro model/property differences.
- ADB state: missing adb, unauthorized device, offline device, ADB server down,
  malformed `adb devices` output, pairing wrong code, pairing wrong port,
  mDNS unavailable, wireless disconnect mid-action, screenshot command failure,
  text input escaping failure.
- Snapshot safety: stale snapshot id, snapshot from another session, snapshot
  from another serial, orientation change after snapshot, device resolution
  change after snapshot, action outside screen bounds, negative coordinates,
  NaN or infinity if numeric parsing permits them.
- Capability safety: stale capability profile, permission enabled after connect,
  permission revoked after connect, companion updated mid-session, orientation
  change after profile, display-size change after profile, action offered while
  backend capability is unavailable.
- Cursor safety: cursor update before failed action, cursor leaked across
  devices, synthetic cursor included in unrelated screenshot, host-visible
  overlay not hidden during capture, phone-native overlay still visible after
  disconnect.
- Companion state: app not installed, app version mismatch, RPC port collision,
  package signature/hash mismatch, auto-install failure, RPC
  timeout, malformed JSON, wrong token, accessibility disabled, notification
  access disabled, service killed mid-request, accessibility tree too large,
  notification burst, notification content redacted, notification/action id
  expired before action, inline reply unavailable after observation.
- App-management state: app with no launcher activity, disabled app, missing
  package, bad intent URI, force-stop denied or ineffective, APK install
  failure, split APK install failure, multi-package install failure, APK
  downgrade rejection, test APK flag required, runtime permission grant ignored,
  setup settings activity missing or OEM-moved.
- scrcpy state: missing binary, codec failure, process crash, external window
  adopted then closed, window resized, letterboxing, device rotation, host scale
  change, V4L2 missing or busy.
- Sensitive output bounding: notification text, accessibility tree text,
  screenshots, pairing codes, and auth tokens must not be written into committed
  artifacts or unbounded logs.

The proof gate for Phase 7 is a test report in `Outcomes & Retrospective`
showing each category as passed, skipped with reason, or blocked with a concrete
follow-up. Skips are acceptable only for hardware/permission modes not available
on the test device.

### Phase 8: Full live-smoke and release proof

Add `scripts/live_phone_use_smoke.py` with profiles that can be run separately
or together:

- `--profile adb-usb`: USB or emulator ADB status, connect, screenshot, tap,
  observe, screenshot-after-action, app-current, app-list, benign app launch,
  disconnect.
- `--profile adb-wireless`: already paired wireless target connect,
  observe, screenshot, tap, reconnect after disconnect.
- `--profile pair-wireless`: manual Android 11+ pairing-code workflow.
- `--profile companion`: install/update, ADB forward, companion status,
  accessibility permission state, native overlay cursor, accessibility tree,
  gesture dispatch, screenshot capability, notification listener diagnostics,
  notification action smoke when available, app-management smoke, and capability
  refresh.
- `--profile scrcpy`: scrcpy launch/adopt, screenshot, tap, host-visible
  overlay, screenshot-synthetic cursor, cleanup.
- `--profile fallback`: missing scrcpy or companion unavailable degrades to ADB
  diagnostics and baseline actions.
- `--profile adversarial`: bounded live cases for stale snapshot, wrong serial,
  disabled permissions, wireless disconnect, and process crash where safe.
- `--profile full`: runs every profile that the current environment can support
  and records skipped prerequisites explicitly.

The full live-smoke proof must be run against the installed MCP surface after
packaging. Source-only proof is not enough. Success should print concise PASS
lines and a final summary like:

    PASS installed_tools phone_status phone_connect phone_screenshot ...
    PASS adb_usb serial=<serial> snapshot=<id>
    PASS adb_wireless serial=<host:port> reconnect=true
    PASS observe capability_profile=<id> available_actions=<n>
    PASS companion version=<version> auto_installed=<bool> native_overlay=true accessibility_tree=true notifications=true
    PASS apps current=<package> launchable_count=<n>
    PASS scrcpy window_title=sky-cua-phone-<safe-serial> host_overlay=true
    PASS adversarial stale_snapshot_rejected=true wrong_serial_rejected=true
    RESULT full_phone_use_smoke passed=<n> skipped=<m> failed=0

Phone-use live smoke is hardware-dependent and should remain in
`scripts/live_phone_use_smoke.py` rather than inside the VM-only desktop smoke
matrix unless a future Android emulator profile is added to
`scripts/run_gui_testing_vm_smoke.py`. The final release proof must still run the
existing standard GUI smoke that is appropriate for the changed desktop/MCP
surface, and then run phone-use full smoke from the installed MCP surface. If the
standard `scripts/run_gui_testing_vm_smoke.py --profile all` is not run, record
the reason in `Outcomes & Retrospective`.

## Concrete Steps

Run commands from `/home/bex/.codex/worktrees/00d8/sky-cua` unless the active
worktree changes.

1. Capture starting state and complete the Phase 0 survey:

       git status --short
       rg --files | rg '(^android/|gradle|adb|scrcpy|phone|smoke)'
       command -v adb
       command -v scrcpy

   Record the chosen Android app location/build command in `Progress` or
   `docs/research/2026-06-phone-use-android-build-survey.md`, and add the open
   `ROADMAP.md` entry pointing at this ExecPlan.

2. Implement and merge the Phase 1 contract spine before spawning backend
   workers. This slice owns shared platform models, service request/response
   variants, config/env allowlist, daemon phone dispatch, fake manager traits,
   MCP tool names, and fake response tests. Run:

       cargo fmt --check
       cargo test -p sky-cua-platform phone
       cargo test -p sky-cua-service phone
       cargo test -p sky-cua-client phone

3. After the contract spine merges, split implementation into parallel lanes
   with disjoint write scopes. The `service-adb`, `snapshot-cursor`,
   `client-mcp`, `companion-protocol`, `android-companion`, `scrcpy`,
   `smoke-harness`, and `packaging-skill-docs` lanes may run concurrently. Each
   lane updates `Progress` with files touched, commands run, and blockers.

4. The `service-adb` lane implements Phase 2 ADB behavior and runs:

       cargo test -p sky-cua-service phone::adb
       uv run python scripts/live_phone_use_smoke.py --profile adb-usb --serial <serial>

   Run the wireless profiles when a paired physical phone is available:

       uv run python scripts/live_phone_use_smoke.py --profile adb-wireless --serial <host:port>
       uv run python scripts/live_phone_use_smoke.py --profile pair-wireless --serial <serial>

5. The `snapshot-cursor` lane implements Phase 3 mapping and cursor behavior
   against fake sessions first, then reruns ADB smoke when ADB exists:

       cargo test -p sky-cua-service phone::mapping
       cargo test -p sky-cua-service phone::cursor
       uv run python scripts/live_phone_use_smoke.py --profile adb-usb --serial <serial>

6. The `client-mcp` lane implements phone MCP registration, schema shaping, and
   tool-list fixtures. It runs:

       cargo test -p sky-cua-client phone

7. The `companion-protocol` and `android-companion` lanes implement Phase 4.
   Run the Android build command chosen in Phase 0, the Android unit tests, and:

       uv run python scripts/live_phone_use_smoke.py --profile companion --serial <serial>

8. The `scrcpy` lane implements Phase 5 and runs:

       cargo test -p sky-cua-service phone::scrcpy
       uv run python scripts/live_phone_use_smoke.py --profile scrcpy --serial <serial>

9. The `smoke-harness` lane implements `scripts/live_phone_use_smoke.py` and
   script tests. It can merge skipped profiles before all backends exist, but
   each skip must name its missing prerequisite. Run:

       uv sync --dev
       uv run ruff format --check scripts
       uv run ruff check scripts
       uv run basedpyright
       uv run pytest

10. The `packaging-skill-docs` lane implements Phase 6 after tool names
    stabilize and at least ADB source proof exists. It updates `skills/phone-use`,
    packaging, `.mcp.json`, `ROADMAP.md`, and later `docs/features/phone-use.md`.
    Run:

       python3 scripts/build_plugin.py

11. After parallel lanes merge, run broad Rust validation:

       cargo fmt --check
       cargo test

12. Install or deploy the plugin through the existing sky-cua install workflow,
    then verify the installed MCP tool list includes the phone tools. Use the
    existing installed-MCP proof pattern from `docs/runtime/mcp-boundary.md` and
    the deploy scripts in this repo.

13. Run the existing standard sky-cua smoke appropriate for the changed desktop
    and MCP surface. If `scripts/run_gui_testing_vm_smoke.py --profile all` is
    not run, record why in `Outcomes & Retrospective`.

14. Run full phone live smoke from the installed MCP surface:

        uv run python scripts/live_phone_use_smoke.py --profile full --serial <serial>

    For the Redmi tablet target, also run:

        uv run python scripts/live_phone_use_smoke.py --profile full --serial <redmi-tablet-serial>

15. Update `Progress`, `Surprises & Discoveries`, `Decision Log`, and
    `Outcomes & Retrospective` with observed versions, skipped profiles,
    failures, fixes, and proof snippets.

## Validation and Acceptance

The feature is accepted only when all applicable proof gates pass.

Contract acceptance:

- The installed MCP `tools/list` includes every initial `phone_*` tool.
- `phone_observe` returns screenshot/snapshot state, current app,
  accessibility/notification summaries when available, cursor state, backend
  state, and dynamic available/unavailable actions from the cached capability
  profile.
- Each phone tool returns structured capabilities and diagnostics, not only
  prose.
- `phone_connect` creates a `PhoneCapabilityProfile`, and every action response
  identifies the profile version or id used for routing.
- `phone_refresh_capabilities` invalidates and rebuilds the profile after
  permission, orientation, display, companion, or wireless-state changes.
- Missing ADB disables phone-use with a structured diagnostic.
- Missing companion or missing scrcpy degrades capabilities without disabling
  ADB baseline.
- Android target acceptance is explicit: the Redmi tablet lane is complete only
  when a real connected Redmi-family tablet reports HyperOS 3.1 or equivalent,
  Android release 16, and SDK 36 through ADB/capability-profile evidence. If the
  tablet is unavailable, the final result must mark that lane blocked or skipped
  with a reason and must not claim full target-device completion. API 35 proof is
  acceptable only as the documented launch-baseline fallback, not as final proof
  for the HyperOS 3.1 tablet target.

ADB acceptance:

- `phone_status` accurately reports adb path/version/server status, configured
  defaults, active sessions, and diagnostics.
- `phone_list_devices` distinguishes USB, emulator, legacy TCP/IP, wireless
  debugging, unauthorized, offline, and disconnected states where ADB exposes
  them.
- `phone_pair_wireless` succeeds for Android 11+ pairing-code flow and reports
  useful diagnostics on same-network, auth, or port failures.
- `phone_connect` works for USB and wireless serials.
- `phone_screenshot` returns a valid phone snapshot with device dimensions,
  backend, mapping id, cursor capabilities, and image delivery.
- `phone_tap`, `phone_swipe`, `phone_type_text`, `phone_press_key`, and
  `phone_app_launch` work against a connected test device.
- ADB-backed device probes populate manufacturer, brand, model, device codename,
  Android API level, Android release, display size, density, orientation,
  connection kind, and detectable privileged state.
- ADB-backed app install reports whether it used single APK install,
  `install-multiple`, or `install-multi-package`, and returns structured errors
  for downgrade, test-only, split-package, and runtime-permission grant failures.

Snapshot and cursor acceptance:

- Coordinate actions from stale, mismatched, or out-of-bounds snapshots are
  rejected with structured errors.
- ADB-only mode returns screenshots with synthetic cursor markers after
  successful actions and does not claim host-visible or phone-native overlay
  support.
- Companion mode, when installed and enabled, shows a phone-native agent cursor
  overlay and reports accessibility, gesture, screenshot, notification, RPC, and
  privileged setup capabilities as structured fields.
- scrcpy-window mode shows host-visible cursor overlay when host mapping is
  current and can still return screenshot-synthetic cursor markers.
- Multiple connected devices keep sessions and cursors isolated.

Companion acceptance:

- `phone_connect` auto-installs or updates the companion APK through ADB when
  companion support is enabled and the installed package is missing, stale, or
  signature/hash-mismatched.
- `phone_install_companion` can explicitly reinstall/update the APK through ADB.
- `phone_companion_status` reports installed version, RPC reachability,
  package id, installed and expected signing fingerprint, APK hash, permission
  state, and capability state.
- `phone_accessibility_tree` returns a bounded active-window tree with redaction
  metadata where appropriate.
- `phone_notifications` returns bounded notification events with sequence ids
  and redaction metadata where Android redacts content.
- `phone_notification_open`, `phone_notification_dismiss`,
  `phone_notification_action`, and `phone_notification_reply` work with explicit
  fresh notification/action ids when Android exposes the operation, and return
  structured unavailable errors otherwise.
- `phone_app_current`, `phone_app_list`, `phone_app_launch`,
  `phone_app_open_intent`, `phone_app_force_stop`, `phone_app_install`, and
  `phone_open_settings` work against benign test apps or settings targets.
- Native gesture dispatch can perform a benign tap/swipe and report failure when
  the service is disabled or lacks `canPerformGestures`.
- The phone-native cursor overlay is non-focusable, non-touchable, and proven by
  smoke to pass taps through to the underlying app.
- Companion screenshots report whether native overlay pixels were captured,
  avoid double cursor composition, and classify secure-window, throttling,
  disabled-service, unsupported-API, and OEM-policy failures where observable.

scrcpy acceptance:

- scrcpy launch/adopt works for a connected serial.
- Host-window content rect mapping is correct under resize, letterboxing,
  rotation, and host scaling tests.
- Codec failure retries with a lower requirement.
- scrcpy crash or window close downgrades capability without corrupting the
  phone session.

Adversarial acceptance:

- Every category in Phase 7 has a passing automated test, a passing live test,
  or a recorded skip with reason.
- No pairing code, token, notification body, accessibility tree dump, or
  screenshot is persisted into committed artifacts by default.

Full live-smoke acceptance:

- `scripts/live_phone_use_smoke.py --profile full --serial <serial>` passes from
  the installed MCP surface on at least one real Android phone or emulator.
- `scripts/live_phone_use_smoke.py --profile full --serial <redmi-tablet-serial>`
  passes from the installed MCP surface on the Redmi tablet target, or the
  `Outcomes & Retrospective` records the exact blocker and leaves the Redmi
  tablet/API 36 lane incomplete.
- The final smoke report records adb version, Android version, companion version
  if installed, scrcpy version if used, connection kind, cursor planes proven,
  and skipped profiles.

## Idempotence and Recovery

All status/list commands are safe to run repeatedly. `phone_connect` is
idempotent for an already connected serial and should reuse or report an
existing session. `phone_disconnect` stops only sky-cua-owned scrcpy processes
and ADB forwards by default. Wireless ADB disconnect is explicit and scoped to
the target serial or host:port.

`phone_pair_wireless` does not persist pairing codes. Companion install/update
uses `adb install -r` for the single companion APK and can be retried when the
package is missing, stale, or the same trusted certificate signs both old and new
APK. Signature mismatch is not auto-recovered by uninstalling; it returns a
structured diagnostic and requires explicit operator recovery. Companion RPC
failures fall back to ADB/scrcpy capability instead of leaving the phone session
stuck. Crashed scrcpy processes are detected and removed from active sessions.
ADB server failures return diagnostics and suggested next commands, not panics.

Capability profiles are session-local and safely refreshable. Re-running
`phone_connect` for the same serial may reuse the session, but must verify the
companion version, RPC reachability, key permissions, display metrics, and
connection kind before claiming the cached profile is current. Explicit
`phone_refresh_capabilities` invalidates the cache and rebuilds it. If refresh
fails, the previous profile may be returned only with `stale=true` and a
diagnostic that prevents the agent from treating unavailable actions as current.

Cursor state expires or hides like the existing agent cursor and never leaks
across devices. If a host window disappears, scrcpy visible overlay capability
becomes false while ADB and companion fallback remain available. If the
companion accessibility service is disabled mid-session, phone-native overlay,
gestures, screenshots, and accessibility tree capability become false until the
service reports enabled again.

## Artifacts and Notes

Recommended manual scrcpy command for development:

    scrcpy --serial=<serial> --window-title=sky-cua-phone-<safe-serial> --window-x=40 --window-y=40 --window-width=540 --window-height=1200 --max-size=1440 --max-fps=60 --video-codec=h265 --no-audio --keyboard=uhid --always-on-top

Fallback command when H.265 fails:

    scrcpy --serial=<serial> --window-title=sky-cua-phone-<safe-serial> --window-width=540 --window-height=1200 --max-size=1440 --max-fps=60 --no-audio --keyboard=uhid --always-on-top

Useful adb commands for manual diagnosis:

    adb devices -l
    adb server-status
    adb mdns track-services --proto-text
    adb pair <host>:<pairing-port>
    adb connect <host>:<adb-port>
    adb -s <serial> shell wm size
    adb -s <serial> exec-out screencap -p
    adb -s <serial> install -r <companion.apk>
    adb -s <serial> install-multiple <base.apk> <split.apk>
    adb -s <serial> install-multi-package <package1.apk> <package2.apk>
    adb -s <serial> forward tcp:47683 tcp:47683

Do not log pairing codes, screenshots from sensitive apps, notification content,
full accessibility trees, auth material, or live phone payloads into committed
artifacts. Live-smoke artifacts should store only bounded summaries, sanitized
metadata, and non-sensitive screenshots when deliberately enabled for debugging.

## Interfaces and Dependencies

External executables:

- Required: `adb`
- Optional visual acceleration: `scrcpy`
- Optional Linux preview sink: `v4l2loopback` plus configured V4L2 device
- Optional privileged Android setup helpers: root shell, Shizuku, or
  device-owner flows when present

No new Rust dependency should be added until the existing workspace patterns are
checked. Prefer small std/process wrappers for `adb` and `scrcpy` first. If a
dependency becomes necessary for image decoding/composition, process control, or
RPC, add it at the workspace root and document why the existing stack was not
enough.

Public model sketch:

    pub enum PhoneRequest {
        Observe(PhoneObserveRequest),
        Status(PhoneStatusRequest),
        ListDevices(PhoneListDevicesRequest),
        RefreshCapabilities(PhoneRefreshCapabilitiesRequest),
        PairWireless(PhonePairWirelessRequest),
        Connect(PhoneConnectRequest),
        Disconnect(PhoneDisconnectRequest),
        Screenshot(PhoneScreenshotRequest),
        Tap(PhoneTapRequest),
        Swipe(PhoneSwipeRequest),
        TypeText(PhoneTypeTextRequest),
        PressKey(PhonePressKeyRequest),
        InstallCompanion(PhoneInstallCompanionRequest),
        CompanionStatus(PhoneCompanionStatusRequest),
        AccessibilityTree(PhoneAccessibilityTreeRequest),
        Notifications(PhoneNotificationsRequest),
        NotificationOpen(PhoneNotificationOpenRequest),
        NotificationDismiss(PhoneNotificationDismissRequest),
        NotificationAction(PhoneNotificationActionRequest),
        NotificationReply(PhoneNotificationReplyRequest),
        AppCurrent(PhoneAppCurrentRequest),
        AppList(PhoneAppListRequest),
        AppLaunch(PhoneAppLaunchRequest),
        AppOpenIntent(PhoneAppOpenIntentRequest),
        AppForceStop(PhoneAppForceStopRequest),
        AppInstall(PhoneAppInstallRequest),
        OpenSettings(PhoneOpenSettingsRequest),
    }

    pub enum PhoneResponse {
        Observe(PhoneObserveResponse),
        Status(PhoneStatusReport),
        Devices(PhoneListDevicesResponse),
        Capabilities(PhoneCapabilityProfile),
        PairedWireless(PhonePairWirelessResponse),
        Connected(PhoneSession),
        Disconnected(PhoneDisconnectResponse),
        Screenshot(PhoneScreenshotResponse),
        Action(PhoneActionResponse),
        CompanionStatus(PhoneCompanionStatusResponse),
        AccessibilityTree(PhoneAccessibilityTreeResponse),
        Notifications(PhoneNotificationsResponse),
        App(PhoneAppResponse),
    }

    pub struct PhoneSession {
        pub session_id: String,
        pub serial: String,
        pub connection_kind: PhoneConnectionKind,
        pub backend: PhoneBackendKind,
        pub capabilities: PhoneBackendCapabilities,
        pub capability_profile: PhoneCapabilityProfile,
        pub companion: Option<PhoneCompanionCapabilities>,
        pub managed_process: bool,
        pub window_title: Option<String>,
        pub created_at_ms: u64,
    }

    pub struct PhoneObserveResponse {
        pub session: PhoneSession,
        pub phone_snapshot_id: Option<String>,
        pub screenshot_path: Option<String>,
        pub inline_image: Option<ImagePayload>,
        pub current_app: Option<PhoneAppInfo>,
        pub accessibility_summary: Option<PhoneAccessibilitySummary>,
        pub recent_notifications: Vec<PhoneNotificationEvent>,
        pub cursor: Option<PhoneCursorState>,
        pub available_actions: Vec<PhoneAvailableAction>,
        pub unavailable_actions: Vec<PhoneUnavailableAction>,
        pub diagnostics: Vec<PhoneDiagnostic>,
    }

    pub struct PhoneScreenshotResponse {
        pub session: PhoneSession,
        pub phone_snapshot_id: String,
        pub screenshot_path: Option<String>,
        pub inline_image: Option<ImagePayload>,
        pub device_size: PixelSize,
        pub coordinate_mapping: PhoneCoordinateMapping,
        pub cursor: Option<PhoneCursorState>,
        pub cursor_capabilities: PhoneCursorCapabilities,
        pub capture_contains_native_overlay: bool,
        pub diagnostics: Vec<PhoneDiagnostic>,
    }

    pub struct PhoneCursorCapabilities {
        pub host_visible_overlay: bool,
        pub screenshot_synthetic_cursor: bool,
        pub phone_native_overlay: bool,
        pub visible_overlay_reason: Option<String>,
    }

    pub struct PhoneCoordinateMapping {
        pub mapping_id: String,
        pub session_id: String,
        pub serial: String,
        pub device_rect: RectF,
        pub screenshot_rect: RectF,
        pub host_window_rect: Option<RectF>,
        pub host_content_rect: Option<RectF>,
        pub rotation_degrees: i32,
        pub captured_at_ms: u64,
    }

    pub struct PhoneCompanionCapabilities {
        pub installed: bool,
        pub package_name: String,
        pub installed_version: Option<String>,
        pub expected_version: Option<String>,
        pub installed_cert_sha256: Option<String>,
        pub expected_cert_sha256: Option<String>,
        pub apk_sha256: Option<String>,
        pub signature_matches_expected: bool,
        pub allow_downgrade: bool,
        pub auto_install_attempted: bool,
        pub rpc_reachable: bool,
        pub rpc_token_expires_at_ms: Option<u64>,
        pub accessibility_enabled: bool,
        pub can_perform_gestures: bool,
        pub can_retrieve_window_content: bool,
        pub can_take_screenshot: bool,
        pub notification_listener_enabled: bool,
        pub native_overlay: bool,
        pub native_overlay_pass_through: bool,
        pub gesture_dispatch: bool,
        pub screenshot: bool,
        pub accessibility_tree: bool,
        pub notifications: bool,
        pub privileged_setup: Option<String>,
    }

    pub struct PhoneCapabilityProfile {
        pub profile_id: String,
        pub session_id: String,
        pub serial: String,
        pub detected_at_ms: u64,
        pub stale: bool,
        pub refresh_state: PhoneCapabilityRefreshState,
        pub manufacturer: Option<String>,
        pub brand: Option<String>,
        pub model: Option<String>,
        pub device: Option<String>,
        pub target_device_kind: PhoneTargetDeviceKind,
        pub hyperos_version: Option<String>,
        pub android_sdk: Option<u32>,
        pub android_release: Option<String>,
        pub display_size: Option<PixelSize>,
        pub density_dpi: Option<u32>,
        pub orientation: Option<String>,
        pub connection_kind: PhoneConnectionKind,
        pub companion: PhoneCompanionCapabilities,
        pub scrcpy: PhoneScrcpyCapabilities,
        pub root_available: bool,
        pub shizuku_available: bool,
        pub device_owner: bool,
        pub available_actions: Vec<PhoneAvailableAction>,
        pub unavailable_actions: Vec<PhoneUnavailableAction>,
    }

MCP response rule: user-visible prose summaries are secondary. Structured fields
carry truth for device state, backend state, cursor capabilities, snapshot ids,
permission state, companion capability, and diagnostics.

## Revision Notes

- 2026-06-17: Initial researched ExecPlan. Changed the architecture from
  scrcpy-first to ADB-baseline plus scrcpy-acceleration, made wireless a public
  workflow, and added cursor/overlay work as part of the core feature rather
  than polish.
- 2026-06-17: Updated for personal/full-privilege operator use. Promoted the
  Android companion app to the preferred rich backend after ADB bootstrap, with
  native overlay, accessibility tree, gestures, screenshots, and notifications.
- 2026-06-17: Reorganized as a dependency-aware multi-phase implementation plan.
  Added phase proof gates, adversarial test categories, full live-smoke profiles,
  installed MCP proof, and explicit acceptance criteria for ADB, companion,
  scrcpy, cursor planes, and fallback behavior.
- 2026-06-17: Added session-start capability detection and caching,
  companion auto-install/update during `phone_connect`, `phone_observe` as the
  primary perception tool, dynamic available/unavailable action lists, actionable
  notifications by explicit IDs, v1 app-management tools, backend routing rules,
  target-device notes for Galaxy S26 Ultra and Redmi Pad 15 Pro, and API
  research references for AccessibilityService, NotificationListenerService,
  PackageManager, and adb install/update.
- 2026-06-17: Corrected the Redmi tablet compatibility lane: Xiaomi's official
  first-batch Redmi Pad 2 Pro docs say HyperOS 2.2 based on Android 15, but the
  target tablet is expected to run HyperOS 3.1, so the practical tablet proof
  target is Android 16 / API 36 with runtime ADB confirmation.
- 2026-06-17: Tightened the ExecPlan after second-pass review and parallelization
  review. Moved config/env/companion identity into the contract spine, removed
  the duplicate app-launch tool, added permission/RPC/signing/cache freshness
  requirements, added Android API failure-mode details, required real Redmi
  tablet API 36 proof or an explicit blocker, tied phone smoke to installed MCP
  and existing GUI smoke, and reorganized implementation into parallel
  subagent-ready lanes with disjoint write scopes.
- 2026-06-19: Recorded reconciled decisions from the ultra-review fix pass over
  the working tree. Closed seven residuals (non-Unix build surface, protocol
  error-fallback classification, notification redaction, scrcpy rotation-quarter
  threading, scrcpy reconnect relaunch, overlay rotation-bounds refresh, and the
  four previously dead `[phone]` config keys). Logged three Decision Log entries:
  the setup-token accepted-risk posture (UID-gate fix reverted), the
  `companion_operator_mode` silent-auto-install scoping, and the
  `primary_target_models` mark-not-filter semantics. The setup-token
  logcat-readback handshake remains the documented future hardening.
- 2026-06-19: Live-verified the phone-native overlay rotation-bounds refresh on
  the Galaxy S26 Ultra (`SM-S948B`, Android 16 / API 36). Rebuilt and reinstalled
  the companion, held the agent overlay up over a session, and forced
  portrait (1440x3120) -> landscape (3120x1440) -> portrait while capturing raw
  screencaps. The "agent in control" edge glow re-fits the full new display
  bounds on each rotation and the cursor reclamps to the new extents; the overlay
  vanishes on disconnect (control frame). This closes the overlay rotation flow,
  which was previously only instrumentation-tested.
