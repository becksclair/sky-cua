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
pub use scene::{CursorPoint, SurfaceDrawRequest, SurfaceDrawSpec};
#[cfg(target_os = "linux")]
pub use surface::SurfaceGuard;
#[cfg(target_os = "linux")]
pub use wgpu::{WgpuOverlayInstance, WgpuOverlayRenderer};

use crate::cursor_asset;
use anyhow::{Context, Result, bail};
use image::imageops::FilterType;

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
        let image = image::load_from_memory(cursor_asset::AGENT_CURSOR_PNG)
            .context("failed to decode bundled agent cursor image")?
            .to_rgba8();
        let (width, height) = image.dimensions();
        if width != cursor_asset::AGENT_CURSOR_SOURCE_WIDTH
            || height != cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT
        {
            bail!(
                "bundled agent cursor image changed size: expected {}x{} got {}x{}",
                cursor_asset::AGENT_CURSOR_SOURCE_WIDTH,
                cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT,
                width,
                height
            );
        }
        let image = image::imageops::resize(
            &image,
            cursor_asset::AGENT_CURSOR_DESKTOP_WIDTH,
            cursor_asset::AGENT_CURSOR_DESKTOP_HEIGHT,
            FilterType::Lanczos3,
        );
        let (width, height) = image.dimensions();
        Ok(Self {
            width,
            height,
            rgba: image.into_raw(),
        })
    }
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
}
