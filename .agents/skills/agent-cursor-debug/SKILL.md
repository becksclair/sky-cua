---
name: agent-cursor-debug
description: "Use for visual debugging of the desktop Rust/wgpu agent-cursor overlay: glyph, smoke aura, shadow, or glide/rotation/arrival/trail motion. Covers KDE captures and deterministic dumps. Not for the phone companion overlay; use overlay-pointer-animations."
---

# Agent Cursor Debug (desktop)

This skill is only for the desktop `sky-cua-overlay-host` cursor: its pointer
glyph, glyph-anchored smoke aura, grounding shadow, and motion behavior. The
phone companion overlay belongs to `overlay-pointer-animations`.

## Mandatory plan contract

- Still proof must name `cargo build --release -p sky-cua-overlay-host` and `python3 .agents/skills/agent-cursor-debug/capture.py <FX> <FY>`, the private `/tmp/agent-cursor-debug/svc.sock` isolated-service lifecycle, and inspection of `/tmp/agent-cursor-debug/cursor_native7x.png` plus `cursor_context.png`. Report FX/FY, changed constants, and separate glyph, outline, tint, smoke placement/density, and light-background shadow verdicts.
- Motion proof for redirect plus arrival feedback must name `uv run python scripts/overlay_motion_animations.py --scenario redirect --scenario tap_settle`, inspect the MP4/contact sheets under `artifacts/overlay-motion-animations/`, judge bowed momentum/eased nose/trail in `redirect`, and judge ripple/squash only after arrival in `tap_settle`. If live capture is unavailable, use exactly `SKY_CUA_CAPTURE_MOTION=1 cargo nextest run --release -p sky-cua-overlay-host -E 'test(capture_motion_frames_when_requested)'`, inspect `/tmp/overlay-demo/motion/manifest.txt` and its frames, and state that KDE composition was not proved.
- Minimum general acceptance is one `capture.py` still over light/text-heavy content plus the exact `redirect` and `tap_settle` scenarios; do not invent scenario names, output flags, or artifact paths.
- For a source-only missing-cursor diagnosis, separately inspect `snapshot_id` plus near-now `updated_at_ms` freshness and the two-second `HOST_START_TIMEOUT`. The deterministic gesture dump can prove renderer health only; it cannot simulate or prove stale-state rejection or delayed host startup. Keep those source-backed hypotheses explicit unless an existing focused test exercises them.
- Never claim artifacts not produced by the documented command. Keep live captures under `/tmp` or ignored `artifacts/`, never commit them, and report every live gate not run.

## Choose the proof path

Identify the requested proof before reading detail. Load only the referenced
catalog or pitfall note that the path needs.

1. **Source-only inspection** — trace ownership or tunables without touching a
   desktop. Read [`references/source-and-tunables.md`](references/source-and-tunables.md).
2. **Still capture** — judge glyph, outline, tint, smoke, or shadow. Use the
   `capture.py` still harness and inspect its native/context artifacts.
3. **Motion capture** — judge glide, redirect, heading rotation, arrival
   feedback, or trail. Prefer the live redirect video; use the offline motion
   dump when the KDE portal is unavailable.
4. **Offline dump** — inspect renderer channels, gesture frames, or the real
   motion driver without KDE or a live overlay.

For every selected path, plans and reports must name the exact documented
command and expected artifact files; do not replace the harness with an ad-hoc
capture, a VM profile, or an unrelated test filter.

For a general visual-acceptance request, the minimum representative proof is
one still over light/text-heavy content plus one motion invocation selecting
both `redirect` and `tap_settle`.
If live composition is unavailable, pair the still (if possible) with
deterministic motion evidence and label the live gate as unrun.
Rust tests and VM smokes are supplemental and never replace this still-plus-
motion visual matrix.

## Hard stops and platform boundary

- Do not use blanket `pkill` for overlay hosts or `sky-cua-service`. Run
  `capture.py` as one lifecycle, or use
  `_overlay_host.terminate_leftover_hosts("/tmp/agent-cursor-debug/agent-cursor.sock")`; pass the private host socket, not the service IPC socket `/tmp/agent-cursor-debug/svc.sock`. It is socket
  scoped and must not touch the operator's service-owned host.
- A live cursor state needs both `snapshot_id` and a near-now
  `updated_at_ms`. A zero or stale timestamp makes the cursor decay away.
- For a missing-cursor diagnosis, check `snapshot_id`, `updated_at_ms`, and the
  two-second `HOST_START_TIMEOUT` path before proposing source or harness
  changes; source-only inspection must remain read-only.
- On KDE/KWin use `spectacle -b -n -f -o <path>` for the whole desktop. Do not
  substitute `grim`; KWin has no `wlr-screencopy` path.
- The live service must use a private socket and freshly built release
  binaries. Screenshots and recordings show the operator's desktop: keep them
  under `/tmp` or ignored `artifacts/` and never commit them.

For the shell, startup, timestamp, and compositor failure signatures behind
these stops, read [`references/live-capture-pitfalls.md`](references/live-capture-pitfalls.md).

## Still appearance proof

Build and run from the repository root:

```bash
cargo build --release -p sky-cua-overlay-host
python3 .agents/skills/agent-cursor-debug/capture.py 0.4 0.45
```

`FX FY` are fractions of the primary capture. Choose a light or text-heavy
background when shadow contrast matters. The isolated harness writes to
`/tmp/agent-cursor-debug/`:

- `capture.png` — full virtual-desktop evidence.
- `cursor_native7x.png` — true native-resolution glyph, nearest-neighbour 7×.
- `cursor_context.png` — cursor in context.

Read `cursor_native7x.png` and `cursor_context.png`. Spectacle is 2× logical
desktop resolution; judging a zoomed raw `capture.png` invents stair-stepping.
Report the fraction, artifact directory, and changed constants.

## Motion behavior proof

Build first, then use the KDE ScreenCast portal for live evidence:

```bash
cargo build --release -p sky-cua-overlay-host
uv run python scripts/overlay_motion_animations.py \
  --scenario redirect --scenario tap_settle
```

The MP4 and montage contact sheets land in
`artifacts/overlay-motion-animations/`. Inspect the redirect trace for a bowed
momentum path, an eased nose into the travel heading, and a trail that ramps
tail to head; inspect `tap_settle` for ripple/squash only after arrival. The harness uses a
private socket and owns its overlay lifecycle; the recording is sensitive live
desktop evidence. The structured, non-visual glide check is:

```bash
python3 scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-motion-glide
```

## Offline renderer and motion proof

Use these when the portal is unavailable, a headless machine is involved, or
the question is renderer/driver behavior rather than KDE composition:

```bash
SKY_CUA_CAPTURE_GESTURES=1 cargo nextest run --release -p sky-cua-overlay-host \
  -E 'test(capture_gesture_frames_when_requested)'
SKY_CUA_CAPTURE_MOTION=1 cargo nextest run --release -p sky-cua-overlay-host \
  -E 'test(capture_motion_frames_when_requested)'
```

Gesture evidence is under `/tmp/overlay-demo/gestures/` (`cursor_texture.rgba`,
`cursor_dims.txt`, and RGBA frames). Inspect R=SDF, G=smoke anchor, and
A=coverage independently. Motion evidence is under
`/tmp/overlay-demo/motion/` with dense frames and a manifest for deterministic
corner-glide, redirect, swipe-chase, and arrival-gated-tap scenarios. Offline
evidence proves renderer/driver behavior, not KDE desktop composition.

## Visual quality dimensions

Use separate verdicts; do not collapse them into “looks good.”

- **Still:** glyph crispness at native resolution; outline/stroke consistency;
  fill/edge tint and contrast; smoke shape, density, and hotspot position; and
  shadow offset, blur, reach, and readability over light content.
- **Motion:** momentum bow on redirect; eased heading rotation; arrival-gated
  ripple/squash; and trail direction/ramp from tail to head.

## Validation, stopping, and reporting

When renderer or motion code changed, run the narrow Rust checks:

```bash
cargo fmt --check && cargo nextest run -p sky-cua-overlay-host
```

Stop after the selected proof produces the expected artifact, the artifact has
been inspected, each applicable quality dimension has a verdict, and live vs
offline evidence is explicit. If a portal or desktop gate is unavailable,
stop after the documented offline fallback and state that limitation; do not
invent a screenshot result or keep mutating the operator's processes.

Report: proof path; cursor fraction when applicable; artifact directory and
files actually inspected; source files/constants changed; per-dimension
verdicts; validation commands; and any live gate not run. For ownership and
the complete tunable map, read [`references/source-and-tunables.md`](references/source-and-tunables.md).
