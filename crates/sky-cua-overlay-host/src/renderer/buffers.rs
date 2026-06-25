//! Vertex and buffer helpers for the GPU-rendered cursor/effect scene.

use crate::cursor_asset;
use crate::renderer::scene::{CursorPoint, EffectScene};
use sky_cua_platform::{model::AgentOverlayGestureKind, overlay_spec};

pub const MAX_EFFECT_POINTS: usize = overlay_spec::shared::effects::MAX_GESTURE_POINTS as usize;

/// A single cursor vertex: 2D normalized-device-coordinate position plus UV.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
}

/// Six-vertex triangle list covering the cursor sprite.
pub type CursorQuad = [CursorVertex; 6];

const _: () = assert!(std::mem::size_of::<CursorVertex>() == 16);
const _: () = assert!(std::mem::align_of::<CursorVertex>() == 4);

/// Uniform consumed by `AgentEffectUniform` in WGSL.
///
/// WGSL uniform layout aligns every `vec2<f32>` on 8 bytes and every
/// `vec4<f32>` on 16 bytes. This struct uses only `[f32; 4]` lanes so each
/// field starts on a 16-byte boundary and the Rust byte layout is direct.
/// Color channels are authored as sRGB 0..255 values in TOML, normalized into
/// 0..1 here, and blended as premultiplied values in the shader.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgentEffectUniform {
    pub surface_size_px: [f32; 4],
    pub cursor: [f32; 4],
    pub cursor_metrics: [f32; 4],
    pub timing: [f32; 4],
    pub effect: [f32; 4],
    pub color_agent_pink: [f32; 4],
    pub color_agent_pink_light: [f32; 4],
    pub color_halo_inner: [f32; 4],
    pub glow: [f32; 4],
    pub wave: [f32; 4],
    pub halo: [f32; 4],
    pub ripple: [f32; 4],
    pub trail: [f32; 4],
    pub no_no: [f32; 4],
    pub flags: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<AgentEffectUniform>() == 240);
const _: () = assert!(std::mem::align_of::<AgentEffectUniform>() == 4);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgentEffectPoint {
    pub xy: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<AgentEffectPoint>() == 16);
const _: () = assert!(std::mem::align_of::<AgentEffectPoint>() == 4);

pub type AgentEffectPointBuffer = [AgentEffectPoint; MAX_EFFECT_POINTS];

#[derive(Debug, Clone, Copy)]
pub struct EffectUniformInput<'a> {
    pub width: u32,
    pub height: u32,
    pub now_ms: u64,
    pub cursor: Option<CursorPoint>,
    pub effect: Option<&'a EffectScene>,
}

#[must_use]
pub fn build_effect_uniform(
    input: EffectUniformInput<'_>,
) -> (AgentEffectUniform, AgentEffectPointBuffer) {
    let mut points = [AgentEffectPoint { xy: [0.0; 4] }; MAX_EFFECT_POINTS];
    let mut effect_kind = 0_u32;
    let mut point_count = 0_u32;
    let mut effect_elapsed_ms = 0.0_f32;
    let mut effect_duration_ms = 1.0_f32;

    if let Some(effect) = input.effect {
        effect_kind = effect_kind_code(effect.kind);
        point_count = effect.points.len().min(MAX_EFFECT_POINTS) as u32;
        effect_elapsed_ms = input.now_ms.saturating_sub(effect.started_at_ms) as f32;
        effect_duration_ms = effect.duration_ms.max(1) as f32;
        for (slot, point) in points.iter_mut().zip(effect.points.iter()) {
            slot.xy = [point.x as f32, point.y as f32, 0.0, 0.0];
        }
    }

    let cursor = input.cursor.unwrap_or_else(|| {
        input
            .effect
            .and_then(|effect| effect.points.first().copied())
            .unwrap_or(CursorPoint { x: 0.0, y: 0.0 })
    });
    let cursor_visible = u32::from(input.cursor.is_some() || input.effect.is_some());

    let uniform = AgentEffectUniform {
        surface_size_px: [
            input.width.max(1) as f32,
            input.height.max(1) as f32,
            0.0,
            0.0,
        ],
        cursor: [cursor.x as f32, cursor.y as f32, 0.0, 0.0],
        cursor_metrics: [
            cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH as f32,
            cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT as f32,
            cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X as f32,
            cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y as f32,
        ],
        timing: [
            (input.now_ms % overlay_spec::shared::timing::WAVE_PERIOD_MS.max(1)) as f32,
            overlay_spec::shared::timing::BREATHE_PERIOD_MS as f32,
            overlay_spec::shared::timing::WAVE_PERIOD_MS as f32,
            overlay_spec::shared::timing::HALO_BREATHE_PERIOD_MS as f32,
        ],
        effect: [
            effect_elapsed_ms,
            effect_duration_ms,
            overlay_spec::shared::timing::RIPPLE_BURST_MS as f32,
            point_count as f32,
        ],
        color_agent_pink: [
            normalize_u8(overlay_spec::shared::colors::AGENT_PINK_RED_0_255),
            normalize_u8(overlay_spec::shared::colors::AGENT_PINK_GREEN_0_255),
            normalize_u8(overlay_spec::shared::colors::AGENT_PINK_BLUE_0_255),
            overlay_spec::shared::effects::GLOW_BASELINE_MIN_ALPHA_0_1 as f32,
        ],
        color_agent_pink_light: [
            normalize_u8(overlay_spec::shared::colors::AGENT_PINK_LIGHT_RED_0_255),
            normalize_u8(overlay_spec::shared::colors::AGENT_PINK_LIGHT_GREEN_0_255),
            normalize_u8(overlay_spec::shared::colors::AGENT_PINK_LIGHT_BLUE_0_255),
            overlay_spec::shared::effects::GLOW_BASELINE_MAX_ALPHA_0_1 as f32,
        ],
        color_halo_inner: [
            normalize_u8(overlay_spec::shared::colors::HALO_INNER_RED_0_255),
            normalize_u8(overlay_spec::shared::colors::HALO_INNER_GREEN_0_255),
            normalize_u8(overlay_spec::shared::colors::HALO_INNER_BLUE_0_255),
            normalize_u8(overlay_spec::shared::colors::HALO_INNER_ALPHA_0_255),
        ],
        glow: [
            overlay_spec::desktop::geometry::GLOW_BASE_STROKE_LOGICAL_PX as f32,
            overlay_spec::desktop::geometry::GLOW_BASE_BLUR_LOGICAL_PX as f32,
            overlay_spec::desktop::geometry::GLOW_CORE_STROKE_LOGICAL_PX as f32,
            overlay_spec::desktop::geometry::GLOW_CORE_BLUR_LOGICAL_PX as f32,
        ],
        wave: [
            overlay_spec::desktop::geometry::WAVE_STROKE_LOGICAL_PX as f32,
            overlay_spec::desktop::geometry::WAVE_BLUR_LOGICAL_PX as f32,
            overlay_spec::desktop::rendering::WAVE_TRAVEL_FRACTION as f32,
            overlay_spec::desktop::rendering::WAVE_MAX_ALPHA_0_255 as f32 / 255.0,
        ],
        halo: [
            overlay_spec::desktop::geometry::CURSOR_HALO_RADIUS_LOGICAL_PX as f32,
            overlay_spec::desktop::rendering::HALO_SCALE_MIN_FRACTION as f32,
            overlay_spec::desktop::rendering::HALO_SCALE_MAX_FRACTION as f32,
            overlay_spec::desktop::rendering::HALO_ALPHA_MAX_FRACTION as f32,
        ],
        ripple: [
            overlay_spec::desktop::geometry::RIPPLE_MIN_LOGICAL_PX as f32,
            overlay_spec::desktop::geometry::RIPPLE_MAX_LOGICAL_PX as f32,
            overlay_spec::desktop::geometry::RIPPLE_STROKE_LOGICAL_PX as f32,
            overlay_spec::desktop::rendering::MAX_RIPPLE_ALPHA_0_255 as f32 / 255.0,
        ],
        trail: [
            overlay_spec::desktop::geometry::TRAIL_STROKE_LOGICAL_PX as f32,
            overlay_spec::desktop::rendering::MAX_TRAIL_ALPHA_0_255 as f32 / 255.0,
            overlay_spec::shared::motion::CURSOR_NOSE_DEG as f32,
            overlay_spec::shared::effects::CURSOR_PRESS_SCALE_FRACTION as f32,
        ],
        no_no: [
            overlay_spec::shared::effects::NO_NO_WIGGLE_DEG as f32,
            overlay_spec::shared::effects::NO_NO_SHAKES_FRACTION as f32,
            overlay_spec::shared::effects::NO_NO_HOLD_FRACTION as f32,
            overlay_spec::shared::effects::PRESS_IN_FRACTION as f32,
        ],
        flags: [cursor_visible, effect_kind, point_count, 0],
    };
    (uniform, points)
}

#[must_use]
pub fn effect_kind_code(kind: AgentOverlayGestureKind) -> u32 {
    match kind {
        AgentOverlayGestureKind::Tap => 1,
        AgentOverlayGestureKind::Drag => 2,
        AgentOverlayGestureKind::Swipe => 3,
        AgentOverlayGestureKind::NoNo => 4,
    }
}

/// Build a flat `[f32; 24]` vertex buffer for a cursor hotspot at `(x, y)` in
/// surface-local pixels, converted to NDC for the given surface size.
pub fn cursor_quad_vertices(x: f64, y: f64, surface_width: u32, surface_height: u32) -> [f32; 24] {
    let left = x - f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X);
    let top = y - f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y);
    let right = left + f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH);
    let bottom = top + f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT);
    let left = ndc_x(left, surface_width);
    let right = ndc_x(right, surface_width);
    let top = ndc_y(top, surface_height);
    let bottom = ndc_y(bottom, surface_height);
    [
        left, top, 0.0, 0.0, right, top, 1.0, 0.0, right, bottom, 1.0, 1.0, left, top, 0.0, 0.0,
        right, bottom, 1.0, 1.0, left, bottom, 0.0, 1.0,
    ]
}

/// Layout description for uploading [`CursorVertex`] to wgpu.
pub fn cursor_vertex_buffer_layout() -> ::wgpu::VertexBufferLayout<'static> {
    ::wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<CursorVertex>() as ::wgpu::BufferAddress,
        step_mode: ::wgpu::VertexStepMode::Vertex,
        attributes: &[
            ::wgpu::VertexAttribute {
                format: ::wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            },
            ::wgpu::VertexAttribute {
                format: ::wgpu::VertexFormat::Float32x2,
                offset: (2 * std::mem::size_of::<f32>()) as ::wgpu::BufferAddress,
                shader_location: 1,
            },
        ],
    }
}

/// Reinterpret an `&[f32]` as `&[u8]` for `queue.write_buffer`.
///
/// # Safety
/// `f32` is safe to transmute to bytes, and the slice length is a multiple of
/// four. The returned slice borrows the input and is valid for its lifetime.
pub fn f32_slice_as_bytes(values: &[f32]) -> &[u8] {
    let byte_len = std::mem::size_of_val(values);
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), byte_len) }
}

pub fn effect_uniform_as_bytes(value: &AgentEffectUniform) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref(value).cast::<u8>(),
            std::mem::size_of::<AgentEffectUniform>(),
        )
    }
}

pub fn effect_points_as_bytes(values: &AgentEffectPointBuffer) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

fn normalize_u8(value: u8) -> f32 {
    f32::from(value) / 255.0
}

fn ndc_x(x: f64, width: u32) -> f32 {
    ((x / f64::from(width.max(1))) * 2.0 - 1.0) as f32
}

fn ndc_y(y: f64, height: u32) -> f32 {
    (1.0 - (y / f64::from(height.max(1))) * 2.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::AgentOverlayGestureKind;

    #[test]
    fn cursor_vertex_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<CursorVertex>(), 16);
        assert_eq!(std::mem::align_of::<CursorVertex>(), 4);
        assert_eq!(std::mem::offset_of!(CursorVertex, position), 0);
        assert_eq!(std::mem::offset_of!(CursorVertex, uv), 8);
    }

    #[test]
    fn cursor_quad_contains_twenty_four_floats() {
        let quad = cursor_quad_vertices(100.0, 100.0, 1920, 1080);
        assert_eq!(quad.len(), 24);
    }

    #[test]
    fn f32_bytes_are_four_byte_length() {
        assert_eq!(f32_slice_as_bytes(&[1.0f32, 2.0]).len(), 8);
    }

    #[test]
    fn effect_uniform_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<AgentEffectUniform>(), 240);
        assert_eq!(std::mem::align_of::<AgentEffectUniform>(), 4);
        assert_eq!(std::mem::offset_of!(AgentEffectUniform, surface_size_px), 0);
        assert_eq!(std::mem::offset_of!(AgentEffectUniform, flags), 224);
        assert_eq!(std::mem::size_of::<AgentEffectPoint>(), 16);
        assert_eq!(
            std::mem::size_of::<AgentEffectPointBuffer>(),
            16 * MAX_EFFECT_POINTS
        );
    }

    #[test]
    fn effect_uniform_uses_spec_colors_and_points() {
        let effect = EffectScene {
            kind: AgentOverlayGestureKind::Swipe,
            started_at_ms: 100,
            duration_ms: 500,
            points: vec![
                CursorPoint { x: 10.0, y: 20.0 },
                CursorPoint { x: 30.0, y: 40.0 },
            ],
        };
        let (uniform, points) = build_effect_uniform(EffectUniformInput {
            width: 100,
            height: 200,
            now_ms: 220,
            cursor: None,
            effect: Some(&effect),
        });

        assert_eq!(
            uniform.flags,
            [1, effect_kind_code(AgentOverlayGestureKind::Swipe), 2, 0]
        );
        assert_eq!(uniform.effect[0], 120.0);
        assert_eq!(uniform.cursor[0], 10.0);
        assert_eq!(uniform.color_agent_pink[0], 1.0);
        assert!((uniform.color_agent_pink[1] - (96.0 / 255.0)).abs() < f32::EPSILON);
        assert_eq!(points[1].xy, [30.0, 40.0, 0.0, 0.0]);
        assert_eq!(effect_uniform_as_bytes(&uniform).len(), 240);
        assert_eq!(
            effect_points_as_bytes(&points).len(),
            16 * MAX_EFFECT_POINTS
        );
    }

    #[test]
    fn cursor_quad_corners_span_expected_ndc() {
        let quad = cursor_quad_vertices(
            f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_X),
            f64::from(cursor_asset::AGENT_CURSOR_DESKTOP_HOTSPOT_Y),
            cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH,
            cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT,
        );
        // When the hotspot is placed at the top-left of the surface, the
        // cursor sprite should fill the NDC square [-1, 1] x [-1, 1].
        // Each vertex is four f32s: position x,y then uv u,v.
        let left_top_x = quad[0];
        let left_top_y = quad[1];
        assert!((left_top_x - -1.0).abs() < f32::EPSILON);
        assert!((left_top_y - 1.0).abs() < f32::EPSILON);
        // Third vertex (index 8..12) is right,bottom.
        let right_bottom_x = quad[8];
        let right_bottom_y = quad[9];
        assert!((right_bottom_x - 1.0).abs() < f32::EPSILON);
        assert!((right_bottom_y - -1.0).abs() < f32::EPSILON);
    }
}
