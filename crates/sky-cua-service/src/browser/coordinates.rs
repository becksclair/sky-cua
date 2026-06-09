use std::path::Path;

use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::protocol::VIEWPORT_SCALE_REQUEST_ID;
use super::snapshot;
use super::transport::execute_cdp_until;

pub(super) async fn screenshot_point_to_css_point_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    x: f64,
    y: f64,
    deadline: TokioInstant,
) -> Result<(f64, f64), DiagnosticEntry> {
    let scale = browser_coordinate_scale_until(stream, socket, tab_id, deadline).await?;
    Ok((
        device_pixels_to_css_pixels(x, scale),
        device_pixels_to_css_pixels(y, scale),
    ))
}

pub(super) async fn browser_coordinate_scale_until(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id: &Value,
    deadline: TokioInstant,
) -> Result<f64, DiagnosticEntry> {
    let response = execute_cdp_until(
        stream,
        socket,
        VIEWPORT_SCALE_REQUEST_ID,
        tab_id,
        "Runtime.evaluate",
        json!({
            "expression": "(() => ({ devicePixelRatio: window.devicePixelRatio || 1 }))()",
            "awaitPromise": true,
            "returnByValue": true,
        }),
        deadline,
    )
    .await?;
    let scale = snapshot::cdp_runtime_value(&response)
        .and_then(|value| value.get("devicePixelRatio").and_then(Value::as_f64))
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(1.0);
    Ok(scale)
}

pub(super) fn device_pixels_to_css_pixels(value: f64, scale: f64) -> f64 {
    value / scale
}
