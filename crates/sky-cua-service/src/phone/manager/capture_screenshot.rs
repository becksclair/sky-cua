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

#![allow(clippy::empty_line_after_doc_comments)]
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::ImageFormat;
use sky_cua_platform::model::{
    DiagnosticEntry, PhoneBackendCapabilities, PhoneBackendKind, PhoneCapabilityProfile,
    PhoneCapabilityRefreshState, PhoneImage, PhoneObserveRequest, PhoneObserveResponse, PhonePoint,
    PhoneScreenshotResponse, PhoneSessionSelector, PixelSize,
};

use super::{ActionContext, PhoneManager, no_companion_diagnostic, now_ms, selector_ids};
use crate::phone::adb;
use crate::phone::cursor;
use crate::phone::mapping;

impl PhoneManager {
    pub(crate) async fn adb_screencap(
        &self,
        ctx: &ActionContext,
    ) -> Result<Vec<u8>, DiagnosticEntry> {
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
    /// falls back. Direct sessions dispatch over the authenticated
    /// `phone-control.v2` ws (`screenshot`); legacy ADB sessions use the
    /// `adb forward` RPC.
    pub(crate) async fn companion_screenshot(
        &mut self,
        ctx: &ActionContext,
    ) -> Result<(Vec<u8>, u32, u32, bool, PhoneBackendKind), DiagnosticEntry> {
        if let Some((device_id, epoch)) = self.direct_identity(&ctx.session_id)
            && let Some(provider) = self.direct_provider.clone()
        {
            match provider
                .dispatch(
                    &device_id,
                    epoch,
                    crate::phone::protocol::methods::SCREENSHOT,
                    serde_json::json!({"include_overlay": false}),
                    true,
                    std::time::Duration::from_secs(10),
                )
                .await
            {
                Ok(value) => {
                    let shot: crate::phone::protocol::ScreenshotResult =
                        serde_json::from_value(value).map_err(|error| DiagnosticEntry {
                            code: "PhoneCompanionScreenshotDecode".to_string(),
                            message: format!("CompanionDirect screenshot decode failed: {error}"),
                            details: None,
                        })?;
                    let bytes = BASE64
                        .decode(shot.data_base64.as_bytes())
                        .map_err(|error| DiagnosticEntry {
                            code: "PhoneCompanionScreenshotDecode".to_string(),
                            message: format!("companion screenshot base64 invalid: {error}"),
                            details: None,
                        })?;
                    return Ok((
                        bytes,
                        shot.width,
                        shot.height,
                        shot.contains_native_overlay,
                        PhoneBackendKind::Companion,
                    ));
                }
                Err(error) => {
                    if super::helpers::is_direct_disconnected(&error) {
                        self.invalidate_companion(&ctx.session_id);
                    }
                    return Err(DiagnosticEntry {
                        code: "PhoneCompanionDirectDispatchFailed".to_string(),
                        message: format!("CompanionDirect screenshot failed: {error:?}"),
                        details: None,
                    });
                }
            }
        }
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
    pub(crate) fn screenshot_failure(
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
    pub(crate) async fn observe(&mut self, request: PhoneObserveRequest) -> PhoneObserveResponse {
        // Resolve the session first, run the observe-only cache-invalidation
        // triggers (wireless drop), then build the action context so this observe
        // reports the freshly-marked freshness. The triggers run only here, never
        // per action, so the cost stays bounded.
        let Some(probe) = self.action_context(&request.session) else {
            return observe_no_session(&request.session);
        };
        self.invalidate_on_observe_triggers(&probe.session_id, &probe.serial)
            .await;

        let Some(ctx) = self.fresh_action_context(&request.session).await else {
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
        let mut appshot = None;
        if self.direct_identity(&ctx.session_id).is_some() {
            match self.direct_appshot(&ctx.session_id).await {
                Ok(value) => appshot = Some(Box::new(value)),
                Err(diag) => diagnostics.push(diag),
            }
        }
        let mut phone_snapshot_id = None;
        let mut screenshot_path = None;
        let mut inline_image = None;
        let mut cursor = None;
        let mut backend = PhoneBackendKind::None;

        if let Some(value) = appshot.as_ref() {
            phone_snapshot_id = Some(value.appshot_id.clone());
            screenshot_path = Some(value.image.content_id.clone());
            backend = PhoneBackendKind::Companion;
            if request.include_image_data {
                match tokio::fs::read(&value.image.content_id).await {
                    Ok(bytes) => {
                        let serial = ctx.serial.clone();
                        let snapshot_id = value.appshot_id.clone();
                        match tokio::task::spawn_blocking(move || {
                            let image = image::load_from_memory(&bytes)
                                .map_err(|error| error.to_string())?;
                            let device_size = PixelSize {
                                width: image.width(),
                                height: image.height(),
                            };
                            Ok::<_, String>(assemble_capture(
                                bytes,
                                Some(image),
                                None,
                                true,
                                device_size,
                                &serial,
                                &snapshot_id,
                            ))
                        })
                        .await
                        {
                            Ok(Ok(assembly)) => {
                                inline_image = assembly.inline_image;
                                diagnostics.extend(assembly.diagnostics);
                            }
                            Ok(Err(error)) => diagnostics.push(DiagnosticEntry {
                                code: "PhoneAppShotImageUnavailable".into(),
                                message: format!(
                                    "failed to decode CompanionDirect AppShot image: {error}"
                                ),
                                details: None,
                            }),
                            Err(error) => diagnostics.push(DiagnosticEntry {
                                code: "PhoneAppShotImageUnavailable".into(),
                                message: format!(
                                    "failed to prepare CompanionDirect AppShot image: {error}"
                                ),
                                details: None,
                            }),
                        }
                    }
                    Err(error) => diagnostics.push(DiagnosticEntry {
                        code: "PhoneAppShotImageUnavailable".into(),
                        message: format!("failed to read CompanionDirect AppShot image: {error}"),
                        details: None,
                    }),
                }
            }
        } else {
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

        if let Some(value) = appshot.as_ref() {
            self.appshots
                .insert(value.appshot_id.clone(), (**value).clone());
        }
        PhoneObserveResponse {
            session: session.clone(),
            appshot,
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

pub(crate) fn observe_actions(
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

/// Decode a captured PNG into pixels. A truncated or non-PNG payload (e.g. a
/// partial `screencap` over a flaky wireless link) must not become a
/// degenerate 0x0 "successful" screenshot; it is a structured capture failure
/// so the caller routes through [`PhoneManager::screenshot_failure`] instead of
/// registering a 0x0 snapshot.
///
/// This is the only decode of a given capture's bytes: callers that need the
/// pixels for compositing or model-image encoding reuse the returned image
/// instead of decoding again.
pub(crate) fn decode_capture(png: &[u8]) -> Result<image::DynamicImage, DiagnosticEntry> {
    match image::load_from_memory_with_format(png, ImageFormat::Png) {
        Ok(image) if image.width() > 0 && image.height() > 0 => Ok(image),
        _ => Err(DiagnosticEntry {
            code: "PhoneScreencapDecodeFailed".to_string(),
            message: "captured screenshot bytes did not decode as a non-empty PNG".to_string(),
            details: Some(format!("{} captured byte(s)", png.len())),
        }),
    }
}

/// Decode a capture off the async executor, handing the bytes back alongside
/// the decoded image so the caller does not need to keep a second copy around.
pub(crate) async fn decode_capture_blocking(
    png: Vec<u8>,
) -> Result<(Vec<u8>, image::DynamicImage), DiagnosticEntry> {
    match tokio::task::spawn_blocking(move || {
        let decoded = decode_capture(&png);
        (png, decoded)
    })
    .await
    {
        Ok((png, Ok(image))) => Ok((png, image)),
        Ok((_, Err(diagnostic))) => Err(diagnostic),
        Err(join_error) => Err(DiagnosticEntry {
            code: "PhoneScreencapDecodeFailed".to_string(),
            message: format!("phone screenshot decode task failed to join cleanly: {join_error}"),
            details: None,
        }),
    }
}

/// Composite the synthetic agent cursor into an already-decoded frame and
/// re-encode the result to PNG. Returns the composited image and its PNG bytes
/// on success so callers can reuse both for model delivery and on-disk
/// persistence; returns `None` on any compose/encode failure, leaving the
/// original capture untouched (never fabricate a corrupted composite).
pub(crate) fn composite_cursor(
    image: image::DynamicImage,
    point: PhonePoint,
) -> Option<(image::DynamicImage, Vec<u8>)> {
    let mut rgba = image.to_rgba8();
    if cursor::compose_synthetic_cursor(&mut rgba, point).is_err() {
        return None;
    }
    let composed = image::DynamicImage::ImageRgba8(rgba);
    let mut out = std::io::Cursor::new(Vec::new());
    if composed.write_to(&mut out, ImageFormat::Png).is_err() {
        return None;
    }
    Some((composed, out.into_inner()))
}

/// Result of the CPU-bound part of a capture: the (possibly composited) raw
/// PNG bytes, the inline model image when one was requested, the pixel size
/// of whatever plane the model actually saw, and any diagnostics collected
/// along the way.
pub(crate) struct CaptureAssembly {
    pub(crate) png: Vec<u8>,
    pub(crate) inline_image: Option<PhoneImage>,
    pub(crate) delivered_size: PixelSize,
    pub(crate) diagnostics: Vec<DiagnosticEntry>,
}

/// Composite the cursor (if a fresh one exists) and — when the model wants
/// inline image data — downscale and re-encode through the shared
/// model-screenshot knobs. Pure and synchronous so it can run inside
/// `spawn_blocking` without touching manager state.
pub(crate) fn assemble_capture(
    mut png: Vec<u8>,
    mut decoded_image: Option<image::DynamicImage>,
    cursor_point: Option<PhonePoint>,
    include_image: bool,
    device_size: PixelSize,
    serial: &str,
    snapshot_id: &str,
) -> CaptureAssembly {
    let mut diagnostics = Vec::new();

    if let Some(point) = cursor_point {
        let source = decoded_image.take().or_else(|| decode_capture(&png).ok());
        if let Some(source) = source
            && let Some((composed, encoded_png)) = composite_cursor(source, point)
        {
            png = encoded_png;
            decoded_image = Some(composed);
        }
    }

    if !include_image {
        return CaptureAssembly {
            png,
            inline_image: None,
            delivered_size: device_size,
            diagnostics,
        };
    }

    let (inline_image, delivered_size) = match decoded_image.or_else(|| decode_capture(&png).ok()) {
        Some(image) => {
            let (built, size, mut prep_diagnostics) =
                prepare_inline_image(serial, snapshot_id, image, device_size.clone(), &png);
            diagnostics.append(&mut prep_diagnostics);
            (built, size)
        }
        None => {
            // The frame decoded fine earlier in this same request (the ADB
            // integrity gate, or the compositing branch above) or never needed
            // decoding until now (a companion frame with no cursor to
            // composite); if it fails to decode here, degrade to the raw
            // full-resolution PNG rather than losing the capture.
            diagnostics.push(DiagnosticEntry {
                code: "PhoneScreenshotModelImageDegraded".to_string(),
                message: "phone screenshot could not be decoded for model-image downscaling; \
                          delivering the full-resolution PNG instead"
                    .to_string(),
                details: None,
            });
            (
                PhoneImage {
                    mime_type: "image/png".to_string(),
                    data_base64: BASE64.encode(&png),
                    width: Some(device_size.width),
                    height: Some(device_size.height),
                },
                device_size.clone(),
            )
        }
    };

    CaptureAssembly {
        png,
        inline_image: Some(inline_image),
        delivered_size,
        diagnostics,
    }
}

/// Downscale and re-encode a decoded capture for model delivery, reusing the
/// shared `sky-cua-capture` resize/encode logic (and its
/// `SKY_CUA_MODEL_SCREENSHOT_*` env knobs) so the phone lane never re-derives
/// its own resolution policy. A capture already within the model bounds is
/// encoded at native size (no upscale, no no-op resize).
///
/// The model image is written to a transient file (the shared encoder is
/// file-based) and read back into base64 immediately; the file is removed
/// right after so this delivery mode leaves nothing behind on disk.
pub(crate) fn prepare_inline_image(
    serial: &str,
    snapshot_id: &str,
    image: image::DynamicImage,
    device_size: PixelSize,
    png: &[u8],
) -> (PhoneImage, PixelSize, Vec<DiagnosticEntry>) {
    let dir = phone_model_captures_dir();
    let source_path = dir.join(format!("{snapshot_id}-device.png"));
    let prepared = sky_cua_capture::prepare_model_capture_from_image(
        &dir,
        snapshot_id,
        image,
        &source_path,
        Some(device_size.clone()),
    );
    match prepared {
        Ok(prepared) => match std::fs::read(&prepared.path) {
            Ok(bytes) => {
                let _ = std::fs::remove_file(&prepared.path);
                let pixel_size = prepared.pixel_size.unwrap_or_else(|| device_size.clone());
                let mime_type = match prepared.format {
                    sky_cua_capture::ModelScreenshotFormat::Jpeg => "image/jpeg",
                    sky_cua_capture::ModelScreenshotFormat::Webp => "image/webp",
                };
                (
                    PhoneImage {
                        mime_type: mime_type.to_string(),
                        data_base64: BASE64.encode(bytes),
                        width: Some(pixel_size.width),
                        height: Some(pixel_size.height),
                    },
                    pixel_size,
                    Vec::new(),
                )
            }
            Err(error) => degraded_full_resolution_image(
                png,
                device_size,
                format!(
                    "failed to read prepared model image for phone {serial} at {}: {error}",
                    prepared.path.display()
                ),
            ),
        },
        Err(error) => degraded_full_resolution_image(
            png,
            device_size,
            format!(
                "model image preparation failed for phone {serial}: {}",
                error.message
            ),
        ),
    }
}

/// Fallback delivery when model-image preparation fails: the full-resolution
/// PNG rides as-is, with a non-fatal diagnostic explaining the degrade. Mirrors
/// the browser capture lane's degrade-instead-of-fail contract.
pub(crate) fn degraded_full_resolution_image(
    png: &[u8],
    device_size: PixelSize,
    message: String,
) -> (PhoneImage, PixelSize, Vec<DiagnosticEntry>) {
    (
        PhoneImage {
            mime_type: "image/png".to_string(),
            data_base64: BASE64.encode(png),
            width: Some(device_size.width),
            height: Some(device_size.height),
        },
        device_size,
        vec![DiagnosticEntry {
            code: "PhoneScreenshotModelImageDegraded".to_string(),
            message: format!(
                "phone model image downscale failed; delivering full-resolution PNG: {message}"
            ),
            details: None,
        }],
    )
}

pub(crate) fn phone_model_captures_dir() -> PathBuf {
    phone_captures_dir().join("model")
}

pub(crate) fn write_phone_capture_file(serial: &str, png: &[u8]) -> Result<PathBuf, String> {
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

pub(crate) fn phone_captures_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("sky-cua")
        .join("captures")
}

pub(crate) fn sanitize_capture_id(value: &str) -> String {
    let slug: String = value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    slug.chars().take(32).collect()
}

pub(crate) fn prune_phone_captures(
    dir: &std::path::Path,
    serial_slug: &str,
    just_written: &std::path::Path,
) {
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

pub(crate) fn observe_no_session(selector: &PhoneSessionSelector) -> PhoneObserveResponse {
    let (session_id, serial) = selector_ids(selector);
    PhoneObserveResponse {
        session: empty_session(&session_id, &serial),
        appshot: None,
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
pub(crate) fn empty_session(
    session_id: &str,
    serial: &str,
) -> sky_cua_platform::model::PhoneSession {
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
        routes: Vec::new(),
    };
    sky_cua_platform::model::PhoneSession {
        session_id: session_id.to_string(),
        serial: serial.to_string(),
        connection: None,
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

pub(crate) fn empty_backend_caps() -> PhoneBackendCapabilities {
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
