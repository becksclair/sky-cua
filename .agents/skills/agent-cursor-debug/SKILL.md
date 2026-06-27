---
name: agent-cursor-debug
description: Use when building, tuning, or visually debugging the DESKTOP agent cursor — the wgpu layer-shell overlay's pointer glyph, its glyph-anchored smoke aura, and its grounding shadow (NOT the phone overlay, which is overlay-pointer-animations). Covers the capture.py harness that places the cursor and screenshots it with spectacle on KDE Wayland, native-resolution inspection, the renderer's tunable constants in sky-cua-overlay-host, the offline gesture/texture dump, and the hard-won live-capture pitfalls (pkill-kills-the-shell, the 2s host startup budget, snapshot_id + live timestamp, spectacle-not-grim).
---

# Agent Cursor Debug (desktop)

Use this skill to **visually** iterate on the desktop agent cursor: the wgpu
layer-shell overlay's pointer glyph, the smoke aura that wreathes it, and the
shadow that grounds it. Quality is judged by eye against a real screenshot, so
the harness places the cursor on the operator's KDE Wayland desktop and captures
it — it is a visual harness, not a pass/fail smoke.

This is the **desktop** overlay (Rust/wgpu, `sky-cua-overlay-host`). For the
**phone** overlay (Kotlin companion app) use `overlay-pointer-animations`.

## Where the cursor is rendered

All in `crates/sky-cua-overlay-host/`:

- `src/renderer/mod.rs` — `render_vector_cursor` bakes the glyph into a texture:
  a Chaikin-rounded path turned into a **signed distance field** (R channel), a
  chamfer-transform **smoke anchor** (G channel), plus B/A for the CPU-blit
  fallback. Also the mip-chain generator and `CursorImage::load`.
- `src/renderer/shaders.rs` — the WGSL. `cursor_sample` reconstructs the glyph
  from the SDF with `fwidth` anti-aliasing and tints it; `cursor_smoke` billows
  the edge-glow recipe off the G-channel anchor; `cursor_shadow` is a separate
  pass drawn UNDER the smoke. `render_pixel` is the composite order.
- `src/renderer/wgpu.rs` — cursor texture upload (mip levels, trilinear sampler,
  linear `Rgba8Unorm` format).

The architecture and the *why* behind each piece live in
`docs/features/agent-cursor-overlay.md` ("Cursor glyph, smoke aura, and shadow").

## Tunable constants

Change these, rebuild, recapture:

- **Size / aura band** — `src/lib.rs` `cursor_asset`:
  `AGENT_CURSOR_DESKTOP_WIDTH/HEIGHT` + `_HOTSPOT_X/Y` (on-screen glyph size; the
  source path is 46×48 and is scaled down), and `AGENT_CURSOR_SMOKE_MARGIN` (how
  far the smoke reaches — bigger = larger cloud).
- **Glyph** — `src/renderer/mod.rs`: `CURSOR_STROKE_EDGE` (outline ring width,
  normalized SDF units — **also change the matching WGSL const**, guarded by
  `stroke_edge_matches_shader_constant`), `CURSOR_CORNER_ROUNDING` (Chaikin
  iterations), `SDF_RANGE_TEXELS` (distance the field encodes; widen it to give
  the shadow more room, and halve `CURSOR_STROKE_EDGE` to keep the outline width).
- **Tint / smoke / shadow** — `src/renderer/shaders.rs`: the `fill`/`edge` tint
  in `cursor_sample`; the `density` threshold and `alpha` multipliers and
  `CURSOR_SMOKE_OFFSET_*` (up-left shift so the cloud centres on the hotspot) in
  `cursor_smoke`; the `CURSOR_SHADOW_*` constants (offset, blur LOD, reach,
  falloff, strength).

## Capture and inspect

```bash
cargo build --release -p sky-cua-overlay-host
python3 .agents/skills/agent-cursor-debug/capture.py 0.4 0.45
```

`capture.py` starts an isolated service on a private socket, places the cursor at
that fraction of the screen, captures with spectacle, and writes to
`/tmp/agent-cursor-debug/`:

- `capture.png` — the full virtual-desktop screenshot.
- `cursor_native7x.png` — the glyph at **true native resolution**, 7× nearest.
- `cursor_context.png` — the cursor in context.

Then **read** those PNGs and judge. Place over light content (a higher `FY`, or
a spot with text) to see the shadow; the glyph and smoke read on any background.

> Run the Bash call with the sandbox disabled — the service needs the KDE portal.

### Inspect at the right zoom

Judge `cursor_native7x.png` (downsampled to native, then nearest-zoomed), **not**
a high zoom of the raw `capture.png`. Spectacle captures at 2× the logical
desktop, so zooming the raw capture fakes stair-stepping that is not on screen.
`capture.py` already does the 2× downsample.

## Offline renderer / texture dump (no live overlay)

To inspect the cursor texture or a gesture scene straight from the GPU renderer
(useful when the live overlay misbehaves, or on a headless box):

```bash
SKY_CUA_CAPTURE_GESTURES=1 cargo test --release -p sky-cua-overlay-host \
  capture_gesture_frames_when_requested -- --nocapture
```

Writes RGBA frames + `cursor_texture.rgba` (+ `cursor_dims.txt`) to
`/tmp/overlay-demo/gestures/`. Split the texture's channels to see R = SDF,
G = smoke anchor, A = coverage independently.

## Pitfalls (these cost real time — heed them)

- **`pkill -f sky-cua-overlay-host` kills your own shell.** The pattern matches
  the shell's argv/env (it carries the host path), so the script dies with no
  output. Use `pkill -x sky-cua-overlay` (the comm is truncated to 15 chars).
- **Do not blanket-kill `sky-cua-service`** — that also kills the operator's
  installed daemon. Kill the smoke service by PID (capture.py does).
- **The Bash tool waits on lingering background daemons.** Do start + capture +
  teardown in ONE process (capture.py owns the whole lifecycle), or the call
  hangs.
- **The shell profile runs `set -e`.** A `pkill` that matches nothing returns
  non-zero and aborts the whole script — guard ad-hoc shell with `|| true`.
- **2-second startup budget.** `CursorImage::load` runs at process start; if it
  exceeds the host's `HOST_START_TIMEOUT` (2 s) the service kills the overlay
  host and only the standalone KWin effect's edge glow survives — it *looks* like
  the overlay works but the cursor is missing. Keep load fast (it is ~hundreds of
  ms). The overlay host's stderr is discarded (`Stdio::null`), so a startup crash
  is silent — use the offline dump above to confirm the renderer itself is fine.
- **The cursor only renders with `snapshot_id` AND a ~now `updated_at_ms`.** A
  zero/stale timestamp reads as a decayed cursor and draws nothing.
- **Use spectacle, not grim.** KWin has no `wlr-screencopy`; grim fails.
  `spectacle -b -n -f -o <path>` captures the whole virtual desktop.
- **The overlay surface is fullscreen per output**, so there is no per-cursor
  damage rect to widen when the glyph/aura grows.

## Verify the Rust side

```bash
cargo fmt --check && cargo test -p sky-cua-overlay-host
```

Covers the SDF/anchor/transparency invariants, the WGSL compute conformance, and
`stroke_edge_matches_shader_constant` (Rust↔WGSL stroke-width guard).

## Safety

- Uses an **isolated** service on `/tmp/agent-cursor-debug/svc.sock` with the
  freshly built `target/release` binaries; it never touches the operator's
  installed daemon, and tears itself down.
- `capture.png` is a screenshot of the operator's live desktop — treat it as
  sensitive, keep it under `/tmp`, and never commit it.

## Reporting

Report the cursor fraction captured, the artifact directory, which constants you
changed, and a one-line visual verdict per dimension (glyph crispness, outline,
tint, smoke shape/density/position, shadow). Note that live KDE-desktop capture
is the only proof run unless you also ran the VM smokes.
