//! Host-owned RAII guard around a [`wgpu::Surface`] created from raw handles.
//!
//! The unsafe raw-handle lifetime is the host's responsibility: the host keeps
//! the Wayland display and `wl_surface` alive while the guard exists, and the
//! guard's drop ensures the wgpu surface is released before those native
//! objects.
use anyhow::{Context, Result};

use crate::renderer::wgpu::WgpuOverlayInstance;

/// A configured (or configurable) wgpu surface plus its current config.
#[derive(Debug)]
pub struct SurfaceGuard {
    surface: ::wgpu::Surface<'static>,
    config: Option<::wgpu::SurfaceConfiguration>,
}

impl SurfaceGuard {
    /// Create a surface from raw display/window handles.
    ///
    /// # Safety
    /// The caller must guarantee that `display` and `window` raw handles remain
    /// valid for the lifetime of the returned guard.
    pub fn from_raw_handles(
        instance: &WgpuOverlayInstance,
        display: ::wgpu::rwh::RawDisplayHandle,
        window: ::wgpu::rwh::RawWindowHandle,
    ) -> Result<Self> {
        let surface = unsafe {
            instance
                .inner()
                .create_surface_unsafe(::wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(display),
                    raw_window_handle: window,
                })
        }
        .context("failed to create wgpu surface from raw handles")?;
        Ok(Self {
            surface,
            config: None,
        })
    }

    /// Surface capabilities against the chosen adapter.
    pub fn capabilities(&self, adapter: &::wgpu::Adapter) -> ::wgpu::SurfaceCapabilities {
        self.surface.get_capabilities(adapter)
    }

    /// Configure or reconfigure the surface for the given size and adapter.
    pub fn configure(
        &mut self,
        device: &::wgpu::Device,
        adapter: &::wgpu::Adapter,
        width: u32,
        height: u32,
    ) -> ::wgpu::SurfaceConfiguration {
        let capabilities = self.capabilities(adapter);
        let format = choose_surface_format(&capabilities.formats)
            .expect("renderer initialization must reject incompatible surface formats");
        let present_mode = choose_present_mode(&capabilities.present_modes);
        let alpha_mode = choose_alpha_mode(&capabilities.alpha_modes);
        let config = ::wgpu::SurfaceConfiguration {
            usage: ::wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            view_formats: vec![format],
        };
        self.surface.configure(device, &config);
        self.config = Some(config.clone());
        config
    }

    /// Configure only if the size changed, returning the active config.
    pub fn ensure_configured(
        &mut self,
        device: &::wgpu::Device,
        adapter: &::wgpu::Adapter,
        width: u32,
        height: u32,
    ) -> ::wgpu::SurfaceConfiguration {
        if let Some(config) = self.config.as_ref()
            && config.width == width
            && config.height == height
        {
            return config.clone();
        }
        self.configure(device, adapter, width, height)
    }

    /// Acquire the next frame, handling transient surface states internally.
    pub fn acquire_frame(&mut self, device: &::wgpu::Device) -> SurfaceAcquisitionResult {
        match self.surface.get_current_texture() {
            ::wgpu::CurrentSurfaceTexture::Success(frame)
            | ::wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                SurfaceAcquisitionResult::Success(frame)
            }
            ::wgpu::CurrentSurfaceTexture::Lost | ::wgpu::CurrentSurfaceTexture::Outdated => {
                if let Some(config) = self.config.as_ref() {
                    self.surface.configure(device, config);
                }
                SurfaceAcquisitionResult::Retry("surface lost or outdated")
            }
            ::wgpu::CurrentSurfaceTexture::Timeout | ::wgpu::CurrentSurfaceTexture::Occluded => {
                SurfaceAcquisitionResult::Retry("surface timed out or is occluded")
            }
            ::wgpu::CurrentSurfaceTexture::Validation => SurfaceAcquisitionResult::Validation,
        }
    }

    /// Borrows the underlying wgpu surface.
    pub fn surface(&self) -> &::wgpu::Surface<'static> {
        &self.surface
    }
}

/// Result of [`SurfaceGuard::acquire_frame`].
#[derive(Debug)]
pub enum SurfaceAcquisitionResult {
    Success(::wgpu::SurfaceTexture),
    Retry(&'static str),
    Validation,
}

pub(crate) fn choose_surface_format(
    formats: &[::wgpu::TextureFormat],
) -> Option<::wgpu::TextureFormat> {
    formats
        .iter()
        .copied()
        .find(|format| *format == ::wgpu::TextureFormat::Bgra8Unorm)
        .or_else(|| {
            formats
                .iter()
                .copied()
                .find(|format| *format == ::wgpu::TextureFormat::Rgba8Unorm)
        })
}

fn choose_present_mode(present_modes: &[::wgpu::PresentMode]) -> ::wgpu::PresentMode {
    if present_modes.contains(&::wgpu::PresentMode::Mailbox) {
        ::wgpu::PresentMode::Mailbox
    } else {
        ::wgpu::PresentMode::Fifo
    }
}

fn choose_alpha_mode(alpha_modes: &[::wgpu::CompositeAlphaMode]) -> ::wgpu::CompositeAlphaMode {
    if alpha_modes.contains(&::wgpu::CompositeAlphaMode::PreMultiplied) {
        ::wgpu::CompositeAlphaMode::PreMultiplied
    } else if alpha_modes.contains(&::wgpu::CompositeAlphaMode::Auto) {
        ::wgpu::CompositeAlphaMode::Auto
    } else {
        // The renderer requires transparent compositing (enforced by the init
        // gate in `WgpuOverlayRenderer`), so never fall back to `Opaque`. Pick
        // the first transparent mode the surface advertises rather than the
        // first mode overall, which could be `Opaque` and silently defeat the
        // transparency guarantee.
        alpha_modes
            .iter()
            .copied()
            .find(|mode| *mode != ::wgpu::CompositeAlphaMode::Opaque)
            .unwrap_or(::wgpu::CompositeAlphaMode::Auto)
    }
}

pub(crate) fn supports_transparent_alpha_mode(alpha_modes: &[::wgpu::CompositeAlphaMode]) -> bool {
    alpha_modes
        .iter()
        .any(|mode| *mode != ::wgpu::CompositeAlphaMode::Opaque)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_choice_prefers_non_srgb_bgra_for_premultiplied_output() {
        assert_eq!(
            choose_surface_format(&[
                ::wgpu::TextureFormat::Rgba8Unorm,
                ::wgpu::TextureFormat::Bgra8UnormSrgb,
                ::wgpu::TextureFormat::Bgra8Unorm,
                ::wgpu::TextureFormat::Rgba8UnormSrgb,
            ]),
            Some(::wgpu::TextureFormat::Bgra8Unorm)
        );
    }

    #[test]
    fn format_choice_uses_non_srgb_rgba_before_srgb_formats() {
        assert_eq!(
            choose_surface_format(&[
                ::wgpu::TextureFormat::Bgra8UnormSrgb,
                ::wgpu::TextureFormat::Rgba8Unorm,
            ]),
            Some(::wgpu::TextureFormat::Rgba8Unorm)
        );
    }

    #[test]
    fn format_choice_rejects_formats_outside_display_space_contract() {
        assert_eq!(
            choose_surface_format(&[
                ::wgpu::TextureFormat::Bgra8UnormSrgb,
                ::wgpu::TextureFormat::Rgba16Float,
                ::wgpu::TextureFormat::Rgb10a2Unorm,
            ]),
            None
        );
    }

    #[test]
    fn present_mode_prefers_mailbox() {
        assert_eq!(
            choose_present_mode(&[::wgpu::PresentMode::Fifo, ::wgpu::PresentMode::Mailbox]),
            ::wgpu::PresentMode::Mailbox
        );
    }

    #[test]
    fn alpha_mode_prefers_premultiplied() {
        assert_eq!(
            choose_alpha_mode(&[
                ::wgpu::CompositeAlphaMode::Opaque,
                ::wgpu::CompositeAlphaMode::PreMultiplied,
            ]),
            ::wgpu::CompositeAlphaMode::PreMultiplied
        );
    }

    #[test]
    fn alpha_mode_does_not_choose_opaque_when_transparent_modes_exist() {
        assert_ne!(
            choose_alpha_mode(&[
                ::wgpu::CompositeAlphaMode::Auto,
                ::wgpu::CompositeAlphaMode::Opaque,
            ]),
            ::wgpu::CompositeAlphaMode::Opaque
        );
    }

    #[test]
    fn alpha_mode_skips_opaque_when_only_postmultiplied_is_transparent() {
        // Neither `PreMultiplied` nor `Auto` is advertised, so selection falls
        // through to the fallback branch. It must still pick the transparent
        // mode rather than the first (opaque) entry.
        assert_eq!(
            choose_alpha_mode(&[
                ::wgpu::CompositeAlphaMode::Opaque,
                ::wgpu::CompositeAlphaMode::PostMultiplied,
            ]),
            ::wgpu::CompositeAlphaMode::PostMultiplied
        );
    }

    #[test]
    fn transparent_alpha_support_rejects_opaque_only_surfaces() {
        assert!(!supports_transparent_alpha_mode(&[
            ::wgpu::CompositeAlphaMode::Opaque
        ]));
        assert!(supports_transparent_alpha_mode(&[
            ::wgpu::CompositeAlphaMode::Opaque,
            ::wgpu::CompositeAlphaMode::PreMultiplied,
        ]));
    }
}
