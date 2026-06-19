---
name: overlay-pointer-animations
description: Use when building, tuning, calibrating, or visually verifying the phone-side agent overlay — the pink screen-edge glow and inward wave, and the agent pointer's glide / momentum-curve / heading rotation / ripple / trail — drawn by the companion app. Covers the scripts/overlay_pointer_animations.py capture harness, the companion GridTestActivity test canvas, the OverlayMath/AgentOverlayView/AgentOverlayController source and its tunable constants, and the screencap/screenrecord+ffmpeg+montage evidence flow.
---

# Overlay Pointer Animations

Use this skill to exercise and visually verify the **phone-side agent overlay**:
the companion app draws the agent cursor and a glowing pink screen edge on the
device (mirrored into scrcpy when present), with a momentum-steered pointer that
noses into its travel direction and curves. Quality here is judged by eye, so the
harness records the device and produces frame contact sheets for review — it is a
visual harness, not a pass/fail smoke.

## Where the overlay lives

All on-device, in the companion app:

- `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlayMath.kt`
  — pure, unit-tested motion/animation math: the `Mover2D` vehicle-steering
  cursor (heading + speed, turn-rate limited, arrival deceleration, resets to the
  resting nose), `breathingIntensity` / `wavePhase` / `breathing01`, ripple/trail
  helpers, angle helpers, and the `CaptureState`.
- `.../overlay/AgentOverlayView.kt` — the single full-screen pass-through
  `TYPE_ACCESSIBILITY_OVERLAY` view: pink edge glow, inward waves, rotated pointer
  bitmap + breathing halo, ripple, trail. All geometry is dp-scaled by density.
- `.../overlay/AgentOverlayController.kt` — owns the window (sized to the full
  display incl. status bar via `currentWindowMetrics` + cutout mode), the ambient
  animator (wave + glow breathing + halo breathing + the cursor spring/mover step
  and rotation), per-gesture animation, and the screenshot hide/restore.
- `.../ui/GridTestActivity.kt` — the neutral white + light-grey grid canvas used
  for testing (see below).

The host wiring (`overlay_active` / `overlay_gesture` RPCs) is in
`crates/sky-cua-service/src/phone/manager/` and is documented in
`docs/features/phone-use.md` and `docs/runtime/phone-companion-protocol.md`.

## Tunable constants (OverlayMath)

The "feel" lives in `OverlayMath` companion constants:

- Motion: `CURSOR_MAX_SPEED_DP_S`, `CURSOR_ACCEL_DP_S2`, `CURSOR_TURN_RATE_DEG_S`
  (lower = wider curves), `CURSOR_ARRIVE_RADIUS_DP`, `CURSOR_NOSE_DEG` (resting
  nose heading / pointer-rotation calibration), `CURSOR_ROTATE_RATE_DEG_S`.
- Glow / wave / halo: in `AgentOverlayView` (`PINK`, `MAX_BASE_ALPHA`,
  `WAVE_*`, `CURSOR_HALO_RADIUS_DP`, `HALO_SCALE_*`); breathing band in
  `OverlayMath` (`GLOW_BASELINE_MIN/MAX`, `WAVE_PERIOD_MS`, `HALO_BREATHE_PERIOD_MS`).

After editing, rebuild + reinstall the APK and re-run the capture harness.

## Run the capture harness

`scripts/overlay_pointer_animations.py` rebuilds + reinstalls the companion APK
(the overlay lives in it), launches `GridTestActivity`, connects through the
**isolated release daemon** on a private socket (never the operator's installed
daemon), drives scripted pointer moves while screen-recording, and writes the
recording + contact sheets to `artifacts/overlay-pointer-animations/`.

```bash
# Full run (rebuilds APK, default scenarios: corners + redirect + swipes)
uv run python scripts/overlay_pointer_animations.py --serial <serial>

# Iterate fast on a constant tweak without rebuilding the APK:
uv run python scripts/overlay_pointer_animations.py --serial <serial> --skip-build

# A single scenario; repeatable. corners | redirect | swipes | fan
uv run python scripts/overlay_pointer_animations.py --serial <serial> --scenario redirect

# Also rebuild the release daemon (rarely needed — overlay is all in the APK):
uv run python scripts/overlay_pointer_animations.py --serial <serial> --build-daemon
```

Scenarios:

- `corners` — big corner-to-corner taps: long glides that curve, rotate, settle.
- `redirect` — rapid taps faster than a glide settles, so the cursor is redirected
  mid-flight and the momentum bows the path hard (best curve evidence).
- `swipes` — diagonal/horizontal swipes: the cursor sails the path, trail follows.
- `fan` — taps out to a ring and back, exercising every heading + rotation.

## Verify

This is a **visual** check. Inspect the artifacts:

- `artifacts/overlay-pointer-animations/overlay-pointer-animations.mp4` — the raw
  recording (watch the motion directly).
- `artifacts/overlay-pointer-animations/contact-*.png` — a tiled contact sheet of
  extracted frames. Trace the pointer across consecutive frames to confirm: the
  edge glow reads (pink, edge-to-edge incl. status bar), the pointer noses into its
  heading, the path curves on direction changes (does not snap to a straight
  line), it eases to a clean stop with no spring-back, and it resets to the resting
  orientation when parked.

For a single still (e.g. judging glow intensity / halo size), a raw
`adb -s <serial> exec-out screencap -p > /tmp/x.png` captures the composited
accessibility overlay; the companion hides the overlay for its own model
screenshots, so use screencap/screenrecord rather than `phone_screenshot` to see
the overlay. The glow reads best on a dark background.

## Unit tests (pure math)

The motion/animation math is unit-tested without a device — run after any
`OverlayMath` change:

```bash
cd android/phone-companion && JAVA_HOME=/usr/lib/jvm/java-21-openjdk ./gradlew testDebugUnitTest --offline
```

`AgentOverlayTest` covers the `Mover2D` vehicle (snap, exact arrival without
overshoot, momentum curve on a turn, frame-step clamp), the angle helpers,
breathing/wave/pulse, and the capture state machine. The harness's own pure
scenario logic is covered by `scripts/test_overlay_pointer_animations.py`.

## Safety

- Uses the isolated daemon (`SKY_CUA_SERVICE_SOCKET_PATH=/tmp/sky-cua-overlay-anim.sock`,
  the freshly built `target/release/sky-cua-service`) — it never disturbs the
  operator's installed daemon.
- Drives only `GridTestActivity`, never the operator's real apps. The grid
  activity is a static canvas with no controls or actions.
- Reinstalling the companion with `adb install -r` preserves the enabled
  accessibility-service grant (same debug signing key).
- `GridTestActivity` is exported so `am start` can launch it; it shows only a
  static grid and performs no actions, so it carries no privilege/data risk.

## Reporting

Report the device serial, the scenarios run, whether the APK/daemon were rebuilt,
the artifact directory, and a one-line visual verdict per dimension (glow, wave,
pointer glide/curve/rotation, ripple, trail). Note any live gate not run.
