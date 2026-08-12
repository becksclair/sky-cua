use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::Utc;
use sha2::{Digest, Sha256};
use sky_cua_platform::appshot_artifacts_dir;
use sky_cua_platform::model::{
    AppShotAccessibilityStatus, AppShotActionSnapshot, AppShotApplication, AppShotCapture,
    AppShotCaptureFlags, AppShotCaptureResult, AppShotConsistency, AppShotCoverage,
    AppShotEnvelope, AppShotImage, AppShotTrigger, AppStateSnapshot, CaptureScope,
    CaptureScreenMode, ContentPersistence, ContentRef, ContentSource, ElementNode,
    ModelImageFormat, SemanticBackendKind, ServiceResponse, WindowInfo, WindowTarget,
};

use super::*;

const APPSHOT_TTL: Duration = Duration::from_secs(60 * 60);
const MAX_AX_TEXT_BYTES: usize = 1_000_000;
const MAX_CLEANUP_ENTRIES: usize = 128;
const MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

impl ServiceDaemon {
    pub(super) async fn handle_appshot_capture(
        &self,
        request_id: String,
        target: Option<WindowTarget>,
        frontmost: bool,
        flags: AppShotCaptureFlags,
    ) -> ServiceResponse {
        if let Err(message) = validate_request_id(&request_id) {
            return error_response(BackendErrorCode::InvalidRequest.as_str(), message);
        }
        if target.is_some() == frontmost {
            return error_response(
                BackendErrorCode::InvalidRequest.as_str(),
                "appshot_capture requires exactly one target selector: target or frontmost=true",
            );
        }

        debug!(
            request_id,
            target = ?target,
            frontmost,
            include_ax_text = flags.include_ax_text,
            "handling appshot_capture request"
        );
        let deadline = tokio::time::Instant::now() + desktop_request_deadline();
        let capture_guard = Some(self.overlay.lock().await.prepare_for_capture());
        let capture = self
            .with_desktop_deadline_until(deadline, async {
                let resolved_window = if let Some(target) = target {
                    let exact = self.backend.resolve_window_target(&target).await?;
                    (
                        WindowTarget {
                            window_id: Some(exact.window_id.clone()),
                            ..Default::default()
                        },
                        exact,
                    )
                } else {
                    let window = self.backend.focused_window().await?.ok_or_else(|| {
                        BackendError::new(
                            BackendErrorCode::InvalidRequest,
                            "frontmost AppShot capture could not resolve a focused window",
                        )
                    })?;
                    (
                        WindowTarget {
                            window_id: Some(window.window_id.clone()),
                            ..Default::default()
                        },
                        window,
                    )
                };

                let mut snapshot = self
                    .backend
                    .screenshot(Some(resolved_window.0), None)
                    .await?;
                let capture = snapshot.capture.as_ref().ok_or_else(|| {
                    BackendError::new(
                        BackendErrorCode::Internal,
                        "AppShot capture did not return capture metadata",
                    )
                })?;
                if capture.capture_scope != CaptureScope::Window {
                    return Err(BackendError::new(
                        BackendErrorCode::CaptureBackendDowngraded,
                        format!(
                            "AppShot requires a target-window crop; backend returned {:?}",
                            capture.capture_scope
                        ),
                    ));
                }

                let mut ax_read_succeeded = false;
                if flags.include_ax_text {
                    match self
                        .backend
                        .get_app_state_for_window(&resolved_window.1, CaptureScreenMode::Never)
                        .await
                    {
                        Ok(ax_snapshot) => {
                            ax_read_succeeded = !ax_snapshot.diagnostics.iter().any(|diagnostic| {
                                matches!(
                                    diagnostic.code.as_str(),
                                    "AccessibilityUnavailable" | "AccessibilityCoverageLimited"
                                )
                            });
                            snapshot.elements = ax_snapshot.elements;
                            snapshot.diagnostics.extend(ax_snapshot.diagnostics);
                        }
                        Err(error) => snapshot.diagnostics.push(error.diagnostic()),
                    }
                }
                Ok((snapshot, resolved_window.1, ax_read_succeeded))
            })
            .await;

        let (mut snapshot, resolved_window, ax_read_succeeded) = match capture {
            Ok(capture) => capture,
            Err(error) => {
                if let Some(capture_guard) = capture_guard {
                    let _ = self
                        .overlay
                        .lock()
                        .await
                        .restore_after_capture(capture_guard);
                }
                return error_response(error.code, error.message);
            }
        };
        if let Some(capture_guard) = capture_guard.as_ref() {
            snapshot
                .diagnostics
                .extend(capture_guard.diagnostics.iter().cloned());
        }
        {
            let mut overlay = self.overlay.lock().await;
            overlay.apply_to_snapshot(&mut snapshot);
            if let Some(capture_guard) = capture_guard {
                snapshot
                    .diagnostics
                    .extend(overlay.restore_after_capture(capture_guard));
            }
        }

        self.snapshots.lock().await.store(snapshot.clone());
        let result_snapshot = snapshot;
        let result_request_id = request_id.clone();
        let result = self
            .with_desktop_deadline_until(deadline, async move {
                tokio::task::spawn_blocking(move || {
                    appshot_result(
                        &result_request_id,
                        &result_snapshot,
                        &resolved_window,
                        flags.include_ax_text,
                        ax_read_succeeded,
                    )
                })
                .await
                .map_err(|error| {
                    BackendError::new(
                        BackendErrorCode::Internal,
                        format!("AppShot artifact worker failed: {error}"),
                    )
                })?
            })
            .await;
        match result {
            Ok(result) => ServiceResponse::AppShotCapture {
                result: Box::new(result),
            },
            Err(error) => error_response(error.code, error.message),
        }
    }
}

pub(crate) fn validate_request_id(request_id: &str) -> Result<(), &'static str> {
    if request_id.is_empty() || request_id.len() > 128 {
        return Err("request_id must contain between 1 and 128 characters");
    }
    if request_id.starts_with('.')
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(
            "request_id may contain only ASCII letters, digits, '-', '_', and '.', and may not start with '.'",
        );
    }
    Ok(())
}

pub(crate) fn persist_image(
    request_id: &str,
    source: &Path,
    format: Option<ModelImageFormat>,
) -> io::Result<(PathBuf, u64, &'static str)> {
    let root = appshot_artifacts_dir();
    create_private_dir(&root)?;
    cleanup_expired(&root, SystemTime::now());

    let source_extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let (extension, mime_type) = match (format, source_extension.as_deref()) {
        (Some(ModelImageFormat::Jpeg), _) | (None, Some("jpg" | "jpeg")) => ("jpg", "image/jpeg"),
        (None, Some("png")) => ("png", "image/png"),
        (Some(ModelImageFormat::Webp), _) | (None, _) => ("webp", "image/webp"),
    };
    // Request ids are caller-controlled and may be reused after a response is
    // lost. Suffix every artifact with a fresh id so a live ContentRef can
    // never be overwritten by a later capture.
    let artifact_id = format!(
        "{request_id}-{}",
        sky_cua_platform::snapshot::new_snapshot_id()
    );
    let destination = root.join(format!("{artifact_id}.{extension}"));
    let temporary = root.join(format!(".{artifact_id}.tmp"));
    let _ = fs::remove_file(&temporary);
    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let source_size = match fs::metadata(source).map(|metadata| metadata.len()) {
        Ok(size) => size,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if source_size == 0 || source_size > MAX_ARTIFACT_BYTES {
        let _ = fs::remove_file(&temporary);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "captured AppShot image size {source_size} is outside the 1-{MAX_ARTIFACT_BYTES} byte range"
            ),
        ));
    }
    let metadata = match fs::metadata(&temporary) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if metadata.len() == 0 {
        let _ = fs::remove_file(&temporary);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "captured AppShot image was empty",
        ));
    }
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        let _ = fs::remove_file(&destination);
        return Err(error);
    }
    Ok((destination, metadata.len(), mime_type))
}

pub(crate) fn ax_text(elements: &[ElementNode]) -> Option<String> {
    let mut output = String::new();
    'elements: for element in elements {
        let fields = [
            element.name.as_deref(),
            element.value.as_deref(),
            element
                .text
                .as_ref()
                .and_then(|text| text.content.as_deref()),
        ];
        let mut appended = Vec::new();
        for field in fields.into_iter().flatten() {
            let field = field.trim();
            if field.is_empty() || appended.contains(&field) {
                continue;
            }
            let separator = if appended.is_empty() {
                if output.is_empty() { "" } else { "\n" }
            } else {
                "\t"
            };
            if !append_bounded(&mut output, separator) || !append_bounded(&mut output, field) {
                break 'elements;
            }
            appended.push(field);
        }
    }
    (!output.is_empty()).then_some(output)
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    let mut boundary = index.min(value.len());
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn append_bounded(output: &mut String, value: &str) -> bool {
    let remaining = MAX_AX_TEXT_BYTES.saturating_sub(output.len());
    if remaining == 0 {
        return false;
    }
    let end = floor_char_boundary(value, remaining);
    output.push_str(&value[..end]);
    end == value.len()
}

fn cleanup_expired(root: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut deleted = 0;
    for entry in entries.flatten() {
        if deleted >= MAX_CLEANUP_ENTRIES {
            break;
        }
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let expired = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= APPSHOT_TTL);
        if expired && fs::remove_file(path).is_ok() {
            deleted += 1;
        }
    }
}

fn appshot_result(
    request_id: &str,
    snapshot: &AppStateSnapshot,
    window: &WindowInfo,
    include_ax_text: bool,
    ax_read_succeeded: bool,
) -> Result<AppShotCaptureResult, BackendError> {
    let capture = snapshot.capture.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::Internal,
            "AppShot capture metadata was unavailable",
        )
    })?;
    let dimensions = capture.pixel_size.clone().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::Internal,
            "AppShot capture did not report image dimensions",
        )
    })?;
    let appshot = desktop_appshot_envelope(
        request_id,
        snapshot,
        window,
        include_ax_text,
        ax_read_succeeded,
        AppShotTrigger::Observe,
    )?;
    let filename = appshot.image.filename.as_deref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::Internal,
            "desktop AppShot ContentRef did not include its private artifact filename",
        )
    })?;
    let path = appshot_artifacts_dir().join(filename);
    let ax_text = (include_ax_text && ax_read_succeeded)
        .then(|| ax_text(&snapshot.elements))
        .flatten();
    let ax_status = appshot_ax_status(
        include_ax_text,
        ax_read_succeeded,
        snapshot.environment.semantic_backend.clone(),
        ax_text.is_some(),
    );
    let focused = snapshot.focused_app.as_ref();
    let focused_is_target = focused.is_some_and(|app| {
        window.pid.is_some() && window.pid == app.pid
            || window.app_id.as_deref() == Some(app.app_id.as_str())
            || window.title.is_some() && window.title.as_deref() == app.window_title.as_deref()
    });

    let result = AppShotCaptureResult {
        request_id: request_id.to_string(),
        application: AppShotApplication {
            name: window
                .wm_class
                .clone()
                .or_else(|| window.app_id.clone())
                .or_else(|| {
                    focused
                        .filter(|_| focused_is_target)
                        .map(|app| app.name.clone())
                })
                .unwrap_or_else(|| "Unknown application".to_string()),
            app_id: window.app_id.clone().or_else(|| {
                focused
                    .filter(|_| focused_is_target)
                    .map(|app| app.app_id.clone())
            }),
            desktop_file_id: focused
                .filter(|_| focused_is_target)
                .and_then(|app| app.desktop_file_id.clone()),
            pid: window.pid.or_else(|| {
                focused
                    .filter(|_| focused_is_target)
                    .and_then(|app| app.pid)
            }),
            window_id: Some(window.window_id.clone()),
            window_title: window.title.clone().or_else(|| {
                focused
                    .filter(|_| focused_is_target)
                    .and_then(|app| app.window_title.clone())
            }),
        },
        image: AppShotImage {
            path: path.display().to_string(),
            mime_type: appshot.image.mime_type.clone(),
            size_bytes: appshot.image.size_bytes,
            dimensions,
        },
        ax_status,
        ax_text,
        capture_scope: capture.capture_scope.clone(),
        capture_backend: capture.backend.clone(),
        image_backend: capture.image_backend.clone(),
        display: capture.display.clone(),
        diagnostics: snapshot.diagnostics.clone(),
        appshot: Some(Box::new(appshot)),
    };
    Ok(result)
}

fn cleanup_capture_artifacts(capture: &sky_cua_platform::model::CaptureInfo) {
    let captures_root = sky_cua_platform::capture_artifacts_dir();
    let paths = [
        capture.screenshot_path.as_deref(),
        capture.original_screenshot_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(Path::new);
    for path in paths {
        if path.parent() == Some(captures_root.as_path()) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Build the universal desktop AppShot envelope from an exact-window capture.
/// The private artifact is persisted once and referenced by ContentRef; no
/// second screenshot is taken, keeping pixels and semantics on one snapshot.
pub(crate) fn desktop_appshot_envelope(
    request_id: &str,
    snapshot: &AppStateSnapshot,
    window: &WindowInfo,
    include_ax_text: bool,
    ax_read_succeeded: bool,
    trigger: AppShotTrigger,
) -> Result<AppShotEnvelope, BackendError> {
    let capture = snapshot.capture.as_ref().ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::Internal,
            "AppShot capture metadata was unavailable",
        )
    })?;
    let _capture_artifact_guard = CaptureArtifactGuard { capture };
    if capture.capture_scope != CaptureScope::Window {
        return Err(BackendError::new(
            BackendErrorCode::CaptureBackendDowngraded,
            format!(
                "desktop AppShot requires a target-window crop; backend returned {:?}",
                capture.capture_scope
            ),
        ));
    }
    let source = capture
        .screenshot_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "AppShot capture did not produce an inspection image path",
            )
        })?;
    if capture.pixel_size.is_none() {
        return Err(BackendError::new(
            BackendErrorCode::Internal,
            "AppShot capture did not report image dimensions",
        ));
    }
    let (path, size_bytes, mime_type) =
        persist_image(request_id, Path::new(source), capture.model_image_format).map_err(
            |error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!("failed to persist the service-owned AppShot artifact: {error}"),
                )
            },
        )?;
    let mut artifact_guard = PersistedArtifactGuard::new(path.clone());
    let content_id = path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| request_id.to_string());
    let sha256 = sha256_file(&path).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to hash the service-owned AppShot artifact: {error}"),
        )
    })?;
    let expires_at_ms = SystemTime::now()
        .checked_add(APPSHOT_TTL)
        .and_then(|value| value.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64);
    let image = ContentRef {
        content_id,
        device_id: None,
        link_epoch: None,
        mime_type: mime_type.to_string(),
        filename: Some(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ),
        size_bytes,
        sha256,
        source: ContentSource::HostPrivateArtifact,
        expires_at_ms,
        persistence: ContentPersistence::Temporary,
    };
    let focused = snapshot.focused_app.as_ref();
    let focused_is_target = focused.is_some_and(|app| {
        window.pid.is_some() && window.pid == app.pid
            || window.app_id.as_deref() == Some(app.app_id.as_str())
            || window.title.is_some() && window.title.as_deref() == app.window_title.as_deref()
    });
    let app_id = window
        .app_id
        .clone()
        .or_else(|| window.wm_class.clone())
        .or_else(|| {
            focused
                .filter(|_| focused_is_target)
                .map(|app| app.app_id.clone())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let bounds = capture
        .logical_rect
        .clone()
        .or_else(|| window.bounds.clone())
        .ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::Internal,
                "desktop AppShot capture did not report exact window bounds",
            )
        })?;
    let semantic_projection = serde_json::json!({
        "elements": snapshot.elements,
        "focused_app": focused.filter(|_| focused_is_target),
        "accessibility": appshot_ax_status(
            include_ax_text,
            ax_read_succeeded,
            snapshot.environment.semantic_backend.clone(),
            ax_text(&snapshot.elements).is_some(),
        ),
    });
    let semantics_complete = include_ax_text
        && ax_read_succeeded
        && snapshot.environment.semantic_backend != SemanticBackendKind::None
        && !snapshot.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.code.as_str(),
                "AccessibilityUnavailable" | "AccessibilityCoverageLimited"
            )
        });
    let mut diagnostics = snapshot.diagnostics.clone();
    diagnostics.push(sky_cua_platform::model::DiagnosticEntry {
        code: "DesktopCaptureFenceUnavailable".to_string(),
        message: "Screenshot and accessibility semantics were sampled separately; stability was not claimed without a target-generation fence.".to_string(),
        details: None,
    });
    if app_id == "unknown" {
        diagnostics.push(sky_cua_platform::model::DiagnosticEntry {
            code: "DesktopSubjectIdentityIncomplete".to_string(),
            message: "The exact window had no application identifier; using unknown.".to_string(),
            details: None,
        });
    }
    let envelope = AppShotEnvelope {
        appshot_id: request_id.to_string(),
        trigger,
        captured_at: Utc::now(),
        consistency: desktop_consistency(&snapshot.diagnostics),
        capture: AppShotCapture::Desktop {
            app_id,
            window_id: window.window_id.clone(),
            title: window.title.clone(),
            bounds,
            semantic_projection,
        },
        image,
        action_snapshot: AppShotActionSnapshot {
            snapshot_id: snapshot.snapshot_id.clone(),
            session_id: None,
            subject_generation: None,
        },
        coverage: AppShotCoverage {
            pixels_complete: true,
            semantics_complete,
            secure_regions_redacted: false,
            projection_truncated: false,
            total_semantic_nodes: Some(snapshot.elements.len() as u64),
            projected_semantic_nodes: Some(snapshot.elements.len() as u64),
        },
        capability_profile_id: format!(
            "desktop:{:?}:{:?}",
            snapshot.environment.semantic_backend, capture.backend
        ),
        diagnostics,
    };
    artifact_guard.disarm();
    Ok(envelope)
}

struct CaptureArtifactGuard<'a> {
    capture: &'a sky_cua_platform::model::CaptureInfo,
}

impl Drop for CaptureArtifactGuard<'_> {
    fn drop(&mut self) {
        cleanup_capture_artifacts(self.capture);
    }
}

struct PersistedArtifactGuard {
    path: Option<PathBuf>,
}

impl PersistedArtifactGuard {
    fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    fn disarm(&mut self) {
        self.path = None;
    }
}

impl Drop for PersistedArtifactGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn desktop_consistency(
    diagnostics: &[sky_cua_platform::model::DiagnosticEntry],
) -> AppShotConsistency {
    if diagnostics.iter().any(|diagnostic| {
        matches!(
            diagnostic.code.as_str(),
            "DesktopTargetChangedDuringCapture" | "CaptureChangedDuringCapture"
        )
    }) {
        AppShotConsistency::ChangedDuringCapture
    } else {
        // Pixels and semantics are independently sampled until a target
        // generation fence is available; never claim a false Stable frame.
        AppShotConsistency::Partial
    }
}

fn appshot_ax_status(
    include_ax_text: bool,
    ax_read_succeeded: bool,
    semantic_backend: SemanticBackendKind,
    has_text: bool,
) -> AppShotAccessibilityStatus {
    if !include_ax_text || !ax_read_succeeded || semantic_backend == SemanticBackendKind::None {
        AppShotAccessibilityStatus::Unavailable
    } else if has_text {
        AppShotAccessibilityStatus::Available
    } else {
        AppShotAccessibilityStatus::Empty
    }
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_cua_platform::model::ElementTextReadback;
    use std::io::Write;

    #[test]
    fn request_ids_are_safe_path_components() {
        assert!(validate_request_id("req_01.good").is_ok());
        for invalid in ["", ".hidden", "../escape", "has space", "💥"] {
            assert!(validate_request_id(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn ax_text_collects_distinct_visible_readbacks() {
        let element = ElementNode {
            element_index: 0,
            parent_index: None,
            role: "label".to_string(),
            name: Some("Heading".to_string()),
            description: None,
            value: Some("Value".to_string()),
            text: Some(ElementTextReadback {
                character_count: 4,
                caret_offset: None,
                content: Some("Body".to_string()),
                content_suppressed: false,
                truncated: false,
                selections: Vec::new(),
            }),
            numeric_value: None,
            supports_editable_text: false,
            state_flags: Vec::new(),
            semantic_actions: Vec::new(),
            bounds: None,
            backend_ref: None,
        };
        assert_eq!(ax_text(&[element]).as_deref(), Some("Heading\tValue\tBody"));
    }

    #[test]
    fn ax_text_never_exceeds_its_utf8_byte_budget() {
        let element = ElementNode {
            element_index: 0,
            parent_index: None,
            role: "label".to_string(),
            name: Some(format!("{}é", "x".repeat(MAX_AX_TEXT_BYTES + 10))),
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: Vec::new(),
            semantic_actions: Vec::new(),
            bounds: None,
            backend_ref: None,
        };
        let text = ax_text(&[element]).expect("oversized text is truncated");
        assert_eq!(text.len(), MAX_AX_TEXT_BYTES);
        assert!(text.is_char_boundary(text.len()));
    }

    #[test]
    fn ax_status_distinguishes_failure_empty_and_available() {
        assert_eq!(
            appshot_ax_status(true, false, SemanticBackendKind::Atspi, false),
            AppShotAccessibilityStatus::Unavailable
        );
        assert_eq!(
            appshot_ax_status(true, true, SemanticBackendKind::Atspi, false),
            AppShotAccessibilityStatus::Empty
        );
        assert_eq!(
            appshot_ax_status(true, true, SemanticBackendKind::Atspi, true),
            AppShotAccessibilityStatus::Available
        );
        assert_eq!(
            appshot_ax_status(true, true, SemanticBackendKind::None, true),
            AppShotAccessibilityStatus::Unavailable
        );
    }

    #[test]
    fn private_appshot_artifacts_use_one_hour_leases() {
        assert_eq!(APPSHOT_TTL, Duration::from_secs(60 * 60));
    }

    #[test]
    fn artifact_hash_is_sha256_of_persisted_bytes() {
        let path = std::env::temp_dir().join(format!(
            "sky-cua-appshot-hash-{}-{}",
            std::process::id(),
            sky_cua_platform::snapshot::new_snapshot_id()
        ));
        let mut file = fs::File::create(&path).expect("create hash fixture");
        file.write_all(b"desktop-appshot")
            .expect("write hash fixture");
        drop(file);
        assert_eq!(
            sha256_file(&path).expect("hash fixture"),
            "e4455bd410dd0aab5284249451579d1f28052dc0fdec4e82521d27a7753ee097"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn consistency_never_claims_stable_without_a_target_fence() {
        assert_eq!(desktop_consistency(&[]), AppShotConsistency::Partial);
        let changed = vec![sky_cua_platform::model::DiagnosticEntry {
            code: "DesktopTargetChangedDuringCapture".to_string(),
            message: "simulated target change".to_string(),
            details: None,
        }];
        assert_eq!(
            desktop_consistency(&changed),
            AppShotConsistency::ChangedDuringCapture
        );
    }

    #[test]
    fn reused_request_ids_get_distinct_artifacts() {
        let source = std::env::temp_dir().join(format!(
            "sky-cua-appshot-source-{}-{}",
            std::process::id(),
            sky_cua_platform::snapshot::new_snapshot_id()
        ));
        fs::write(&source, b"same request, new content ref").expect("write source fixture");
        let first = persist_image("reused-request", &source, None).expect("first artifact");
        let second = persist_image("reused-request", &source, None).expect("second artifact");
        assert_ne!(first.0, second.0);
        assert_eq!(
            fs::read(&first.0).expect("first remains"),
            b"same request, new content ref"
        );
        let _ = fs::remove_file(first.0);
        let _ = fs::remove_file(second.0);
        let _ = fs::remove_file(source);
    }
}
