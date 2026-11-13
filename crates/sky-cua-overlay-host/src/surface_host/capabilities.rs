//! Structured capability and coverage reporting for the surface host.
//! Clients never infer backend state from prose: these fields carry the truth
//! (backend and renderer kind, visible-overlay support, output coverage,
//! pointer tracking, system-cursor backend).

use anyhow::{Result, bail};
use sky_cua_platform::model::{
    AgentCursorBackendKind, AgentCursorCapabilities, AgentCursorRendererBackendKind,
    AgentOverlayCoverageKind, AgentOverlayEffectsCapabilities, CoordinateSpace,
};

use super::{
    OverlaySurfaceHost, SelectedRenderer,
    platform::{PointerTrackingInfo, SurfacePlatform},
};
use crate::{OVERLAY_HOST_PROTOCOL_VERSION, system_cursor::SystemCursorAdapter};

/// Everything the capability report is built from.
struct CapabilityInputs<'a> {
    backend: AgentCursorBackendKind,
    /// Lead sentence naming the native mechanism and its surface count.
    description: String,
    has_open_surface: bool,
    capture_exclusion: bool,
    renderer_backend: AgentCursorRendererBackendKind,
    renderer_reason: Option<&'a str>,
    coverage: AgentOverlayCoverageKind,
    active_output_count: usize,
    rendered_output_count: usize,
    adapter_name: Option<&'a str>,
    last_frame_submission_us: Option<u128>,
    frames_submitted: u64,
    pointer_tracking: PointerTrackingInfo<'a>,
    system_cursor: &'a SystemCursorAdapter,
}

impl<P: SurfacePlatform> OverlaySurfaceHost<P> {
    pub(super) fn capabilities(&self) -> AgentCursorCapabilities {
        let open_surfaces = self.open_surface_count();
        surface_host_capabilities(CapabilityInputs {
            backend: self.platform.backend_kind(),
            description: self.platform.describe(open_surfaces),
            has_open_surface: open_surfaces > 0,
            capture_exclusion: self.platform.excluded_from_capture(),
            renderer_backend: self.renderer.kind(),
            renderer_reason: self.renderer.reason(),
            coverage: self.coverage_kind(),
            active_output_count: self.active_output_count(),
            rendered_output_count: self.rendered_output_count(),
            adapter_name: self.renderer.adapter_name(),
            last_frame_submission_us: self.last_frame_submission_us,
            frames_submitted: self.frames_submitted,
            pointer_tracking: self.platform.pointer_tracking(),
            system_cursor: self.platform.system_cursor(),
        })
    }

    pub(super) fn visible_overlay_supported(&self) -> bool {
        self.renderer.supports_visible_overlay()
            && self.coverage_kind() == AgentOverlayCoverageKind::Full
    }

    fn active_output_count(&self) -> usize {
        self.platform
            .advertised_output_count()
            .max(self.open_surface_count())
    }

    /// Surfaces that are drawable and have a wgpu surface.
    fn drawable_surface_count(&self) -> usize {
        (0..self.platform.surface_count())
            .filter(|index| {
                self.platform.slot(*index).active()
                    && self
                        .surface_guards
                        .get(*index)
                        .is_some_and(|guard| guard.is_some())
            })
            .count()
    }

    fn rendered_output_count(&self) -> usize {
        match self.renderer {
            SelectedRenderer::Wgpu(..) => self.drawable_surface_count(),
            SelectedRenderer::Unsupported { .. } => 0,
        }
    }

    fn coverage_kind(&self) -> AgentOverlayCoverageKind {
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
        let rendered_outputs = self.drawable_surface_count();
        if active_outputs == 0 {
            bail!("no active outputs are available");
        }
        if active_outputs != rendered_outputs {
            bail!(
                "incomplete wgpu output coverage: active_outputs={active_outputs} rendered_outputs={rendered_outputs}"
            );
        }
        Ok(())
    }
}

fn surface_host_capabilities(inputs: CapabilityInputs<'_>) -> AgentCursorCapabilities {
    let CapabilityInputs {
        backend,
        description: mut reason,
        has_open_surface,
        capture_exclusion,
        renderer_backend,
        renderer_reason,
        coverage,
        active_output_count,
        rendered_output_count,
        adapter_name,
        last_frame_submission_us,
        frames_submitted,
        pointer_tracking,
        system_cursor,
    } = inputs;
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
    if let Some(pointer_tracking_reason) = pointer_tracking.reason {
        reason.push_str("; pointer tracking: ");
        reason.push_str(pointer_tracking_reason);
    }
    if let Some(last_frame_submission_us) = last_frame_submission_us {
        reason.push_str(&format!(
            "; frame pacing: last_cpu_submit_us={last_frame_submission_us} frames_submitted={frames_submitted}"
        ));
    }
    let wgpu = renderer_backend == AgentCursorRendererBackendKind::Wgpu;
    let visible_overlay = has_open_surface && coverage == AgentOverlayCoverageKind::Full && wgpu;
    AgentCursorCapabilities {
        backend,
        renderer_backend,
        visible_overlay,
        screenshot_synthetic_cursor: false,
        click_through: true,
        capture_exclusion,
        pointer_tracking_backend: pointer_tracking.backend,
        pointer_tracking_exact: pointer_tracking.exact,
        system_cursor_hide_supported: system_cursor.supported(),
        system_cursor_hidden: system_cursor.hidden(),
        system_cursor_backend: system_cursor.backend(),
        needs_user_install: false,
        reason: Some(reason),
        effects: Some(AgentOverlayEffectsCapabilities {
            glide: wgpu,
            rotation: wgpu,
            halo: wgpu,
            ripple: wgpu,
            trail: wgpu,
            edge_glow: wgpu,
            inward_wave: wgpu,
            no_no_render: wgpu,
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

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{
        AgentCursorBackendKind, AgentCursorPointerTrackingBackendKind,
        AgentCursorRendererBackendKind, AgentCursorSystemCursorBackendKind,
        AgentOverlayCoverageKind,
    };

    use super::{CapabilityInputs, surface_host_capabilities};
    use crate::OVERLAY_HOST_PROTOCOL_VERSION;
    use crate::surface_host::platform::PointerTrackingInfo;
    use crate::system_cursor::SystemCursorAdapter;

    fn full_coverage_inputs(system_cursor: &SystemCursorAdapter) -> CapabilityInputs<'_> {
        CapabilityInputs {
            backend: AgentCursorBackendKind::WaylandLayerShell,
            description: "test overlay active on 2 output surface(s)".to_string(),
            has_open_surface: true,
            capture_exclusion: false,
            renderer_backend: AgentCursorRendererBackendKind::Wgpu,
            renderer_reason: Some("wgpu renderer active"),
            coverage: AgentOverlayCoverageKind::Full,
            active_output_count: 2,
            rendered_output_count: 2,
            adapter_name: Some("llvmpipe"),
            last_frame_submission_us: Some(120),
            frames_submitted: 3,
            pointer_tracking: PointerTrackingInfo {
                backend: AgentCursorPointerTrackingBackendKind::None,
                exact: false,
                reason: None,
            },
            system_cursor,
        }
    }

    #[test]
    fn capabilities_report_backend_renderer_coverage_and_frame_pacing() {
        let system_cursor = SystemCursorAdapter::unsupported_with_backend(
            AgentCursorSystemCursorBackendKind::MacosNative,
            "pointer hiding needs the frontmost application",
        );
        let capabilities = surface_host_capabilities(CapabilityInputs {
            backend: AgentCursorBackendKind::MacosPanel,
            capture_exclusion: true,
            ..full_coverage_inputs(&system_cursor)
        });

        assert_eq!(capabilities.backend, AgentCursorBackendKind::MacosPanel);
        assert_eq!(
            capabilities.renderer_backend,
            AgentCursorRendererBackendKind::Wgpu
        );
        assert!(capabilities.visible_overlay);
        assert!(capabilities.click_through);
        assert!(capabilities.capture_exclusion);
        assert_eq!(
            capabilities.pointer_tracking_backend,
            AgentCursorPointerTrackingBackendKind::None
        );
        assert!(!capabilities.system_cursor_hide_supported);
        assert!(!capabilities.system_cursor_hidden);
        assert_eq!(
            capabilities.system_cursor_backend,
            AgentCursorSystemCursorBackendKind::MacosNative
        );
        assert_eq!(capabilities.coverage, Some(AgentOverlayCoverageKind::Full));
        assert_eq!(capabilities.active_output_count, Some(2));
        assert_eq!(capabilities.rendered_output_count, Some(2));
        assert_eq!(capabilities.adapter_name.as_deref(), Some("llvmpipe"));
        assert_eq!(
            capabilities.protocol_version,
            Some(OVERLAY_HOST_PROTOCOL_VERSION)
        );
        let reason = capabilities.reason.as_deref().unwrap_or_default();
        assert!(reason.starts_with("test overlay active on 2 output surface(s)"));
        assert!(reason.contains("system cursor hide unsupported: pointer hiding needs"));
        assert!(reason.contains("last_cpu_submit_us=120"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn capabilities_report_kwin_system_cursor_split_path() {
        let system_cursor = SystemCursorAdapter::test_kwin_effect(true);
        let capabilities = surface_host_capabilities(CapabilityInputs {
            pointer_tracking: PointerTrackingInfo {
                backend: AgentCursorPointerTrackingBackendKind::KwinEffectSignal,
                exact: true,
                reason: Some("KWin signal tracker active"),
            },
            ..full_coverage_inputs(&system_cursor)
        });

        assert_eq!(
            capabilities.backend,
            AgentCursorBackendKind::WaylandLayerShell
        );
        assert!(capabilities.visible_overlay);
        assert!(!capabilities.capture_exclusion);
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
    }

    #[test]
    fn capabilities_fail_closed_for_incomplete_output_coverage() {
        let system_cursor = SystemCursorAdapter::unsupported_with_backend(
            AgentCursorSystemCursorBackendKind::Unsupported,
            "not available",
        );
        let capabilities = surface_host_capabilities(CapabilityInputs {
            coverage: AgentOverlayCoverageKind::None,
            rendered_output_count: 1,
            last_frame_submission_us: None,
            frames_submitted: 0,
            ..full_coverage_inputs(&system_cursor)
        });

        assert_eq!(capabilities.coverage, Some(AgentOverlayCoverageKind::None));
        assert_eq!(capabilities.active_output_count, Some(2));
        assert_eq!(capabilities.rendered_output_count, Some(1));
        assert!(!capabilities.visible_overlay);
    }
}
