//! Renderer selection for the surface host: the selected-renderer state and
//! the wgpu-or-honestly-unsupported selection logic. Which renderer is
//! requested comes from the platform's configuration.

use anyhow::{Context, Result};
use sky_cua_platform::model::{AgentCursorRendererBackendKind, AgentOverlayHostLifecycleState};

use super::{
    OverlaySurfaceHost,
    platform::{RendererRequest, SurfacePlatform},
};
use crate::renderer::WgpuOverlayRenderer;

#[derive(Debug)]
// internal renderer-selection state, not a wire contract; boxing the renderer
// would just add indirection on a hot path for no real benefit here, but
// revisit if size matters
#[allow(clippy::large_enum_variant)]
pub(super) enum SelectedRenderer {
    Wgpu(WgpuOverlayRenderer, String),
    Unsupported { reason: Option<String> },
}

impl SelectedRenderer {
    pub(super) fn kind(&self) -> AgentCursorRendererBackendKind {
        match self {
            Self::Wgpu(_, _) => AgentCursorRendererBackendKind::Wgpu,
            Self::Unsupported { .. } => AgentCursorRendererBackendKind::None,
        }
    }

    pub(super) fn reason(&self) -> Option<&str> {
        match self {
            Self::Wgpu(_, reason) => Some(reason.as_str()),
            Self::Unsupported { reason } => reason.as_deref(),
        }
    }

    pub(super) fn adapter_name(&self) -> Option<&str> {
        match self {
            Self::Wgpu(renderer, _) => Some(renderer.info().adapter_name.as_str()),
            Self::Unsupported { .. } => None,
        }
    }

    pub(super) fn supports_visible_overlay(&self) -> bool {
        matches!(self, Self::Wgpu(..))
    }
}

impl<P: SurfacePlatform> OverlaySurfaceHost<P> {
    pub(super) fn select_renderer(&mut self) -> Result<()> {
        match self.platform.requested_renderer() {
            RendererRequest::Unsupported(reason) => {
                self.renderer = SelectedRenderer::Unsupported {
                    reason: Some(reason),
                };
                self.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
                Ok(())
            }
            RendererRequest::Wgpu => {
                self.ensure_wgpu_output_coverage()
                    .context("explicit wgpu renderer failed output coverage validation")?;
                let renderer = WgpuOverlayRenderer::new(
                    &self.instance,
                    &mut self.surface_guards,
                    &self.cursor,
                )
                .context("explicit wgpu renderer failed to initialize")?;
                self.activate_wgpu(renderer);
                Ok(())
            }
            RendererRequest::Auto => {
                let wgpu_result = self.ensure_wgpu_output_coverage().and_then(|()| {
                    WgpuOverlayRenderer::new(&self.instance, &mut self.surface_guards, &self.cursor)
                });
                match wgpu_result {
                    Ok(renderer) => self.activate_wgpu(renderer),
                    Err(error) => {
                        self.renderer = SelectedRenderer::Unsupported {
                            reason: Some(format!(
                                "wgpu unavailable; visible overlay failed closed: {error}"
                            )),
                        };
                        self.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
                    }
                }
                Ok(())
            }
        }
    }

    fn activate_wgpu(&mut self, renderer: WgpuOverlayRenderer) {
        let reason = format!(
            "wgpu renderer active on {} via {}",
            renderer.info().adapter_name,
            renderer.info().backend
        );
        self.renderer = SelectedRenderer::Wgpu(renderer, reason);
        self.lifecycle_state = AgentOverlayHostLifecycleState::BackendReady;
    }
}
