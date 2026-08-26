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
    DiagnosticEntry, PhoneActionResponse, PhoneBackendKind, PhonePoint, PhonePressKeyRequest,
    PhoneSwipeRequest, PhoneTapRequest, PhoneTypeTextRequest,
};

pub(crate) use super::routing_backend::{
    action_failure, action_no_session, companion_required_diagnostic, direct_error_diagnostic,
    populate_actions, validate_device_point,
};
use super::{ActionContext, PhoneManager, no_companion_diagnostic, now_ms};
use crate::phone::adb;
use crate::phone::mapping;
use crate::phone::protocol::{GestureKind, GesturePoint};
use crate::phone::snapshot;

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
        // CompanionDirect is first-class: dispatch the visual `overlay_gesture`
        // over ws when the session is a direct link, not only for the legacy
        // `adb forward` runtime. Reuse the canonical helper so identity never drifts.
        let direct = self.direct_identity(session_id);
        if let Some((device_id, epoch)) = direct {
            // Only animate when the profile actually advertises a phone-native overlay;
            // mirrors the legacy `entry.companion` reachable gate and keeps
            // capability-gated tests from seeing a spurious `overlay_gesture`.
            let capable = self
                .sessions
                .get(session_id)
                .is_some_and(|e| e.session.capabilities.phone_native_overlay);
            if !capable {
                return;
            }
            let Some(provider) = self.direct_provider.clone() else {
                return;
            };
            let points = spec
                .points
                .iter()
                .map(|p| serde_json::json!({"x": p.x.round() as i64, "y": p.y.round() as i64}))
                .collect::<Vec<_>>();
            let result = provider
                .dispatch(
                    &device_id,
                    epoch,
                    "overlay_gesture",
                    serde_json::json!({"kind": spec.kind, "points": points, "duration_ms": spec.duration_ms}),
                    false,
                    std::time::Duration::from_secs(5),
                )
                .await;
            if let Err(error) = result
                && super::helpers::is_direct_disconnected(&error)
            {
                if let Some(entry) = self.sessions.get_mut(session_id) {
                    entry.overlay_active = false;
                }
                self.invalidate_companion(session_id);
            }
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
}
