//! Vector cursor texture synthesis.
//!
//! Builds the agent-pointer texture the WGPU shader samples: a signed distance
//! field of the glyph path (R channel), a chamfer-transform smoke anchor (G), and
//! a stepped luminance / coverage (B/A) used only by the CPU blit fallback — plus
//! the box-filtered mip chain the GPU minifies trilinear. Pure CPU and
//! platform-agnostic; no Wayland/wgpu types.
//!
//! Overrides the parent renderer module's blanket `allow(dead_code)` so unused
//! cursor helpers surface here instead of being silently masked.
#![warn(dead_code)]

use crate::cursor_asset;
use anyhow::Result;

/// One downsampled level of the cursor texture mip chain.
#[derive(Debug)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Decoded RGBA cursor image used by the WGPU renderer (and a CPU blit fallback
/// in tests).
#[derive(Debug)]
pub struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Downsampled levels 1..N (level 0 is `rgba`). The WGPU renderer uploads
    /// these so the GPU can trilinear-minify the 4x-oversized texture without
    /// aliasing the glyph edges; the CPU blit fallback only uses level 0.
    pub mips: Vec<MipLevel>,
}

impl CursorImage {
    pub fn load() -> Result<Self> {
        // The texture covers the glyph PLUS a smoke margin on every side so the
        // shader can billow border-style smoke off the glyph silhouette.
        let width = cursor_asset::AGENT_CURSOR_FOOTPRINT_WIDTH * CURSOR_TEXTURE_SCALE;
        let height = cursor_asset::AGENT_CURSOR_FOOTPRINT_HEIGHT * CURSOR_TEXTURE_SCALE;
        let image = render_vector_cursor(width, height);
        let mips = generate_cursor_mips(&image, width, height);
        Ok(Self {
            width,
            height,
            rgba: image,
            mips,
        })
    }
}

/// Box-downsample the cursor texture into a full mip chain (levels 1..N). The
/// texture is rendered at `CURSOR_TEXTURE_SCALE`x its on-screen footprint, so
/// without mips the GPU minifies a 4x-oversized texture with a single bilinear
/// tap. Every channel is a plain box average: R is a signed distance field and G
/// the smoke anchor — both smooth fields that average linearly — while B/A are
/// the (fallback-only) luminance and coverage.
fn generate_cursor_mips(base: &[u8], width: u32, height: u32) -> Vec<MipLevel> {
    let mut levels = Vec::new();
    let mut prev = base.to_vec();
    let (mut pw, mut ph) = (width, height);
    while pw > 1 || ph > 1 {
        let nw = (pw / 2).max(1);
        let nh = (ph / 2).max(1);
        let mut next = vec![0_u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let x0 = (2 * x).min(pw - 1);
                let x1 = (2 * x + 1).min(pw - 1);
                let y0 = (2 * y).min(ph - 1);
                let y1 = (2 * y + 1).min(ph - 1);
                let mut sum = [0.0_f32; 4];
                for (sx, sy) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                    let i = ((sy * pw + sx) * 4) as usize;
                    for ch in 0..4 {
                        sum[ch] += prev[i + ch] as f32;
                    }
                }
                let o = ((y * nw + x) * 4) as usize;
                for ch in 0..4 {
                    next[o + ch] = float_to_u8(sum[ch] / 4.0);
                }
            }
        }
        prev = next;
        levels.push(MipLevel {
            width: nw,
            height: nh,
            rgba: prev.clone(),
        });
        pw = nw;
        ph = nh;
    }
    levels
}

#[derive(Debug, Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}

/// White-outline ring half-width as a fraction of the SDF's normalized range —
/// MUST equal the shader's `CURSOR_STROKE_EDGE` (guarded by a test). The ring's
/// on-screen width is `CURSOR_STROKE_EDGE * SDF_RANGE_TEXELS / CURSOR_TEXTURE_SCALE`
/// logical px (independent of the glyph size), so a smaller cursor keeps a
/// proportionally bolder outline.
const CURSOR_STROKE_EDGE: f32 = 0.15;
/// Chaikin corner-rounding iterations applied to the flattened glyph path before
/// it is turned into a distance field. Higher = rounder, softer corners.
const CURSOR_CORNER_ROUNDING: u32 = 2;
/// Supersample factor for the cursor texture. The vector cursor is rasterized
/// at this multiple of its on-screen footprint so the GPU samples a
/// high-resolution texture down to size — crisp on hidpi / fractionally-scaled
/// outputs instead of a blocky blit. The footprint stays the
/// `AGENT_CURSOR_DESKTOP_*` size via `cursor_metrics`.
const CURSOR_TEXTURE_SCALE: u32 = 4;
/// Texel span of the glyph signed-distance field packed into the texture's R
/// channel: the `[0,1]` channel encodes a signed distance of `[-R/2, +R/2]`
/// texels. Generous enough that the white ring plus its anti-aliasing fades
/// fully before the field saturates (otherwise a faint ghost ring lingers at the
/// saturation boundary), AND that the soft shadow has room to spread well beyond
/// the glyph. Scales with the texture scale.
const SDF_RANGE_TEXELS: f32 = 12.0 * CURSOR_TEXTURE_SCALE as f32;

// Same pathData as android/phone-companion/.../res/drawable/agent_cursor.xml.
const CURSOR_PATH: &[PathCommand] = &[
    PathCommand::Move(Point { x: 10.0, y: 11.0 }),
    PathCommand::Quad(Point { x: 10.5, y: 9.5 }, Point { x: 11.99, y: 10.03 }),
    PathCommand::Line(Point { x: 37.01, y: 18.97 }),
    PathCommand::Quad(Point { x: 38.5, y: 19.5 }, Point { x: 38.0, y: 21.0 }),
    PathCommand::Line(Point { x: 38.0, y: 21.0 }),
    PathCommand::Quad(Point { x: 37.5, y: 22.5 }, Point { x: 36.0, y: 23.0 }),
    PathCommand::Line(Point { x: 29.77, y: 25.08 }),
    PathCommand::Quad(Point { x: 25.5, y: 26.5 }, Point { x: 24.08, y: 30.77 }),
    PathCommand::Line(Point { x: 22.29, y: 36.13 }),
    PathCommand::Quad(Point { x: 21.5, y: 38.5 }, Point { x: 19.5, y: 37.0 }),
    PathCommand::Line(Point { x: 19.5, y: 37.0 }),
    PathCommand::Quad(Point { x: 17.5, y: 35.5 }, Point { x: 16.68, y: 33.14 }),
    PathCommand::Line(Point { x: 10.02, y: 13.99 }),
    PathCommand::Quad(Point { x: 9.5, y: 12.5 }, Point { x: 10.0, y: 11.0 }),
];

#[derive(Debug, Clone, Copy)]
enum PathCommand {
    Move(Point),
    Line(Point),
    Quad(Point, Point),
}

fn render_vector_cursor(width: u32, height: u32) -> Vec<u8> {
    // `width`/`height` cover the glyph plus the smoke margin on every side. The
    // glyph is rasterized at its own size and OFFSET into the canvas by the
    // margin so the surrounding band is free space for the shader's smoke.
    let margin = (cursor_asset::AGENT_CURSOR_SMOKE_MARGIN * CURSOR_TEXTURE_SCALE) as f32;
    let glyph_w = (cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH * CURSOR_TEXTURE_SCALE) as f32;
    let glyph_h = (cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT * CURSOR_TEXTURE_SCALE) as f32;
    let scale_x = glyph_w / cursor_asset::AGENT_CURSOR_SOURCE_WIDTH as f32;
    let scale_y = glyph_h / cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT as f32;
    // Flatten the glyph path, then round its corners (Chaikin) so the cursor
    // reads softer/less pointy.
    let path = chaikin_round(
        &flattened_cursor_path(scale_x, scale_y, margin, margin),
        CURSOR_CORNER_ROUNDING,
    );
    // Outward extent of the white outline ring (texture px), derived from the
    // shared SDF parameters so it matches what the shader reconstructs.
    let stroke_extent = CURSOR_STROKE_EDGE * SDF_RANGE_TEXELS;

    // Glyph raster bounds: the glyph rect plus a pad covering the full SDF range
    // (the shader reads the field out to its saturation distance). The coverage
    // pass only runs here — outside this box the canvas is pure smoke margin.
    let pad = (SDF_RANGE_TEXELS * 0.5).ceil() as u32 + 2;
    let gx0 = (margin as u32).saturating_sub(pad);
    let gy0 = (margin as u32).saturating_sub(pad);
    let gx1 = (margin as u32 + glyph_w as u32 + pad).min(width);
    let gy1 = (margin as u32 + glyph_h as u32 + pad).min(height);

    // Per-pixel SIGNED distance to the glyph path (negative inside the fill,
    // positive outside), packed into the texture's R channel as a distance
    // FIELD. The shader reconstructs the black fill + white outline from it with
    // fwidth-based anti-aliasing at the FINAL framebuffer resolution. A thin
    // outline pre-rasterized into pixels cannot survive the GPU minifying this
    // oversized texture without stair-stepping; a smooth SDF can.
    //
    // `glyph_alpha` (coverage) drives the smoke-anchor seed and the A channel;
    // `glyph_lum` (stepped) is the B channel for the CPU blit fallback only.
    // R defaults to "far outside" (1.0 = fully transparent) so untouched margin
    // pixels never reconstruct as fill.
    let mut glyph_sdf = vec![1.0_f32; (width * height) as usize];
    let mut glyph_alpha = vec![0.0_f32; (width * height) as usize];
    let mut glyph_lum = vec![0.0_f32; (width * height) as usize];
    for y in gy0..gy1 {
        for x in gx0..gx1 {
            let point = Point {
                x: x as f32 + 0.5,
                y: y as f32 + 0.5,
            };
            let dist = distance_to_path(point, &path);
            let signed = if point_in_polygon(point, &path) {
                -dist
            } else {
                dist
            };
            let index = (y * width + x) as usize;
            glyph_sdf[index] = (signed / SDF_RANGE_TEXELS + 0.5).clamp(0.0, 1.0);
            // Coverage = fill plus the ring out to `stroke_extent` (used as the
            // smoke seed and the straight-alpha A channel; the shader gets its
            // crisp coverage from the SDF instead).
            glyph_alpha[index] = (0.5 + stroke_extent - signed).clamp(0.0, 1.0);
            glyph_lum[index] = if signed > 0.0 { 1.0 } else { 0.0 };
        }
    }

    // Smoke anchor field (G channel): 1 on/inside the glyph, falling to 0 over
    // the `margin` band outward. This is the cursor's analogue of the screen's
    // edge distance — the shader uses it to billow border-style smoke off the
    // glyph silhouette instead of ringing a point (which always reads as a disc).
    let smoke_anchor = cursor_smoke_anchor(&glyph_alpha, width, height, margin);

    // Pack: R = signed distance field (shader arrow), G = smoke anchor (shader
    // smoke), B = stepped luminance (CPU blit gray), A = coverage (CPU blit alpha
    // / transparency). The shader reconstructs the arrow purely from R + fwidth,
    // so no pre-blurred shadow/glow layers are baked in.
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for index in 0..(width * height) as usize {
        let offset = index * 4;
        rgba[offset] = float_to_u8(glyph_sdf[index] * 255.0);
        rgba[offset + 1] = float_to_u8(smoke_anchor[index] * 255.0);
        rgba[offset + 2] = float_to_u8(glyph_lum[index] * 255.0);
        rgba[offset + 3] = float_to_u8(glyph_alpha[index] * 255.0);
    }
    rgba
}

/// Distance-anchor field for the cursor smoke: `1` on (and inside) the glyph
/// silhouette, ramping to `0` at `reach` pixels outward. Mirrors the screen
/// `edge_distance` so `cursor_smoke` can reuse the `edge_glow` recipe with the
/// glyph silhouette as the anchor instead of the screen border.
///
/// Built with a two-pass chamfer distance transform seeded on the glyph
/// coverage — O(width*height) with a tiny constant, vs. a per-pixel
/// distance-to-path scan that dominated `CursorImage::load` and overran the
/// host's startup budget.
fn cursor_smoke_anchor(glyph_alpha: &[f32], width: u32, height: u32, reach: f32) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    let inf = f32::MAX / 4.0;
    let mut dist = vec![inf; w * h];
    for (cell, &cov) in dist.iter_mut().zip(glyph_alpha.iter()) {
        if cov > 0.5 {
            *cell = 0.0; // seed: the covered glyph (fill + outline)
        }
    }
    let d1 = 1.0_f32;
    let d2 = std::f32::consts::SQRT_2;
    // Forward pass: propagate from the top-left neighborhood.
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut v = dist[i];
            if x > 0 {
                v = v.min(dist[i - 1] + d1);
            }
            if y > 0 {
                v = v.min(dist[i - w] + d1);
                if x > 0 {
                    v = v.min(dist[i - w - 1] + d2);
                }
                if x + 1 < w {
                    v = v.min(dist[i - w + 1] + d2);
                }
            }
            dist[i] = v;
        }
    }
    // Backward pass: propagate from the bottom-right neighborhood.
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut v = dist[i];
            if x + 1 < w {
                v = v.min(dist[i + 1] + d1);
            }
            if y + 1 < h {
                v = v.min(dist[i + w] + d1);
                if x + 1 < w {
                    v = v.min(dist[i + w + 1] + d2);
                }
                if x > 0 {
                    v = v.min(dist[i + w - 1] + d2);
                }
            }
            dist[i] = v;
        }
    }
    let reach = reach.max(1.0);
    dist.iter()
        .map(|&d| (1.0 - d / reach).clamp(0.0, 1.0))
        .collect()
}

/// Chaikin corner-cutting on a CLOSED polygon: each iteration replaces every
/// vertex with two points at 1/4 and 3/4 along its outgoing edge, rounding the
/// corners. More iterations -> rounder. Used to soften the cursor glyph.
fn chaikin_round(points: &[Point], iterations: u32) -> Vec<Point> {
    let mut pts = points.to_vec();
    for _ in 0..iterations {
        let n = pts.len();
        if n < 3 {
            break;
        }
        let mut next = Vec::with_capacity(n * 2);
        for i in 0..n {
            let p = pts[i];
            let q = pts[(i + 1) % n];
            next.push(Point {
                x: 0.75 * p.x + 0.25 * q.x,
                y: 0.75 * p.y + 0.25 * q.y,
            });
            next.push(Point {
                x: 0.25 * p.x + 0.75 * q.x,
                y: 0.25 * p.y + 0.75 * q.y,
            });
        }
        pts = next;
    }
    pts
}

fn flattened_cursor_path(scale_x: f32, scale_y: f32, offset_x: f32, offset_y: f32) -> Vec<Point> {
    let mut points = Vec::new();
    let mut current = Point { x: 0.0, y: 0.0 };
    for command in CURSOR_PATH {
        match *command {
            PathCommand::Move(point) => {
                current = scale_point(point, scale_x, scale_y, offset_x, offset_y);
                points.push(current);
            }
            PathCommand::Line(point) => {
                current = scale_point(point, scale_x, scale_y, offset_x, offset_y);
                points.push(current);
            }
            PathCommand::Quad(control, end) => {
                let start = current;
                let control = scale_point(control, scale_x, scale_y, offset_x, offset_y);
                let end = scale_point(end, scale_x, scale_y, offset_x, offset_y);
                for step in 1..=12 {
                    let t = step as f32 / 12.0;
                    points.push(quadratic_point(start, control, end, t));
                }
                current = end;
            }
        }
    }
    points
}

fn scale_point(point: Point, scale_x: f32, scale_y: f32, offset_x: f32, offset_y: f32) -> Point {
    Point {
        x: point.x * scale_x + offset_x,
        y: point.y * scale_y + offset_y,
    }
}

fn quadratic_point(start: Point, control: Point, end: Point, t: f32) -> Point {
    let mt = 1.0 - t;
    Point {
        x: mt * mt * start.x + 2.0 * mt * t * control.x + t * t * end.x,
        y: mt * mt * start.y + 2.0 * mt * t * control.y + t * t * end.y,
    }
}

fn point_in_polygon(point: Point, polygon: &[Point]) -> bool {
    let mut inside = false;
    for index in 0..polygon.len() {
        let a = polygon[index];
        let b = polygon[(index + 1) % polygon.len()];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}

fn distance_to_path(point: Point, path: &[Point]) -> f32 {
    let mut best = f32::MAX;
    for index in 0..path.len() {
        best = best.min(distance_to_segment(
            point,
            path[index],
            path[(index + 1) % path.len()],
        ));
    }
    best
}

fn distance_to_segment(point: Point, a: Point, b: Point) -> f32 {
    let ab_x = b.x - a.x;
    let ab_y = b.y - a.y;
    let denom = (ab_x * ab_x + ab_y * ab_y).max(0.0001);
    let t = (((point.x - a.x) * ab_x + (point.y - a.y) * ab_y) / denom).clamp(0.0, 1.0);
    let x = a.x + ab_x * t;
    let y = a.y + ab_y * t;
    ((point.x - x).powi(2) + (point.y - y).powi(2)).sqrt()
}

fn float_to_u8(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// CPU blit of the cursor asset into an ARGB8888 canvas.
///
/// Retained only for tests (the live SHM fallback and the playground both render
/// through the WGPU path now). It reads the glyph's stepped luminance from the B
/// channel and coverage from A; the SDF (R) and smoke anchor (G) are shader-only.
/// Allowed dead in non-test builds so the module's `warn(dead_code)` does not
/// fire on this intentionally-retained fallback.
#[allow(dead_code)]
pub fn draw_cursor_asset(
    canvas: &mut [u8],
    width: u32,
    height: u32,
    cursor: &CursorImage,
    left: i32,
    top: i32,
) {
    for source_y in 0..cursor.height {
        let dest_y = top + i32::try_from(source_y).expect("cursor source y fits i32");
        if dest_y < 0 || dest_y >= i32::try_from(height).expect("surface height fits i32") {
            continue;
        }
        for source_x in 0..cursor.width {
            let dest_x = left + i32::try_from(source_x).expect("cursor source x fits i32");
            if dest_x < 0 || dest_x >= i32::try_from(width).expect("surface width fits i32") {
                continue;
            }
            let source_offset = ((source_y * cursor.width + source_x) * 4) as usize;
            // Channels are R = signed distance field, G = smoke anchor, B =
            // stepped luminance, A = coverage. This static fallback blit only
            // needs gray (from B) and coverage (from A); the SDF and anchor are
            // shader-only.
            let gray = cursor.rgba[source_offset + 2];
            let r = gray;
            let g = gray;
            let b = gray;
            let a = cursor.rgba[source_offset + 3];
            if a == 0 {
                continue;
            };
            let dest_x = u32::try_from(dest_x).expect("nonnegative destination x");
            let dest_y = u32::try_from(dest_y).expect("nonnegative destination y");
            let offset = ((dest_y * width + dest_x) * 4) as usize;
            let r = premultiply(r, a);
            let g = premultiply(g, a);
            let b = premultiply(b, a);
            let color = ((u32::from(a)) << 24)
                | ((u32::from(r)) << 16)
                | ((u32::from(g)) << 8)
                | u32::from(b);
            canvas[offset..offset + 4].copy_from_slice(&color.to_le_bytes());
        }
    }
}

#[allow(dead_code)] // only reachable via the test-only `draw_cursor_asset`
fn premultiply(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::{CURSOR_STROKE_EDGE, CURSOR_TEXTURE_SCALE, CursorImage, cursor_asset};

    #[cfg(target_os = "linux")]
    #[test]
    fn stroke_edge_matches_shader_constant() {
        // The glyph SDF is reconstructed in the shader: the white outline ring
        // spans `0..CURSOR_STROKE_EDGE` in normalized distance, and the Rust side
        // derives `stroke_extent` (coverage seed / A channel) from the SAME
        // constant. If the WGSL literal drifts from the Rust value the outline and
        // the smoke seed disagree — guard against it.
        let needle = format!("const CURSOR_STROKE_EDGE: f32 = {CURSOR_STROKE_EDGE};");
        assert!(
            crate::renderer::shaders::EFFECT_SHADER.contains(&needle),
            "shader CURSOR_STROKE_EDGE must equal Rust {CURSOR_STROKE_EDGE}; expected line `{needle}`"
        );
    }

    #[test]
    fn cursor_image_is_supersampled_above_desktop_size() {
        let cursor = CursorImage::load().expect("load cursor");
        // The texture covers the glyph footprint PLUS the smoke margin, at the
        // supersample factor.
        assert_eq!(
            cursor.width,
            cursor_asset::AGENT_CURSOR_FOOTPRINT_WIDTH * CURSOR_TEXTURE_SCALE
        );
        assert_eq!(
            cursor.height,
            cursor_asset::AGENT_CURSOR_FOOTPRINT_HEIGHT * CURSOR_TEXTURE_SCALE
        );
        assert!(
            cursor.width > cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH * CURSOR_TEXTURE_SCALE,
            "footprint includes a smoke margin beyond the glyph"
        );
        assert_eq!(
            cursor.rgba.len() as u32,
            cursor.width * cursor.height * 4,
            "rgba buffer matches the supersampled texture dimensions"
        );
    }

    #[test]
    fn cursor_image_is_vector_rendered_with_transparent_corners() {
        let cursor = CursorImage::load().expect("load cursor");
        assert_eq!(cursor.rgba[3], 0, "the margin corner is transparent");
        // The hotspot in texture space is the footprint hotspot (glyph hotspot
        // shifted by the margin), scaled by the supersample factor.
        let hotspot_x =
            cursor_asset::AGENT_CURSOR_FOOTPRINT_HOTSPOT_X as u32 * CURSOR_TEXTURE_SCALE;
        let hotspot_y =
            cursor_asset::AGENT_CURSOR_FOOTPRINT_HOTSPOT_Y as u32 * CURSOR_TEXTURE_SCALE;
        let hotspot = ((hotspot_y * cursor.width + hotspot_x) * 4) as usize;
        assert!(
            cursor.rgba[hotspot + 3] > 0,
            "vector cursor should cover the hotspot"
        );
    }

    #[test]
    fn cursor_smoke_anchor_peaks_at_glyph_and_fades_into_margin() {
        let cursor = CursorImage::load().expect("load cursor");
        // The smoke anchor lives in the G channel: high on the glyph hotspot,
        // zero out in the far transparent corner of the margin.
        let hotspot_x =
            cursor_asset::AGENT_CURSOR_FOOTPRINT_HOTSPOT_X as u32 * CURSOR_TEXTURE_SCALE;
        let hotspot_y =
            cursor_asset::AGENT_CURSOR_FOOTPRINT_HOTSPOT_Y as u32 * CURSOR_TEXTURE_SCALE;
        let hotspot = ((hotspot_y * cursor.width + hotspot_x) * 4) as usize;
        assert!(
            cursor.rgba[hotspot + 1] > 200,
            "anchor saturates inside the glyph"
        );
        assert_eq!(cursor.rgba[1], 0, "anchor is zero in the far margin corner");
    }
}
