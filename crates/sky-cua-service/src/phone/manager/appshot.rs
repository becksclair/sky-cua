//! CompanionDirect universal phone AppShot capture and host artifact projection.

use std::time::Duration;

use chrono::Utc;
use sha2::{Digest, Sha256};
use sky_cua_platform::{
    appshot_artifacts_dir,
    model::{
        AppShotActionSnapshot, AppShotCapture, AppShotConsistency, AppShotCoverage,
        AppShotEnvelope, ContentPersistence, ContentRef, ContentSource, DiagnosticEntry,
    },
};

use super::{CompanionDirectProvider, PhoneManager, now_ms};
use crate::phone::{mapping, snapshot};

const DEVICE_CAPTURE_DEADLINE_MS: u64 = 2_000;
// Leave enough time for the bounded device capture result (including a
// truthful partial result at its deadline) to be serialized and returned.
const DIRECT_APPSHOT_RPC_TIMEOUT: Duration = Duration::from_secs(5);

impl PhoneManager {
    /// Capture the canonical AppShot from a direct Companion link. The wire
    /// response may contain screenshot bytes, but those are immediately moved
    /// into a private temporary host artifact; the returned envelope contains
    /// descriptors only.
    pub(super) async fn direct_appshot(
        &mut self,
        session_id: &str,
    ) -> Result<AppShotEnvelope, DiagnosticEntry> {
        let (device_id, epoch) =
            self.direct_identity(session_id)
                .ok_or_else(|| DiagnosticEntry {
                    code: "PhoneCompanionDirectUnavailable".into(),
                    message: "session is not backed by CompanionDirect".into(),
                    details: None,
                })?;
        let provider = self
            .direct_provider
            .as_ref()
            .ok_or_else(|| DiagnosticEntry {
                code: "PhoneCompanionDirectUnavailable".into(),
                message: "CompanionDirect provider is unavailable".into(),
                details: None,
            })?;
        let value = provider
            .dispatch(
                &device_id,
                epoch,
                "appshot",
                serde_json::json!({"max_nodes": 5000, "deadline_ms": DEVICE_CAPTURE_DEADLINE_MS}),
                true,
                DIRECT_APPSHOT_RPC_TIMEOUT,
            )
            .await
            .map_err(|error| DiagnosticEntry {
                code: "PhoneAppShotCaptureFailed".into(),
                message: format!("CompanionDirect AppShot capture failed: {error:?}"),
                details: None,
            })?;
        let id = value
            .get("appshot_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let captured_at_ms = value
            .get("captured_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(now_ms);
        let consistency = match value.get("consistency").and_then(serde_json::Value::as_str) {
            Some("stable") => AppShotConsistency::Stable,
            Some("changed_during_capture") => AppShotConsistency::ChangedDuringCapture,
            _ => AppShotConsistency::Partial,
        };
        let foreground = value.get("foreground");
        let package_name = foreground
            .and_then(|v| v.get("package"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let activity_name = foreground
            .and_then(|v| v.get("activity"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let display_id = value
            .get("display")
            .and_then(|v| v.get("id"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32;
        let display_width = value
            .get("display")
            .and_then(|v| v.get("width"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        let display_height = value
            .get("display")
            .and_then(|v| v.get("height"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        let screenshot_width = value
            .get("screenshot")
            .and_then(|v| v.get("width"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(display_width);
        let screenshot_height = value
            .get("screenshot")
            .and_then(|v| v.get("height"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(display_height);
        let window_ids = value
            .get("windows")
            .and_then(serde_json::Value::as_array)
            .map(|xs| {
                xs.iter()
                    .filter_map(|x| {
                        x.get("id")
                            .and_then(serde_json::Value::as_i64)
                            .map(|n| n as i32)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let tree_bytes = if let Some(reference) = value.get("full_tree_content_ref") {
            read_direct_content(provider, reference, &device_id, epoch)?
        } else {
            serde_json::to_vec(&value).map_err(|e| DiagnosticEntry {
                code: "PhoneAppShotEncodeFailed".into(),
                message: e.to_string(),
                details: None,
            })?
        };
        let tree = write_artifact(
            &id,
            "tree.json",
            "application/json",
            &tree_bytes,
            &device_id,
            epoch,
        )?;
        let image =
            if let Some(reference) = value.get("screenshot").and_then(|v| v.get("content_ref")) {
                let bytes = read_direct_content(provider, reference, &device_id, epoch)?;
                let mime = value
                    .get("screenshot")
                    .and_then(|v| v.get("mime_type"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("image/png");
                write_artifact(&id, "screen", mime, &bytes, &device_id, epoch)?
            } else {
                return Err(DiagnosticEntry {
                    code: "PhoneAppShotPartial".into(),
                    message: "CompanionDirect AppShot omitted its screenshot ContentRef".into(),
                    details: None,
                });
            };
        let coverage = value
            .get("coverage")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let (semantic_projection, projected_nodes, host_projection_truncated) =
            project_phone_windows(value.get("windows"), 200);
        if display_width == 0
            || display_height == 0
            || screenshot_width == 0
            || screenshot_height == 0
        {
            return Err(DiagnosticEntry {
                code: "PhoneAppShotCoordinateMappingUnavailable".into(),
                message: "CompanionDirect AppShot omitted valid display or screenshot dimensions"
                    .into(),
                details: None,
            });
        }
        let snapshot_registered_at_ms = now_ms();
        let mapping = mapping::build_mapping(&mapping::MappingBuild {
            mapping_id: &format!("{id}-map"),
            session_id,
            serial: "",
            device_size: sky_cua_platform::model::PixelSize {
                width: display_width,
                height: display_height,
            },
            screenshot_size: sky_cua_platform::model::PixelSize {
                width: screenshot_width,
                height: screenshot_height,
            },
            rotation_degrees: 0,
            host_window_rect: None,
            host_content_rect: None,
            captured_at_ms: snapshot_registered_at_ms,
        })
        .map_err(|error| DiagnosticEntry {
            code: error.code().to_string(),
            message: format!("CompanionDirect AppShot coordinate mapping is invalid: {error}"),
            details: None,
        })?;
        if let Some(entry) = self.sessions.get_mut(session_id) {
            entry.snapshots.register(snapshot::record_from_mapping(
                &id,
                sky_cua_platform::model::PhoneBackendKind::Companion,
                sky_cua_platform::model::PixelSize {
                    width: display_width,
                    height: display_height,
                },
                &mapping,
            ));
        }

        Ok(AppShotEnvelope {
            appshot_id: id.clone(),
            trigger: sky_cua_platform::model::AppShotTrigger::Observe,
            captured_at: chrono::DateTime::<Utc>::from_timestamp_millis(captured_at_ms as i64)
                .unwrap_or_else(Utc::now),
            consistency,
            capture: AppShotCapture::Phone {
                device_id,
                link_epoch: epoch,
                package_name,
                activity_name,
                display_id,
                window_ids,
                semantic_projection,
                event_sequence_before: value
                    .get("event_sequence_before")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                event_sequence_after: value
                    .get("event_sequence_after")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                full_tree_artifact: tree,
            },
            image,
            action_snapshot: AppShotActionSnapshot {
                snapshot_id: id,
                session_id: Some(session_id.to_owned()),
                subject_generation: Some(epoch),
            },
            coverage: AppShotCoverage {
                pixels_complete: coverage
                    .get("pixels_complete")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
                semantics_complete: coverage
                    .get("semantics_complete")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                secure_regions_redacted: coverage
                    .get("secure_regions_redacted")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                projection_truncated: host_projection_truncated
                    || coverage
                        .get("projection_truncated")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                total_semantic_nodes: coverage
                    .get("total_semantic_nodes")
                    .and_then(serde_json::Value::as_u64),
                projected_semantic_nodes: Some(projected_nodes),
            },
            capability_profile_id: self
                .profiles
                .get(session_id)
                .map(|p| p.profile.profile_id.clone())
                .unwrap_or_default(),
            diagnostics: Vec::new(),
        })
    }
}

fn project_phone_windows(
    windows: Option<&serde_json::Value>,
    node_limit: usize,
) -> (serde_json::Value, u64, bool) {
    let Some(windows) = windows.and_then(serde_json::Value::as_array) else {
        return (serde_json::json!([]), 0, false);
    };
    let mut remaining = node_limit;
    let mut total = 0usize;
    let projected = windows
        .iter()
        .map(|window| {
            let mut window = window.clone();
            let truncated = if let Some(nodes) = window
                .get_mut("nodes")
                .and_then(serde_json::Value::as_array_mut)
            {
                let original = nodes.len();
                total = total.saturating_add(original);
                let keep = remaining.min(original);
                nodes.truncate(keep);
                remaining -= keep;
                keep < original
            } else {
                false
            };
            if truncated && let Some(object) = window.as_object_mut() {
                object.insert("projection_truncated".into(), serde_json::Value::Bool(true));
            }
            window
        })
        .collect::<Vec<_>>();
    let projected_nodes = node_limit.saturating_sub(remaining);
    (
        serde_json::Value::Array(projected),
        projected_nodes as u64,
        total > projected_nodes,
    )
}

fn read_direct_content(
    provider: &CompanionDirectProvider,
    value: &serde_json::Value,
    device_id: &str,
    epoch: u64,
) -> Result<Vec<u8>, DiagnosticEntry> {
    let reference: ContentRef =
        serde_json::from_value(value.clone()).map_err(|error| DiagnosticEntry {
            code: "PhoneAppShotContentRefInvalid".into(),
            message: format!("invalid CompanionDirect ContentRef: {error}"),
            details: None,
        })?;
    if reference.device_id.as_deref() != Some(device_id) || reference.link_epoch != Some(epoch) {
        return Err(DiagnosticEntry {
            code: "PhoneAppShotContentIdentityMismatch".into(),
            message: "AppShot ContentRef does not match the authenticated device and link epoch"
                .into(),
            details: None,
        });
    }
    provider
        .runtime()
        .read_content_artifact(device_id, epoch, &reference)
        .map_err(|error| DiagnosticEntry {
            code: "PhoneAppShotContentUnavailable".into(),
            message: format!("AppShot content was not committed: {error}"),
            details: None,
        })
}

fn write_artifact(
    id: &str,
    suffix: &str,
    mime: &str,
    bytes: &[u8],
    device_id: &str,
    epoch: u64,
) -> Result<ContentRef, DiagnosticEntry> {
    let dir = appshot_artifacts_dir();
    std::fs::create_dir_all(&dir).map_err(|e| artifact_error(e.to_string()))?;
    let path = dir.join(format!("phone-{id}-{suffix}"));
    std::fs::write(&path, bytes).map_err(|e| artifact_error(e.to_string()))?;
    let mut hash = Sha256::new();
    hash.update(bytes);
    let sha256 = hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(ContentRef {
        content_id: path.to_string_lossy().into_owned(),
        device_id: Some(device_id.to_owned()),
        link_epoch: Some(epoch),
        mime_type: mime.to_owned(),
        filename: Some(
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ),
        size_bytes: bytes.len() as u64,
        sha256,
        source: ContentSource::HostPrivateArtifact,
        expires_at_ms: Some(now_ms() + 60 * 60 * 1000),
        persistence: ContentPersistence::Temporary,
    })
}
fn artifact_error(message: String) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneAppShotArtifactFailed".into(),
        message,
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use super::project_phone_windows;

    #[test]
    fn phone_semantic_projection_is_bounded_without_changing_full_tree_source() {
        let full = serde_json::json!([
            {"id": 1, "nodes": [{"id": 1}, {"id": 2}]},
            {"id": 2, "nodes": [{"id": 3}, {"id": 4}]}
        ]);
        let (projection, count, truncated) = project_phone_windows(Some(&full), 3);
        assert_eq!(count, 3);
        assert!(truncated);
        assert_eq!(projection[0]["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(projection[1]["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(full[1]["nodes"].as_array().unwrap().len(), 2);
    }
}
