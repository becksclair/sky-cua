//! Screenshot capture and the `phone_observe` aggregation.
//!
//! Capture routes companion-first (its on-device screenshot carries native
//! overlay metadata) then ADB `screencap`, decodes the frame exactly once,
//! composites the screenshot-synthetic cursor when a fresh cursor exists (never
//! when the native overlay already captured it), and — when the model wants
//! inline image data — downscales and re-encodes it through the same
//! model-screenshot knobs the desktop and browser capture lanes honor. Decode,
//! composite, and encode all run off the async executor via
//! `tokio::task::spawn_blocking`. Coordinate actions resolve through the
//! delivered (possibly downscaled) image plane, never blindly against device
//! pixels, so a `phone_tap` on a downscaled snapshot still lands on the right
//! device pixel. `phone_observe` stitches a capture together with the current
//! app, an optional accessibility summary and recent notifications, the
//! cursor, and the dynamic action menu.

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneBackendKind, PhoneCapabilityProfile, PhoneResponse,
    PhoneScreenshotRequest, PhoneScreenshotResponse, PixelSize,
};

use super::capture_screenshot::{
    assemble_capture, decode_capture_blocking, write_phone_capture_file,
};
use super::{ActionContext, PhoneManager, now_ms};
use crate::phone::mapping;
use crate::phone::snapshot;

impl PhoneManager {
    // ===================================================================
    // Screenshot / observe
    // ===================================================================

    /// `phone_screenshot`: capture through the preferred backend, mint+register a
    /// snapshot, composite the synthetic cursor when a fresh cursor exists, and
    /// return device size + coordinate mapping + cursor capabilities.
    pub(super) async fn screenshot(&mut self, request: PhoneScreenshotRequest) -> PhoneResponse {
        let Some(ctx) = self.fresh_action_context(&request.session).await else {
            return PhoneResponse::Status(self.status(false).await);
        };
        match self
            .capture(&ctx, request.include_image_data, request.backend)
            .await
        {
            Ok(response) => PhoneResponse::Screenshot(response),
            Err(diag) => PhoneResponse::Screenshot(self.screenshot_failure(&ctx, diag)),
        }
    }

    /// Capture a frame and assemble a [`PhoneScreenshotResponse`]. ADB
    /// `screencap` is the baseline; the companion screenshot API is preferred when
    /// its capability is proven (it carries native-overlay metadata).
    pub(crate) async fn capture(
        &mut self,
        ctx: &ActionContext,
        include_image: bool,
        requested_backend: Option<PhoneBackendKind>,
    ) -> Result<PhoneScreenshotResponse, DiagnosticEntry> {
        let backend = self.screenshot_backend_for(&ctx.profile, requested_backend)?;
        // Diagnostics accumulated while capturing (e.g. a classified companion
        // screenshot failure that forced an ADB fallback). The plan requires
        // the fallback to be honest about why the preferred backend was not used
        // instead of silently returning the ADB image as if nothing happened.
        let mut diagnostics: Vec<DiagnosticEntry> = Vec::new();
        let (mut png, width, height, contains_native_overlay, backend_used, decoded_image) =
            match backend {
                PhoneBackendKind::Companion => match self.companion_screenshot(ctx).await {
                    Ok((bytes, w, h, native, used)) => (bytes, w, h, native, used, None),
                    Err(diag) => {
                        // Companion screenshot failed; record the classified
                        // reason, then fall back to ADB screencap.
                        diagnostics.push(diag);
                        let raw = self.adb_screencap(ctx).await?;
                        let (png, image) = decode_capture_blocking(raw).await?;
                        let (w, h) = (image.width(), image.height());
                        (png, w, h, false, PhoneBackendKind::Adb, Some(image))
                    }
                },
                _ => {
                    let raw = self.adb_screencap(ctx).await?;
                    let (png, image) = decode_capture_blocking(raw).await?;
                    let (w, h) = (image.width(), image.height());
                    (png, w, h, false, PhoneBackendKind::Adb, Some(image))
                }
            };

        let device_size = PixelSize { width, height };
        // Drift guard: a live orientation flip or resolution change shows up as a
        // freshly-captured frame whose dimensions differ from the profile's
        // recorded display_size. Mark the cached profile stale and fail closed so
        // the agent refreshes capabilities instead of receiving a snapshot id that
        // the next coordinate action must reject.
        if self.mark_profile_stale_for_drift(&ctx.session_id, &device_size) {
            return Err(DiagnosticEntry {
                code: "PhoneCapabilityProfileDrifted".to_string(),
                message: "captured frame dimensions no longer match the cached phone capability profile; refresh capabilities before acting".to_string(),
                details: PhoneManager::expected_capture_size(&ctx.profile).map(|expected| {
                    format!(
                        "expected={}x{}, captured={}x{}",
                        expected.width, expected.height, device_size.width, device_size.height
                    )
                }),
            });
        }
        let captured_at = now_ms();
        let snapshot_id = snapshot::mint(&ctx.serial, captured_at);

        // Composite the synthetic cursor when one is live for this session and
        // the native overlay did not already capture it (never double-composite),
        // then — when the model wants inline image data — downscale and re-encode
        // through the shared model-screenshot knobs. Both are CPU-bound, so they
        // run together off the async executor; the adb/companion round trip above
        // is the only I/O and stays async.
        let cursor_state = self
            .sessions
            .get(&ctx.session_id)
            .and_then(|entry| entry.cursor.current(captured_at));
        let cursor_point = if self.selection.screenshot_cursor && !contains_native_overlay {
            self.sessions
                .get(&ctx.session_id)
                .and_then(|entry| entry.cursor.screenshot_point(captured_at))
        } else {
            None
        };

        let serial_owned = ctx.serial.clone();
        let snapshot_id_owned = snapshot_id.clone();
        let device_size_for_blocking = device_size.clone();
        let assembly = tokio::task::spawn_blocking(move || {
            assemble_capture(
                png,
                decoded_image,
                cursor_point,
                include_image,
                device_size_for_blocking,
                &serial_owned,
                &snapshot_id_owned,
            )
        })
        .await
        .map_err(|join_error| DiagnosticEntry {
            code: "PhoneScreenshotAssemblyFailed".to_string(),
            message: format!(
                "phone screenshot compositing/encoding task failed to join cleanly: {join_error}"
            ),
            details: None,
        })?;
        png = assembly.png;
        diagnostics.extend(assembly.diagnostics);

        // The mapping for a 1:1 ADB screencap is identity; a downscaled model
        // delivery instead needs `screenshot_size` distinct from `device_size` so
        // `screenshot_to_device` scales coordinate actions back to the real
        // device pixel, not the smaller delivered plane.
        let mapping_id = format!("{snapshot_id}-map");
        let mapping = mapping::build_mapping(&mapping::MappingBuild {
            mapping_id: &mapping_id,
            session_id: &ctx.session_id,
            serial: &ctx.serial,
            device_size: device_size.clone(),
            screenshot_size: assembly.delivered_size,
            rotation_degrees: 0,
            host_window_rect: None,
            host_content_rect: None,
            captured_at_ms: captured_at,
        })
        .map_err(|error| DiagnosticEntry {
            code: error.code().to_string(),
            message: format!("phone capture produced an invalid coordinate mapping: {error}"),
            details: None,
        })?;

        // Register the snapshot so a later coordinate action can resolve it.
        if let Some(entry) = self.sessions.get_mut(&ctx.session_id) {
            let record = snapshot::record_from_mapping(
                &snapshot_id,
                backend_used,
                device_size.clone(),
                &mapping,
            );
            entry.snapshots.register(record);
        }

        let cursor_caps = self.cursor_capabilities(&ctx.profile);
        let inline_image = assembly.inline_image;
        let screenshot_path = if include_image {
            None
        } else {
            match write_phone_capture_file(&ctx.serial, &png) {
                Ok(path) => Some(path.display().to_string()),
                Err(message) => {
                    diagnostics.push(DiagnosticEntry {
                        code: "PhoneScreenshotDegraded".to_string(),
                        message: format!(
                            "phone screenshot could not be persisted to disk; no path-backed capture can be delivered in path-only mode: {message}"
                        ),
                        details: None,
                    });
                    None
                }
            }
        };

        Ok(PhoneScreenshotResponse {
            session_id: ctx.session_id.clone(),
            serial: ctx.serial.clone(),
            phone_snapshot_id: snapshot_id,
            backend: backend_used,
            capability_profile_id: ctx.profile.profile_id.clone(),
            profile_refresh_state: ctx.profile.refresh_state,
            screenshot_path,
            inline_image,
            device_size,
            coordinate_mapping: mapping,
            cursor: cursor_state,
            cursor_capabilities: cursor_caps,
            capture_contains_native_overlay: contains_native_overlay,
            diagnostics,
        })
    }

    fn screenshot_backend_for(
        &self,
        profile: &PhoneCapabilityProfile,
        requested: Option<PhoneBackendKind>,
    ) -> Result<PhoneBackendKind, DiagnosticEntry> {
        match requested.unwrap_or(PhoneBackendKind::Auto) {
            PhoneBackendKind::Auto | PhoneBackendKind::None => Ok(self.screenshot_backend(profile)),
            PhoneBackendKind::Adb => Ok(PhoneBackendKind::Adb),
            PhoneBackendKind::Companion => {
                if !profile.stale && profile.companion.rpc_reachable && profile.companion.screenshot
                {
                    Ok(PhoneBackendKind::Companion)
                } else {
                    Err(DiagnosticEntry {
                        code: "PhoneBackendUnavailable".to_string(),
                        message:
                            "requested companion screenshot backend is not available for this session"
                                .to_string(),
                        details: None,
                    })
                }
            }
            PhoneBackendKind::Scrcpy => Err(DiagnosticEntry {
                code: "PhoneBackendUnavailable".to_string(),
                message: "scrcpy still-frame screenshots are not implemented in phone-use v1"
                    .to_string(),
                details: None,
            }),
        }
    }
}
