use std::path::Path;

use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::coordinates::{
    browser_coordinate_scale_until, device_pixels_to_css_pixels,
    screenshot_point_to_css_point_until,
};
use super::protocol::{
    CLICK_DOWN_REQUEST_ID, CLICK_MOVE_REQUEST_ID, CLICK_UP_REQUEST_ID, KEY_DOWN_REQUEST_ID,
    KEY_UP_REQUEST_ID, NAVIGATE_REQUEST_ID, SCREENSHOT_REQUEST_ID, SCROLL_REQUEST_ID,
    SNAPSHOT_REQUEST_ID, TYPE_TEXT_REQUEST_ID,
};
use super::snapshot;
use super::transport::execute_cdp_until;

#[derive(Debug)]
pub(super) enum BrowserCdpAction {
    Navigate {
        url: String,
    },
    Snapshot,
    Screenshot,
    Click {
        x: f64,
        y: f64,
    },
    TypeText {
        text: String,
    },
    PressKey {
        key: String,
    },
    Scroll {
        delta_x: f64,
        delta_y: f64,
        x: f64,
        y: f64,
    },
}

pub(super) enum BrowserCdpResult {
    Navigate {
        url: String,
    },
    Snapshot {
        title: Option<String>,
        url: Option<String>,
        snapshot: Value,
    },
    Screenshot {
        data_base64: String,
    },
    Action,
}

pub(super) async fn cdp_action_on_stream(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id_value: &Value,
    action: &BrowserCdpAction,
    deadline: TokioInstant,
) -> Result<BrowserCdpResult, DiagnosticEntry> {
    match action {
        BrowserCdpAction::Navigate { url } => {
            let response = execute_cdp_until(
                stream,
                socket,
                NAVIGATE_REQUEST_ID,
                tab_id_value,
                "Page.navigate",
                json!({ "url": url }),
                deadline,
            )
            .await?;
            if let Some(error_text) = response
                .get("result")
                .and_then(|result| result.get("errorText"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                return Err(DiagnosticEntry {
                    code: "BrowserNavigationFailed".to_string(),
                    message: format!("Browser navigation failed: {error_text}"),
                    details: None,
                });
            }
            Ok(BrowserCdpResult::Navigate { url: url.clone() })
        }
        BrowserCdpAction::Snapshot => {
            let response = execute_cdp_until(
                stream,
                socket,
                SNAPSHOT_REQUEST_ID,
                tab_id_value,
                "Runtime.evaluate",
                snapshot::snapshot_evaluate_params(),
                deadline,
            )
            .await?;
            let (title, url, snapshot) = snapshot::snapshot_from_cdp_response(&response)?;
            Ok(BrowserCdpResult::Snapshot {
                title,
                url,
                snapshot,
            })
        }
        BrowserCdpAction::Screenshot => {
            let response = execute_cdp_until(
                stream,
                socket,
                SCREENSHOT_REQUEST_ID,
                tab_id_value,
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "fromSurface": true,
                    "captureBeyondViewport": true,
                }),
                deadline,
            )
            .await?;
            let data_base64 = response
                .get("result")
                .and_then(|result| result.get("data"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| DiagnosticEntry {
                    code: "BrowserBridgeRequestFailed".to_string(),
                    message: "Browser screenshot CDP response did not include image data."
                        .to_string(),
                    details: None,
                })?
                .to_string();
            Ok(BrowserCdpResult::Screenshot { data_base64 })
        }
        BrowserCdpAction::Click { x, y } => {
            let (css_x, css_y) =
                screenshot_point_to_css_point_until(stream, socket, tab_id_value, *x, *y, deadline)
                    .await?;
            execute_cdp_until(
                stream,
                socket,
                CLICK_MOVE_REQUEST_ID,
                tab_id_value,
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": css_x, "y": css_y }),
                deadline,
            )
            .await?;
            execute_cdp_until(
                stream,
                socket,
                CLICK_DOWN_REQUEST_ID,
                tab_id_value,
                "Input.dispatchMouseEvent",
                json!({ "type": "mousePressed", "x": css_x, "y": css_y, "button": "left", "clickCount": 1 }),
                deadline,
            )
            .await?;
            execute_cdp_until(
                stream,
                socket,
                CLICK_UP_REQUEST_ID,
                tab_id_value,
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseReleased", "x": css_x, "y": css_y, "button": "left", "clickCount": 1 }),
                deadline,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::TypeText { text } => {
            execute_cdp_until(
                stream,
                socket,
                TYPE_TEXT_REQUEST_ID,
                tab_id_value,
                "Input.insertText",
                json!({ "text": text }),
                deadline,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::PressKey { key } => {
            execute_cdp_until(
                stream,
                socket,
                KEY_DOWN_REQUEST_ID,
                tab_id_value,
                "Input.dispatchKeyEvent",
                json!({ "type": "keyDown", "key": key }),
                deadline,
            )
            .await?;
            execute_cdp_until(
                stream,
                socket,
                KEY_UP_REQUEST_ID,
                tab_id_value,
                "Input.dispatchKeyEvent",
                json!({ "type": "keyUp", "key": key }),
                deadline,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::Scroll {
            delta_x,
            delta_y,
            x,
            y,
        } => {
            let scale =
                browser_coordinate_scale_until(stream, socket, tab_id_value, deadline).await?;
            let delta_x = device_pixels_to_css_pixels(*delta_x, scale);
            let delta_y = device_pixels_to_css_pixels(*delta_y, scale);
            let x = device_pixels_to_css_pixels(*x, scale);
            let y = device_pixels_to_css_pixels(*y, scale);
            let expression = format!(
                "(() => {{ window.scrollBy({delta_x}, {delta_y}); return {{ x: window.scrollX, y: window.scrollY, eventX: {x}, eventY: {y} }}; }})()"
            );
            execute_cdp_until(
                stream,
                socket,
                SCROLL_REQUEST_ID,
                tab_id_value,
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                }),
                deadline,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
    }
}
