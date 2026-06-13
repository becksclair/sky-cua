use std::path::Path;

use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::coordinates::viewport_metrics_until;
use super::protocol::{
    CLICK_DOWN_REQUEST_ID, CLICK_MOVE_REQUEST_ID, CLICK_UP_REQUEST_ID, EVAL_REQUEST_ID,
    KEY_DOWN_REQUEST_ID, KEY_UP_REQUEST_ID, NAVIGATE_REQUEST_ID, SCREENSHOT_REQUEST_ID,
    SCROLL_REQUEST_ID, SNAPSHOT_REQUEST_ID, TYPE_TEXT_REQUEST_ID,
};
use super::snapshot;
use super::transport::execute_cdp_until;

#[derive(Debug)]
pub(super) enum BrowserCdpAction {
    Navigate {
        url: String,
    },
    Snapshot {
        text_limit: Option<usize>,
        element_offset: Option<usize>,
        element_limit: Option<usize>,
        element_query: Option<String>,
    },
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
    Eval {
        expression: String,
    },
    Scroll {
        delta_x: f64,
        delta_y: f64,
        x: Option<f64>,
        y: Option<f64>,
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
        css_width: f64,
        css_height: f64,
    },
    Eval {
        value: Option<Value>,
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
        BrowserCdpAction::Snapshot {
            text_limit,
            element_offset,
            element_limit,
            element_query,
        } => {
            let response = execute_cdp_until(
                stream,
                socket,
                SNAPSHOT_REQUEST_ID,
                tab_id_value,
                "Runtime.evaluate",
                snapshot::snapshot_evaluate_params(
                    *text_limit,
                    *element_offset,
                    *element_limit,
                    element_query.as_deref(),
                ),
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
            // Capture the visible viewport only. `captureBeyondViewport`
            // forces a repaint so hidden/occluded windows still produce a
            // frame, while the clip keeps the capture to the current viewport
            // in page coordinates. The image is normalized to CSS-pixel
            // dimensions afterwards so screenshot pixels, snapshot element
            // bounds, and pointer coordinates share one space.
            let metrics = viewport_metrics_until(stream, socket, tab_id_value, deadline).await?;
            let mut params = json!({
                "format": "png",
                "fromSurface": true,
                "captureBeyondViewport": true,
            });
            if metrics.css_width > 0.0 && metrics.css_height > 0.0 {
                params["clip"] = json!({
                    "x": metrics.scroll_x,
                    "y": metrics.scroll_y,
                    "width": metrics.css_width,
                    "height": metrics.css_height,
                    "scale": 1,
                });
            }
            let response = execute_cdp_until(
                stream,
                socket,
                SCREENSHOT_REQUEST_ID,
                tab_id_value,
                "Page.captureScreenshot",
                params,
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
            Ok(BrowserCdpResult::Screenshot {
                data_base64,
                css_width: metrics.css_width,
                css_height: metrics.css_height,
            })
        }
        BrowserCdpAction::Click { x, y } => {
            execute_cdp_until(
                stream,
                socket,
                CLICK_MOVE_REQUEST_ID,
                tab_id_value,
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseMoved", "x": x, "y": y }),
                deadline,
            )
            .await?;
            execute_cdp_until(
                stream,
                socket,
                CLICK_DOWN_REQUEST_ID,
                tab_id_value,
                "Input.dispatchMouseEvent",
                json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
                deadline,
            )
            .await?;
            execute_cdp_until(
                stream,
                socket,
                CLICK_UP_REQUEST_ID,
                tab_id_value,
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
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
            let key = BrowserKeyStroke::parse(key);
            execute_cdp_until(
                stream,
                socket,
                KEY_DOWN_REQUEST_ID,
                tab_id_value,
                "Input.dispatchKeyEvent",
                key.event_params("keyDown"),
                deadline,
            )
            .await?;
            execute_cdp_until(
                stream,
                socket,
                KEY_UP_REQUEST_ID,
                tab_id_value,
                "Input.dispatchKeyEvent",
                key.event_params("keyUp"),
                deadline,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::Eval { expression } => {
            let response = execute_cdp_until(
                stream,
                socket,
                EVAL_REQUEST_ID,
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
            if let Some(exception) = response
                .get("result")
                .and_then(|result| result.get("exceptionDetails"))
            {
                return Err(eval_exception_diagnostic(exception));
            }
            Ok(BrowserCdpResult::Eval {
                value: snapshot::cdp_runtime_value(&response),
            })
        }
        BrowserCdpAction::Scroll {
            delta_x,
            delta_y,
            x,
            y,
        } => {
            let expression = match (*x, *y) {
                (Some(x), Some(y)) => format!(
                    r#"
(() => {{
  const eventX = {x};
  const eventY = {y};
  const deltaX = {delta_x};
  const deltaY = {delta_y};
  const canScroll = (el) => {{
    if (!el || el === document || el === document.documentElement || el === document.body) return false;
    const style = getComputedStyle(el);
    const yScrollable = /(auto|scroll|overlay)/.test(style.overflowY) && el.scrollHeight > el.clientHeight;
    const xScrollable = /(auto|scroll|overlay)/.test(style.overflowX) && el.scrollWidth > el.clientWidth;
    return yScrollable || xScrollable;
  }};
  let target = document.elementFromPoint(eventX, eventY);
  while (target && !canScroll(target)) target = target.parentElement;
  if (target) {{
    target.scrollBy(deltaX, deltaY);
    return {{ target: "element", scrollLeft: target.scrollLeft, scrollTop: target.scrollTop, eventX, eventY }};
  }}
  window.scrollBy(deltaX, deltaY);
  return {{ target: "window", x: window.scrollX, y: window.scrollY, eventX, eventY }};
}})()
"#
                ),
                _ => format!(
                    r#"
(() => {{
  const deltaX = {delta_x};
  const deltaY = {delta_y};
  window.scrollBy(deltaX, deltaY);
  return {{ target: "window", x: window.scrollX, y: window.scrollY }};
}})()
"#
                ),
            };
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

struct BrowserKeyStroke {
    key: String,
    modifiers: i32,
}

impl BrowserKeyStroke {
    fn parse(raw: &str) -> Self {
        // Chords are `+`-separated (Ctrl+K, Shift+Tab). Only `+` is a
        // separator: splitting on `-` too would mangle hyphen-target chords
        // such as the zoom-out chord `Ctrl+-`.
        let trimmed = raw.trim();
        let mut modifiers = 0;
        let mut key = trimmed;
        let segments: Vec<&str> = trimmed.split('+').collect();
        for (index, segment) in segments.iter().enumerate() {
            let segment = segment.trim();
            if segment.is_empty() {
                // A trailing empty segment means the chord ended with `+`, so
                // the target key is a literal `+` (e.g. the zoom-in `Ctrl++`).
                if index == segments.len() - 1 && segments.len() > 1 {
                    key = "+";
                }
                continue;
            }
            match segment.to_ascii_lowercase().as_str() {
                "alt" | "option" => modifiers |= 1,
                "ctrl" | "control" => modifiers |= 2,
                "meta" | "cmd" | "command" | "super" => modifiers |= 4,
                "shift" => modifiers |= 8,
                _ => key = segment,
            }
        }
        Self {
            key: key.to_string(),
            modifiers,
        }
    }

    fn event_params(&self, event_type: &str) -> Value {
        json!({
            "type": event_type,
            "key": self.key,
            "modifiers": self.modifiers,
        })
    }
}

/// A `Runtime.evaluate` response can be transport-successful while the page
/// expression threw or rejected; `exceptionDetails` is the only signal.
fn eval_exception_diagnostic(exception: &Value) -> DiagnosticEntry {
    let description = exception
        .get("exception")
        .and_then(|value| value.get("description"))
        .and_then(Value::as_str)
        .or_else(|| exception.get("text").and_then(Value::as_str))
        .unwrap_or("JavaScript evaluation threw an exception.");
    DiagnosticEntry {
        code: "BrowserEvalException".to_string(),
        message: format!("browser_eval expression threw: {description}"),
        details: None,
    }
}

#[cfg(test)]
mod keystroke_tests {
    use super::BrowserKeyStroke;

    const CTRL: i32 = 2;
    const SHIFT: i32 = 8;

    #[test]
    fn plus_separated_chord_extracts_key_and_modifier() {
        let stroke = BrowserKeyStroke::parse("Ctrl+K");
        assert_eq!(stroke.key, "K");
        assert_eq!(stroke.modifiers, CTRL);
    }

    #[test]
    fn hyphen_target_chord_is_not_split_on_minus() {
        // Zoom-out chord: the minus is the target key, not a separator.
        let stroke = BrowserKeyStroke::parse("Ctrl+-");
        assert_eq!(stroke.key, "-");
        assert_eq!(stroke.modifiers, CTRL);
    }

    #[test]
    fn trailing_plus_is_a_literal_key() {
        // Zoom-in chord: the chord ends with the literal `+` key.
        let stroke = BrowserKeyStroke::parse("Ctrl++");
        assert_eq!(stroke.key, "+");
        assert_eq!(stroke.modifiers, CTRL);
    }

    #[test]
    fn multiple_modifiers_accumulate() {
        let stroke = BrowserKeyStroke::parse("Ctrl+Shift+Tab");
        assert_eq!(stroke.key, "Tab");
        assert_eq!(stroke.modifiers, CTRL | SHIFT);
    }

    #[test]
    fn bare_key_has_no_modifiers() {
        let stroke = BrowserKeyStroke::parse("Enter");
        assert_eq!(stroke.key, "Enter");
        assert_eq!(stroke.modifiers, 0);
    }
}
