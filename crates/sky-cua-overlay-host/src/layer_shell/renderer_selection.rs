//! Renderer selection for the layer-shell host: the `SKY_CUA_LAYER_SHELL_RENDERER`
//! request parsing, the selected-renderer state (`LayerShellRenderer`), and the
//! wgpu-or-honestly-unsupported selection logic.

use super::*;

const RENDERER_ENV: &str = "SKY_CUA_LAYER_SHELL_RENDERER";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestedLayerShellRenderer {
    Auto,
    Wgpu,
    UnsupportedLegacy(&'static str),
}
fn requested_renderer() -> RequestedLayerShellRenderer {
    match std::env::var(RENDERER_ENV)
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "shm" | "wayland_shm" | "wayland-shm" => RequestedLayerShellRenderer::UnsupportedLegacy(
            "Wayland SHM visible rendering was retired; WGPU is required",
        ),
        "wgpu" | "gpu" => RequestedLayerShellRenderer::Wgpu,
        "auto" | "" => RequestedLayerShellRenderer::Auto,
        _ => RequestedLayerShellRenderer::Auto,
    }
}
#[derive(Debug)]
// internal renderer-selection state, not a wire contract; boxing the renderer
// would just add indirection on a hot path for no real benefit here, but
// revisit if size matters
#[allow(clippy::large_enum_variant)]
pub(super) enum LayerShellRenderer {
    Wgpu(WgpuOverlayRenderer, String),
    Unsupported { reason: Option<String> },
}
impl LayerShellRenderer {
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

impl LayerShellApp {
    pub(super) fn select_renderer(&mut self, _conn: &Connection) -> Result<()> {
        match requested_renderer() {
            RequestedLayerShellRenderer::UnsupportedLegacy(reason) => {
                self.renderer = LayerShellRenderer::Unsupported {
                    reason: Some(format!("{RENDERER_ENV}: {reason}")),
                };
                self.lifecycle_state = AgentOverlayHostLifecycleState::BackendUnsupported;
                Ok(())
            }
            RequestedLayerShellRenderer::Wgpu => {
                self.ensure_wgpu_output_coverage().context(
                    "explicit wgpu layer-shell renderer failed output coverage validation",
                )?;
                let instance = self.instance.as_ref().context("wgpu instance is missing")?;
                let renderer =
                    WgpuOverlayRenderer::new(instance, &mut self.surface_guards, &self.cursor)
                        .context("explicit wgpu layer-shell renderer failed to initialize")?;
                let reason = format!(
                    "wgpu renderer active on {} via {}",
                    renderer.info().adapter_name,
                    renderer.info().backend
                );
                self.renderer = LayerShellRenderer::Wgpu(renderer, reason);
                self.lifecycle_state = AgentOverlayHostLifecycleState::BackendReady;
                Ok(())
            }
            RequestedLayerShellRenderer::Auto => {
                let wgpu_result = self.ensure_wgpu_output_coverage().and_then(|()| {
                    let instance = self.instance.as_ref().context("wgpu instance is missing")?;
                    WgpuOverlayRenderer::new(instance, &mut self.surface_guards, &self.cursor)
                });
                match wgpu_result {
                    Ok(renderer) => {
                        let reason = format!(
                            "wgpu renderer active on {} via {}",
                            renderer.info().adapter_name,
                            renderer.info().backend
                        );
                        self.renderer = LayerShellRenderer::Wgpu(renderer, reason);
                        self.lifecycle_state = AgentOverlayHostLifecycleState::BackendReady;
                    }
                    Err(error) => {
                        self.renderer = LayerShellRenderer::Unsupported {
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
    pub(super) fn renderer_kind(&self) -> AgentCursorRendererBackendKind {
        self.renderer.kind()
    }
    pub(super) fn renderer_reason(&self) -> Option<&str> {
        self.renderer.reason()
    }
    pub(super) fn adapter_name(&self) -> Option<&str> {
        self.renderer.adapter_name()
    }
}

#[cfg(test)]
mod tests {
    use super::{RequestedLayerShellRenderer, requested_renderer};

    #[test]
    fn layer_shell_renderer_env_selects_wgpu_and_rejects_legacy_shm() {
        unsafe { std::env::set_var(super::RENDERER_ENV, "wgpu") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Wgpu);
        unsafe { std::env::set_var(super::RENDERER_ENV, "shm") };
        assert!(matches!(
            requested_renderer(),
            RequestedLayerShellRenderer::UnsupportedLegacy(reason)
                if reason.contains("SHM visible rendering was retired")
        ));
        unsafe { std::env::set_var(super::RENDERER_ENV, "auto") };
        assert_eq!(requested_renderer(), RequestedLayerShellRenderer::Auto);
        unsafe { std::env::remove_var(super::RENDERER_ENV) };
    }
}
