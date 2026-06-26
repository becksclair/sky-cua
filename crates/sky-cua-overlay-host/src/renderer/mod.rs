#![allow(dead_code)]

//! Platform-agnostic WGPU renderer for the desktop agent cursor overlay.
//!
//! This module intentionally imports no Wayland, X11, GNOME Shell, D-Bus, or
//! service types. The host (`layer_shell.rs`) owns native surface lifetime and
//! passes raw display/window handles into [`SurfaceGuard`], which wraps the
//! unsafe wgpu surface creation. The renderer borrows those guards each frame
//! and is responsible only for adapter selection, device setup, cursor texture
//! management, shader/pipeline state, and per-surface draw submission.

#[cfg(target_os = "linux")]
pub mod animation;
#[cfg(target_os = "linux")]
pub mod buffers;
#[cfg(target_os = "linux")]
pub mod scene;
#[cfg(target_os = "linux")]
pub mod shaders;
#[cfg(target_os = "linux")]
pub mod surface;
#[cfg(target_os = "linux")]
pub mod wgpu;

#[cfg(target_os = "linux")]
pub use scene::{CursorPoint, EffectScene, SurfaceDrawRequest, SurfaceDrawSpec};
#[cfg(target_os = "linux")]
pub use surface::SurfaceGuard;
#[cfg(target_os = "linux")]
pub use wgpu::{WgpuOverlayInstance, WgpuOverlayRenderer};

use crate::cursor_asset;
use anyhow::Result;

/// Decoded RGBA cursor image used by both the WGPU renderer and the CPU
/// fallbacks in `layer_shell`/`playground`.
#[derive(Debug)]
pub struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CursorImage {
    pub fn load() -> Result<Self> {
        let image = render_vector_cursor(
            cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH,
            cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT,
        );
        Ok(Self {
            width: cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH,
            height: cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT,
            rgba: image,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}

const VECTOR_STROKE_WIDTH: f32 = 3.3;
const SUPERSAMPLE: u32 = 4;

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
    let scale_x = width as f32 / cursor_asset::AGENT_CURSOR_SOURCE_WIDTH as f32;
    let scale_y = height as f32 / cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT as f32;
    let path = flattened_cursor_path(scale_x, scale_y);
    let stroke_radius = VECTOR_STROKE_WIDTH * scale_x.min(scale_y) * 0.5;
    let mut glyph_alpha = vec![0.0_f32; (width * height) as usize];
    let sample_count = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    for y in 0..height {
        for x in 0..width {
            let mut alpha = 0.0_f32;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let px = x as f32 + (sx as f32 + 0.5) / SUPERSAMPLE as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SUPERSAMPLE as f32;
                    let point = Point { x: px, y: py };
                    if point_in_polygon(point, &path)
                        || distance_to_path(point, &path) <= stroke_radius
                    {
                        alpha += 1.0;
                    }
                }
            }
            glyph_alpha[(y * width + x) as usize] = alpha / sample_count;
        }
    }

    let shadow = blurred_shadow(&glyph_alpha, width, height);
    let mut rgba = vec![0_u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let offset = index * 4;
            let shadow_alpha = shadow[index] * overlay_spec_shadow_alpha();
            let glyph = glyph_alpha[index];
            let stroke = distance_to_path(
                Point {
                    x: x as f32 + 0.5,
                    y: y as f32 + 0.5,
                },
                &path,
            ) <= stroke_radius;
            let glyph_r = if stroke { 255.0 } else { 0.0 };
            let glyph_g = glyph_r;
            let glyph_b = glyph_r;
            let out_alpha = glyph + shadow_alpha * (1.0 - glyph);
            if out_alpha <= 0.0 {
                continue;
            }
            let out_r = glyph_r * glyph / out_alpha;
            let out_g = glyph_g * glyph / out_alpha;
            let out_b = glyph_b * glyph / out_alpha;
            rgba[offset] = float_to_u8(out_r);
            rgba[offset + 1] = float_to_u8(out_g);
            rgba[offset + 2] = float_to_u8(out_b);
            rgba[offset + 3] = float_to_u8(out_alpha * 255.0);
        }
    }
    rgba
}

fn flattened_cursor_path(scale_x: f32, scale_y: f32) -> Vec<Point> {
    let mut points = Vec::new();
    let mut current = Point { x: 0.0, y: 0.0 };
    for command in CURSOR_PATH {
        match *command {
            PathCommand::Move(point) => {
                current = scale_point(point, scale_x, scale_y);
                points.push(current);
            }
            PathCommand::Line(point) => {
                current = scale_point(point, scale_x, scale_y);
                points.push(current);
            }
            PathCommand::Quad(control, end) => {
                let start = current;
                let control = scale_point(control, scale_x, scale_y);
                let end = scale_point(end, scale_x, scale_y);
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

fn scale_point(point: Point, scale_x: f32, scale_y: f32) -> Point {
    Point {
        x: point.x * scale_x,
        y: point.y * scale_y,
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

fn blurred_shadow(alpha: &[f32], width: u32, height: u32) -> Vec<f32> {
    let dx = sky_cua_platform::overlay_spec::desktop::rendering::SHADOW_DX_VIEWBOX_FRACTION as f32
        * height as f32
        / sky_cua_platform::overlay_spec::desktop::rendering::VIEWBOX_HEIGHT as f32;
    let dy = sky_cua_platform::overlay_spec::desktop::rendering::SHADOW_DY_VIEWBOX_FRACTION as f32
        * height as f32
        / sky_cua_platform::overlay_spec::desktop::rendering::VIEWBOX_HEIGHT as f32;
    let radius = (sky_cua_platform::overlay_spec::desktop::rendering::SHADOW_BLUR_VIEWBOX_FRACTION
        as f32
        * height as f32
        / sky_cua_platform::overlay_spec::desktop::rendering::VIEWBOX_HEIGHT as f32)
        .ceil() as i32;
    let mut shadow = vec![0.0_f32; alpha.len()];
    for y in 0..height {
        for x in 0..width {
            let src_x = x as f32 - dx;
            let src_y = y as f32 - dy;
            let mut total = 0.0;
            let mut weight = 0.0;
            for oy in -radius..=radius {
                for ox in -radius..=radius {
                    let sx = src_x.round() as i32 + ox;
                    let sy = src_y.round() as i32 + oy;
                    if sx < 0 || sy < 0 || sx >= width as i32 || sy >= height as i32 {
                        continue;
                    }
                    let dist2 = (ox * ox + oy * oy) as f32;
                    let w = 1.0 / (1.0 + dist2);
                    total += alpha[(sy as u32 * width + sx as u32) as usize] * w;
                    weight += w;
                }
            }
            if weight > 0.0 {
                shadow[(y * width + x) as usize] = total / weight;
            }
        }
    }
    shadow
}

fn overlay_spec_shadow_alpha() -> f32 {
    sky_cua_platform::overlay_spec::desktop::rendering::SHADOW_ALPHA_0_1 as f32
}

fn float_to_u8(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// CPU blit of the cursor asset into an ARGB8888 canvas.
///
/// This is used by the SHM fallback and the playground preview; the WGPU
/// renderer uploads the cursor texture directly and does not need this path.
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
            let r = cursor.rgba[source_offset];
            let g = cursor.rgba[source_offset + 1];
            let b = cursor.rgba[source_offset + 2];
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

fn premultiply(channel: u8, alpha: u8) -> u8 {
    ((u16::from(channel) * u16::from(alpha) + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::{CursorImage, cursor_asset};

    #[test]
    fn cursor_image_load_matches_desktop_size() {
        let cursor = CursorImage::load().expect("load cursor");
        assert_eq!(cursor.width, cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH);
        assert_eq!(cursor.height, cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT);
        assert!(!cursor.rgba.is_empty());
    }

    #[test]
    fn cursor_image_is_vector_rendered_with_transparent_corners() {
        let cursor = CursorImage::load().expect("load cursor");
        assert_eq!(cursor.rgba[3], 0);
        let hotspot = ((cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y as u32 * cursor.width
            + cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X as u32)
            * 4) as usize;
        assert!(
            cursor.rgba[hotspot + 3] > 0,
            "vector cursor should cover the hotspot"
        );
    }
}
