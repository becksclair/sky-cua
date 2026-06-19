//! Screenshot capture and the `phone_observe` aggregation.
//!
//! Capture routes companion-first (its on-device screenshot carries native
//! overlay metadata) then ADB `screencap`, mints and registers a snapshot,
//! composites the screenshot-synthetic cursor when a fresh cursor exists (never
//! when the native overlay already captured it), and returns the device size,
//! coordinate mapping, and cursor capabilities. `phone_observe` stitches a
//! capture together with the current app, an optional accessibility summary and
//! recent notifications, the cursor, and the dynamic action menu.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::ImageFormat;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneBackendCapabilities, PhoneBackendKind, PhoneCapabilityProfile,
    PhoneCapabilityRefreshState, PhoneImage, PhoneObserveRequest, PhoneObserveResponse, PhonePoint,
    PhoneResponse, PhoneScreenshotRequest, PhoneScreenshotResponse, PhoneSessionSelector,
    PixelSize,
};

use super::{ActionContext, PhoneManager, no_companion_diagnostic, now_ms, selector_ids};
use crate::phone::adb;
use crate::phone::cursor;
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
        let Some(ctx) = self.action_context(&request.session) else {
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
    async fn capture(
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
        let (mut png, width, height, contains_native_overlay, backend_used) = match backend {
            PhoneBackendKind::Companion => match self.companion_screenshot(ctx).await {
                Ok(shot) => shot,
                Err(diag) => {
                    // Companion screenshot failed; record the classified reason,
                    // then fall back to ADB screencap.
                    diagnostics.push(diag);
                    let png = self.adb_screencap(ctx).await?;
                    let (w, h) = decode_png_dimensions(&png)?;
                    (png, w, h, false, PhoneBackendKind::Adb)
                }
            },
            _ => {
                let png = self.adb_screencap(ctx).await?;
                let (w, h) = decode_png_dimensions(&png)?;
                (png, w, h, false, PhoneBackendKind::Adb)
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
        let mapping = mapping::identity_mapping(
            &format!("{snapshot_id}-map"),
            &ctx.session_id,
            &ctx.serial,
            device_size.clone(),
            captured_at,
        );

        // Composite the synthetic cursor when one is live for this session and
        // the native overlay did not already capture it. Never double-composite.
        let cursor_state = self
            .sessions
            .get(&ctx.session_id)
            .and_then(|entry| entry.cursor.current(captured_at));
        if self.selection.screenshot_cursor
            && !contains_native_overlay
            && let Some(point) = self
                .sessions
                .get(&ctx.session_id)
                .and_then(|entry| entry.cursor.screenshot_point(captured_at))
        {
            composite_cursor(&mut png, point);
        }

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
        let inline_image = include_image.then(|| PhoneImage {
            mime_type: "image/png".to_string(),
            data_base64: BASE64.encode(&png),
            width: Some(width),
            height: Some(height),
        });
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

    /// ADB `exec-out screencap -p`, mapped to a structured diagnostic on failure.
    async fn adb_screencap(&self, ctx: &ActionContext) -> Result<Vec<u8>, DiagnosticEntry> {
        adb::screencap_png(
            self.runner.as_ref(),
            self.configured_adb_path(),
            &ctx.serial,
        )
        .await
        .map_err(|error| adb::command_error_diagnostic("adb exec-out screencap -p", &error))
    }

    /// Companion screenshot RPC, decoded to raw PNG bytes plus native-overlay
    /// metadata. A transport failure drops the companion runtime so later routing
    /// falls back.
    async fn companion_screenshot(
        &mut self,
        ctx: &ActionContext,
    ) -> Result<(Vec<u8>, u32, u32, bool, PhoneBackendKind), DiagnosticEntry> {
        let Some(entry) = self.sessions.get_mut(&ctx.session_id) else {
            return Err(no_companion_diagnostic());
        };
        let Some(runtime) = entry.companion.as_mut() else {
            return Err(no_companion_diagnostic());
        };
        match runtime.client.screenshot(false).await {
            Ok(shot) => {
                let bytes = BASE64
                    .decode(shot.data_base64.as_bytes())
                    .map_err(|error| DiagnosticEntry {
                        code: "PhoneCompanionScreenshotDecode".to_string(),
                        message: format!("companion screenshot base64 invalid: {error}"),
                        details: None,
                    })?;
                Ok((
                    bytes,
                    shot.width,
                    shot.height,
                    shot.contains_native_overlay,
                    PhoneBackendKind::Companion,
                ))
            }
            Err(error) => {
                if error.is_fallback() {
                    entry.companion = None;
                    self.invalidate_companion(&ctx.session_id);
                }
                Err(DiagnosticEntry {
                    code: error.code().to_string(),
                    message: format!("companion screenshot failed: {error}"),
                    details: None,
                })
            }
        }
    }

    /// A screenshot-failure response that still carries an honest device mapping
    /// and the failure diagnostic instead of a fabricated image.
    fn screenshot_failure(
        &self,
        ctx: &ActionContext,
        diagnostic: DiagnosticEntry,
    ) -> PhoneScreenshotResponse {
        let device_size = ctx.profile.display_size.clone().unwrap_or(PixelSize {
            width: 0,
            height: 0,
        });
        let mapping = mapping::identity_mapping(
            "phone-screenshot-failed",
            &ctx.session_id,
            &ctx.serial,
            device_size.clone(),
            now_ms(),
        );
        PhoneScreenshotResponse {
            session_id: ctx.session_id.clone(),
            serial: ctx.serial.clone(),
            phone_snapshot_id: String::new(),
            backend: PhoneBackendKind::None,
            capability_profile_id: ctx.profile.profile_id.clone(),
            profile_refresh_state: ctx.profile.refresh_state,
            screenshot_path: None,
            inline_image: None,
            device_size,
            coordinate_mapping: mapping,
            cursor: None,
            cursor_capabilities: self.cursor_capabilities(&ctx.profile),
            capture_contains_native_overlay: false,
            diagnostics: vec![diagnostic],
        }
    }

    /// `phone_observe`: the primary perception tool. Aggregates a screenshot +
    /// snapshot id + current app + accessibility summary (when requested and
    /// available) + recent notifications (when requested) + cursor + the dynamic
    /// available/unavailable action list, all stamped with the backend and
    /// profile id in force.
    pub(super) async fn observe(&mut self, request: PhoneObserveRequest) -> PhoneObserveResponse {
        // Resolve the session first, run the observe-only cache-invalidation
        // triggers (wireless drop), then build the action context so this observe
        // reports the freshly-marked freshness. The triggers run only here, never
        // per action, so the cost stays bounded.
        let Some(probe) = self.action_context(&request.session) else {
            return observe_no_session(&request.session);
        };
        self.invalidate_on_observe_triggers(&probe.session_id, &probe.serial)
            .await;

        let Some(ctx) = self.action_context(&request.session) else {
            return observe_no_session(&request.session);
        };
        let session = self
            .sessions
            .get(&ctx.session_id)
            .map(|entry| entry.session.clone());
        let Some(session) = session else {
            return observe_no_session(&request.session);
        };

        let mut diagnostics = Vec::new();
        let mut phone_snapshot_id = None;
        let mut screenshot_path = None;
        let mut inline_image = None;
        let mut cursor = None;
        let mut backend = PhoneBackendKind::None;

        match self
            .capture(&ctx, request.include_image_data, request.backend)
            .await
        {
            Ok(shot) => {
                phone_snapshot_id = Some(shot.phone_snapshot_id);
                screenshot_path = shot.screenshot_path;
                inline_image = shot.inline_image;
                cursor = shot.cursor;
                backend = shot.backend;
                diagnostics.extend(shot.diagnostics);
            }
            Err(diag) => diagnostics.push(diag),
        }

        let current_app = match self.current_app_info(&ctx.session_id, &ctx.serial).await {
            Ok(app) => app,
            Err(diag) => {
                diagnostics.push(diag);
                None
            }
        };

        let accessibility_summary = if request.include_accessibility {
            self.accessibility_summary(&ctx.session_id).await
        } else {
            None
        };
        let recent_notifications = if request.include_notifications {
            self.recent_notifications(&ctx.session_id).await
        } else {
            Vec::new()
        };

        PhoneObserveResponse {
            session: session.clone(),
            phone_snapshot_id,
            screenshot_path,
            inline_image,
            current_app,
            accessibility_summary,
            recent_notifications,
            cursor,
            backend,
            capability_profile_id: ctx.profile.profile_id.clone(),
            profile_refresh_state: ctx.profile.refresh_state,
            available_actions: observe_actions(
                &ctx.profile,
                self.backend_capabilities(&ctx.profile),
            )
            .0,
            unavailable_actions: observe_actions(
                &ctx.profile,
                self.backend_capabilities(&ctx.profile),
            )
            .1,
            diagnostics,
        }
    }
}

fn observe_actions(
    profile: &PhoneCapabilityProfile,
    caps: PhoneBackendCapabilities,
) -> (
    Vec<sky_cua_platform::model::PhoneAvailableAction>,
    Vec<sky_cua_platform::model::PhoneUnavailableAction>,
) {
    let mut profile = profile.clone();
    super::routing::populate_actions(&mut profile, &caps);
    (profile.available_actions, profile.unavailable_actions)
}

/// Decode the dimensions of a captured PNG. A truncated or non-PNG payload
/// (e.g. a partial `screencap` over a flaky wireless link) must not become a
/// degenerate 0x0 "successful" screenshot; it is a structured capture failure so
/// the caller routes through [`PhoneManager::screenshot_failure`] instead of
/// registering a 0x0 snapshot.
fn decode_png_dimensions(png: &[u8]) -> Result<(u32, u32), DiagnosticEntry> {
    match image::load_from_memory_with_format(png, ImageFormat::Png) {
        Ok(image) if image.width() > 0 && image.height() > 0 => Ok((image.width(), image.height())),
        _ => Err(DiagnosticEntry {
            code: "PhoneScreencapDecodeFailed".to_string(),
            message: "captured screenshot bytes did not decode as a non-empty PNG".to_string(),
            details: Some(format!("{} captured byte(s)", png.len())),
        }),
    }
}

/// Composite the synthetic agent cursor into a PNG in memory, re-encoding the
/// result. A composition error leaves the original bytes untouched.
fn composite_cursor(png: &mut Vec<u8>, point: PhonePoint) {
    let Ok(image) = image::load_from_memory_with_format(png, ImageFormat::Png) else {
        return;
    };
    let mut rgba = image.to_rgba8();
    if cursor::compose_synthetic_cursor(&mut rgba, point).is_err() {
        return;
    }
    let mut out = std::io::Cursor::new(Vec::new());
    if image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut out, ImageFormat::Png)
        .is_ok()
    {
        *png = out.into_inner();
    }
}

fn write_phone_capture_file(serial: &str, png: &[u8]) -> Result<PathBuf, String> {
    let dir = phone_captures_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create {}: {error}", dir.display()))?;

    let serial_slug = sanitize_capture_id(serial);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    static CAPTURE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = CAPTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 10_000;
    let path = dir.join(format!("phone-{serial_slug}-{millis}-{sequence:04}.png"));

    let mut file = std::fs::File::create(&path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))?;
    file.write_all(png)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;

    prune_phone_captures(&dir, &serial_slug, &path);
    Ok(path)
}

fn phone_captures_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("sky-cua")
        .join("captures")
}

fn sanitize_capture_id(value: &str) -> String {
    let slug: String = value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    slug.chars().take(32).collect()
}

fn prune_phone_captures(dir: &std::path::Path, serial_slug: &str, just_written: &std::path::Path) {
    const KEPT_CAPTURES_PER_SERIAL: usize = 8;
    let prefix = format!("phone-{serial_slug}-");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut matching: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path != just_written
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
        })
        .collect();
    matching.sort();
    if matching.len() >= KEPT_CAPTURES_PER_SERIAL {
        for stale in &matching[..=matching.len() - KEPT_CAPTURES_PER_SERIAL] {
            let _ = std::fs::remove_file(stale);
        }
    }
}

fn observe_no_session(selector: &PhoneSessionSelector) -> PhoneObserveResponse {
    let (session_id, serial) = selector_ids(selector);
    PhoneObserveResponse {
        session: empty_session(&session_id, &serial),
        phone_snapshot_id: None,
        screenshot_path: None,
        inline_image: None,
        current_app: None,
        accessibility_summary: None,
        recent_notifications: Vec::new(),
        cursor: None,
        backend: PhoneBackendKind::None,
        capability_profile_id: String::new(),
        profile_refresh_state: sky_cua_platform::model::PhoneCapabilityRefreshState::Stale,
        available_actions: Vec::new(),
        unavailable_actions: Vec::new(),
        diagnostics: vec![super::no_session_diagnostic(selector)],
    }
}

/// A placeholder session for the no-session observe path. Carries no capabilities
/// and an empty profile so nothing is fabricated.
fn empty_session(session_id: &str, serial: &str) -> sky_cua_platform::model::PhoneSession {
    use sky_cua_platform::model::{
        PhoneCompanionCapabilities, PhoneScrcpyCapabilities, PhoneTargetDeviceKind,
    };
    let profile = PhoneCapabilityProfile {
        profile_id: String::new(),
        session_id: session_id.to_string(),
        serial: serial.to_string(),
        detected_at_ms: 0,
        stale: true,
        refresh_state: PhoneCapabilityRefreshState::Stale,
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
        connection_kind: sky_cua_platform::model::PhoneConnectionKind::Unknown,
        companion: PhoneCompanionCapabilities::absent(""),
        scrcpy: PhoneScrcpyCapabilities::absent(),
        root_available: false,
        shizuku_available: false,
        device_owner: false,
        available_actions: Vec::new(),
        unavailable_actions: Vec::new(),
    };
    sky_cua_platform::model::PhoneSession {
        session_id: session_id.to_string(),
        serial: serial.to_string(),
        connection_kind: sky_cua_platform::model::PhoneConnectionKind::Unknown,
        backend: PhoneBackendKind::None,
        capabilities: empty_backend_caps(),
        capability_profile: profile,
        companion: None,
        managed_process: false,
        window_title: None,
        created_at_ms: 0,
    }
}

fn empty_backend_caps() -> PhoneBackendCapabilities {
    PhoneBackendCapabilities {
        adb: false,
        companion: false,
        scrcpy: false,
        screenshot: false,
        gestures: false,
        text_input: false,
        key_input: false,
        accessibility_tree: false,
        notifications: false,
        app_management: false,
        host_visible_overlay: false,
        screenshot_synthetic_cursor: false,
        phone_native_overlay: false,
    }
}
