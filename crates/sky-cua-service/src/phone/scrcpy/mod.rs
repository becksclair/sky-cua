//! scrcpy acceleration backend (LANE: service-scrcpy).
//!
//! Optional low-latency visual control. scrcpy mirrors and controls an Android
//! device over USB or TCP/IP without root or a phone-side app. This module owns
//! the host-side scrcpy seam, split across cohesive children to keep each file
//! under the god-file threshold:
//!
//! - [`resolve`]: binary resolution (config -> `SKY_CUA_SCRCPY` -> `PATH`) and
//!   `scrcpy --version` probing. A missing binary degrades the capability with a
//!   structured reason; it never panics and never disables baseline phone-use.
//! - [`command`]: deterministic launch-command construction with a sanitized
//!   window title and the codec retry policy (H.265 -> H.264 -> default).
//! - [`process`]: the managed/adopted/external ownership model and crash-aware
//!   liveness tracking. Only sky-cua *managed* processes are ever stopped.
//! - [`geometry`]: host-window letterboxed content-rect math so tap coordinates
//!   survive letterboxing, rotation, and fractional host scale.
//!
//! This file keeps the capability constructors the rest of the runtime routes on
//! and re-exports the children as `scrcpy::*`, so callers reach every item
//! through the stable `crate::phone::scrcpy::X` path.
//!
//! scrcpy is acceleration only; ADB remains the control authority. Real launches
//! go through the [`crate::phone::command::CommandRunner`] seam; unit tests use a
//! `FakeCommandRunner` and the pure builders/geometry helpers — nothing here
//! spawns real scrcpy in tests.

mod command;
mod geometry;
mod process;
mod resolve;

// The host-display phone-scale sizing policy is reached by the daemon (outside
// `crate::phone`), so it carries crate visibility rather than the phone-internal
// re-export the rest of the command surface uses.
pub(crate) use command::host_scrcpy_default_max_size;
// `scrcpy_window_title` is consumed by the manager's window-adoption lane (match a
// pre-existing mirror window by its deterministic title); `WINDOW_TITLE_PREFIX`
// and `launch_with_retry` are wired only by the command-builder tests.
pub(in crate::phone) use command::scrcpy_window_title;
// Re-export the child surface so callers keep reaching every item through the
// stable `crate::phone::scrcpy::X` path the integrator and tests already use. The
// managed-launch lane (`manager::scrcpy_lane`) consumes the launch/codec/process
// primitives wired below; the host-window geometry and the
// adopt/external/liveness items belong to later increments (host overlay mapping,
// crash detection, window adoption) and are re-exported now but intentionally
// unused, carrying the spine's `expect` idiom until those lanes land.
pub(in crate::phone) use command::{
    CODEC_RETRY_ORDER, LaunchAttempt, ScrcpyCodec, ScrcpyLaunchSpec,
};
#[expect(unused_imports)]
pub(in crate::phone) use command::{WINDOW_TITLE_PREFIX, launch_with_retry};
// The host-window content-rect geometry is wired by the daemon's window-mapping
// lane (`content_rect`) and the manager's host-overlay path (`ScrcpyContentRect`).
pub(in crate::phone) use geometry::{ScrcpyContentRect, content_rect};
pub(in crate::phone) use process::ScrcpyProcess;
// `ScrcpyLiveness` gates the active/mapped capability shape here; `ScrcpyOwnership`
// is read by the manager's adoption guard (skip adopting a window we already own a
// managed process for).
pub(in crate::phone) use process::{ScrcpyLiveness, ScrcpyOwnership};
pub(in crate::phone) use resolve::{ScrcpyResolution, probe_version, resolve_scrcpy};
use sky_cua_platform::model::{DiagnosticEntry, PhoneScrcpyCapabilities};

/// scrcpy capabilities for a session with no mirror active.
///
/// Retained for the spine's central tests and used as the base shape every
/// richer constructor below specializes.
#[cfg_attr(not(test), expect(dead_code))]
pub(in crate::phone) fn absent_scrcpy() -> PhoneScrcpyCapabilities {
    PhoneScrcpyCapabilities::absent()
}

/// scrcpy installed but no mirror active for this session.
pub(in crate::phone) fn installed_idle(version: Option<String>) -> PhoneScrcpyCapabilities {
    PhoneScrcpyCapabilities {
        installed: true,
        version,
        active: false,
        host_window_mapped: false,
        window_title: None,
        video_codec: None,
        reason: None,
    }
}

/// scrcpy missing: not installed, with a structured reason.
pub(in crate::phone) fn missing_capabilities(reason: impl Into<String>) -> PhoneScrcpyCapabilities {
    PhoneScrcpyCapabilities {
        reason: Some(reason.into()),
        ..PhoneScrcpyCapabilities::absent()
    }
}

/// An active, host-window-mapped scrcpy mirror. `host_window_mapped` reflects
/// whether the content-rect mapping is current; the host-visible overlay is
/// enabled only when it is.
pub(in crate::phone) fn active_capabilities(
    version: Option<String>,
    process: &ScrcpyProcess,
    host_window_mapped: bool,
) -> PhoneScrcpyCapabilities {
    PhoneScrcpyCapabilities {
        installed: true,
        version,
        active: process.liveness == ScrcpyLiveness::Running,
        host_window_mapped: host_window_mapped && process.liveness == ScrcpyLiveness::Running,
        window_title: Some(process.window_title.clone()),
        video_codec: process.codec.capability_value(),
        reason: None,
    }
}

/// Capability after a crash: installed but inactive and unmapped, with a
/// structured reason so the agent sees the downgrade rather than silent loss.
pub(in crate::phone) fn crashed_capabilities(version: Option<String>) -> PhoneScrcpyCapabilities {
    PhoneScrcpyCapabilities {
        installed: true,
        version,
        active: false,
        host_window_mapped: false,
        window_title: None,
        video_codec: None,
        reason: Some("scrcpy process exited unexpectedly".to_string()),
    }
}

/// Whether the host-visible cursor overlay may be enabled for this scrcpy
/// capability: only when a live mirror is mapped into a current host window.
pub(in crate::phone) fn host_overlay_enabled(caps: &PhoneScrcpyCapabilities) -> bool {
    caps.active && caps.host_window_mapped
}

/// Structured diagnostic for a scrcpy launch that failed across every codec in
/// the retry order. The caller keeps the session and degrades to ADB/companion.
pub(in crate::phone) fn launch_failed_diagnostic(detail: impl Into<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneScrcpyLaunchFailed".to_string(),
        message: "scrcpy mirror could not start after codec retries; degrading to ADB/companion"
            .to_string(),
        details: Some(detail.into()),
    }
}

/// Placeholder for the resolved scrcpy version string. Real launch flows thread
/// the parsed `scrcpy --version` output here; crash downgrades that happen before
/// a version is known use `None`.
fn scrcpy_version_placeholder() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_binary_degrades_with_reason() {
        let caps = missing_capabilities("scrcpy not found on PATH and no scrcpy_path configured");
        assert!(!caps.installed);
        assert!(!caps.active);
        assert!(caps.reason.is_some());
        // Degrade must never imply a usable overlay.
        assert!(!host_overlay_enabled(&caps));
    }

    #[test]
    fn installed_idle_reports_no_active_mirror() {
        let caps = installed_idle(Some("4.0".to_string()));
        assert!(caps.installed);
        assert!(!caps.active);
        assert!(!caps.host_window_mapped);
        assert!(!host_overlay_enabled(&caps));
    }

    #[test]
    fn launch_failed_diagnostic_is_structured() {
        let diag = launch_failed_diagnostic("all codecs failed: h265,h264,default");
        assert_eq!(diag.code, "PhoneScrcpyLaunchFailed");
        assert!(diag.details.is_some());
    }
}
