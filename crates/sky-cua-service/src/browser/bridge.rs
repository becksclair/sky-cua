use std::path::Path;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sky_cua_platform::appshot_artifacts_dir;
use sky_cua_platform::model::{
    APPSHOT_ARTIFACT_DEFAULT_LEASE_MS, AppShotActionSnapshot, AppShotBrowserCaptureOutcome,
    AppShotBrowserCaptureStatus, AppShotBrowserReadiness, AppShotBrowserReadinessState,
    AppShotCapture, AppShotConsistency, AppShotCoverage, AppShotEnvelope, AppShotTrigger,
    BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS, BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS,
    BrowserActionResponse, BrowserAppShotResponse, BrowserClaimTabResponse, BrowserEvalResponse,
    BrowserListTabsResponse, BrowserMoveMouseResponse, BrowserNavigateResponse,
    BrowserOpenResponse, BrowserScreenshotResponse, BrowserSessionIdentity,
    BrowserSnapshotResponse, BrowserTargetKind, ContentPersistence, ContentRef, ContentSource,
    DiagnosticEntry, PixelSize, normalize_browser_open_url,
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
            total: 0,
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn observe_appshot_with_identity(
    target: Option<BrowserTargetKind>,
    tab_id: String,
    text_limit: Option<usize>,
    element_offset: Option<usize>,
    element_limit: Option<usize>,
    element_query: Option<String>,
    include_image_data: bool,
    capture_timeout_ms: Option<u64>,
    identity: Option<BrowserSessionIdentity>,
) -> BrowserAppShotResponse {
    let target = target.unwrap_or(BrowserTargetKind::UserChrome);
    let tab_id = tab_id.trim().to_string();
    let timeout_ms = capture_timeout_ms
        .unwrap_or_else(|| {
            adaptive_capture_timeout_ms(
                text_limit,
                element_offset,
                element_limit,
                include_image_data,
            )
        })
        .clamp(
            BROWSER_APPSHOT_MIN_CAPTURE_TIMEOUT_MS,
            BROWSER_APPSHOT_MAX_CAPTURE_TIMEOUT_MS,
        );
    let deadline = TokioInstant::now() + Duration::from_millis(timeout_ms);
    let mut progress = CaptureProgress::new(tab_id.clone(), include_image_data, identity.clone());

    for attempt in 0..2 {
        if attempt > 0 {
            progress.snapshot = None;
            progress.screenshot = None;
            progress.fence_metadata = None;
        }
        if progress.metadata.is_none() {
            progress.phase = Some("metadata".into());
            match run_cdp_action_until(
                target,
                &tab_id,
                BrowserCdpAction::Metadata,
                deadline,
                identity.clone(),
            )
            .await
            {
                Ok(BrowserCdpResult::Metadata { metadata }) => {
                    progress.metadata = Some(metadata);
                }
                Ok(_) => unreachable!("metadata action returns metadata result"),
                Err(diagnostic) => {
                    let deadline_exceeded = is_capture_deadline(&diagnostic, deadline);
                    return progress_response(
                        progress,
                        deadline,
                        timeout_ms,
                        diagnostic,
                        deadline_exceeded,
                    );
                }
            }
        }
        if deadline <= TokioInstant::now() {
            return progress_response(
                progress,
                deadline,
                timeout_ms,
                deadline_diagnostic("metadata", timeout_ms),
                true,
            );
        }

        progress.phase = Some("semantics".into());
        let snapshot = match run_cdp_action_until(
            target,
            &tab_id,
            BrowserCdpAction::Snapshot {
                text_limit,
                element_offset,
                element_limit,
                element_query: element_query.clone(),
            },
            deadline,
            identity.clone(),
        )
        .await
        {
            Ok(BrowserCdpResult::Snapshot {
                title,
                url,
                snapshot,
            }) => {
                let response = BrowserSnapshotResponse {
                    target,
                    tab_id: tab_id.clone(),
                    title,
                    url,
                    snapshot: Some(snapshot),
                    diagnostics: Vec::new(),
                };
                progress.snapshot = Some(response.clone());
                response
            }
            Ok(_) => unreachable!("snapshot action returns snapshot result"),
            Err(diagnostic) => {
                return progress_response(
                    progress,
                    deadline,
                    timeout_ms,
                    diagnostic.clone(),
                    is_capture_deadline(&diagnostic, deadline),
                );
            }
        };

        if deadline <= TokioInstant::now() {
            return progress_response(
                progress,
                deadline,
                timeout_ms,
                deadline_diagnostic("semantics", timeout_ms),
                true,
            );
        }

        progress.phase = Some("pixels".into());
        let screenshot = match run_cdp_action_until(
            target,
            &tab_id,
            BrowserCdpAction::Screenshot,
            deadline,
            identity.clone(),
        )
        .await
        {
            Ok(BrowserCdpResult::Screenshot {
                data_base64,
                css_width,
                css_height,
            }) => {
                let tab_id_for_task = tab_id.clone();
                let task = tokio::task::spawn_blocking(move || {
                    super::model_image::prepare_browser_capture(
                        &tab_id_for_task,
                        &data_base64,
                        css_width,
                        css_height,
                        include_image_data,
                    )
                });
                match tokio::time::timeout_at(deadline, task).await {
                    Ok(Ok(prepared)) => {
                        progress.screenshot = Some(prepared.clone());
                        prepared
                    }
                    Ok(Err(join_error)) => {
                        let diagnostic = DiagnosticEntry {
                            code: "BrowserScreenshotDegraded".into(),
                            message: format!(
                                "Browser screenshot post-processing task failed to join cleanly: {join_error}"
                            ),
                            details: None,
                        };
                        return progress_response(
                            progress, deadline, timeout_ms, diagnostic, false,
                        );
                    }
                    Err(_) => {
                        return progress_response(
                            progress,
                            deadline,
                            timeout_ms,
                            deadline_diagnostic("pixel_post_processing", timeout_ms),
                            true,
                        );
                    }
                }
            }
            Ok(_) => unreachable!("screenshot action returns screenshot result"),
            Err(diagnostic) => {
                return progress_response(
                    progress,
                    deadline,
                    timeout_ms,
                    diagnostic.clone(),
                    is_capture_deadline(&diagnostic, deadline),
                );
            }
        };

        if deadline <= TokioInstant::now() {
            return progress_response(
                progress,
                deadline,
                timeout_ms,
                deadline_diagnostic("pixels", timeout_ms),
                true,
            );
        }

        progress.phase = Some("fence".into());
        let after = match run_cdp_action_until(
            target,
            &tab_id,
            BrowserCdpAction::Metadata,
            deadline,
            identity.clone(),
        )
        .await
        {
            Ok(BrowserCdpResult::Metadata { metadata }) => metadata,
            Ok(_) => unreachable!("metadata action returns metadata result"),
            Err(diagnostic) => {
                return progress_response(
                    progress,
                    deadline,
                    timeout_ms,
                    diagnostic.clone(),
                    is_capture_deadline(&diagnostic, deadline),
                );
            }
        };
        progress.fence_metadata = Some(after.clone());

        let bytes = screenshot.bytes.as_ref();
        let metadata = progress
            .fence_metadata
            .as_ref()
            .or(progress.metadata.as_ref());
        let snapshot_value = snapshot.snapshot.as_ref();
        let semantic_viewport = snapshot_value.and_then(|value| value.get("viewport"));
        let width = screenshot.width;
        let height = screenshot.height;
        let viewport_matches = semantic_viewport
            .and_then(|value| value.get("width"))
            .and_then(|value| value.as_u64())
            .zip(
                semantic_viewport
                    .and_then(|value| value.get("height"))
                    .and_then(|value| value.as_u64()),
            )
            .is_some_and(|(width, height)| {
                width as u32 == screenshot.width && height as u32 == screenshot.height
            });
        let image_matches_viewport = !bytes.is_empty();
        let mut hasher = Sha256::new();
        hasher.update(
            snapshot_value
                .and_then(|v| v.get("documentGeneration"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    metadata
                        .and_then(|v| v.get("documentGeneration"))
                        .and_then(Value::as_str)
                })
                .or(snapshot.url.as_deref())
                .unwrap_or("")
                .as_bytes(),
        );
        let digest = hasher.finalize();
        let generation = u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]));
        let mut diagnostics = snapshot.diagnostics.clone();
        diagnostics.extend(screenshot.diagnostics.clone());
        let mut consistency = classify_document_capture(
            snapshot.url.as_deref(),
            after.get("url").and_then(Value::as_str),
            snapshot_value,
            Some(&after),
            screenshot.diagnostics.is_empty()
                && !bytes.is_empty()
                && viewport_matches
                && image_matches_viewport
                && semantic_viewport == after.get("viewport")
                && snapshot.target == target
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
            .map(|d| (d.as_millis() as u64).saturating_add(APPSHOT_ARTIFACT_DEFAULT_LEASE_MS));
        let capture_was_consistent = consistency == AppShotConsistency::Stable;
        progress.phase = Some("artifact".into());
        let artifact_bytes = screenshot.bytes.clone();
        let artifact_mime_type = screenshot.mime_type.clone();
        let artifact_task = tokio::task::spawn_blocking(move || {
            persist_browser_appshot_artifact(&artifact_bytes, &artifact_mime_type)
        });
        let final_artifact = match tokio::time::timeout_at(deadline, artifact_task).await {
            Ok(Ok(Ok(path))) => Some(path),
            Ok(Ok(Err(error))) => {
                diagnostics.push(DiagnosticEntry {
                    code: "BrowserArtifactPersistFailed".into(),
                    message: format!(
                        "Browser AppShot pixels could not be committed to a private artifact: {error}"
                    ),
                    details: None,
                });
                None
            }
            Ok(Err(join_error)) => {
                diagnostics.push(DiagnosticEntry {
                    code: "BrowserArtifactPersistFailed".into(),
                    message: format!(
                        "Browser AppShot artifact task failed to join cleanly: {join_error}"
                    ),
                    details: None,
                });
                None
            }
            Err(_) => {
                return progress_response(
                    progress,
                    deadline,
                    timeout_ms,
                    deadline_diagnostic("artifact_persistence", timeout_ms),
                    true,
                );
            }
        };
        if final_artifact.is_none() && consistency == AppShotConsistency::Stable {
            consistency = AppShotConsistency::Partial;
        }
        let image = capture_content_ref(&screenshot, final_artifact.as_deref(), expires_at_ms);
        let envelope = AppShotEnvelope {
            appshot_id: uuid::Uuid::new_v4().to_string(),
            trigger: AppShotTrigger::Observe,
            captured_at: chrono::Utc::now(),
            consistency,
            capture: AppShotCapture::Browser {
                tab_id: tab_id.clone(),
                url: snapshot
                    .url
                    .clone()
                    .or_else(|| {
                        metadata
                            .and_then(|value| value.get("url"))
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned)
                    })
                    .unwrap_or_default(),
                title: snapshot.title.clone(),
                viewport: PixelSize { width, height },
                document_generation: generation,
                semantic_snapshot: snapshot
                    .snapshot
                    .clone()
                    .unwrap_or_else(|| serde_json::json!({})),
                readiness: readiness_from_metadata(Some(&after)),
                capture_outcome: AppShotBrowserCaptureOutcome {
                    status: if consistency == AppShotConsistency::Stable {
                        AppShotBrowserCaptureStatus::Complete
                    } else {
                        AppShotBrowserCaptureStatus::Partial
                    },
                    retryable: false,
                    phase: None,
                    timeout_ms: None,
                },
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
        if attempt == 1 || capture_was_consistent {
            return BrowserAppShotResponse {
                appshot: envelope,
                image_data_base64: attachment_data(bytes, include_image_data),
                image_mime_type: screenshot.mime_type,
            };
        }
        progress.last_screenshot = Some(screenshot);
        progress.last_envelope = Some(envelope);
        progress.metadata = Some(after);
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    progress_response(
        progress,
        deadline,
        timeout_ms,
        deadline_diagnostic("consistency_retry", timeout_ms),
        true,
    )
}

#[derive(Debug)]
struct CaptureProgress {
    tab_id: String,
    identity: Option<BrowserSessionIdentity>,
    include_image_data: bool,
    metadata: Option<Value>,
    fence_metadata: Option<Value>,
    snapshot: Option<BrowserSnapshotResponse>,
    screenshot: Option<super::model_image::PreparedBrowserCapture>,
    last_envelope: Option<AppShotEnvelope>,
    last_screenshot: Option<super::model_image::PreparedBrowserCapture>,
    phase: Option<String>,
}

impl CaptureProgress {
    fn new(
        tab_id: String,
        include_image_data: bool,
        identity: Option<BrowserSessionIdentity>,
    ) -> Self {
        Self {
            tab_id,
            identity,
            include_image_data,
            metadata: None,
            fence_metadata: None,
            snapshot: None,
            screenshot: None,
            last_envelope: None,
            last_screenshot: None,
            phase: None,
        }
    }
}

fn adaptive_capture_timeout_ms(
    text_limit: Option<usize>,
    element_offset: Option<usize>,
    element_limit: Option<usize>,
    include_image_data: bool,
) -> u64 {
    let text_units = text_limit.unwrap_or(4_000).saturating_add(3_999) / 4_000;
    let element_end = element_offset
        .unwrap_or(0)
        .saturating_add(element_limit.unwrap_or(200))
        .min(5_000);
    let element_units = element_end.saturating_add(199) / 200;
    let image_ms = if include_image_data { 500 } else { 0 };
    6_000_u64
        .saturating_add((text_units as u64).saturating_mul(500))
        .saturating_add((element_units as u64).saturating_mul(250))
        .saturating_add(image_ms)
        .clamp(6_000, 15_000)
}

fn is_capture_deadline(diagnostic: &DiagnosticEntry, deadline: TokioInstant) -> bool {
    TokioInstant::now() >= deadline
        || diagnostic.code == "BrowserBridgeRequestTimedOut"
        || (diagnostic.code == "BrowserBridgeRequestFailed"
            && diagnostic.message.contains("Timed out after"))
}

fn deadline_diagnostic(phase: &str, timeout_ms: u64) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserCaptureDeadlineExceeded".into(),
        message: format!(
            "Browser AppShot capture exceeded its {timeout_ms} ms aggregate deadline during {phase}."
        ),
        details: None,
    }
}

fn capture_content_ref(
    capture: &super::model_image::PreparedBrowserCapture,
    private_artifact: Option<&Path>,
    private_expires_at_ms: Option<u64>,
) -> ContentRef {
    let private_filename = private_artifact
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    let fallback_filename = capture
        .screenshot_path
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .map(|name| name.to_string_lossy().into_owned());
    ContentRef {
        content_id: uuid::Uuid::new_v4().to_string(),
        device_id: None,
        link_epoch: None,
        mime_type: capture.mime_type.clone(),
        filename: private_filename.or(fallback_filename),
        size_bytes: capture.bytes.len() as u64,
        sha256: capture.sha256.clone(),
        source: if private_artifact.is_some() {
            ContentSource::HostPrivateArtifact
        } else {
            ContentSource::Screenshot
        },
        expires_at_ms: private_artifact
            .is_some()
            .then_some(private_expires_at_ms)
            .flatten(),
        persistence: ContentPersistence::Temporary,
    }
}

fn readiness_from_metadata(metadata: Option<&Value>) -> AppShotBrowserReadiness {
    let Some(metadata) = metadata else {
        return AppShotBrowserReadiness::default();
    };
    let raw_ready_state = metadata
        .get("readyState")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let state = if matches!(raw_ready_state.as_deref(), Some("interactive" | "complete"))
        && metadata.get("bodyPresent").and_then(Value::as_bool) == Some(true)
    {
        AppShotBrowserReadinessState::Ready
    } else if raw_ready_state.is_some() {
        AppShotBrowserReadinessState::Loading
    } else {
        AppShotBrowserReadinessState::Unknown
    };
    AppShotBrowserReadiness {
        state,
        raw_ready_state,
    }
}

fn metadata_viewport(metadata: Option<&Value>) -> Option<PixelSize> {
    let viewport = metadata?.get("viewport")?;
    Some(PixelSize {
        width: viewport.get("width")?.as_u64()?.try_into().ok()?,
        height: viewport.get("height")?.as_u64()?.try_into().ok()?,
    })
}

fn generation_for_capture(
    snapshot: Option<&BrowserSnapshotResponse>,
    metadata: Option<&Value>,
    url: &str,
) -> u64 {
    let source = snapshot
        .and_then(|value| value.snapshot.as_ref())
        .and_then(|value| value.get("documentGeneration"))
        .and_then(Value::as_str)
        .or_else(|| {
            metadata
                .and_then(|value| value.get("documentGeneration"))
                .and_then(Value::as_str)
        })
        .unwrap_or(url);
    let digest = Sha256::digest(source.as_bytes());
    u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]))
}

fn progress_response(
    mut progress: CaptureProgress,
    _deadline: TokioInstant,
    timeout_ms: u64,
    diagnostic: DiagnosticEntry,
    deadline_exceeded: bool,
) -> BrowserAppShotResponse {
    let phase = progress.phase.clone();
    let outcome = AppShotBrowserCaptureOutcome {
        status: if deadline_exceeded {
            AppShotBrowserCaptureStatus::DeadlineExceeded
        } else {
            AppShotBrowserCaptureStatus::Partial
        },
        retryable: deadline_exceeded,
        phase,
        timeout_ms: deadline_exceeded.then_some(timeout_ms),
    };
    let deadline_entry = deadline_exceeded
        .then(|| deadline_diagnostic(progress.phase.as_deref().unwrap_or("unknown"), timeout_ms));
    if let Some(mut envelope) = progress.last_envelope.take() {
        if !envelope
            .diagnostics
            .iter()
            .any(|item| item.code == diagnostic.code)
        {
            envelope.diagnostics.push(diagnostic);
        }
        if let Some(deadline_entry) = deadline_entry
            && !envelope
                .diagnostics
                .iter()
                .any(|item| item.code == deadline_entry.code)
        {
            envelope.diagnostics.push(deadline_entry);
        }
        if let AppShotCapture::Browser {
            capture_outcome, ..
        } = &mut envelope.capture
        {
            *capture_outcome = outcome;
        }
        let bytes = progress
            .last_screenshot
            .as_ref()
            .map(|capture| capture.bytes.as_ref())
            .unwrap_or_default();
        return BrowserAppShotResponse {
            image_data_base64: attachment_data(bytes, progress.include_image_data),
            image_mime_type: progress
                .last_screenshot
                .as_ref()
                .map(|value| value.mime_type.clone())
                .unwrap_or_else(|| envelope.image.mime_type.clone()),
            appshot: envelope,
        };
    }

    let metadata = progress
        .fence_metadata
        .as_ref()
        .or(progress.metadata.as_ref());
    let snapshot = progress.snapshot.as_ref();
    let url = snapshot
        .and_then(|value| value.url.clone())
        .or_else(|| {
            metadata
                .and_then(|value| value.get("url"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default();
    let title = snapshot.and_then(|value| value.title.clone()).or_else(|| {
        metadata
            .and_then(|value| value.get("title"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    });
    let screenshot = progress.screenshot.as_ref();
    let bytes = screenshot
        .map(|capture| capture.bytes.as_ref())
        .unwrap_or_default();
    let viewport = snapshot
        .and_then(|value| value.snapshot.as_ref())
        .and_then(|value| value.get("viewport"))
        .and_then(|value| {
            Some(PixelSize {
                width: value.get("width")?.as_u64()?.try_into().ok()?,
                height: value.get("height")?.as_u64()?.try_into().ok()?,
            })
        })
        .or_else(|| metadata_viewport(metadata))
        .or_else(|| {
            screenshot.map(|value| PixelSize {
                width: value.width,
                height: value.height,
            })
        })
        .unwrap_or(PixelSize {
            width: 0,
            height: 0,
        });
    let expires_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|value| value.as_millis() as u64 + 60 * 60 * 1000);
    let image = screenshot.map_or_else(
        || ContentRef {
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
        |capture| capture_content_ref(capture, None, expires_at_ms),
    );
    let mut diagnostics = snapshot
        .map(|value| value.diagnostics.clone())
        .unwrap_or_default();
    if !diagnostics.iter().any(|item| item.code == diagnostic.code) {
        diagnostics.push(diagnostic);
    }
    if let Some(deadline_entry) = deadline_entry
        && !diagnostics
            .iter()
            .any(|item| item.code == deadline_entry.code)
    {
        diagnostics.push(deadline_entry);
    }
    let semantic_snapshot = snapshot
        .and_then(|value| value.snapshot.clone())
        .unwrap_or_else(|| serde_json::json!({}));
    let envelope = AppShotEnvelope {
        appshot_id: uuid::Uuid::new_v4().to_string(),
        trigger: AppShotTrigger::Observe,
        captured_at: chrono::Utc::now(),
        consistency: AppShotConsistency::Partial,
        capture: AppShotCapture::Browser {
            tab_id: progress.tab_id,
            url: url.clone(),
            title,
            viewport,
            document_generation: generation_for_capture(snapshot, metadata, &url),
            semantic_snapshot: semantic_snapshot.clone(),
            readiness: readiness_from_metadata(metadata),
            capture_outcome: outcome,
        },
        image,
        action_snapshot: AppShotActionSnapshot {
            snapshot_id: uuid::Uuid::new_v4().to_string(),
            session_id: progress.identity.map(|value| value.session_id),
            subject_generation: Some(generation_for_capture(snapshot, metadata, &url)),
        },
        coverage: AppShotCoverage {
            pixels_complete: !bytes.is_empty(),
            semantics_complete: snapshot.and_then(|value| value.snapshot.as_ref()).is_some(),
            secure_regions_redacted: false,
            projection_truncated: semantic_snapshot
                .get("textTruncated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            total_semantic_nodes: semantic_snapshot
                .get("elementCount")
                .and_then(Value::as_u64),
            projected_semantic_nodes: semantic_snapshot
                .get("elements")
                .and_then(Value::as_array)
                .map(|value| value.len() as u64),
        },
        capability_profile_id: "browser-v1".into(),
        diagnostics,
    };
    let image_mime_type = envelope.image.mime_type.clone();
    BrowserAppShotResponse {
        image_data_base64: attachment_data(bytes, progress.include_image_data),
        image_mime_type,
        appshot: envelope,
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

const BROWSER_APPSHOT_MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;

fn persist_browser_appshot_artifact(
    bytes: &[u8],
    mime_type: &str,
) -> std::io::Result<std::path::PathBuf> {
    if bytes.is_empty() || bytes.len() > BROWSER_APPSHOT_MAX_ARTIFACT_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "browser AppShot artifact size {} is outside the 1-{} byte range",
                bytes.len(),
                BROWSER_APPSHOT_MAX_ARTIFACT_BYTES
            ),
        ));
    }
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
        .checked_sub(Duration::from_millis(APPSHOT_ARTIFACT_DEFAULT_LEASE_MS))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    for entry in std::fs::read_dir(&root)?.flatten().take(256) {
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
    run_cdp_action_until(
        target,
        tab_id,
        action,
        TokioInstant::now() + browser_open_timeout(),
        identity,
    )
    .await
}

async fn run_cdp_action_until(
    target: BrowserTargetKind,
    tab_id: &str,
    action: BrowserCdpAction,
    deadline: TokioInstant,
    identity: Option<BrowserSessionIdentity>,
) -> Result<BrowserCdpResult, DiagnosticEntry> {
    let executor = BrowserBridgeExecutor::from_env(deadline, identity)?;
    executor.bind_tab(target, tab_id).run_cdp(action).await
}

#[cfg(test)]
mod appshot_capture_tests {
    use serde_json::json;

    use super::*;

    fn prepared_capture(bytes: &[u8]) -> super::super::model_image::PreparedBrowserCapture {
        super::super::model_image::PreparedBrowserCapture {
            bytes: std::sync::Arc::from(bytes.to_vec()),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
            mime_type: "image/png".into(),
            screenshot_path: None,
            width: 10,
            height: 10,
            diagnostics: Vec::new(),
        }
    }

    fn browser_envelope(
        capture: &super::super::model_image::PreparedBrowserCapture,
    ) -> AppShotEnvelope {
        AppShotEnvelope {
            appshot_id: "shot-1".into(),
            trigger: AppShotTrigger::Observe,
            captured_at: chrono::Utc::now(),
            consistency: AppShotConsistency::Partial,
            capture: AppShotCapture::Browser {
                tab_id: "tab-1".into(),
                url: "https://example.test/".into(),
                title: Some("Example".into()),
                viewport: PixelSize {
                    width: 10,
                    height: 10,
                },
                document_generation: 7,
                semantic_snapshot: json!({"elementCount": 1, "elements": [{}]}),
                readiness: AppShotBrowserReadiness::default(),
                capture_outcome: AppShotBrowserCaptureOutcome {
                    status: AppShotBrowserCaptureStatus::Partial,
                    retryable: false,
                    phase: None,
                    timeout_ms: None,
                },
            },
            image: capture_content_ref(capture, None, None),
            action_snapshot: AppShotActionSnapshot {
                snapshot_id: "actions-1".into(),
                session_id: Some("session-1".into()),
                subject_generation: Some(7),
            },
            coverage: AppShotCoverage {
                pixels_complete: true,
                semantics_complete: true,
                secure_regions_redacted: false,
                projection_truncated: false,
                total_semantic_nodes: Some(1),
                projected_semantic_nodes: Some(1),
            },
            capability_profile_id: "browser-v1".into(),
            diagnostics: Vec::new(),
        }
    }

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
    fn browser_artifact_rejects_empty_bytes() {
        let error = persist_browser_appshot_artifact(b"", "image/png").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn image_attachment_is_omitted_for_text_only_capture() {
        assert!(attachment_data(b"pixels", false).is_empty());
        assert_eq!(attachment_data(b"pixels", true), "cGl4ZWxz");
    }

    #[test]
    fn adaptive_capture_budget_scales_and_clamps_requested_work() {
        assert_eq!(
            adaptive_capture_timeout_ms(Some(4_000), Some(0), Some(200), true),
            7_250
        );
        assert_eq!(
            adaptive_capture_timeout_ms(Some(20_000), Some(4_900), Some(5_000), true),
            15_000
        );
        assert_eq!(
            adaptive_capture_timeout_ms(Some(20_000), Some(usize::MAX), Some(usize::MAX), true),
            15_000
        );
    }

    #[test]
    fn wrapped_cdp_timeout_is_retryable_but_attach_failure_is_not() {
        let deadline = TokioInstant::now() + Duration::from_secs(1);
        assert!(is_capture_deadline(
            &DiagnosticEntry {
                code: "BrowserBridgeRequestFailed".into(),
                message: "Timed out after 1ms waiting for CDP command Runtime.evaluate.".into(),
                details: None,
            },
            deadline,
        ));
        assert!(!is_capture_deadline(
            &DiagnosticEntry {
                code: "BrowserBridgeRequestFailed".into(),
                message: "The browser debugger is unattached.".into(),
                details: None,
            },
            deadline,
        ));
    }

    #[test]
    fn retry_deadline_returns_matching_first_attempt_envelope_and_pixels() {
        let first = prepared_capture(b"first-attempt");
        let second = prepared_capture(b"second-attempt");
        let mut progress = CaptureProgress::new("tab-1".into(), true, None);
        progress.phase = Some("fence".into());
        progress.last_envelope = Some(browser_envelope(&first));
        progress.last_screenshot = Some(first.clone());
        progress.screenshot = Some(second);

        let response = progress_response(
            progress,
            TokioInstant::now() + Duration::from_secs(1),
            1_000,
            deadline_diagnostic("fence", 1_000),
            true,
        );

        assert_eq!(
            response.image_data_base64,
            base64::engine::general_purpose::STANDARD.encode(b"first-attempt")
        );
        assert_eq!(response.appshot.image.sha256, first.sha256);
    }

    #[test]
    fn content_ref_claims_private_artifact_only_when_one_exists() {
        let capture = prepared_capture(b"pixels");
        let fallback = capture_content_ref(&capture, None, Some(123));
        assert_eq!(fallback.source, ContentSource::Screenshot);
        assert_eq!(fallback.expires_at_ms, None);

        let artifact = persist_browser_appshot_artifact(b"pixels", "image/png").unwrap();
        let private = capture_content_ref(&capture, Some(&artifact), Some(123));
        assert_eq!(private.source, ContentSource::HostPrivateArtifact);
        assert_eq!(private.expires_at_ms, Some(123));
        assert_eq!(
            private.filename.as_deref(),
            artifact.file_name().and_then(|name| name.to_str())
        );
        let _ = std::fs::remove_file(artifact);
    }

    #[test]
    fn deadline_progress_preserves_metadata_and_is_retryable() {
        let mut progress = CaptureProgress::new("tab-1".into(), false, None);
        progress.phase = Some("semantics".into());
        progress.metadata = Some(serde_json::json!({
            "title": "Loading",
            "url": "https://example.test/messages",
            "documentGeneration": "doc-1",
            "readyState": "interactive",
            "bodyPresent": true,
            "paintObserved": true,
            "viewport": {"width": 1400, "height": 885}
        }));
        let response = progress_response(
            progress,
            TokioInstant::now() + Duration::from_secs(1),
            1_000,
            deadline_diagnostic("semantics", 1_000),
            true,
        );
        let AppShotCapture::Browser {
            url,
            viewport,
            document_generation,
            readiness,
            capture_outcome,
            ..
        } = response.appshot.capture
        else {
            panic!("expected browser AppShot");
        };
        assert_eq!(url, "https://example.test/messages");
        assert_eq!(viewport.width, 1400);
        assert_eq!(viewport.height, 885);
        assert_ne!(document_generation, 0);
        assert_eq!(readiness.state, AppShotBrowserReadinessState::Ready);
        assert_eq!(
            capture_outcome.status,
            AppShotBrowserCaptureStatus::DeadlineExceeded
        );
        assert!(capture_outcome.retryable);
        assert_eq!(capture_outcome.phase.as_deref(), Some("semantics"));
        assert!(
            response
                .appshot
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "BrowserCaptureDeadlineExceeded")
        );
    }
}
