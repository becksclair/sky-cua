use std::time::Duration;

use serde_json::Value;
use sky_cua_platform::model::{BrowserSessionIdentity, DiagnosticEntry};
use tokio::net::UnixStream;
use tokio::time::{Instant, sleep};

use super::protocol::{NAVIGATION_METADATA_REQUEST_ID, NAVIGATION_RENDER_REQUEST_ID};
use super::snapshot;
use super::transport::execute_cdp_until;

const READINESS_BUDGET: Duration = Duration::from_secs(5);
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Default)]
pub(super) struct NavigationMetadata {
    pub(super) identity: Option<String>,
    pub(super) ready_state: Option<String>,
    pub(super) body_present: Option<bool>,
    pub(super) paint_observed: Option<bool>,
}

pub(super) async fn read_metadata(
    stream: &mut UnixStream,
    socket: &std::path::Path,
    tab_id: &Value,
    deadline: Instant,
    identity: &BrowserSessionIdentity,
) -> Result<NavigationMetadata, DiagnosticEntry> {
    let response = execute_cdp_until(
        stream,
        socket,
        NAVIGATION_METADATA_REQUEST_ID,
        tab_id,
        "Runtime.evaluate",
        snapshot::metadata_evaluate_params(),
        deadline,
        identity,
    )
    .await?;
    let value = snapshot::cdp_runtime_value(&response).ok_or_else(|| DiagnosticEntry {
        code: "BrowserBridgeRequestFailed".to_string(),
        message: "Browser metadata CDP response did not include a runtime value.".to_string(),
        details: None,
    })?;
    Ok(NavigationMetadata {
        identity: value
            .get("documentGeneration")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        ready_state: value
            .get("readyState")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        body_present: value.get("bodyPresent").and_then(Value::as_bool),
        paint_observed: value.get("paintObserved").and_then(Value::as_bool),
    })
}

pub(super) async fn wait_for_navigation_readiness(
    stream: &mut UnixStream,
    socket: &std::path::Path,
    tab_id: &Value,
    before: Option<NavigationMetadata>,
    deadline: Instant,
    identity: &BrowserSessionIdentity,
) {
    let readiness_deadline = std::cmp::min(deadline, Instant::now() + READINESS_BUDGET);
    let before_identity = before.and_then(|metadata| metadata.identity);
    loop {
        if Instant::now() >= readiness_deadline {
            return;
        }
        match read_metadata(stream, socket, tab_id, readiness_deadline, identity).await {
            Ok(metadata) => {
                let is_new_document = before_identity
                    .as_deref()
                    .is_none_or(|before| metadata.identity.as_deref() != Some(before));
                let is_ready = matches!(
                    metadata.ready_state.as_deref(),
                    Some("interactive" | "complete")
                ) && metadata.body_present == Some(true)
                    && metadata.paint_observed == Some(true);
                if is_new_document && is_ready {
                    let _ = execute_cdp_until(
                        stream,
                        socket,
                        NAVIGATION_RENDER_REQUEST_ID,
                        tab_id,
                        "Runtime.evaluate",
                        snapshot::render_opportunity_evaluate_params(),
                        readiness_deadline,
                        identity,
                    )
                    .await;
                    return;
                }
            }
            Err(_) => return,
        }
        if Instant::now() >= readiness_deadline {
            return;
        }
        sleep(READINESS_POLL_INTERVAL).await;
    }
}
