#![allow(dead_code)]

//! Platform-agnostic WGPU renderer for the desktop agent cursor overlay.
//!
//! This module intentionally imports no Wayland, X11, GNOME Shell, D-Bus, or
//! service types. The host (`layer_shell.rs`) owns native surface lifetime and
//! passes raw display/window handles into [`SurfaceGuard`], which wraps the
//! unsafe wgpu surface creation. The renderer borrows those guards each frame
//! and is responsible only for adapter selection, device setup, cursor texture
//! management, shader/pipeline state, and per-surface draw submission.
//!
//! The CPU-side cursor texture synthesis (signed distance field, smoke anchor,
//! mip chain) lives in [`cursor_texture`]; this file is the module-wiring and
//! re-export surface. The blanket `allow(dead_code)` above covers the
//! test-only helpers in the wgpu submodules (`buffers`/`scene` carry
//! tested-but-otherwise-unused geometry helpers); `cursor_texture` overrides it
//! back to `warn` so its own dead code surfaces.

#[cfg(target_os = "linux")]
pub mod animation;
#[cfg(target_os = "linux")]
pub mod buffers;
mod cursor_texture;
#[cfg(target_os = "linux")]
pub mod scene;
#[cfg(target_os = "linux")]
pub mod shaders;
#[cfg(target_os = "linux")]
pub mod surface;
#[cfg(target_os = "linux")]
pub mod wgpu;

pub use cursor_texture::CursorImage;
// Test-only CPU blit fallback; re-exported only where a test consumes it.
#[cfg(test)]
pub use cursor_texture::draw_cursor_asset;
#[cfg(target_os = "linux")]
pub use scene::{CursorPoint, EffectScene, SurfaceDrawRequest, SurfaceDrawSpec};
#[cfg(target_os = "linux")]
pub use surface::SurfaceGuard;
#[cfg(target_os = "linux")]
pub use wgpu::{WgpuOverlayInstance, WgpuOverlayRenderer};
