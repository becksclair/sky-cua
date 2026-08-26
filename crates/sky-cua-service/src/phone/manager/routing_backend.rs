//! Backend routing plus the perception/action execution paths.
//!
//! Routing is deterministic and read straight from the session's cached
//! capability profile: companion (when its RPC is reachable and the specific
//! capability is proven) is preferred, then scrcpy when a live mapped mirror
//! exists, then the ADB baseline for non-coordinate operations. Every response
//! states the backend that actually serviced it and the capability profile id in
//! force, and a stale profile rejects backends it can no longer prove.
//!
//! Coordinate actions (`phone_tap`, `phone_swipe`) require a reachable companion
//! gesture lane plus a fresh `phone_snapshot_id` (unless the caller opts into
//! device coordinates). They run through snapshot stale/mismatch/out-of-bounds
//! validation before dispatch and never fall back to ADB, so visible phone-side
//! feedback stays coupled to real input.

#![allow(clippy::empty_line_after_doc_comments)]
use sky_cua_platform::model::{
    DiagnosticEntry, PhoneActionResponse, PhoneActivationClass, PhoneAvailableAction,
    PhoneBackendCapabilities, PhoneBackendKind, PhoneCapabilityAvailability,
    PhoneCapabilityFidelity, PhoneCapabilityProfile, PhoneCapabilityRefreshState,
    PhoneCapabilityRoute, PhoneConnectionKind, PhoneOperationProvider, PhoneUnavailableAction,
};

use super::{ActionContext, PhoneManager, selector_ids};
use crate::phone::direct::DirectRuntimeError;
use crate::phone::mapping;
use crate::phone::{cursor, scrcpy};

impl PhoneManager {
    pub(crate) fn coordinate_backend(
        &self,
        profile: &PhoneCapabilityProfile,
    ) -> Result<PhoneBackendKind, DiagnosticEntry> {
        if !profile.stale && profile.companion.rpc_reachable && profile.companion.gesture_dispatch {
            Ok(PhoneBackendKind::Companion)
        } else {
            Err(companion_required_diagnostic())
        }
    }

    /// Backend for a screenshot: companion on-device capture when proven, then
    /// ADB. scrcpy frames are not pulled as still screenshots in v1.
    pub(crate) fn screenshot_backend(&self, profile: &PhoneCapabilityProfile) -> PhoneBackendKind {
        if !profile.stale && profile.companion.rpc_reachable && profile.companion.screenshot {
            PhoneBackendKind::Companion
        } else {
            PhoneBackendKind::Adb
        }
    }

    /// Cursor capabilities for a session, derived from the profile: the
    /// host-visible overlay tracks the companion overlay mirrored into a live
    /// mapped scrcpy window, the synthetic cursor tracks config, and the native
    /// overlay tracks the companion.
    pub(crate) fn cursor_capabilities(
        &self,
        profile: &PhoneCapabilityProfile,
    ) -> sky_cua_platform::model::PhoneCursorCapabilities {
        // The on-device visible overlay disabled in config forces both visible
        // planes off and reports the resolved state honestly: the host suppresses
        // every companion visible-overlay call, so neither the native overlay nor
        // its host-mirrored plane can be live. The screenshot-synthetic marker is a
        // separate plane driven by `screenshot_cursor`, so it is unaffected (mirror
        // of the ADB-only `visible_overlay=false`/`screenshot_synthetic_cursor=true`
        // contract). Default is enabled, so this branch is skipped unless the
        // operator opted out.
        if !self.selection.visible_overlay {
            return sky_cua_platform::model::PhoneCursorCapabilities {
                host_visible_overlay: false,
                screenshot_synthetic_cursor: self.selection.screenshot_cursor,
                phone_native_overlay: false,
                visible_overlay_reason: Some(
                    "visible overlay disabled in config ([phone] visible_overlay=false); companion overlay calls are suppressed"
                        .to_string(),
                ),
            };
        }
        let native = profile.companion.rpc_reachable && profile.companion.native_overlay;
        // The host-visible cursor plane is now the companion's on-device overlay
        // mirrored into a mapped scrcpy window: the host no longer draws the phone
        // cursor itself, so a host-visible cursor exists only when the companion
        // overlay is reachable AND a scrcpy mirror is mapped to display it.
        let host_visible = native && scrcpy::host_overlay_enabled(&profile.scrcpy);
        if !host_visible && !native {
            // ADB-only: synthetic marker only (or nothing if disabled in config).
            return cursor::adb_only_capabilities(self.selection.screenshot_cursor);
        }
        sky_cua_platform::model::PhoneCursorCapabilities {
            host_visible_overlay: host_visible,
            screenshot_synthetic_cursor: self.selection.screenshot_cursor,
            phone_native_overlay: native,
            visible_overlay_reason: None,
        }
    }

    /// Drop the companion capability from a session's cached profile after an RPC
    /// failure, so subsequent routing re-evaluates as ADB-only until the next
    /// refresh re-proves it.
    pub(crate) fn invalidate_companion(&mut self, session_id: &str) {
        if let Some(cached) = self.profiles.get_mut(session_id) {
            cached.profile.companion.rpc_reachable = false;
            cached.profile.companion.gesture_dispatch = false;
            cached.profile.companion.screenshot = false;
            cached.profile.companion.accessibility_tree = false;
            cached.profile.companion.notifications = false;
            // Keep the stale bool and refresh_state in lockstep (matching
            // device.rs and cached_profile): a Stale refresh_state always implies
            // stale=true.
            cached.profile.stale = true;
            cached.profile.refresh_state = PhoneCapabilityRefreshState::Stale;
        }
        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.session.capabilities.companion = false;
            entry.session.capabilities.phone_native_overlay = false;
        }
    }
}

pub(crate) fn validate_device_point(
    profile: &PhoneCapabilityProfile,
    x: f64,
    y: f64,
) -> Result<(), DiagnosticEntry> {
    if !x.is_finite() || !y.is_finite() {
        let error = mapping::MappingError::NonFinite { plane: "device" };
        return Err(DiagnosticEntry {
            code: error.code().to_string(),
            message: format!("coordinate translation failed: {error}"),
            details: None,
        });
    }
    if let Some(size) = profile.display_size.as_ref()
        && (x < 0.0 || y < 0.0 || x >= f64::from(size.width) || y >= f64::from(size.height))
    {
        let error = mapping::MappingError::OutOfBounds { plane: "device" };
        return Err(DiagnosticEntry {
            code: error.code().to_string(),
            message: format!("coordinate translation failed: {error}"),
            details: None,
        });
    }
    Ok(())
}

/// Compose the available/unavailable action list onto a freshly-detected profile
/// from its backend capabilities. The action strings are the canonical tool names
/// so the agent reads a device-tailored menu.
pub(crate) fn populate_actions(
    profile: &mut PhoneCapabilityProfile,
    caps: &PhoneBackendCapabilities,
) {
    let mut available = Vec::new();
    let mut unavailable = Vec::new();

    let coordinate_backend = (caps.companion && profile.companion.gesture_dispatch)
        .then_some(PhoneBackendKind::Companion);
    let screenshot_backend = if caps.companion && profile.companion.screenshot {
        PhoneBackendKind::Companion
    } else {
        PhoneBackendKind::Adb
    };
    // Launch / open-intent are companion-preferred (the companion uses
    // `getLaunchIntentForPackage`) whenever its RPC is reachable, with ADB as the
    // fallback. Force-stop stays on ADB: a non-privileged companion cannot
    // force-stop, so its affordance must not advertise the companion.
    let app_op_backend = if caps.companion {
        PhoneBackendKind::Companion
    } else {
        PhoneBackendKind::Adb
    };

    let mut add = |action: &str, backend: PhoneBackendKind| {
        available.push(PhoneAvailableAction {
            action: action.to_string(),
            backend,
            detail: None,
        });
    };

    if caps.screenshot {
        add("phone_observe", screenshot_backend);
        add("phone_screenshot", screenshot_backend);
    }
    let interactive_backend =
        if caps.companion && profile.connection_kind == PhoneConnectionKind::CompanionDirect {
            PhoneBackendKind::Companion
        } else {
            PhoneBackendKind::Adb
        };
    if caps.text_input {
        add("phone_type_text", interactive_backend);
    }
    if caps.key_input {
        add("phone_press_key", interactive_backend);
    }
    if caps.app_management {
        add("phone_app_current", interactive_backend);
        add("phone_app_list", interactive_backend);
    }
    add("phone_app_launch", app_op_backend);
    add("phone_app_open_intent", app_op_backend);
    if caps.adb {
        add("phone_app_force_stop", PhoneBackendKind::Adb);
        add("phone_app_install", PhoneBackendKind::Adb);
    } else {
        for action in ["phone_app_force_stop", "phone_app_install"] {
            unavailable.push(PhoneUnavailableAction {
                action: action.into(),
                reason: "operation requires the optional ADB backend".into(),
                detail: None,
            });
        }
    }
    add("phone_open_settings", interactive_backend);
    if caps.companion {
        for action in [
            "phone_content",
            "phone_clipboard",
            "phone_editor",
            "phone_camera",
            "phone_storage",
        ] {
            add(action, PhoneBackendKind::Companion);
        }
    }

    // Companion-gated actions.
    if let Some(backend) = coordinate_backend {
        add("phone_tap", backend);
        add("phone_swipe", backend);
    } else {
        for action in ["phone_tap", "phone_swipe"] {
            unavailable.push(PhoneUnavailableAction {
                action: action.to_string(),
                reason: "companion gesture dispatch not enabled or unreachable".to_string(),
                detail: None,
            });
        }
    }
    if caps.accessibility_tree {
        add("phone_accessibility_tree", PhoneBackendKind::Companion);
    } else {
        unavailable.push(PhoneUnavailableAction {
            action: "phone_accessibility_tree".to_string(),
            reason: "companion accessibility service not enabled or unreachable".to_string(),
            detail: None,
        });
    }
    if caps.notifications {
        for action in [
            "phone_notifications",
            "phone_notification_open",
            "phone_notification_dismiss",
            "phone_notification_action",
            "phone_notification_reply",
        ] {
            add(action, PhoneBackendKind::Companion);
        }
    } else {
        for action in [
            "phone_notifications",
            "phone_notification_open",
            "phone_notification_dismiss",
            "phone_notification_action",
            "phone_notification_reply",
        ] {
            unavailable.push(PhoneUnavailableAction {
                action: action.to_string(),
                reason: "companion notification listener not enabled or unreachable".to_string(),
                detail: None,
            });
        }
    }

    let evidenced_at_ms = profile.detected_at_ms;
    profile.routes = available
        .iter()
        .flat_map(|action| {
            route_operations(&action.action)
                .into_iter()
                .map(move |operation| {
                    let provider = operation_provider(&operation, action.backend);
                    let activation = operation_activation(&operation, provider);
                    PhoneCapabilityRoute {
                        operation,
                        provider,
                        availability: PhoneCapabilityAvailability::Ready,
                        prerequisites: Vec::new(),
                        activation,
                        fidelity: if provider == PhoneOperationProvider::Adb {
                            PhoneCapabilityFidelity::Exact
                        } else {
                            PhoneCapabilityFidelity::Native
                        },
                        evidenced_at_ms,
                        link_epoch: None,
                        detail: action.detail.clone(),
                    }
                })
        })
        .chain(unavailable.iter().map(|action| PhoneCapabilityRoute {
            operation: action.action.clone(),
            provider: PhoneOperationProvider::None,
            availability: if action.reason.contains("not enabled") {
                PhoneCapabilityAvailability::ActivationRequired
            } else {
                PhoneCapabilityAvailability::Unsupported
            },
            prerequisites: vec![action.reason.clone()],
            activation: PhoneActivationClass::UserSettings,
            fidelity: PhoneCapabilityFidelity::Partial,
            evidenced_at_ms,
            link_epoch: None,
            detail: action.detail.clone(),
        }))
        .collect();
    profile.available_actions = available;
    profile.unavailable_actions = unavailable;
}

pub(crate) fn route_operations(action: &str) -> Vec<String> {
    let operations: &[&str] = match action {
        "phone_content" => &[
            "phone_content.describe",
            "phone_content.import_host_file",
            "phone_content.export_host_file",
            "phone_content.release",
        ],
        "phone_clipboard" => &[
            "phone_clipboard.get",
            "phone_clipboard.set",
            "phone_clipboard.clear",
            "phone_clipboard.changes",
        ],
        "phone_editor" => &[
            "phone_editor.context",
            "phone_editor.set_text",
            "phone_editor.insert_text",
            "phone_editor.set_selection",
            "phone_editor.select_all",
            "phone_editor.copy",
            "phone_editor.cut",
            "phone_editor.paste",
            "phone_editor.insert_content",
        ],
        "phone_camera" => &[
            "phone_camera.enumerate",
            "phone_camera.capabilities",
            "phone_camera.photo",
            "phone_camera.video_start",
            "phone_camera.video_pause",
            "phone_camera.video_resume",
            "phone_camera.video_stop",
            "phone_camera.preview_start",
            "phone_camera.preview_frame",
            "phone_camera.preview_stop",
            "phone_camera.controls",
        ],
        "phone_storage" => &[
            "phone_storage.roots",
            "phone_storage.list",
            "phone_storage.stat",
            "phone_storage.read",
            "phone_storage.write",
            "phone_storage.mkdir",
            "phone_storage.copy",
            "phone_storage.move",
            "phone_storage.rename",
            "phone_storage.delete",
            "phone_storage.trash",
            "phone_storage.hash",
            "phone_storage.search",
            "phone_storage.thumbnail",
            "phone_storage.metadata",
            "phone_storage.add_saf_root",
            "phone_storage.remove_saf_root",
            "phone_storage.list_saf_roots",
        ],
        _ => return vec![action.to_owned()],
    };
    operations
        .iter()
        .map(|operation| (*operation).to_owned())
        .collect()
}

pub(crate) fn operation_provider(
    operation: &str,
    backend: PhoneBackendKind,
) -> PhoneOperationProvider {
    match backend {
        PhoneBackendKind::Adb => PhoneOperationProvider::Adb,
        PhoneBackendKind::Scrcpy => PhoneOperationProvider::Scrcpy,
        PhoneBackendKind::Companion if operation.starts_with("phone_camera") => {
            PhoneOperationProvider::CompanionCamera
        }
        PhoneBackendKind::Companion if operation.starts_with("phone_storage") => {
            PhoneOperationProvider::CompanionStorage
        }
        PhoneBackendKind::Companion
            if operation.starts_with("phone_editor")
                || operation.contains("tap")
                || operation.contains("swipe")
                || operation.contains("observe")
                || operation.contains("accessibility") =>
        {
            PhoneOperationProvider::CompanionAccessibility
        }
        PhoneBackendKind::Companion => PhoneOperationProvider::CompanionNative,
        _ => PhoneOperationProvider::None,
    }
}

pub(crate) fn operation_activation(
    operation: &str,
    provider: PhoneOperationProvider,
) -> PhoneActivationClass {
    if operation.starts_with("phone_camera.")
        && !matches!(
            operation,
            "phone_camera.enumerate" | "phone_camera.capabilities"
        )
    {
        PhoneActivationClass::VisibleActivity
    } else if provider == PhoneOperationProvider::CompanionAccessibility {
        PhoneActivationClass::AccessibilityService
    } else {
        PhoneActivationClass::None
    }
}

/// An action response for a selector that resolved to no session.
pub(crate) fn action_no_session(
    selector: &sky_cua_platform::model::PhoneSessionSelector,
    action: &str,
) -> PhoneActionResponse {
    let (session_id, serial) = selector_ids(selector);
    PhoneActionResponse {
        session_id,
        serial,
        action: action.to_string(),
        backend: PhoneBackendKind::None,
        capability_profile_id: String::new(),
        profile_refresh_state: PhoneCapabilityRefreshState::Stale,
        phone_snapshot_id: None,
        cursor: None,
        diagnostics: vec![super::no_session_diagnostic(selector)],
    }
}

/// An action response that failed before dispatch (e.g. snapshot rejected,
/// coordinate translation failed). The cursor is never updated.
pub(crate) fn action_failure(
    ctx: &ActionContext,
    action: &str,
    diagnostic: DiagnosticEntry,
) -> PhoneActionResponse {
    PhoneActionResponse {
        session_id: ctx.session_id.clone(),
        serial: ctx.serial.clone(),
        action: action.to_string(),
        backend: PhoneBackendKind::None,
        capability_profile_id: ctx.profile.profile_id.clone(),
        profile_refresh_state: ctx.profile.refresh_state,
        phone_snapshot_id: None,
        cursor: None,
        diagnostics: vec![diagnostic],
    }
}

pub(crate) fn direct_error_diagnostic(error: DirectRuntimeError) -> DiagnosticEntry {
    let message = format!("CompanionDirect dispatch failed: {error:?}");
    DiagnosticEntry {
        code: "PhoneCompanionDirectDispatchFailed".to_string(),
        message,
        details: None,
    }
}

pub(crate) fn companion_required_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneCompanionRequired".to_string(),
        message: "coordinate actions require a reachable companion with gesture dispatch; reconnect or run phone_setup before tapping or swiping".to_string(),
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::{
        PhoneCompanionCapabilities, PhoneConnectionKind, PhoneScrcpyCapabilities,
        PhoneTargetDeviceKind,
    };

    pub(crate) fn profile_with(companion: PhoneCompanionCapabilities) -> PhoneCapabilityProfile {
        PhoneCapabilityProfile {
            profile_id: "p".to_string(),
            session_id: "s".to_string(),
            serial: "serial".to_string(),
            detected_at_ms: 0,
            stale: false,
            refresh_state: PhoneCapabilityRefreshState::Detected,
            manufacturer: None,
            brand: None,
            model: None,
            device: None,
            target_device_kind: PhoneTargetDeviceKind::UnknownAndroid,
            hyperos_version: None,
            android_sdk: None,
            android_release: None,
            display_size: None,
            density_dpi: None,
            orientation: None,
            display_rotation_degrees: None,
            connection_kind: PhoneConnectionKind::Usb,
            companion,
            scrcpy: PhoneScrcpyCapabilities::absent(),
            root_available: false,
            shizuku_available: false,
            device_owner: false,
            available_actions: Vec::new(),
            unavailable_actions: Vec::new(),
            routes: Vec::new(),
        }
    }

    pub(crate) fn caps(companion: bool) -> PhoneBackendCapabilities {
        PhoneBackendCapabilities {
            adb: true,
            companion,
            scrcpy: false,
            screenshot: true,
            gestures: companion,
            text_input: true,
            key_input: true,
            accessibility_tree: companion,
            notifications: companion,
            app_management: true,
            host_visible_overlay: false,
            screenshot_synthetic_cursor: true,
            phone_native_overlay: companion,
        }
    }

    #[test]
    pub(crate) fn populate_actions_routes_tap_to_companion_when_gesture_proven() {
        let mut companion = PhoneCompanionCapabilities::absent("pkg");
        companion.rpc_reachable = true;
        companion.gesture_dispatch = true;
        companion.screenshot = true;
        companion.accessibility_tree = true;
        companion.notifications = true;
        let mut profile = profile_with(companion);
        populate_actions(&mut profile, &caps(true));

        let tap = profile
            .available_actions
            .iter()
            .find(|a| a.action == "phone_tap")
            .expect("tap available");
        assert_eq!(tap.backend, PhoneBackendKind::Companion);
        // Launch / open-intent are companion-preferred; force-stop stays on ADB.
        let backend_of = |action: &str| {
            profile
                .available_actions
                .iter()
                .find(|a| a.action == action)
                .unwrap_or_else(|| panic!("{action} available"))
                .backend
        };
        assert_eq!(backend_of("phone_app_launch"), PhoneBackendKind::Companion);
        assert_eq!(
            backend_of("phone_app_open_intent"),
            PhoneBackendKind::Companion
        );
        assert_eq!(backend_of("phone_app_force_stop"), PhoneBackendKind::Adb);
        // Companion-gated tools are available, not in the unavailable list.
        assert!(
            profile
                .available_actions
                .iter()
                .any(|a| a.action == "phone_accessibility_tree")
        );
        assert!(profile.unavailable_actions.is_empty());
    }

    #[test]
    pub(crate) fn populate_actions_gates_coordinates_without_companion() {
        let mut profile = profile_with(PhoneCompanionCapabilities::absent("pkg"));
        populate_actions(&mut profile, &caps(false));

        assert!(
            profile
                .available_actions
                .iter()
                .all(|a| a.action != "phone_tap" && a.action != "phone_swipe"),
            "coordinate actions must not advertise ADB fallback: {:?}",
            profile.available_actions
        );
        assert!(
            profile
                .unavailable_actions
                .iter()
                .any(|a| a.action == "phone_tap")
        );
        assert!(
            profile
                .unavailable_actions
                .iter()
                .any(|a| a.action == "phone_swipe")
        );
        let screenshot = profile
            .available_actions
            .iter()
            .find(|a| a.action == "phone_screenshot")
            .expect("screenshot available");
        assert_eq!(screenshot.backend, PhoneBackendKind::Adb);
        // Without a companion, launch / open-intent fall back to ADB affordances.
        assert!(
            profile
                .available_actions
                .iter()
                .any(|a| a.action == "phone_app_launch" && a.backend == PhoneBackendKind::Adb)
        );
        assert!(
            profile
                .available_actions
                .iter()
                .any(|a| a.action == "phone_app_open_intent" && a.backend == PhoneBackendKind::Adb)
        );
        // Companion-gated tools are reported unavailable with a reason.
        assert!(
            profile
                .unavailable_actions
                .iter()
                .any(|a| a.action == "phone_accessibility_tree")
        );
        assert!(
            profile
                .unavailable_actions
                .iter()
                .any(|a| a.action == "phone_notification_reply")
        );
    }
}
