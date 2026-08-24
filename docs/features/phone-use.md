# Phone Use (Android control)

## Status

Partial. Source-complete and green across the Rust workspace. The installed
grouped MCP path and ADB-baseline emulator smoke are proven; broader
companion, scrcpy, and real-device release proof remain open. Last verified:
2026-06-22.

## Summary

Lets a sky-cua agent control a real Android phone from the existing
`sky-cua-client mcp` process over USB or wireless ADB: discover, pair, connect,
observe, screenshot, tap/swipe/type, press keys, read and act on notifications,
manage apps, and render an agent cursor. `observe(surface="phone")` is the
default per-turn perception call; ADB is the required baseline, with an
optional companion app and optional scrcpy acceleration layered on top.

## Contract surface

A static phone-capable tool family inside the single `computer-use` MCP server
(no new server). Phone work uses the grouped MCP surface:

- Session and discovery: `status(component="phone")`,
  `list_resources(surface="phone", resource="devices")`,
  `phone_pair_wireless`, and
  `phone_connection(operation="connect"|"disconnect"|"refresh")`.
- Perception: `observe(surface="phone")` (default),
  `capture_screen(surface="phone")`, `phone_accessibility_tree`, and
  `phone_notifications`.
- Input: `phone_pointer(operation="tap"|"swipe")` and
  `phone_keyboard(operation="type_text"|"press_key")`.
- Companion/setup: `phone_setup(operation="install_companion"|"open_settings")`
  and `status(component="phone_companion")`.
- Notifications (explicit ids required):
  `phone_notification_action(operation="open"|"dismiss"|"action")` and
  `phone_notification_reply`.
- Apps: `list_resources(surface="phone", resource="apps"|"current_app")`,
  `phone_app_action(operation="launch"|"open_intent")`,
  `phone_app_force_stop`, and `phone_app_install`.

Callers can rely on: structured responses carrying the truth (backend used,
capability profile id/version, snapshot ids, cursor capabilities, permission
state, diagnostics) — prose summaries are secondary. Coordinate actions require
a fresh `phone_snapshot_id`. `available_actions` / `unavailable_actions` come
from the cached capability profile, not a static schema. The tool list is
static; action affordances are dynamic.

Phone observe requests inline pixels only for image-capable model sessions.
Response shaping independently enforces the same gate and strips
`inline_image.data_base64` from structured content, so an older service cannot
leak pixels through either channel after a text-only request. Unknown model
capability fails closed. Codex CLI 0.147.0 does not advertise model image
capability during MCP initialize, but its per-call
`_meta.x-codex-turn-metadata.model` identifies the active model. sky-cua uses
that turn model after explicit capability and process overrides, so recognized
image-capable Codex models need no environment override.
Pi and other hosts may retain `structuredContent` only as UI/session metadata,
so text-only phone observes also append the canonical accessibility projection
to the model-facing text as compact JSON bounded to 32 KiB. The full bounded
projection remains in `structuredContent` and the AppShot artifact. Explicit
`phone_accessibility_tree` calls use the same model-facing bounded projection.
The installed Pi integration additionally filters tool-result image blocks
against the active Pi model's declared input modalities immediately before the
result enters model context. This covers Pi model switches without disabling
images for image-capable Pi models.

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
`SKY_CUA_PHONE_DIRECT`, `SKY_CUA_PHONE_DIRECT_LISTEN_ADDR`,
`SKY_CUA_PHONE_DIRECT_ADVERTISED_ENDPOINT`,
`SKY_CUA_PHONE_DIRECT_ENROLLMENT_TTL_MS`, `SKY_CUA_PHONE_DIRECT_STATE_PATH`,
`SKY_CUA_PHONE_CAPABILITY_CACHE_TTL_MS`, `SKY_CUA_PHONE_TARGET_MODELS`
(legacy `SKY_CUA_PHONE_COMPANION_RPC_*` removed 2026-08-25 — **breaking change**, use `SKY_CUA_PHONE_DIRECT_*` `ws://` private LAN/tether or `wss://` tailnet).

Intentionally not stable: the action affordance lists vary by device and
session; the companion APK and its identity sidecar are optional packaging
inputs that may be absent.

## Behavior

`phone_connection(operation="connect")` resolves `adb` (config, then
environment, then `PATH`), probes device identity and display metrics, detects
and caches a per-session
`PhoneCapabilityProfile`, and — when companion support is enabled — verifies,
installs, or updates the companion APK, enables its required services, and
establishes the RPC forward before finalizing the profile. The cache is per
session and is invalidated on reconnect, companion install/update, orientation
or display-size change, RPC failure, wireless disconnect, and explicit
`phone_connection(operation="refresh")`. Permission revocation is not
auto-detected mid-session; reconnect or refresh after permission state may have
changed.

Companion permission setup (as built): an install-bearing bootstrap — the same
`allow_install` gate that authorizes the APK install
(`phone_setup(operation="install_companion")`, or
`phone_connection(operation="connect")` under operator auto-install or an
explicit install-companion request) — also enables the two services the
companion needs to function, so a freshly deployed companion is usable without a
manual trip through Android settings. The companion declares no runtime permissions
(its overlay rides a
`TYPE_ACCESSIBILITY_OVERLAY` window), so setup reduces to two service
enablements, both performed by the adb `shell` user after the signature gate
passes, never for an untrusted package. The accessibility service is enabled by
a read-merge-write of `enabled_accessibility_services` plus the global
`accessibility_enabled` flag; the existing list is always preserved, never
clobbered. The notification listener is bound through `cmd notification
allow_listener` (additive and immediate; a bare `settings put` can leave the
entry present but unbound until the next reconcile). Each enable is reported as a
structured diagnostic (`PhoneCompanionPermissionEnabled`, or
`PhoneCompanionPermissionWriteRejected` on a hard failure); the already-enabled
steady state is silent. After the capability probe, the companion health
booleans are the ground truth: a service the device still reports off (some OEM
builds, notably Samsung One UI, gate a sideloaded app's accessibility behind a
manual "Restricted settings" confirmation the adb write cannot satisfy) yields an
actionable `PhoneCompanion*ManualSetup` diagnostic, and for accessibility the
host best-effort opens the on-device Accessibility screen so the operator can
finish setup by hand.

Freshness contract (as built): the tools that act on or perceive through the
profile — the action tools (`phone_pointer(operation="tap")`,
`phone_pointer(operation="swipe")`, `phone_keyboard(operation="type_text")`,
`phone_keyboard(operation="press_key")`, the notification and app operations),
`observe(surface="phone")`, and `capture_screen(surface="phone")` — carry the
profile id/version used and a
`profile_refresh_state` of `refreshed`, `reused`, or `stale`. `reused` is
emitted on a within-TTL cache hit, and the `profile_refresh_state` is kept in
lockstep with the `stale` boolean. The pure-status tools
(`status(component="phone_companion")`, `phone_accessibility_tree`,
`phone_notifications`, and the app resource reads) do not act on the profile
and intentionally omit the freshness field.

Three backends, deterministically routed from the cached profile:

- ADB baseline (required): discovery, USB and Android 11+ wireless pairing,
  connect/disconnect, device property probing, fallback screenshots, fallback
  input, app inventory/management, companion install/update primitives, and all
  recovery flows. Missing `adb` disables phone-use with a structured
  diagnostic.
- CompanionDirect (preferred when enabled and healthy, since 2026-08-24): native gesture dispatch,
  accessibility window tree, on-device screenshots, notification events and
  actions, and the phone-native agent overlay, reached over a persistent
  `ws://`/`wss://` direct link (`phone-control.v2`, HMAC + epoch-fenced) on private LAN/tether (`192.168/16`, `10/8`, `172.16/12`, `fd00::/7`, `fe80::/10`) or Tailscale. The legacy localhost-only `POST /rpc` via `adb forward` + ephemeral token (`SetupActivity`/`RpcController`/`RpcServer`) was removed 2026-08-24; `status(component="phone")` now surfaces `DirectLanCandidates` for QR generation.
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

Production also attaches a separate tiny input-only catcher window over the
idle cursor. It is pass-through while the agent is moving or animating, then
becomes touchable only after the idle guard so tapping the cursor can trigger
the "no-no" shake and blip without making the full-screen overlay intercept
normal app taps.

- A pronounced pink screen-edge glow signals that the agent is in control while a
  phone session is held: a gently breathing border with a bright wave of light
  travelling around the perimeter (a rotating sweep gradient). The host turns it
  on when a session establishes with a reachable companion, clears it after 20
  seconds without session-bound phone activity, turns it back on when activity
  resumes, and turns it off immediately on disconnect/release. This is an overlay
  lease, not session lifetime: an idle-cleared session keeps the same
  `session_id` and cached state. All geometry is sized in dp from the display
  density so the glow reads at a consistent physical size rather than vanishing
  on a high-DPI panel.
- The cursor is the same `cursor-chat` pointer the desktop agent uses, scaled up
  and seated in a soft pink halo. It is parked at screen centre the moment the
  session connects so the pointer is visible immediately. The phone-native
  active overlay auto-hides with the 20-second idle lease above; disconnect also
  removes it.
- The cursor animates per action: a tap shows an expanding, fading ripple; a
  swipe or drag moves the cursor along the path with a fading trail. The edge
  glow pulses brighter for the action's duration, then returns to the breathing
  baseline. The overlay animation is visual only, but coordinate input is now
  companion-owned: tap/swipe must dispatch through the companion `gesture` method
  so real input and visible feedback stay coupled.
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

- `enabled` (default `true`; env `SKY_CUA_PHONE`): the master switch for the
  phone-use subsystem. Phone-use is **on by default**. Setting `[phone]
  enabled = false` (or the process override to false) also removes the phone MCP
  surface: `phone_*` names disappear and shared tools no longer advertise phone
  branches. `[surfaces].phone = false` has the same model-facing effect and also
  forces the resolved phone manager disabled. The config file is the intended
  durable control; environment values are per-process overrides.
- `visible_overlay` (default `true`; env `SKY_CUA_PHONE_VISIBLE_OVERLAY`):
  whether the on-phone agent overlay (edge glow, cursor, per-action gesture
  animation) is shown. When `false`, the host suppresses every companion
  visible-overlay call — `overlay_active` on connect/disconnect/refresh, idle
  expiry/relight, and `overlay_gesture` after an action — so the device draws no
  agent overlay, while the real input still dispatches. The session's reported
  cursor capabilities and backend capabilities then carry `host_visible_overlay=false` and
  `phone_native_overlay=false` with a config-grounded `visible_overlay_reason`.
  The screenshot-synthetic cursor is a separate plane driven by
  `screenshot_cursor` and is unaffected. ADB-only sessions continue to report
  `visible_overlay=false` regardless, preserving the Phase 3 contract.
- `wireless_auto_connect` (default `false`; env
  `SKY_CUA_PHONE_WIRELESS_AUTO_CONNECT`): when `true` and the configured
  `default_serial` is a wireless `host:port` target, `phone_connection(operation="connect")` runs `adb
  connect <host:port>` for that default before serial resolution, so a
  pre-configured wireless link is brought up and the device becomes present for
  targeting. Best-effort and idempotent; a failed link resurfaces as the normal
  device-unavailable diagnostic. A USB/emulator default or an empty default is a
  no-op.
- `companion_operator_mode` (default `true`; env
  `SKY_CUA_PHONE_COMPANION_OPERATOR_MODE`): gates the host-side privileged
  operator conveniences. As wired, it gates the *silent* companion auto-install
  convenience (`adb install -r`): the automatic install/update on connect or
  `phone_connection(operation="refresh")` runs only when operator mode AND
  `companion_auto_install` are both on. An explicit `phone_setup(operation="install_companion")`
  (or `install_companion: true` on connect) is the operator acting deliberately
  and still installs regardless of operator mode. With operator mode off, an
  already-installed companion still connects (forward + token + probe); only the
  unattended install convenience is suppressed. This is the minimal honest gate;
  no settings-screen automation or deeper privileged setup exists yet to gate.
- `primary_target_models` (default empty; env `SKY_CUA_PHONE_TARGET_MODELS`,
  comma-separated): the operator's known device models. In the
  `list_resources(surface="phone", resource="devices")` path, each device whose reported `model` matches a
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

Live and installed proof:

```bash
# Tool-driver smoke: exercises the whole phone_* family (non-destructive; assumes
# the companion is already installed for its companion profile).
uv run python scripts/live_phone_use_smoke.py --profile adb-usb --serial <serial>
uv run python scripts/live_phone_use_smoke.py --profile full --serial <serial>

# Companion-setup smoke: validates the cold-device -> reachable-companion setup
# workflow. From a cold reset (companion uninstalled, accessibility + notification
# services disabled — only the sky-cua companion's own entries are touched) it
# drives install + service-enable, then proves the result by GROUND TRUTH (adb:
# package installed, both services bound, RPC port listening) plus a pure MCP
# probe with companion auto-install OFF, so reachability can only pass when the
# driver actually completed setup.
uv run python scripts/live_phone_companion_setup_smoke.py --driver agent --serial <serial>
uv run python scripts/live_phone_companion_setup_smoke.py --driver direct --serial <serial>

# Workflow smoke: crystallizes the live agentic workflows (an external agent
# driving a ready device through the phone tools) into a repeatable check —
# Settings -> Accessibility navigation, and a Chrome web search. Each workflow is
# proven by GROUND TRUTH read from adb (the device's resumed activity must be the
# target screen), independent of the agent's own claims; web-page text is not in
# the a11y tree, so the prose answer is a soft signal. Every run also probes the
# phone-native pointer overlay (companion advertises the plane, a screenshot
# reports it live, and a benign device-space tap + swipe route backend=companion).
uv run python scripts/live_phone_workflow_smoke.py --workflow full --serial <serial>
uv run python scripts/live_phone_workflow_smoke.py --workflow settings --agent claude --serial <serial>
```

`--driver agent` (default) has an agent CLI run
`phone_connection(operation="connect")` +
`phone_setup(operation="install_companion")`; the ground-truth checks gate the
pass. The default agent is `claude` (Claude Code reliably surfaces the phone MCP
tools). In this companion-setup harness, the default OpenCode model used during
the 2026-06-20 run saw the phone-use skill but did not expose the phone MCP
tools to the agent, so the ground-truth gate treats that condition honestly
rather than counting prose as setup proof. Tool-call evidence is parsed when the
agent emits structured tool events; `--require-tool-evidence` makes it a hard
gate. `--driver direct` drives the MCP tools deterministically (no agent CLI).
Proven live on the API-36 emulator 2026-06-20 (`--driver direct` 8/8, and
`--driver agent --agent claude`). The pure helpers are unit-tested in
`test_phone_companion_setup_smoke.py`.

The workflow smoke's adb resumed-activity parser, per-workflow ground-truth
evaluation, prompt builders, and CLI options are unit-tested in
`test_live_phone_workflow_smoke.py`; the live workflows themselves require a
ready device and an agent CLI, and SKIP honestly otherwise. Because the agent
reaches sky-cua through its locally deployed MCP runtime, the workflow smoke (via
`run_agent`) is subject to the deploy-freshness gate — run `cua-deploy` first so
the agent drives the current runtime. Proven green on the API-36 emulator
2026-06-20 (`--workflow full --agent claude`, 7 passed / 2 skipped / 0 failed):
the settings agent landed on `com.android.settings/.Settings$AccessibilitySettingsActivity`,
the browser agent landed on `com.android.chrome` and produced the expected answer
(a soft signal — the browser ground truth proves the agent reached Chrome, not
that a specific search ran), and the overlay probe reported `native_overlay=true`
with the tap and swipe routing `backend=companion`. The `tool_evidence` steps SKIP
under `claude` (plain-text mode emits no structured tool events); pass
`--require-tool-evidence` with a JSON-mode agent to make them a hard gate. Evidence
is any-of: each workflow accepts any of several phone action tools, so an efficient
agent that reaches Settings via `phone_setup(operation="open_settings")`
(instead of tapping) still passes. Proven live 2026-06-21 with `--agent
opencode --model kimi-for-coding/k2p7`: kimi drove the Settings →
Accessibility workflow in three tool calls
(`phone_connection(operation="connect")` →
`phone_setup(operation="open_settings")` → `phone_accessibility_tree`), reaching
`com.android.settings/.Settings$AccessibilitySettingsActivity` with no redundant
captures or taps. The harness runs OpenCode from a neutral working directory so
repository fixtures cannot override the installed global MCP configuration.

The full live smoke should also run from the installed MCP surface
(`--installed`) after packaging and record adb version, Android version,
companion version when installed, scrcpy version when used, connection kind,
cursor planes proven, and skipped profiles.

Physical-device CompanionDirect phone observe was proven on 2026-08-09 with
an S26 Ultra over Tailscale. Pi using the text-only
`opencode/deepseek-v4-flash-free` model received exactly one text block and no
image block, and its text included the bounded accessibility projection with
real node text, classes, clickability, and bounds. Codex CLI 0.147.0 using
An image-capable Codex run received one text block and one image block from the same live
phone, without the redundant text fallback. Both results kept structured
content base64-free, and neither client disconnected the retained session.

## Known limitations

- The installed grouped MCP path and ADB-baseline emulator smoke are proven,
  but the full phone-use smoke matrix across wireless pairing, companion,
  scrcpy, adversarial geometry changes, and real devices is still pending.
- Redmi/API-36 tablet lane is blocked until that device is connected. The
  Redmi-family target is complete only when a real device reports HyperOS 3.1
  (or equivalent), Android release 16, and SDK 36 through ADB/capability
  evidence; API 35 is only a documented launch-baseline fallback.
- The companion APK and its identity sidecar may not be built on a host without
  the Android toolchain. A build-bearing `deploy_plugin.py` runs an automatic,
  toolchain-gated, change-detected companion build/stage lane (`_companion.py`)
  before bundling; when JDK 21 + the Android SDK are absent it skips gracefully
  and packaging bundles whatever APK is already staged (conditionally, never
  hard-required), so only the ADB baseline and (if present) scrcpy paths are
  exercised. Building the companion requires JDK 21 (the host default `java` is
  unsupported by AGP) plus SDK build-tools/platforms 35–37; that toolchain note
  lives with the Android build survey research.
- Companion signature checks read the expected signing cert + version + APK
  SHA-256 from the bundled `phone-companion.json` metadata sidecar (env/machine
  config override). A confirmed mismatch — a same-named package whose *readable*
  certificate differs from the packaged companion — is refused, not silently
  replaced. An *unreadable* installed certificate is not a mismatch: modern
  Android (API 28+) does not expose the installed cert SHA-256 through
  `dumpsys package` (only a short signature hash), so refusing it would make the
  companion unusable on every current device while detecting no real impostor.
  The host proceeds in that case and reports `signature_matches_expected=false`
  honestly. Validated live on the API-36 emulator 2026-06-20.
- Legacy RPC token delivery (`am start --es sky_cua_rpc_token`) and the `SetupActivity`/`RpcServer` were removed 2026-08-24. CompanionDirect now uses a persistent HMAC device secret + epoch (enrolled via QR `skycua://enroll`, stored in `CredentialStore` per-host) over `ws://`/`wss://`; no `am start` token is used. See `docs/runtime/direct-lan.md` and `docs/runtime/phone-companion-protocol.md` (now direct-only).
- Coordinate input is companion-only: normal tap/swipe requests fail with
  `PhoneCompanionRequired` when the companion gesture lane is not fresh,
  reachable, and gesture-capable. ADB `input tap`/`swipe` is no longer used as
  the normal coordinate fallback; ADB remains the setup, text/key, app-management,
  and screenshot-fallback lane.
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
