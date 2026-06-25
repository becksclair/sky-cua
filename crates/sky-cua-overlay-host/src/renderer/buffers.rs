//! Vertex and buffer helpers for the static cursor quad.

use crate::cursor_asset;

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

fn ndc_x(x: f64, width: u32) -> f32 {
    ((x / f64::from(width.max(1))) * 2.0 - 1.0) as f32
}

fn ndc_y(y: f64, height: u32) -> f32 {
    (1.0 - (y / f64::from(height.max(1))) * 2.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

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
