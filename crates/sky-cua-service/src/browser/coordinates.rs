use std::path::Path;

use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::protocol::VIEWPORT_SCALE_REQUEST_ID;
use super::snapshot;
use super::transport::execute_cdp_until;

/// Current CSS viewport geometry for a tab. All browser tool coordinates are
/// CSS pixels, so this is the single source for sizing screenshot captures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ViewportMetrics {
    pub(super) css_width: f64,
    pub(super) css_height: f64,
    /// Page-coordinate scroll offsets, used to clip captures to the visible
    /// viewport when `captureBeyondViewport` forces a full-page repaint.
    pub(super) scroll_x: f64,
    pub(super) scroll_y: f64,
}

pub(super) async fn viewport_metrics_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<ViewportMetrics, DiagnosticEntry> {
    let response = execute_cdp_until(
        stream,
        socket,
        VIEWPORT_SCALE_REQUEST_ID,
        tab_id,
        "Runtime.evaluate",
        json!({
            "expression": "(() => ({ width: innerWidth, height: innerHeight, scrollX: scrollX, scrollY: scrollY }))()",
            "awaitPromise": true,
            "returnByValue": true,
        }),
        deadline,
    )
    .await?;
    let value = snapshot::cdp_runtime_value(&response);
    let number = |name: &str| {
        value
            .as_ref()
            .and_then(|value| value.get(name).and_then(Value::as_f64))
            .filter(|value| value.is_finite())
    };
    Ok(ViewportMetrics {
        css_width: number("width").filter(|v| *v > 0.0).unwrap_or(0.0),
        css_height: number("height").filter(|v| *v > 0.0).unwrap_or(0.0),
        scroll_x: number("scrollX").filter(|v| *v >= 0.0).unwrap_or(0.0),
        scroll_y: number("scrollY").filter(|v| *v >= 0.0).unwrap_or(0.0),
    })
}
