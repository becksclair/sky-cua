# Phone overlay source map and tunables

Load this reference for source-only diagnosis or when tuning constants. Paths
are relative to the `sky-cua` repository.

## Ownership map

- `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlayMath.kt`
  owns pure motion/animation math: the `Mover2D` vehicle (heading, speed,
  turn-rate limit, arrival deceleration, resting-nose reset), breathing/wave
  helpers, ripple/trail helpers, angle helpers, and `CaptureState`.
- `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/AgentOverlayView.kt`
  owns the full-screen pass-through
  `TYPE_ACCESSIBILITY_OVERLAY` drawing: pink edge glow, inward waves, rotated
  pointer bitmap, breathing halo, ripple, and trail. Geometry is density-scaled.
- `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/AgentOverlayController.kt`
  owns the full-display window,
  ambient and gesture animation loops, cursor rotation, and screenshot
  hide/restore. Its `currentWindowMetrics`/cutout handling covers the status bar.
- `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/ui/GridTestActivity.kt`
  is the neutral white/light-grey grid canvas used
  by the harness. Host `overlay_active`/`overlay_gesture` RPC wiring lives in
  `crates/sky-cua-service/src/phone/manager/`; it does not own rendering.
- `scripts/overlay_pointer_animations.py` owns the isolated capture lifecycle,
  scripted scenarios, screen recording, and contact-sheet production;
  `scripts/test_overlay_pointer_animations.py` owns its pure scenario tests.

## Tunable constants

- Motion in `OverlayMath`: `CURSOR_MAX_SPEED_DP_S`, `CURSOR_ACCEL_DP_S2`,
  `CURSOR_TURN_RATE_DEG_S` (lower means wider curves),
  `CURSOR_ARRIVE_RADIUS_DP`, `CURSOR_NOSE_DEG`, and
  `CURSOR_ROTATE_RATE_DEG_S`.
- Glow/wave/halo in `AgentOverlayView`: `PINK`, `MAX_BASE_ALPHA`, `WAVE_*`,
  `CURSOR_HALO_RADIUS_DP`, and `HALO_SCALE_*`.
- Breathing bands/periods in `OverlayMath`: `GLOW_BASELINE_MIN/MAX`,
  `WAVE_PERIOD_MS`, and `HALO_BREATHE_PERIOD_MS`.

After any source/tunable edit, rebuild and reinstall the APK, then recapture.
Rebuild the daemon only when service/client wiring changes; the overlay itself
lives in the APK.
