---
name: overlay-pointer-animations
description: "Use only for visual appearance, animation, or composited capture/hide-restore evidence of the Android phone-companion overlay, or source that directly owns its rendering/motion (OverlayMath, AgentOverlayView, AgentOverlayController). Do not use for sky-cua-service phone-manager or overlay_active/overlay_gesture RPC payload/routing work, the Rust/wgpu desktop cursor, or generic Android UI."
---

# Overlay Pointer Animations

This skill is for the phone companion's full-screen Android accessibility
overlay: pink edge glow plus animated agent pointer. The Rust/wgpu desktop
cursor belongs to `agent-cursor-debug`; service-manager RPC routing without a
visual-overlay behavior question belongs to the phone runtime implementation.

## Mandatory plan contract

- The only scenario ids are `corners`, `redirect`, `swipes`, and optional `fan`; never rename or invent equivalents.
- Minimum representative capture is exactly `uv run python scripts/overlay_pointer_animations.py --serial <serial> --scenario corners --scenario redirect --scenario swipes`. The default command selects the same trio and writes one combined `artifacts/overlay-pointer-animations/overlay-pointer-animations.mp4` plus `contact-corners-redirect-swipes*.png`; there is no `--out-dir`.
- A request to avoid real apps still requires the isolated daemon/private socket and `GridTestActivity` on a connected device unless device use is explicitly forbidden. Never replace composited ADB `screencap`/`screenrecord` evidence with synthetic rendering. If no device is available, report the live visual gate unavailable.
- After overlay source/tunable changes, run the documented offline math checks first, rebuild/reinstall the APK for live capture, and rebuild the daemon only for daemon/protocol changes. For a turn-rate or glow change, select `--scenario redirect` and inspect its composited recording/sheets; `phone_screenshot` may supply dimensions/provenance but cannot prove overlay pixels.
- Every visual plan/report must name the serial, exact scenarios and command, `GridTestActivity` isolation, APK rebuilt yes/no, daemon rebuilt yes/no, artifact directory/files, missing gates, and one verdict each for edge glow/status bar, inward wave, pointer nose/curve/rotation and clean stop/reset, ripple timing, and tail-following trail when in scope.

## Choose one path first

Pick the narrowest path that matches the request. Do not run a broader path
unless its stop condition is unmet.

- **Source-only** — the user asks about Kotlin ownership, `OverlayMath`,
  hide/restore, or harness logic and forbids a device/pixel check. Read
  `references/source-map.md`; run the offline math tests when relevant; stop
  with no visual claim.
- **Still inspection** — the question is glow/wave/halo appearance or an
  existing PNG/recording. Use a composited `adb screencap` for a new still, or
  inspect the MP4/contact sheet; never use `phone_screenshot` as overlay-pixel
  proof. Stop after the requested dimensions are judged.
- **Minimum representative** — the change needs movement, a hard redirect,
  and a trail but not every heading. Run `corners`, `redirect`, and `swipes`
  only; `fan` is optional. Stop after those three artifacts are inspected.
- **Full capture** — broad acceptance or several visual dimensions are in
  scope. Run the harness default (`corners + redirect + swipes`), and add
  `--scenario fan` when every heading/resting reset is required. Stop after
  the selected recording/contact sheets and report are complete.

## Source ownership

All paths are repository-relative. Load `references/source-map.md` only for
source/tunable detail; it maps `OverlayMath`, `AgentOverlayView`,
`AgentOverlayController`, `GridTestActivity`, host wiring, and the harness.
Do not infer rendered pixels from service-manager RPC wiring.

## Capture commands

Run from the repository root with a connected `<serial>`:

```bash
# Full/default: rebuild and reinstall the APK.
uv run python scripts/overlay_pointer_animations.py --serial <serial>

# Minimum representative; --scenario is repeatable.
uv run python scripts/overlay_pointer_animations.py --serial <serial> \
  --scenario corners --scenario redirect --scenario swipes

# Fast iteration only when the installed APK has the change.
uv run python scripts/overlay_pointer_animations.py --serial <serial> \
  --scenario redirect --skip-build

# Also rebuild daemon/client only for daemon/protocol changes.
uv run python scripts/overlay_pointer_animations.py --serial <serial> \
  --build-daemon
```

The harness launches only `GridTestActivity`, drives scripted gestures through
the isolated release daemon at
`SKY_CUA_SERVICE_SOCKET_PATH=/tmp/sky-cua-overlay-anim.sock`, and writes
`artifacts/overlay-pointer-animations/`. Default runs rebuild/reinstall the APK
and reuse the daemon. After overlay edits, do not use `--skip-build`.

Scenario-to-quality coverage:

- `corners`: long glides, curved turns, heading rotation, arrival, and reset.
- `redirect`: mid-flight retargeting, momentum bow, heading, and ripple timing.
- `swipes`: diagonal/horizontal travel and tail-following trail.
- `fan`: every heading and reset; optional, not part of the minimum trio.

## Evidence and visual quality

Inspect consecutive frames in `overlay-pointer-animations.mp4` and
`contact-*.png`; a single model screenshot cannot prove animation quality.
Report one verdict per dimension:

- **Edge glow** — pink edge-to-edge, including the status bar.
- **Inward wave** — coherent progression, not a static edge tint.
- **Pointer glide/curve/rotation** — nose follows travel; redirects bow with
  momentum; arrival stops cleanly without spring-back and resets when parked.
- **Ripple** — appears at intended gesture/arrival timing.
- **Trail** — follows direction and fades tail-to-head.

For a new still, use the composited device pixels:

```bash
adb -s <serial> exec-out screencap -p > /tmp/overlay-pointer.png
```

The harness may call `phone_screenshot` for device size and a fresh dispatch
snapshot ID, but model screenshots hide the accessibility overlay. Use `adb
screencap`, `screenrecord`, and the resulting MP4/sheets for visibility proof.
If a model screenshot is missing the overlay, inspect
`AgentOverlayController`'s hide/restore state first; source alone is not visual
proof.

## Math checks and failure diagnostics

After changing `OverlayMath` or pure harness scenario logic, run both checks:

```bash
cd android/phone-companion && \
  JAVA_HOME=/usr/lib/jvm/java-21-openjdk ./gradlew testDebugUnitTest --offline
cd ../.. && uv run pytest scripts/test_overlay_pointer_animations.py
```

The Kotlin tests cover steering, arrival/overshoot, turns, angles,
breathing/wave/pulse, trails, and capture hide/restore. If dispatches are
rejected, resolve or report the stale-snapshot diagnostic before judging a
static recording. If ffmpeg/ImageMagick cannot make sheets, inspect the MP4
and report that limitation. Missing hardware/tools are unrun gates, not passes.

## Safety and stopping criteria

- Never use the operator's daemon or a real app: the private socket and fresh
  release daemon isolate the run, while exported `GridTestActivity` is a static
  grid with no controls/actions.
- Same-key `adb install -r` preserves the enabled accessibility-service grant;
  disconnect after capture.
- Treat recordings of any live device as artifacts to inspect and report, not
  as repository material.

Stop when the selected route has its required artifacts, all five visual
dimensions that are in scope have verdicts, and build/rebuild plus missing
live gates are reported. For source-only or math-only work, explicitly state
that no pixels were inspected.

## Reporting

Return the device serial (if any), selected route and scenarios, APK/daemon
rebuild status, artifact directory and files inspected, one-line verdicts for
glow/wave/pointer/ripple/trail, diagnostics or limitations, and any live gate
not run.
