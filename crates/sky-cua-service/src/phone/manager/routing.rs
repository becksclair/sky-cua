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

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneActionResponse, PhoneActivationClass, PhoneAvailableAction,
    PhoneBackendCapabilities, PhoneBackendKind, PhoneCapabilityAvailability,
    PhoneCapabilityFidelity, PhoneCapabilityProfile, PhoneCapabilityRefreshState,
    PhoneCapabilityRoute, PhoneConnectionKind, PhoneOperationProvider, PhonePoint,
    PhonePressKeyRequest, PhoneSwipeRequest, PhoneTapRequest, PhoneTypeTextRequest,
    PhoneUnavailableAction,
};

use super::{ActionContext, PhoneManager, no_companion_diagnostic, now_ms, selector_ids};
use crate::phone::adb;
use crate::phone::companion::protocol::{GestureKind, GesturePoint};
use crate::phone::direct::DirectRuntimeError;
use crate::phone::mapping;
use crate::phone::snapshot;
use crate::phone::{cursor, scrcpy};

/// Stroke duration for a companion tap gesture. Android `dispatchGesture`
/// rejects a non-positive stroke duration (`bad_request: duration_ms must be
/// positive`), so a tap is dispatched as a brief press rather than the
/// zero-duration stroke ADB `input tap` uses.
const COMPANION_TAP_DURATION_MS: u32 = 50;

/// Animation duration hint for a phone-side overlay tap ripple, in milliseconds.
/// Longer than the dispatch stroke ([`COMPANION_TAP_DURATION_MS`]) so the
/// expanding ripple is perceptible, while staying short enough to track a brisk
/// agent. The companion clamps this to its own sane minimum.
const OVERLAY_TAP_DURATION_MS: u32 = 250;

/// Default animation duration hint for a swipe/drag overlay trail when the
/// request omitted an explicit gesture duration, in milliseconds. Mirrors the
/// ADB/companion swipe default so the trail tracks the dispatched motion.
const OVERLAY_SWIPE_DEFAULT_DURATION_MS: u32 = 300;

/// A description of the phone-side overlay animation for one coordinate action,
/// in device pixels. Built by the coordinate paths and handed to `finish_action`,
/// which fires it on the companion overlay after a successful dispatch. The
/// `kind` is the free-form wire string the overlay supports (`tap`/`swipe`/
/// `drag`), independent of the real-input dispatch backend.
pub(super) struct OverlayGestureSpec {
    pub(super) kind: &'static str,
    pub(super) points: Vec<PhonePoint>,
    pub(super) duration_ms: u32,
}

impl PhoneManager {
    // ===================================================================
    // Coordinate / text / key actions
    // ===================================================================

    /// `phone_tap`: translate snapshot coordinates to device pixels (or accept
    /// raw device coordinates), dispatch through the companion gesture lane, and
    /// update the cursor on success.
    pub(super) async fn tap(&mut self, request: PhoneTapRequest) -> PhoneActionResponse {
        let Some(pre_ctx) = self.action_context(&request.session) else {
            return action_no_session(&request.session, "phone_tap");
        };
        if let Err(diag) = self.device_point_for(
            &pre_ctx,
            request.phone_snapshot_id.as_deref(),
            request.x,
            request.y,
            request.use_device_coordinates,
        ) {
            return action_failure(&pre_ctx, "phone_tap", diag);
        }

        let Some(ctx) = self.fresh_action_context(&request.session).await else {
            return action_no_session(&request.session, "phone_tap");
        };

        let device_point = match self.device_point_for(
            &ctx,
            request.phone_snapshot_id.as_deref(),
            request.x,
            request.y,
            request.use_device_coordinates,
        ) {
            Ok(point) => point,
            Err(diag) => return action_failure(&ctx, "phone_tap", diag),
        };
        let screenshot_point = (!request.use_device_coordinates).then_some(PhonePoint {
            x: request.x,
            y: request.y,
        });

        let backend = match self.coordinate_backend(&ctx.profile) {
            Ok(backend) => backend,
            Err(diag) => return action_failure(&ctx, "phone_tap", diag),
        };
        let result = self.dispatch_tap(&ctx, backend, device_point).await;
        let overlay_gesture = Some(OverlayGestureSpec {
            kind: "tap",
            points: vec![device_point],
            duration_ms: OVERLAY_TAP_DURATION_MS,
        });
        self.finish_action(
            &ctx,
            "phone_tap",
            backend,
            request.phone_snapshot_id.clone(),
            Some(device_point),
            screenshot_point,
            overlay_gesture,
            result,
        )
        .await
    }

    /// `phone_swipe`: same translation/routing as tap, over a start/end pair.
    pub(super) async fn swipe(&mut self, request: PhoneSwipeRequest) -> PhoneActionResponse {
        let Some(pre_ctx) = self.action_context(&request.session) else {
            return action_no_session(&request.session, "phone_swipe");
        };
        for (x, y) in [
            (request.start_x, request.start_y),
            (request.end_x, request.end_y),
        ] {
            if let Err(diag) = self.device_point_for(
                &pre_ctx,
                request.phone_snapshot_id.as_deref(),
                x,
                y,
                request.use_device_coordinates,
            ) {
                return action_failure(&pre_ctx, "phone_swipe", diag);
            }
        }

        let Some(ctx) = self.fresh_action_context(&request.session).await else {
            return action_no_session(&request.session, "phone_swipe");
        };

        let start = match self.device_point_for(
            &ctx,
            request.phone_snapshot_id.as_deref(),
            request.start_x,
            request.start_y,
            request.use_device_coordinates,
        ) {
            Ok(point) => point,
            Err(diag) => return action_failure(&ctx, "phone_swipe", diag),
        };
        let end = match self.device_point_for(
            &ctx,
            request.phone_snapshot_id.as_deref(),
            request.end_x,
            request.end_y,
            request.use_device_coordinates,
        ) {
            Ok(point) => point,
            Err(diag) => return action_failure(&ctx, "phone_swipe", diag),
        };
        let screenshot_point = (!request.use_device_coordinates).then_some(PhonePoint {
            x: request.end_x,
            y: request.end_y,
        });

        let backend = match self.coordinate_backend(&ctx.profile) {
            Ok(backend) => backend,
            Err(diag) => return action_failure(&ctx, "phone_swipe", diag),
        };
        let result = self
            .dispatch_swipe(&ctx, backend, start, end, request.duration_ms)
            .await;
        let overlay_gesture = Some(OverlayGestureSpec {
            kind: "swipe",
            points: vec![start, end],
            duration_ms: request
                .duration_ms
                .unwrap_or(OVERLAY_SWIPE_DEFAULT_DURATION_MS),
        });
        self.finish_action(
            &ctx,
            "phone_swipe",
            backend,
            request.phone_snapshot_id.clone(),
            Some(end),
            screenshot_point,
            overlay_gesture,
            result,
        )
        .await
    }

    /// `phone_type_text`: companion IME path when reachable, else `adb shell input
    /// text` (escaped by the ADB lane).
    pub(super) async fn type_text(&mut self, request: PhoneTypeTextRequest) -> PhoneActionResponse {
        let Some(ctx) = self.action_context(&request.session) else {
            return action_no_session(&request.session, "phone_type_text");
        };
        if let Some((device_id, epoch)) = self.direct_identity(&ctx.session_id) {
            let result = self
                .direct_provider
                .as_ref()
                .expect("direct identity requires provider")
                .dispatch(
                    &device_id,
                    epoch,
                    "input.text",
                    serde_json::json!({"text": request.text}),
                    false,
                    std::time::Duration::from_secs(5),
                )
                .await
                .map(|_| true)
                .map_err(direct_error_diagnostic);
            return self
                .finish_action(
                    &ctx,
                    "phone_type_text",
                    PhoneBackendKind::Companion,
                    None,
                    None,
                    None,
                    None,
                    result,
                )
                .await;
        }
        // Text input has no companion RPC method in v1; route through ADB.
        let backend = PhoneBackendKind::Adb;
        let result = adb::input_text(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &ctx.serial,
            &request.text,
        )
        .await
        .map(|outcome| outcome.success)
        .map_err(|error| adb::command_error_diagnostic("adb shell input text", &error));
        self.finish_action(
            &ctx,
            "phone_type_text",
            backend,
            None,
            None,
            None,
            None,
            result,
        )
        .await
    }

    /// `phone_press_key`: companion has no key method in v1, so route through
    /// `adb shell input keyevent` (normalized by the ADB lane).
    pub(super) async fn press_key(&mut self, request: PhonePressKeyRequest) -> PhoneActionResponse {
        let Some(ctx) = self.action_context(&request.session) else {
            return action_no_session(&request.session, "phone_press_key");
        };
        if let Some((device_id, epoch)) = self.direct_identity(&ctx.session_id) {
            let result = self
                .direct_provider
                .as_ref()
                .expect("direct identity requires provider")
                .dispatch(
                    &device_id,
                    epoch,
                    "input.key",
                    serde_json::json!({"key": request.key}),
                    false,
                    std::time::Duration::from_secs(5),
                )
                .await
                .map(|_| true)
                .map_err(direct_error_diagnostic);
            return self
                .finish_action(
                    &ctx,
                    "phone_press_key",
                    PhoneBackendKind::Companion,
                    None,
                    None,
                    None,
                    None,
                    result,
                )
                .await;
        }
        let backend = PhoneBackendKind::Adb;
        let result = adb::input_keyevent(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &ctx.serial,
            &request.key,
        )
        .await
        .map(|outcome| outcome.success)
        .map_err(|error| adb::command_error_diagnostic("adb shell input keyevent", &error));
        self.finish_action(
            &ctx,
            "phone_press_key",
            backend,
            None,
            None,
            None,
            None,
            result,
        )
        .await
    }

    /// Dispatch a tap through the companion gesture lane.
    async fn dispatch_tap(
        &mut self,
        ctx: &ActionContext,
        backend: PhoneBackendKind,
        point: PhonePoint,
    ) -> Result<bool, DiagnosticEntry> {
        match backend {
            PhoneBackendKind::Companion => {
                let dispatched = self
                    .companion_gesture(
                        ctx,
                        GestureKind::Tap,
                        vec![point],
                        COMPANION_TAP_DURATION_MS,
                    )
                    .await?;
                Ok(dispatched)
            }
            _ => Err(companion_required_diagnostic()),
        }
    }

    /// Dispatch a swipe through the companion gesture lane.
    async fn dispatch_swipe(
        &mut self,
        ctx: &ActionContext,
        backend: PhoneBackendKind,
        start: PhonePoint,
        end: PhonePoint,
        duration_ms: Option<u32>,
    ) -> Result<bool, DiagnosticEntry> {
        match backend {
            PhoneBackendKind::Companion => {
                self.companion_gesture(
                    ctx,
                    GestureKind::Swipe,
                    vec![start, end],
                    duration_ms.unwrap_or(300),
                )
                .await
            }
            _ => Err(companion_required_diagnostic()),
        }
    }

    /// Dispatch a companion gesture over the live RPC client. A transport/auth
    /// failure invalidates the companion capability (so a later action re-routes
    /// to ADB) and surfaces a structured diagnostic; a per-method error is
    /// surfaced without claiming success.
    async fn companion_gesture(
        &mut self,
        ctx: &ActionContext,
        kind: GestureKind,
        points: Vec<PhonePoint>,
        duration_ms: u32,
    ) -> Result<bool, DiagnosticEntry> {
        if let Some((device_id, epoch)) = self.direct_identity(&ctx.session_id) {
            let kind = match kind {
                GestureKind::Tap => "tap",
                GestureKind::Swipe => "swipe",
            };
            let points = points.into_iter().map(|point| serde_json::json!({"x": point.x.round() as i64, "y": point.y.round() as i64})).collect::<Vec<_>>();
            return self
                .direct_provider
                .as_ref()
                .expect("direct identity requires provider")
                .dispatch(
                    &device_id,
                    epoch,
                    "gesture",
                    serde_json::json!({"kind": kind, "points": points, "duration_ms": duration_ms}),
                    false,
                    std::time::Duration::from_secs(5),
                )
                .await
                .map(|_| true)
                .map_err(direct_error_diagnostic);
        }
        let Some(entry) = self.sessions.get_mut(&ctx.session_id) else {
            return Err(no_companion_diagnostic());
        };
        let Some(runtime) = entry.companion.as_mut() else {
            return Err(no_companion_diagnostic());
        };
        let gesture_points = points
            .iter()
            // Round to whole device pixels before the wire so the companion
            // lands on the same pixel as the ADB path (which rounds at
            // `input tap`); the companion's Kotlin side truncates a fractional
            // coordinate toward zero, which would otherwise bias top-left.
            .map(|p| GesturePoint {
                x: p.x.round(),
                y: p.y.round(),
            })
            .collect();
        match runtime
            .client
            .gesture(kind, gesture_points, duration_ms)
            .await
        {
            Ok(result) => Ok(result.dispatched),
            Err(error) => {
                if error.is_fallback() {
                    // The companion is no longer reachable; drop it and mark the
                    // profile stale so coordinate actions fail closed until a
                    // refresh or reconnect re-proves gesture dispatch.
                    entry.companion = None;
                    self.invalidate_companion(&ctx.session_id);
                }
                Err(DiagnosticEntry {
                    code: error.code().to_string(),
                    message: format!("companion gesture failed: {error}"),
                    details: None,
                })
            }
        }
    }

    /// Animate the phone-side agent overlay for one coordinate action, best-effort.
    ///
    /// Visual only: it draws a tap ripple or a swipe/drag trail and pulses the
    /// edge glow on the device; it never dispatches real input (that already
    /// happened via the companion `gesture`). A session with no reachable
    /// companion is a no-op. A transport failure drops the companion runtime and
    /// marks the profile stale, mirroring `companion_gesture`; a per-method error
    /// is swallowed (the action already succeeded — only the cosmetic overlay is
    /// unavailable).
    async fn animate_overlay_gesture(&mut self, session_id: &str, spec: OverlayGestureSpec) {
        // The per-action overlay draw is one of the companion visible-overlay
        // calls; with the visible overlay disabled in config the host never issues
        // it (the action's real input already dispatched). Default is enabled, so
        // this is a no-op unless the operator opted out.
        if !self.selection.visible_overlay {
            return;
        }
        let Some(entry) = self.sessions.get_mut(session_id) else {
            return;
        };
        let Some(runtime) = entry.companion.as_mut() else {
            return;
        };
        let points = spec
            .points
            .iter()
            // Match the real-input rounding so the cosmetic overlay trail lands
            // on the same device pixels as the dispatched gesture (see
            // `companion_gesture`).
            .map(|p| GesturePoint {
                x: p.x.round(),
                y: p.y.round(),
            })
            .collect();
        if let Err(error) = runtime
            .client
            .overlay_gesture(spec.kind, points, spec.duration_ms)
            .await
            && error.is_fallback()
        {
            entry.companion = None;
            self.invalidate_companion(session_id);
        }
    }

    /// Compute the device-pixel point for an action. With `use_device_coordinates`
    /// the caller-supplied point is taken verbatim (bounds-checked against the
    /// device size); otherwise a fresh `phone_snapshot_id` is required and the
    /// screenshot-plane point is translated through that snapshot's mapping.
    fn device_point_for(
        &self,
        ctx: &ActionContext,
        snapshot_id: Option<&str>,
        x: f64,
        y: f64,
        use_device_coordinates: bool,
    ) -> Result<PhonePoint, DiagnosticEntry> {
        if use_device_coordinates {
            validate_device_point(&ctx.profile, x, y)?;
            return Ok(PhonePoint { x, y });
        }
        let Some(snapshot_id) = snapshot_id else {
            return Err(DiagnosticEntry {
                code: "PhoneSnapshotRequired".to_string(),
                message:
                    "coordinate actions require a fresh phone_snapshot_id (or use_device_coordinates)"
                        .to_string(),
                details: None,
            });
        };
        let entry = self
            .sessions
            .get(&ctx.session_id)
            .ok_or_else(no_companion_diagnostic)?;
        let record = entry
            .snapshots
            .resolve(snapshot_id, &ctx.session_id, &ctx.serial, now_ms())
            .map_err(|error| DiagnosticEntry {
                code: error.code().to_string(),
                message: format!("snapshot rejected: {error}"),
                details: None,
            })?;
        // Reject a snapshot whose captured device size no longer matches the
        // profile's current rotation-adjusted screenshot extent: the device
        // rotated or resized after capture, so the snapshot's coordinate mapping
        // would land in the wrong place. A clean width/height swap is an
        // orientation flip; any other mismatch is a resolution change. Skipped
        // when the profile's display size is unknown, to avoid false rejects.
        if let Some(current) = Self::expected_capture_size(&ctx.profile).as_ref() {
            let captured = &record.device_size;
            if captured != current {
                let error = if captured.width == current.height && captured.height == current.width
                {
                    snapshot::SnapshotError::OrientationMismatch {
                        captured: captured.clone(),
                        current: current.clone(),
                    }
                } else {
                    snapshot::SnapshotError::ResolutionMismatch {
                        captured: captured.clone(),
                        current: current.clone(),
                    }
                };
                return Err(DiagnosticEntry {
                    code: error.code().to_string(),
                    message: format!("snapshot rejected: {error}"),
                    details: None,
                });
            }
        }
        // Rebuild the mapping from the record's geometry so out-of-bounds points
        // are rejected by the mapping lane rather than dispatched to the device.
        // `screenshot_size` may be smaller than `device_size` when the model was
        // handed a downscaled capture; `build_mapping` scales through that ratio
        // (it degenerates to the identity case when the two sizes match).
        let mapping = mapping::build_mapping(&mapping::MappingBuild {
            mapping_id: &record.mapping_id,
            session_id: &ctx.session_id,
            serial: &ctx.serial,
            device_size: record.device_size.clone(),
            screenshot_size: record.screenshot_size.clone(),
            rotation_degrees: record.rotation_degrees,
            host_window_rect: None,
            host_content_rect: None,
            captured_at_ms: record.captured_at_ms,
        })
        .map_err(|error| DiagnosticEntry {
            code: error.code().to_string(),
            message: format!("snapshot mapping invalid: {error}"),
            details: None,
        })?;
        mapping::screenshot_to_device(&mapping, PhonePoint { x, y }).map_err(|error| {
            DiagnosticEntry {
                code: error.code().to_string(),
                message: format!("coordinate translation failed: {error}"),
                details: None,
            }
        })
    }

    /// Finalize an action: build the response, on success update the cursor
    /// tracker for this session (never on failure, never across serials), and —
    /// when the session's companion is reachable — animate the phone-side agent
    /// overlay for the action.
    #[allow(clippy::too_many_arguments)]
    async fn finish_action(
        &mut self,
        ctx: &ActionContext,
        action: &str,
        backend: PhoneBackendKind,
        snapshot_id: Option<String>,
        device_point: Option<PhonePoint>,
        screenshot_point: Option<PhonePoint>,
        overlay_gesture: Option<OverlayGestureSpec>,
        result: Result<bool, DiagnosticEntry>,
    ) -> PhoneActionResponse {
        let mut diagnostics = Vec::new();
        let success = match result {
            Ok(success) => {
                if !success {
                    diagnostics.push(DiagnosticEntry {
                        code: "PhoneActionRejected".to_string(),
                        message: format!("{action} dispatched but the backend reported failure"),
                        details: None,
                    });
                }
                success
            }
            Err(diag) => {
                diagnostics.push(diag);
                false
            }
        };

        let cursor = if success {
            self.sessions.get_mut(&ctx.session_id).and_then(|entry| {
                entry
                    .cursor
                    .record_action(
                        &ctx.session_id,
                        &ctx.serial,
                        action,
                        snapshot_id.as_deref(),
                        device_point,
                        screenshot_point,
                        now_ms(),
                    )
                    .ok()
            })
        } else {
            None
        };

        // Animate the agent cursor on the device for a successful coordinate
        // action whenever the companion is reachable to draw it. Companion-owned
        // coordinate dispatch means this visual feedback is coupled to the real
        // input path; this extra overlay call remains best-effort, so a visual
        // failure never changes the action result.
        if success && let Some(spec) = overlay_gesture {
            self.animate_overlay_gesture(&ctx.session_id, spec).await;
        }

        PhoneActionResponse {
            session_id: ctx.session_id.clone(),
            serial: ctx.serial.clone(),
            action: action.to_string(),
            backend: if success {
                backend
            } else {
                PhoneBackendKind::None
            },
            capability_profile_id: ctx.profile.profile_id.clone(),
            profile_refresh_state: ctx.profile.refresh_state,
            phone_snapshot_id: snapshot_id,
            cursor,
            diagnostics,
        }
    }

    // ===================================================================
    // Backend selection
    // ===================================================================

    /// Backend for a coordinate action: companion gestures when proven and
    /// reachable on a fresh profile. ADB is intentionally not a coordinate
    /// fallback because the companion owns both real input and visible feedback.
    pub(super) fn coordinate_backend(
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
    pub(super) fn screenshot_backend(&self, profile: &PhoneCapabilityProfile) -> PhoneBackendKind {
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
    pub(super) fn cursor_capabilities(
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
    pub(super) fn invalidate_companion(&mut self, session_id: &str) {
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

fn validate_device_point(
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
pub(super) fn populate_actions(
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

fn route_operations(action: &str) -> Vec<String> {
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

fn operation_provider(operation: &str, backend: PhoneBackendKind) -> PhoneOperationProvider {
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

fn operation_activation(operation: &str, provider: PhoneOperationProvider) -> PhoneActivationClass {
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
fn action_no_session(
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
fn action_failure(
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

fn direct_error_diagnostic(error: DirectRuntimeError) -> DiagnosticEntry {
    let message = format!("CompanionDirect dispatch failed: {error:?}");
    DiagnosticEntry {
        code: "PhoneCompanionDirectDispatchFailed".to_string(),
        message,
        details: None,
    }
}

fn companion_required_diagnostic() -> DiagnosticEntry {
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

    fn profile_with(companion: PhoneCompanionCapabilities) -> PhoneCapabilityProfile {
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

    fn caps(companion: bool) -> PhoneBackendCapabilities {
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
    fn populate_actions_routes_tap_to_companion_when_gesture_proven() {
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
    fn populate_actions_gates_coordinates_without_companion() {
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
