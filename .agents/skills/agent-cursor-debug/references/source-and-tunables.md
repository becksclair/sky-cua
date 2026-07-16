# Desktop cursor source ownership and tunables

Read this reference for source-only inspection or when a visual change needs
the owning Rust/WGSL constant. The desktop implementation is under
`crates/sky-cua-overlay-host/`; do not infer ownership from the capture
harness.

## Source ownership

- `src/renderer/mod.rs`: `render_vector_cursor`, Chaikin-rounded glyph path,
  SDF in R, chamfer smoke anchor in G, B/A CPU-blit fallback, mip generation,
  and `CursorImage::load`.
- `src/renderer/shaders.rs`: WGSL `cursor_sample` glyph reconstruction and
  tint, `cursor_smoke` edge glow, `cursor_shadow`, and `render_pixel` composite
  order. Shadow is drawn under smoke.
- `src/renderer/wgpu.rs`: cursor texture upload, mip levels, trilinear
  sampler, and linear `Rgba8Unorm` format.
- `src/lib.rs` `cursor_asset`: on-screen size, hotspot, and smoke margin.
- `src/motion.rs` and `src/cursor_motion.rs`: Mover2D motion, heading,
  arrival-gated feedback, and resampled trail; shared motion values come from
  the `[shared.motion]` specification.
- `docs/features/agent-cursor-overlay.md`: architecture and rationale.

## Tunable map

Change the owning source, rebuild, and recapture the relevant proof path.

- **Size and aura band** — `AGENT_CURSOR_DESKTOP_WIDTH`,
  `AGENT_CURSOR_DESKTOP_HEIGHT`, `AGENT_CURSOR_DESKTOP_HOTSPOT_X`,
  `AGENT_CURSOR_DESKTOP_HOTSPOT_Y`, and `AGENT_CURSOR_SMOKE_MARGIN` in
  `src/lib.rs` / `cursor_asset`. The source path is 46×48 and is scaled down;
  a larger margin reaches farther from the glyph.
- **Glyph geometry** — `CURSOR_STROKE_EDGE`,
  `CURSOR_CORNER_ROUNDING`, and `SDF_RANGE_TEXELS` in `src/renderer/mod.rs`.
  `CURSOR_STROKE_EDGE` has a matching WGSL constant and is guarded by
  `stroke_edge_matches_shader_constant`. Widening the SDF range gives the
  shadow more room; halve the stroke edge to preserve outline width.
- **Tint, smoke, and shadow** — fill/edge tint in `cursor_sample`; density
  threshold, alpha multipliers, and `CURSOR_SMOKE_OFFSET_*` in
  `cursor_smoke`; `CURSOR_SHADOW_*` offset, blur LOD, reach, falloff, and
  strength in `src/renderer/shaders.rs`.

After changing Rust/WGSL stroke width, run the guard in the scoped Rust tests.
For a shadow-only change, still capture over light content. For a motion-only
change, use video or the deterministic motion dump; a still cannot prove
heading, arrival, or trail behavior.
