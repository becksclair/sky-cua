//! Structured capability and coverage reporting for the layer-shell host.
//! Clients never infer backend state from prose: these fields carry the truth
//! (renderer kind, visible-overlay support, output coverage, pointer tracking,
//! system-cursor backend).

use super::*;
use crate::OVERLAY_HOST_PROTOCOL_VERSION;

impl LayerShellOverlayBackend {
    pub(super) fn capabilities(&self) -> AgentCursorCapabilities {
        layer_shell_capabilities(
            self.app.open_layer_count(),
            self.app.has_open_layer(),
            self.app.renderer_kind(),
            self.app.renderer_reason(),
            self.app.coverage_kind(),
            self.app.active_output_count(),
            self.app.rendered_output_count(),
            self.app.adapter_name(),
            self.app.last_frame_submission_us,
            self.app.frames_submitted,
            self.pointer_tracker.backend(),
            self.pointer_tracker.exact(),
            self.pointer_tracker.reason(),
            &self.system_cursor,
        )
    }
}

// This function's signature is not the ROADMAP-tracked plan_capture config-struct
// refactor; a config-struct split here is a signature refactor out of scope for
// this lint pass.
#[allow(clippy::too_many_arguments)]
fn layer_shell_capabilities(
    open_layer_count: usize,
    has_open_layer: bool,
    renderer_backend: AgentCursorRendererBackendKind,
    renderer_reason: Option<&str>,
    coverage: AgentOverlayCoverageKind,
    active_output_count: usize,
    rendered_output_count: usize,
    adapter_name: Option<&str>,
    last_frame_submission_us: Option<u128>,
    frames_submitted: u64,
    pointer_tracking_backend: AgentCursorPointerTrackingBackendKind,
    pointer_tracking_exact: bool,
    pointer_tracking_reason: Option<&str>,
    system_cursor: &SystemCursorAdapter,
) -> AgentCursorCapabilities {
    let mut reason = format!(
        "zwlr_layer_shell_v1 visible overlay active on {} output surface(s)",
        open_layer_count
    );
    if let Some(system_cursor_reason) = system_cursor.reason() {
        if system_cursor.supported() {
            reason.push_str("; system cursor: ");
        } else {
            reason.push_str("; system cursor hide unsupported: ");
        }
        reason.push_str(system_cursor_reason);
    }
    if let Some(renderer_reason) = renderer_reason {
        reason.push_str("; renderer: ");
        reason.push_str(renderer_reason);
    }
    if let Some(pointer_tracking_reason) = pointer_tracking_reason {
        reason.push_str("; pointer tracking: ");
        reason.push_str(pointer_tracking_reason);
    }
    if let Some(last_frame_submission_us) = last_frame_submission_us {
        reason.push_str(&format!(
            "; frame pacing: last_cpu_submit_us={last_frame_submission_us} frames_submitted={frames_submitted}"
        ));
    }
    let visible_overlay = has_open_layer
        && coverage == AgentOverlayCoverageKind::Full
        && renderer_backend == AgentCursorRendererBackendKind::Wgpu;
    AgentCursorCapabilities {
        backend: AgentCursorBackendKind::WaylandLayerShell,
        renderer_backend,
        visible_overlay,
        screenshot_synthetic_cursor: false,
        click_through: true,
        capture_exclusion: false,
        pointer_tracking_backend,
        pointer_tracking_exact,
        system_cursor_hide_supported: system_cursor.supported(),
        system_cursor_hidden: system_cursor.hidden(),
        system_cursor_backend: system_cursor.backend(),
        needs_user_install: false,
        reason: Some(reason),
        effects: Some(AgentOverlayEffectsCapabilities {
            glide: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            rotation: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            halo: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            ripple: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            trail: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            edge_glow: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            inward_wave: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            no_no_render: renderer_backend == AgentCursorRendererBackendKind::Wgpu,
            hit_test: false,
            sound: sky_cua_platform::overlay_spec::sound::ENABLED,
        }),
        coverage: Some(coverage),
        supported_coordinate_spaces: vec![
            CoordinateSpace::DesktopLogical,
            CoordinateSpace::StreamLogical,
            CoordinateSpace::StreamPixels,
        ],
        max_gesture_points: Some(
            sky_cua_platform::overlay_spec::shared::effects::MAX_GESTURE_POINTS,
        ),
        protocol_version: Some(OVERLAY_HOST_PROTOCOL_VERSION),
        effect_schema_version: Some(sky_cua_platform::overlay_spec::SCHEMA_VERSION),
        active_output_count: Some(active_output_count.min(u32::MAX as usize) as u32),
        rendered_output_count: Some(rendered_output_count.min(u32::MAX as usize) as u32),
        adapter_name: adapter_name.map(str::to_string),
    }
}

impl LayerShellApp {
    pub(super) fn visible_overlay_supported(&self) -> bool {
        self.renderer.supports_visible_overlay()
            && self.coverage_kind() == AgentOverlayCoverageKind::Full
    }
    pub(super) fn active_output_count(&self) -> usize {
        self.output_state
            .outputs()
            .count()
            .max(self.layers.iter().filter(|entry| !entry.closed).count())
    }
    pub(super) fn rendered_output_count(&self) -> usize {
        match self.renderer {
            LayerShellRenderer::Wgpu(..) => self
                .layers
                .iter()
                .enumerate()
                .filter(|(index, entry)| {
                    !entry.closed
                        && entry.configured
                        && self
                            .surface_guards
                            .get(*index)
                            .is_some_and(|guard| guard.is_some())
                })
                .count(),
            LayerShellRenderer::Unsupported { .. } => 0,
        }
    }
    pub(super) fn coverage_kind(&self) -> AgentOverlayCoverageKind {
        let active_outputs = self.active_output_count();
        let rendered_outputs = self.rendered_output_count();
        if active_outputs > 0 && active_outputs == rendered_outputs {
            AgentOverlayCoverageKind::Full
        } else {
            AgentOverlayCoverageKind::None
        }
    }
    pub(super) fn ensure_wgpu_output_coverage(&self) -> Result<()> {
        let active_outputs = self.active_output_count();
        let rendered_outputs = self
            .layers
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                !entry.closed
                    && entry.configured
                    && self
                        .surface_guards
                        .get(*index)
                        .is_some_and(|guard| guard.is_some())
            })
            .count();
        if active_outputs == 0 {
            bail!("no active Wayland outputs are available");
        }
        if active_outputs != rendered_outputs {
            bail!(
                "incomplete wgpu output coverage: active_outputs={active_outputs} rendered_outputs={rendered_outputs}"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{
        AgentCursorBackendKind, AgentCursorPointerTrackingBackendKind,
        AgentCursorRendererBackendKind, AgentCursorSystemCursorBackendKind,
        AgentOverlayCoverageKind,
    };

    use super::layer_shell_capabilities;
    use crate::OVERLAY_HOST_PROTOCOL_VERSION;
    use crate::system_cursor::SystemCursorAdapter;

    #[test]
    fn layer_shell_capabilities_report_kwin_system_cursor_split_path() {
        let capabilities = layer_shell_capabilities(
            1,
            true,
            AgentCursorRendererBackendKind::Wgpu,
            Some("wgpu renderer active"),
            AgentOverlayCoverageKind::Full,
            2,
            2,
            Some("llvmpipe"),
            Some(120),
            3,
            AgentCursorPointerTrackingBackendKind::KwinEffectSignal,
            true,
            Some("KWin signal tracker active"),
            &SystemCursorAdapter::test_kwin_effect(true),
        );

        assert_eq!(
            capabilities.backend,
            AgentCursorBackendKind::WaylandLayerShell
        );
        assert_eq!(
            capabilities.renderer_backend,
            AgentCursorRendererBackendKind::Wgpu
        );
        assert!(capabilities.visible_overlay);
        assert!(capabilities.click_through);
        assert_eq!(
            capabilities.pointer_tracking_backend,
            AgentCursorPointerTrackingBackendKind::KwinEffectSignal
        );
        assert!(capabilities.pointer_tracking_exact);
        assert!(capabilities.system_cursor_hide_supported);
        assert!(capabilities.system_cursor_hidden);
        assert_eq!(
            capabilities.system_cursor_backend,
            AgentCursorSystemCursorBackendKind::KwinEffect
        );
        assert_eq!(capabilities.coverage, Some(AgentOverlayCoverageKind::Full));
        assert_eq!(capabilities.active_output_count, Some(2));
        assert_eq!(capabilities.rendered_output_count, Some(2));
        assert_eq!(capabilities.adapter_name.as_deref(), Some("llvmpipe"));
        assert_eq!(
            capabilities.protocol_version,
            Some(OVERLAY_HOST_PROTOCOL_VERSION)
        );
        assert!(
            capabilities
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("last_cpu_submit_us=120")
        );
    }
    #[test]
    fn layer_shell_capabilities_fail_closed_for_incomplete_output_coverage() {
        let capabilities = layer_shell_capabilities(
            2,
            true,
            AgentCursorRendererBackendKind::Wgpu,
            Some("wgpu renderer active"),
            AgentOverlayCoverageKind::None,
            2,
            1,
            Some("llvmpipe"),
            None,
            0,
            AgentCursorPointerTrackingBackendKind::None,
            false,
            None,
            &SystemCursorAdapter::test_kwin_effect(false),
        );

        assert_eq!(capabilities.coverage, Some(AgentOverlayCoverageKind::None));
        assert_eq!(capabilities.active_output_count, Some(2));
        assert_eq!(capabilities.rendered_output_count, Some(1));
        assert!(!capabilities.visible_overlay);
    }
}
