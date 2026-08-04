use base64::Engine as _;
use sha2::{Digest, Sha256};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, path::Path};

use sky_cua_platform::appshot_artifacts_dir;
use sky_cua_platform::model::{
    AppShotActionSnapshot, AppShotCapture, AppShotConsistency, AppShotCoverage, AppShotEnvelope,
    AppShotTrigger, BrowserActionResponse, BrowserAppShotResponse, BrowserClaimTabResponse,
    BrowserEvalResponse, BrowserListTabsResponse, BrowserMoveMouseResponse,
    BrowserNavigateResponse, BrowserOpenResponse, BrowserScreenshotResponse,
    BrowserSessionIdentity, BrowserSnapshotResponse, BrowserTargetKind, ContentPersistence,
    ContentRef, ContentSource, DiagnosticEntry, PixelSize, normalize_browser_open_url,
};
use tokio::time::Instant as TokioInstant;

use super::cdp::{BrowserCdpAction, BrowserCdpResult};
use super::diagnostics::{
    invalid_key_diagnostic, invalid_scroll_diagnostic, invalid_text_diagnostic,
    normalize_action_tab_id, unsupported_open_url_diagnostic, validate_open_url, validate_point,
    validate_tab_id,
};
use super::executor::BrowserBridgeExecutor;
use super::probe::first_responsive_bridge_socket;
use super::sockets::{
    BrowserSocketSelection, browser_bridge_disconnected_for_selection,
    browser_socket_selection_from_env, find_bridge_sockets,
};

/// Overall deadline for a browser bridge operation. Defaults to 12s but is
/// raised by `SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS` for slow or remote desktops
/// where the extension / native-host CDP relay is sluggish. Keep the default in
/// sync with `BROWSER_OPEN_TIMEOUT_MS` in `diagnostics.rs`.
#[cfg(not(test))]
pub(super) fn browser_open_timeout() -> Duration {
    Duration::from_millis(super::transport::browser_request_timeout_override_ms().unwrap_or(12_000))
}
// Short enough that the aggregate-deadline test stays fast, but with enough
// headroom that happy-path operation tests do not time out under scheduler load.
#[cfg(test)]
pub(super) fn browser_open_timeout() -> Duration {
    Duration::from_secs(2)
}

#[cfg(test)]
pub(crate) async fn list_tabs(target: Option<BrowserTargetKind>) -> BrowserListTabsResponse {
    list_tabs_with_identity(target, None).await
}

#[cfg(test)]
pub(crate) async fn open_tab(
    target: Option<BrowserTargetKind>,
    url: Option<String>,
) -> BrowserOpenResponse {
    open_tab_with_identity(target, url, None).await
}

#[cfg(test)]
pub(crate) async fn claim_tab(
    target: Option<BrowserTargetKind>,
    tab_id: String,
) -> BrowserClaimTabResponse {
    claim_tab_with_identity(target, tab_id, None).await
}

#[cfg(test)]
pub(crate) async fn move_mouse(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
    wait_for_arrival: bool,
) -> BrowserMoveMouseResponse {
    move_mouse_with_identity(target, tab_id, x, y, wait_for_arrival, None).await
}

#[cfg(test)]
pub(crate) async fn navigate(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    url: String,
) -> BrowserNavigateResponse {
    navigate_with_identity(target, tab_id, url, None).await
}

#[cfg(test)]
pub(crate) async fn snapshot(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text_limit: Option<usize>,
    element_offset: Option<usize>,
    element_limit: Option<usize>,
    element_query: Option<String>,
) -> BrowserSnapshotResponse {
    snapshot_with_identity(
        target,
        tab_id,
        text_limit,
        element_offset,
        element_limit,
        element_query,
        None,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn screenshot(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    include_image_data: bool,
) -> BrowserScreenshotResponse {
    screenshot_with_identity(target, tab_id, include_image_data, None).await
}

#[cfg(test)]
pub(crate) async fn click(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
) -> BrowserActionResponse {
    click_with_identity(target, tab_id, x, y, None).await
}

#[cfg(test)]
pub(crate) async fn type_text(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text: String,
) -> BrowserActionResponse {
    type_text_with_identity(target, tab_id, text, None).await
}

#[cfg(test)]
pub(crate) async fn click_element(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    element_ref: String,
) -> BrowserActionResponse {
    click_element_with_identity(target, tab_id, element_ref, None).await
}

#[cfg(test)]
pub(crate) async fn type_text_element(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    element_ref: String,
    text: String,
) -> BrowserActionResponse {
    type_text_element_with_identity(target, tab_id, element_ref, text, None).await
}

#[cfg(test)]
pub(crate) async fn press_key(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    key: String,
) -> BrowserActionResponse {
    press_key_with_identity(target, tab_id, key, None).await
}

#[cfg(test)]
pub(crate) async fn eval_with_policy(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    expression: String,
    browser_eval_enabled: bool,
) -> BrowserEvalResponse {
    eval_with_policy_and_identity(target, tab_id, expression, browser_eval_enabled, None).await
}

#[cfg(test)]
pub(crate) async fn scroll(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    delta_x: f64,
    delta_y: f64,
    x: Option<f64>,
    y: Option<f64>,
) -> BrowserActionResponse {
    scroll_with_identity(target, tab_id, delta_x, delta_y, x, y, None).await
}

pub(crate) async fn list_tabs_with_identity(
    target: Option<BrowserTargetKind>,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserListTabsResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);

    match BrowserBridgeExecutor::from_env(TokioInstant::now() + browser_open_timeout(), identity) {
        Ok(executor) => executor.list_tabs(Some(resolved_target)).await,
        Err(diagnostic) => BrowserListTabsResponse {
            target: Some(resolved_target),
            tabs: Vec::new(),
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn open_tab_with_identity(
    target: Option<BrowserTargetKind>,
    url: Option<String>,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserOpenResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);

    let url = match validate_open_url(url) {
        Ok(url) => url,
        Err(diagnostic) => {
            return BrowserOpenResponse {
                target: resolved_target,
                tab: None,
                destination_appshot: None,
                diagnostics: vec![diagnostic],
            };
        }
    };

    let executor = match BrowserBridgeExecutor::from_env(
        TokioInstant::now() + browser_open_timeout(),
        identity,
    ) {
        Ok(executor) => executor,
        Err(diagnostic) => {
            return BrowserOpenResponse {
                target: resolved_target,
                tab: None,
                destination_appshot: None,
                diagnostics: vec![diagnostic],
            };
        }
    };

    match executor.open_tab(resolved_target, url.as_deref()).await {
        Ok(response) => response,
        Err(diagnostic) => BrowserOpenResponse {
            target: resolved_target,
            tab: None,
            destination_appshot: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn claim_tab_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserClaimTabResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);

    let tab_id = match validate_tab_id(tab_id) {
        Ok(tab_id) => tab_id,
        Err(diagnostic) => {
            return BrowserClaimTabResponse {
                target: resolved_target,
                tab: None,
                diagnostics: vec![diagnostic],
            };
        }
    };

    let executor = match BrowserBridgeExecutor::from_env(
        TokioInstant::now() + browser_open_timeout(),
        identity,
    ) {
        Ok(executor) => executor,
        Err(diagnostic) => {
            return BrowserClaimTabResponse {
                target: resolved_target,
                tab: None,
                diagnostics: vec![diagnostic],
            };
        }
    };

    match executor.claim_tab(resolved_target, &tab_id).await {
        Ok(response) => response,
        Err(diagnostic) => BrowserClaimTabResponse {
            target: resolved_target,
            tab: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn move_mouse_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
    wait_for_arrival: bool,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserMoveMouseResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, mut diagnostics) = normalize_action_tab_id(tab_id);
    if let Err(diagnostic) = validate_point(x, y) {
        diagnostics.push(diagnostic);
    }
    if !diagnostics.is_empty() {
        return BrowserMoveMouseResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            x,
            y,
            wait_for_arrival,
            diagnostics,
        };
    }

    let executor = match BrowserBridgeExecutor::from_env(
        TokioInstant::now() + browser_open_timeout(),
        identity,
    ) {
        Ok(executor) => executor,
        Err(diagnostic) => {
            return BrowserMoveMouseResponse {
                target: resolved_target,
                tab_id: normalized_tab_id,
                x,
                y,
                wait_for_arrival,
                diagnostics: vec![diagnostic],
            };
        }
    };

    match executor
        .bind_tab(resolved_target, &normalized_tab_id)
        .move_mouse(x, y, wait_for_arrival)
        .await
    {
        Ok(response) => response,
        Err(diagnostic) => BrowserMoveMouseResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            x,
            y,
            wait_for_arrival,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn navigate_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    url: String,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserNavigateResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, mut diagnostics) = normalize_action_tab_id(tab_id);
    let normalized_url = normalize_browser_open_url(&url).unwrap_or_default();
    if normalized_url.is_empty() {
        diagnostics.push(unsupported_open_url_diagnostic("browser_navigate"));
    }
    if !diagnostics.is_empty() {
        return BrowserNavigateResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            url: normalized_url,
            destination_appshot: None,
            diagnostics,
        };
    }

    match run_cdp_action(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Navigate {
            url: normalized_url.clone(),
        },
        identity,
    )
    .await
    {
        Ok(BrowserCdpResult::Navigate { url }) => BrowserNavigateResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            url,
            destination_appshot: None,
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("navigate action returns navigate result"),
        Err(diagnostic) => BrowserNavigateResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            url: normalized_url,
            destination_appshot: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn snapshot_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text_limit: Option<usize>,
    element_offset: Option<usize>,
    element_limit: Option<usize>,
    element_query: Option<String>,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserSnapshotResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, diagnostics) = normalize_action_tab_id(tab_id);
    if !diagnostics.is_empty() {
        return BrowserSnapshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            title: None,
            url: None,
            snapshot: None,
            diagnostics,
        };
    }

    match run_cdp_action(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Snapshot {
            text_limit,
            element_offset,
            element_limit,
            element_query,
        },
        identity,
    )
    .await
    {
        Ok(BrowserCdpResult::Snapshot {
            title,
            url,
            snapshot,
        }) => BrowserSnapshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            title,
            url,
            snapshot: Some(snapshot),
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("snapshot action returns snapshot result"),
        Err(diagnostic) => BrowserSnapshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            title: None,
            url: None,
            snapshot: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn observe_appshot_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text_limit: Option<usize>,
    element_limit: Option<usize>,
    include_image_data: bool,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserAppShotResponse {
    let fallback_tab = tab_id.clone();
    match tokio::time::timeout(
        Duration::from_secs(2),
        observe_appshot_with_identity_inner(
            target,
            tab_id,
            text_limit,
            element_limit,
            include_image_data,
            identity.clone(),
        ),
    )
    .await
    {
        Ok(response) => response,
        Err(_) => BrowserAppShotResponse {
            appshot: AppShotEnvelope {
                appshot_id: uuid::Uuid::new_v4().to_string(),
                trigger: AppShotTrigger::Observe,
                captured_at: chrono::Utc::now(),
                consistency: AppShotConsistency::Partial,
                capture: AppShotCapture::Browser {
                    tab_id: fallback_tab,
                    url: String::new(),
                    title: None,
                    viewport: PixelSize {
                        width: 0,
                        height: 0,
                    },
                    document_generation: 0,
                    semantic_snapshot: serde_json::json!({}),
                },
                image: ContentRef {
                    content_id: uuid::Uuid::new_v4().to_string(),
                    device_id: None,
                    link_epoch: None,
                    mime_type: "application/octet-stream".into(),
                    filename: None,
                    size_bytes: 0,
                    sha256: "00".repeat(32),
                    source: ContentSource::Screenshot,
                    expires_at_ms: None,
                    persistence: ContentPersistence::Temporary,
                },
                action_snapshot: AppShotActionSnapshot {
                    snapshot_id: uuid::Uuid::new_v4().to_string(),
                    session_id: identity.map(|value| value.session_id),
                    subject_generation: None,
                },
                coverage: AppShotCoverage {
                    pixels_complete: false,
                    semantics_complete: false,
                    secure_regions_redacted: false,
                    projection_truncated: false,
                    total_semantic_nodes: None,
                    projected_semantic_nodes: None,
                },
                capability_profile_id: "browser-v1".into(),
                diagnostics: vec![DiagnosticEntry {
                    code: "BrowserCaptureDeadlineExceeded".into(),
                    message: "Browser AppShot capture exceeded its two-second aggregate deadline."
                        .into(),
                    details: None,
                }],
            },
            image_data_base64: String::new(),
            image_mime_type: "application/octet-stream".into(),
        },
    }
}

async fn observe_appshot_with_identity_inner(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text_limit: Option<usize>,
    element_limit: Option<usize>,
    include_image_data: bool,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserAppShotResponse {
    let target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let tab_id = tab_id.trim().to_string();
    let mut last: Option<AppShotEnvelope> = None;
    for attempt in 0..2 {
        let snapshot = snapshot_with_identity(
            Some(target),
            tab_id.clone(),
            text_limit,
            None,
            element_limit,
            None,
            identity.clone(),
        )
        .await;
        let screenshot = screenshot_with_identity(
            Some(target),
            tab_id.clone(),
            include_image_data,
            identity.clone(),
        )
        .await;
        // Fence the capture against navigation/document replacement: a second
        // semantic read after pixels must agree on URL and viewport before the
        // envelope can claim stable consistency.
        let after = snapshot_with_identity(
            Some(target),
            tab_id.clone(),
            Some(0),
            None,
            Some(0),
            None,
            identity.clone(),
        )
        .await;
        let url = snapshot.url.clone().unwrap_or_default();
        let width = screenshot.width.unwrap_or(0);
        let height = screenshot.height.unwrap_or(0);
        let bytes = if !screenshot.data_base64.is_empty() {
            base64::engine::general_purpose::STANDARD
                .decode(&screenshot.data_base64)
                .unwrap_or_default()
        } else {
            screenshot
                .screenshot_path
                .as_deref()
                .and_then(|path| fs::read(path).ok())
                .unwrap_or_default()
        };
        let semantic_viewport = snapshot
            .snapshot
            .as_ref()
            .and_then(|value| value.get("viewport"));
        let viewport_matches = semantic_viewport
            .and_then(|value| value.get("width"))
            .and_then(|value| value.as_u64())
            .zip(
                semantic_viewport
                    .and_then(|value| value.get("height"))
                    .and_then(|value| value.as_u64()),
            )
            .is_some_and(|(width, height)| {
                Some(width as u32) == screenshot.width && Some(height as u32) == screenshot.height
            });
        let decoded_dimensions = image::load_from_memory(&bytes)
            .ok()
            .map(|image| (image.width(), image.height()));
        let image_matches_viewport = decoded_dimensions.is_some_and(|(width, height)| {
            Some(width) == screenshot.width && Some(height) == screenshot.height
        });
        let mut hasher = Sha256::new();
        hasher.update(
            snapshot
                .snapshot
                .as_ref()
                .and_then(|v| v.get("documentGeneration"))
                .and_then(|v| v.as_str())
                .unwrap_or(&url)
                .as_bytes(),
        );
        let digest = hasher.finalize();
        let generation = u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]));
        let mut diagnostics = snapshot.diagnostics.clone();
        diagnostics.extend(screenshot.diagnostics.clone());
        let mut consistency = classify_document_capture(
            snapshot.url.as_deref(),
            after.url.as_deref(),
            snapshot.snapshot.as_ref(),
            after.snapshot.as_ref(),
            screenshot.diagnostics.is_empty()
                && !bytes.is_empty()
                && viewport_matches
                && image_matches_viewport
                && semantic_viewport
                    == after
                        .snapshot
                        .as_ref()
                        .and_then(|value| value.get("viewport"))
                && screenshot.target == target
                && snapshot.target == target
                && screenshot.tab_id == tab_id
                && snapshot.tab_id == tab_id,
            attempt,
        );
        if consistency == AppShotConsistency::ChangedDuringCapture {
            diagnostics.push(DiagnosticEntry {
                code: "ChangedDuringCapture".into(),
                message: "Browser document changed while capturing pixels and semantics.".into(),
                details: None,
            });
        }
        let expires_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as u64 + 60 * 60 * 1000);
        let sha = Sha256::digest(&bytes);
        let final_artifact = if consistency == AppShotConsistency::Stable || attempt == 1 {
            persist_browser_appshot_artifact(&bytes, &screenshot.mime_type).ok()
        } else {
            None
        };
        if final_artifact.is_none() && consistency == AppShotConsistency::Stable {
            consistency = AppShotConsistency::Partial;
            diagnostics.push(DiagnosticEntry {
                code: "BrowserArtifactPersistFailed".into(),
                message: "Browser AppShot pixels could not be committed to a private artifact."
                    .into(),
                details: None,
            });
        }
        let image = ContentRef {
            content_id: uuid::Uuid::new_v4().to_string(),
            device_id: None,
            link_epoch: None,
            mime_type: screenshot.mime_type.clone(),
            filename: screenshot
                .screenshot_path
                .as_deref()
                .and_then(|path| Path::new(path).file_name())
                .map(|name| name.to_string_lossy().into_owned()),
            size_bytes: bytes.len() as u64,
            sha256: format!("{sha:x}"),
            source: ContentSource::HostPrivateArtifact,
            expires_at_ms,
            persistence: ContentPersistence::Temporary,
        };
        let image = final_artifact
            .as_ref()
            .and_then(|path| fs::metadata(path).ok().map(|metadata| (path, metadata)))
            .map(|(path, metadata)| ContentRef {
                content_id: uuid::Uuid::new_v4().to_string(),
                device_id: None,
                link_epoch: None,
                mime_type: screenshot.mime_type.clone(),
                filename: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                size_bytes: metadata.len(),
                sha256: Sha256::digest(&bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                source: ContentSource::HostPrivateArtifact,
                expires_at_ms,
                persistence: ContentPersistence::Temporary,
            })
            .unwrap_or(image);
        let envelope = AppShotEnvelope {
            appshot_id: uuid::Uuid::new_v4().to_string(),
            trigger: AppShotTrigger::Observe,
            captured_at: chrono::Utc::now(),
            consistency,
            capture: AppShotCapture::Browser {
                tab_id: tab_id.clone(),
                url,
                title: snapshot.title.clone(),
                viewport: PixelSize { width, height },
                document_generation: generation,
                semantic_snapshot: snapshot
                    .snapshot
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({})),
            },
            image,
            action_snapshot: AppShotActionSnapshot {
                snapshot_id: uuid::Uuid::new_v4().to_string(),
                session_id: identity.clone().map(|i| i.session_id),
                subject_generation: Some(generation),
            },
            coverage: AppShotCoverage {
                pixels_complete: !bytes.is_empty(),
                semantics_complete: snapshot.snapshot.is_some(),
                secure_regions_redacted: false,
                projection_truncated: snapshot
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.get("textTruncated"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                total_semantic_nodes: snapshot
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.get("elementCount"))
                    .and_then(|v| v.as_u64()),
                projected_semantic_nodes: snapshot
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.get("elements"))
                    .and_then(|v| v.as_array())
                    .map(|v| v.len() as u64),
            },
            capability_profile_id: "browser-v1".to_string(),
            diagnostics,
        };
        if attempt == 1 || envelope.consistency == AppShotConsistency::Stable {
            return BrowserAppShotResponse {
                appshot: envelope,
                image_data_base64: attachment_data(&bytes, include_image_data),
                image_mime_type: screenshot.mime_type,
            };
        }
        last = Some(envelope);
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    BrowserAppShotResponse {
        appshot: last.expect("observe always produces an envelope"),
        image_data_base64: String::new(),
        image_mime_type: "image/png".into(),
    }
}

fn classify_document_capture(
    before_url: Option<&str>,
    after_url: Option<&str>,
    before: Option<&serde_json::Value>,
    after: Option<&serde_json::Value>,
    screenshot_ok: bool,
    attempt: usize,
) -> AppShotConsistency {
    let before_generation = before.and_then(|v| v.get("documentGeneration"));
    let after_generation = after.and_then(|v| v.get("documentGeneration"));
    let changed = before_url != after_url || before_generation != after_generation;
    if !changed && before_url.is_some() && screenshot_ok {
        AppShotConsistency::Stable
    } else if changed && attempt > 0 {
        AppShotConsistency::ChangedDuringCapture
    } else {
        AppShotConsistency::Partial
    }
}

fn attachment_data(bytes: &[u8], include_image_data: bool) -> String {
    if include_image_data && !bytes.is_empty() {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    } else {
        String::new()
    }
}

fn persist_browser_appshot_artifact(
    bytes: &[u8],
    mime_type: &str,
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    use std::time::{Duration, SystemTime};
    let root = appshot_artifacts_dir();
    std::fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
    }
    let expiry = SystemTime::now()
        .checked_sub(Duration::from_secs(60 * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    for entry in std::fs::read_dir(&root)?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified < expiry)
        {
            let _ = std::fs::remove_file(path);
        }
    }
    let extension = match mime_type {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    };
    let artifact = root.join(format!(
        "browser-appshot-{}.{}",
        uuid::Uuid::new_v4(),
        extension
    ));
    let temporary = root.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, &artifact).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&artifact, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(artifact)
}

pub(crate) async fn screenshot_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    include_image_data: bool,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserScreenshotResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, diagnostics) = normalize_action_tab_id(tab_id);
    if !diagnostics.is_empty() {
        return BrowserScreenshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            screenshot_path: None,
            width: None,
            height: None,
            diagnostics,
        };
    }

    match run_cdp_action(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Screenshot,
        identity,
    )
    .await
    {
        Ok(BrowserCdpResult::Screenshot {
            data_base64,
            css_width,
            css_height,
        }) => {
            let tab_id_for_task = normalized_tab_id.clone();
            let prepared = match tokio::task::spawn_blocking(move || {
                super::model_image::prepare_browser_capture(
                    &tab_id_for_task,
                    &data_base64,
                    css_width,
                    css_height,
                    include_image_data,
                )
            })
            .await
            {
                Ok(prepared) => prepared,
                Err(join_error) => {
                    return BrowserScreenshotResponse {
                        target: resolved_target,
                        tab_id: normalized_tab_id,
                        mime_type: "image/png".to_string(),
                        data_base64: String::new(),
                        screenshot_path: None,
                        width: None,
                        height: None,
                        diagnostics: vec![DiagnosticEntry {
                            code: "BrowserScreenshotDegraded".to_string(),
                            message: format!(
                                "Browser screenshot post-processing task failed to join \
                                 cleanly: {join_error}"
                            ),
                            details: None,
                        }],
                    };
                }
            };
            BrowserScreenshotResponse {
                target: resolved_target,
                tab_id: normalized_tab_id,
                mime_type: prepared.mime_type,
                data_base64: prepared.data_base64,
                screenshot_path: prepared.screenshot_path,
                width: (prepared.width > 0).then_some(prepared.width),
                height: (prepared.height > 0).then_some(prepared.height),
                diagnostics: prepared.diagnostics,
            }
        }
        Ok(_) => unreachable!("screenshot action returns screenshot result"),
        Err(diagnostic) => BrowserScreenshotResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            mime_type: "image/png".to_string(),
            data_base64: String::new(),
            screenshot_path: None,
            width: None,
            height: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn click_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    x: f64,
    y: f64,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    browser_action_response(
        target,
        tab_id,
        "click",
        validate_point(x, y),
        Some((x, y)),
        BrowserCdpAction::Click { x, y },
        identity,
    )
    .await
}

pub(crate) async fn type_text_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text: String,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    let validation = (!text.is_empty())
        .then_some(())
        .ok_or_else(invalid_text_diagnostic);
    browser_action_response(
        target,
        tab_id,
        "type_text",
        validation,
        None,
        BrowserCdpAction::TypeText { text },
        identity,
    )
    .await
}

/// Click an element by its opaque snapshot reference. Routes through the same
/// executor path as the coordinate [`click`] (session recovery, replay
/// classification, diagnostics); the live center is resolved and dispatched
/// inside the `ClickElement` CDP arm. There is no pre-action cursor point
/// because the target center is unknown until resolution, so `None` is passed
/// for `cursor_before_action`.
pub(crate) async fn click_element_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    element_ref: String,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    browser_action_response(
        target,
        tab_id,
        "click",
        Ok(()),
        None,
        BrowserCdpAction::ClickElement { element_ref },
        identity,
    )
    .await
}

/// Type into an element by its opaque snapshot reference. Focuses the element by
/// clicking its resolved live center, then inserts `text`, all on the shared
/// executor path used by [`type_text`]. As with [`click_element`], the center
/// is unknown until resolution, so no pre-action cursor point is supplied.
pub(crate) async fn type_text_element_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    element_ref: String,
    text: String,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    browser_action_response(
        target,
        tab_id,
        "type_text",
        Ok(()),
        None,
        BrowserCdpAction::TypeTextElement { element_ref, text },
        identity,
    )
    .await
}

pub(crate) async fn press_key_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    key: String,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    let key = key.trim().to_string();
    let validation = (!key.is_empty())
        .then_some(())
        .ok_or_else(invalid_key_diagnostic);
    browser_action_response(
        target,
        tab_id,
        "press_key",
        validation,
        None,
        BrowserCdpAction::PressKey { key },
        identity,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn eval(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    expression: String,
) -> BrowserEvalResponse {
    eval_with_policy(
        target,
        tab_id,
        expression,
        sky_cua_platform::model::browser_eval_enabled(),
    )
    .await
}

pub(crate) async fn eval_with_policy_and_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    expression: String,
    browser_eval_enabled: bool,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserEvalResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, mut diagnostics) = normalize_action_tab_id(tab_id);
    if !browser_eval_enabled {
        return BrowserEvalResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            value: None,
            diagnostics: vec![DiagnosticEntry {
                code: "BrowserEvalDisabled".to_string(),
                message: "browser_eval is disabled via SKY_CUA_BROWSER_EVAL. Remove the \
                          override, or set it to on, 1, or true, to re-enable \
                          page-JavaScript execution."
                    .to_string(),
                details: None,
            }],
        };
    }
    if expression.trim().is_empty() {
        diagnostics.push(invalid_text_diagnostic());
    }
    if !diagnostics.is_empty() {
        return BrowserEvalResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            value: None,
            diagnostics,
        };
    }

    match run_cdp_action(
        resolved_target,
        &normalized_tab_id,
        BrowserCdpAction::Eval { expression },
        identity,
    )
    .await
    {
        Ok(BrowserCdpResult::Eval { value }) => BrowserEvalResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            value,
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("eval action returns eval result"),
        Err(diagnostic) => BrowserEvalResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            value: None,
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn scroll_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    delta_x: f64,
    delta_y: f64,
    x: Option<f64>,
    y: Option<f64>,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    let coordinates = match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 => {
            Ok(Some((x, y)))
        }
        (None, None) => Ok(None),
        _ => Err(invalid_scroll_diagnostic()),
    };
    let validation = if delta_x.is_finite()
        && delta_y.is_finite()
        && (delta_x != 0.0 || delta_y != 0.0)
        && coordinates.is_ok()
    {
        Ok(())
    } else {
        Err(invalid_scroll_diagnostic())
    };
    let coordinates = coordinates.ok().flatten();
    let (x, y) = coordinates
        .map(|(x, y)| (Some(x), Some(y)))
        .unwrap_or((None, None));
    browser_action_response(
        target,
        tab_id,
        "scroll",
        validation,
        coordinates,
        BrowserCdpAction::Scroll {
            delta_x,
            delta_y,
            x,
            y,
        },
        identity,
    )
    .await
}

async fn browser_action_response(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    action_name: &'static str,
    action_validation: Result<(), DiagnosticEntry>,
    cursor_before_action: Option<(f64, f64)>,
    action: BrowserCdpAction,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserActionResponse {
    let resolved_target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let (normalized_tab_id, mut diagnostics) = normalize_action_tab_id(tab_id);
    if let Err(diagnostic) = action_validation {
        diagnostics.push(diagnostic);
    }
    if !diagnostics.is_empty() {
        return BrowserActionResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            action: action_name.to_string(),
            diagnostics,
        };
    }

    let executor = match BrowserBridgeExecutor::from_env(
        TokioInstant::now() + browser_open_timeout(),
        identity,
    ) {
        Ok(executor) => executor,
        Err(diagnostic) => {
            return BrowserActionResponse {
                target: resolved_target,
                tab_id: normalized_tab_id,
                action: action_name.to_string(),
                diagnostics: vec![diagnostic],
            };
        }
    };
    let binding = executor.bind_tab(resolved_target, &normalized_tab_id);

    if let Some((x, y)) = cursor_before_action
        && let Err(diagnostic) = binding.move_mouse(x, y, true).await
    {
        return BrowserActionResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            action: action_name.to_string(),
            diagnostics: vec![diagnostic],
        };
    }

    match binding.run_cdp(action).await {
        Ok(BrowserCdpResult::Action) => BrowserActionResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            action: action_name.to_string(),
            diagnostics: Vec::new(),
        },
        Ok(_) => unreachable!("browser action returns action result"),
        Err(diagnostic) => BrowserActionResponse {
            target: resolved_target,
            tab_id: normalized_tab_id,
            action: action_name.to_string(),
            diagnostics: vec![diagnostic],
        },
    }
}

pub(crate) async fn browser_bridge_diagnostics() -> Vec<DiagnosticEntry> {
    match browser_socket_selection_from_env() {
        Ok(selection) => bridge_readiness_diagnostic(selection)
            .await
            .into_iter()
            .collect(),
        Err(diagnostic) => vec![diagnostic],
    }
}

pub(crate) fn browser_env_values_present() -> std::collections::BTreeMap<String, String> {
    super::sockets::browser_env_values_present()
}

async fn bridge_readiness_diagnostic(selection: BrowserSocketSelection) -> Option<DiagnosticEntry> {
    let sockets = find_bridge_sockets(selection);
    if sockets.is_empty() {
        return Some(browser_bridge_disconnected_for_selection(selection));
    }

    first_responsive_bridge_socket(sockets).await.err()
}

async fn run_cdp_action(
    target: BrowserTargetKind,
    tab_id: &str,
    action: BrowserCdpAction,
    identity: Option<BrowserSessionIdentity>,
) -> Result<BrowserCdpResult, DiagnosticEntry> {
    let executor =
        BrowserBridgeExecutor::from_env(TokioInstant::now() + browser_open_timeout(), identity)?;
    executor.bind_tab(target, tab_id).run_cdp(action).await
}

#[cfg(test)]
mod appshot_capture_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stable_generation_is_accepted() {
        let before = json!({"documentGeneration": "doc-1"});
        assert_eq!(
            classify_document_capture(Some("u"), Some("u"), Some(&before), Some(&before), true, 0),
            AppShotConsistency::Stable
        );
    }

    #[test]
    fn generation_changes_are_reported_across_the_single_retry() {
        let before = json!({"documentGeneration": "doc-1"});
        let after = json!({"documentGeneration": "doc-2"});
        assert_eq!(
            classify_document_capture(Some("u"), Some("u"), Some(&before), Some(&after), true, 0),
            AppShotConsistency::Partial
        );
        assert_eq!(
            classify_document_capture(Some("u"), Some("u"), Some(&before), Some(&after), true, 1),
            AppShotConsistency::ChangedDuringCapture
        );
    }

    #[test]
    fn browser_artifacts_are_private_unique_and_exactly_hashed() {
        let first = persist_browser_appshot_artifact(b"first", "image/png").unwrap();
        let second = persist_browser_appshot_artifact(b"second", "image/png").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&first).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(appshot_artifacts_dir())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
    }

    #[test]
    fn image_attachment_is_omitted_for_text_only_capture() {
        assert!(attachment_data(b"pixels", false).is_empty());
        assert_eq!(attachment_data(b"pixels", true), "cGl4ZWxz");
    }
}
