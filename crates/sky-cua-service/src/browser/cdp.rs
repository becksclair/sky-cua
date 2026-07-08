use std::path::Path;

use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::coordinates::viewport_metrics_until;
use super::protocol::{
    CLICK_DOWN_REQUEST_ID, CLICK_MOVE_REQUEST_ID, CLICK_UP_REQUEST_ID, EVAL_REQUEST_ID,
    FOCUS_EMULATION_REQUEST_ID, KEY_DOWN_REQUEST_ID, KEY_UP_REQUEST_ID, NAVIGATE_REQUEST_ID,
    SCREENSHOT_REQUEST_ID, SCROLL_REQUEST_ID, SNAPSHOT_REQUEST_ID, TYPE_TEXT_REQUEST_ID,
};
use super::session;
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
    /// Click the element named by an opaque snapshot `element_ref`. The live
    /// center is resolved over the bridge at dispatch time, then the same
    /// trusted mouse sequence as [`BrowserCdpAction::Click`] is dispatched at
    /// that center.
    ClickElement {
        element_ref: String,
    },
    TypeText {
        text: String,
    },
    /// Focus the element named by an opaque snapshot `element_ref` (by clicking
    /// its resolved center) and insert `text`, mirroring
    /// [`BrowserCdpAction::TypeText`] but aimed by identity instead of relying
    /// on ambient focus.
    TypeTextElement {
        element_ref: String,
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

/// Dispatch a CDP command whose double-apply harms the page (input dispatch,
/// evaluated scroll/eval, navigation). `mutated` is raised before the dispatch
/// and lowered again only when the extension provably rejected the command
/// without executing it (an upfront "Debugger unattached" — its session
/// bookkeeping refused the target before dispatch). A timeout or mid-execution
/// detach leaves `mutated` raised: the command may have taken effect, so the
/// executor must not replay the operation. Read-only or absolute sub-commands
/// (mouseMoved, focus emulation, metrics) go through plain `execute_cdp_until`
/// and never touch the flag.
#[allow(clippy::too_many_arguments)]
async fn execute_compounding_cdp_until(
    stream: &mut UnixStream,
    socket: &Path,
    request_id: &'static str,
    tab_id_value: &Value,
    method: &'static str,
    command_params: Value,
    deadline: TokioInstant,
    mutated: &mut bool,
) -> Result<Value, DiagnosticEntry> {
    let previously_mutated = *mutated;
    *mutated = true;
    match execute_cdp_until(
        stream,
        socket,
        request_id,
        tab_id_value,
        method,
        command_params,
        deadline,
    )
    .await
    {
        Ok(response) => Ok(response),
        Err(diagnostic) => {
            if !previously_mutated && session::is_upfront_unattached_diagnostic(&diagnostic) {
                *mutated = false;
            }
            Err(diagnostic)
        }
    }
}

pub(super) async fn cdp_action_on_stream(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id_value: &Value,
    action: &BrowserCdpAction,
    deadline: TokioInstant,
    mutated: &mut bool,
) -> Result<BrowserCdpResult, DiagnosticEntry> {
    match action {
        BrowserCdpAction::Navigate { url } => {
            let response = execute_compounding_cdp_until(
                stream,
                socket,
                NAVIGATE_REQUEST_ID,
                tab_id_value,
                "Page.navigate",
                json!({ "url": url }),
                deadline,
                mutated,
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
            dispatch_click_at(stream, socket, tab_id_value, *x, *y, deadline, mutated).await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::ClickElement { element_ref } => {
            // Resolve first: a failed resolution surfaces
            // BrowserElementUnresolved / BrowserElementNotActionable via `?`
            // before any input is dispatched, so the agent gets a clean
            // "re-observe" signal and the page is untouched.
            let center =
                super::resolve::resolve_element_center(stream, socket, element_ref, deadline)
                    .await?;
            dispatch_click_at(
                stream,
                socket,
                tab_id_value,
                center.x,
                center.y,
                deadline,
                mutated,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::TypeText { text } => {
            ensure_focus_emulation(stream, socket, tab_id_value, deadline).await;
            execute_compounding_cdp_until(
                stream,
                socket,
                TYPE_TEXT_REQUEST_ID,
                tab_id_value,
                "Input.insertText",
                json!({ "text": text }),
                deadline,
                mutated,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::TypeTextElement { element_ref, text } => {
            // Resolve first (see ClickElement): an unresolved / not-actionable
            // ref propagates via `?` before any focus click or text insert.
            let center =
                super::resolve::resolve_element_center(stream, socket, element_ref, deadline)
                    .await?;
            // Focus the field with a real click at its live center, then insert
            // the text. dispatch_click_at runs ensure_focus_emulation, so the
            // insert lands even on a background/unfocused tab.
            dispatch_click_at(
                stream,
                socket,
                tab_id_value,
                center.x,
                center.y,
                deadline,
                mutated,
            )
            .await?;
            execute_compounding_cdp_until(
                stream,
                socket,
                TYPE_TEXT_REQUEST_ID,
                tab_id_value,
                "Input.insertText",
                json!({ "text": text }),
                deadline,
                mutated,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::PressKey { key } => {
            ensure_focus_emulation(stream, socket, tab_id_value, deadline).await;
            let key = BrowserKeyStroke::parse(key);
            execute_compounding_cdp_until(
                stream,
                socket,
                KEY_DOWN_REQUEST_ID,
                tab_id_value,
                "Input.dispatchKeyEvent",
                key.event_params(key.key_down_type()),
                deadline,
                mutated,
            )
            .await?;
            execute_compounding_cdp_until(
                stream,
                socket,
                KEY_UP_REQUEST_ID,
                tab_id_value,
                "Input.dispatchKeyEvent",
                key.event_params("keyUp"),
                deadline,
                mutated,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
        BrowserCdpAction::Eval { expression } => {
            let response = execute_compounding_cdp_until(
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
                mutated,
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
            execute_compounding_cdp_until(
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
                mutated,
            )
            .await?;
            Ok(BrowserCdpResult::Action)
        }
    }
}

/// Dispatch one trusted left click at the CSS-pixel point `(x, y)`: enable
/// focus emulation, then `Input.dispatchMouseEvent` `mouseMoved` /
/// `mousePressed` / `mouseReleased`. This is the single code path shared by the
/// coordinate [`BrowserCdpAction::Click`] and the element-targeted
/// [`BrowserCdpAction::ClickElement`] / [`BrowserCdpAction::TypeTextElement`]
/// arms, so an aim-by-identity click is byte-for-byte the same trusted gesture
/// as an aim-by-pixel one. `mouseMoved` is absolute and read-only-ish so it
/// goes through plain `execute_cdp_until`; press/release compound on replay and
/// therefore raise `mutated` via `execute_compounding_cdp_until`.
pub(super) async fn dispatch_click_at(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id_value: &Value,
    x: f64,
    y: f64,
    deadline: TokioInstant,
    mutated: &mut bool,
) -> Result<(), DiagnosticEntry> {
    ensure_focus_emulation(stream, socket, tab_id_value, deadline).await;
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
    execute_compounding_cdp_until(
        stream,
        socket,
        CLICK_DOWN_REQUEST_ID,
        tab_id_value,
        "Input.dispatchMouseEvent",
        json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        deadline,
        mutated,
    )
    .await?;
    execute_compounding_cdp_until(
        stream,
        socket,
        CLICK_UP_REQUEST_ID,
        tab_id_value,
        "Input.dispatchMouseEvent",
        json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        deadline,
        mutated,
    )
    .await?;
    Ok(())
}

/// Force the tab's renderer to treat itself as focused before an input action.
///
/// sky-cua routinely drives a background tab (or a browser window that is not
/// the foreground OS window), where `document.hasFocus()` is false. Blink then
/// drops click-to-focus and does not deliver `Input.insertText` to the focused
/// element, so clicks and typing silently land nowhere.
/// `Emulation.setFocusEmulationEnabled` is a per-target override that persists
/// for the debugger session, so the first input action that runs restores it
/// for every later one on the same session. Applied before clicks, typing, and
/// key presses alike: it makes a click focus its target and makes `type_text`
/// land on an already-focused field even with no preceding click.
///
/// It does NOT make synthetic `Input.dispatchKeyEvent` editing/navigation
/// (Backspace, arrows, Ctrl+A) act on a field that was only focused
/// programmatically: Blink routes those through the activation and caret a real
/// mouse gesture establishes, which focus emulation cannot replicate. Pressing
/// keys against a field therefore still requires a preceding click — the normal
/// focus-then-edit flow — which the click's own emulation call already covers.
///
/// Best effort: a browser without the method must not fail the action, and the
/// following input command re-surfaces any real session failure.
async fn ensure_focus_emulation(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id_value: &Value,
    deadline: TokioInstant,
) {
    let _ = execute_cdp_until(
        stream,
        socket,
        FOCUS_EMULATION_REQUEST_ID,
        tab_id_value,
        "Emulation.setFocusEmulationEnabled",
        json!({ "enabled": true }),
        deadline,
    )
    .await;
}

/// CDP `modifiers` bit for Shift; any modifier other than Shift suppresses the
/// text a printable key would otherwise emit (Ctrl/Meta/Alt turn the press into
/// an accelerator, not typed input). Matches Puppeteer/Playwright.
const SHIFT_MODIFIER_BIT: i32 = 8;

struct BrowserKeyStroke {
    key: String,
    /// DOM `code` (e.g. `Backspace`, `KeyA`), when the key name is recognized.
    code: Option<String>,
    /// Windows virtual key code (e.g. Backspace=8, Delete=46, A=65). `0` for
    /// unrecognized keys, which are dispatched by `key`/`modifiers` alone.
    windows_virtual_key_code: i32,
    modifiers: i32,
    /// Text the key inserts, present only for a printable press with at most
    /// Shift held. `None` forces a `rawKeyDown` so editing/navigation and
    /// modifier chords perform their default action instead of typing.
    text: Option<String>,
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

        let (code, windows_virtual_key_code) = key_identity(key);
        // A printable key inserts text, but only when no accelerator modifier is
        // held. Blink keys the default editing action off the virtual key code +
        // modifier flags, so clearing text here (and thus emitting `rawKeyDown`)
        // is what lets Ctrl+A select-all and Backspace/Delete delete instead of
        // typing. `Enter` and the `Space`/`Spacebar` aliases carry the text
        // their key inserts (`"\r"`, `" "`); the named space forms are the only
        // way to reach this path, since a literal " " is trimmed away before
        // parsing.
        let mut text = if key.chars().count() == 1 {
            Some(key.to_string())
        } else if key == "Enter" {
            Some("\r".to_string())
        } else if key == "Space" || key == "Spacebar" {
            Some(" ".to_string())
        } else {
            None
        };
        if modifiers & !SHIFT_MODIFIER_BIT != 0 {
            text = None;
        }

        Self {
            key: key.to_string(),
            code,
            windows_virtual_key_code,
            modifiers,
            text,
        }
    }

    /// The down-event type: `keyDown` when the press produces text (so a
    /// `char`/`beforeinput` follows), `rawKeyDown` otherwise. Puppeteer and
    /// Playwright use the same `text ? 'keyDown' : 'rawKeyDown'` rule.
    fn key_down_type(&self) -> &'static str {
        if self.text.is_some() {
            "keyDown"
        } else {
            "rawKeyDown"
        }
    }

    fn event_params(&self, event_type: &str) -> Value {
        let mut params = json!({
            "type": event_type,
            "key": self.key,
            "modifiers": self.modifiers,
        });
        if let Some(code) = &self.code {
            params["code"] = Value::from(code.clone());
        }
        if self.windows_virtual_key_code != 0 {
            params["windowsVirtualKeyCode"] = Value::from(self.windows_virtual_key_code);
        }
        // CDP only consumes `text` on `keyDown`; it is ignored (and omitted by
        // the reference implementations) for `rawKeyDown` and `keyUp`.
        if event_type == "keyDown"
            && let Some(text) = &self.text
        {
            params["text"] = Value::from(text.clone());
            params["unmodifiedText"] = Value::from(text.clone());
        }
        params
    }
}

/// Resolve a key name to its DOM `code` and Windows virtual key code using the
/// US layout shared by Puppeteer and Playwright. Unrecognized keys return
/// `(None, 0)` and fall back to `key`/`modifiers`-only dispatch, preserving the
/// prior behavior for chords like the `Ctrl+-` / `Ctrl++` browser-zoom keys.
fn key_identity(key: &str) -> (Option<String>, i32) {
    let named = match key {
        "Backspace" => Some(("Backspace", 8)),
        "Tab" => Some(("Tab", 9)),
        "Enter" => Some(("Enter", 13)),
        "Escape" | "Esc" => Some(("Escape", 27)),
        " " | "Space" | "Spacebar" => Some(("Space", 32)),
        "PageUp" => Some(("PageUp", 33)),
        "PageDown" => Some(("PageDown", 34)),
        "End" => Some(("End", 35)),
        "Home" => Some(("Home", 36)),
        "ArrowLeft" | "Left" => Some(("ArrowLeft", 37)),
        "ArrowUp" | "Up" => Some(("ArrowUp", 38)),
        "ArrowRight" | "Right" => Some(("ArrowRight", 39)),
        "ArrowDown" | "Down" => Some(("ArrowDown", 40)),
        "Delete" | "Del" => Some(("Delete", 46)),
        _ => None,
    };
    if let Some((code, key_code)) = named {
        return (Some(code.to_string()), key_code);
    }
    // A single ASCII letter or digit derives its `code`/keyCode directly: the
    // virtual key code is the uppercase/digit ASCII value, matching the layout.
    if key.chars().count() == 1 {
        let c = key.chars().next().expect("one char");
        if c.is_ascii_alphabetic() {
            let upper = c.to_ascii_uppercase();
            return (Some(format!("Key{upper}")), i32::from(upper as u8));
        }
        if c.is_ascii_digit() {
            return (Some(format!("Digit{c}")), i32::from(c as u8));
        }
    }
    (None, 0)
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

    #[test]
    fn editing_key_carries_virtual_key_code_and_is_raw_key_down() {
        // Backspace/Delete only delete when Blink receives the virtual key
        // code; with text absent they must dispatch as `rawKeyDown`.
        for (name, code, key_code) in [("Backspace", "Backspace", 8), ("Delete", "Delete", 46)] {
            let stroke = BrowserKeyStroke::parse(name);
            assert_eq!(stroke.code.as_deref(), Some(code));
            assert_eq!(stroke.windows_virtual_key_code, key_code);
            assert!(stroke.text.is_none());
            assert_eq!(stroke.key_down_type(), "rawKeyDown");
            let params = stroke.event_params("rawKeyDown");
            assert_eq!(params["windowsVirtualKeyCode"], key_code);
            assert_eq!(params["code"], code);
            assert!(params.get("text").is_none());
        }
    }

    #[test]
    fn ctrl_letter_chord_suppresses_text_for_select_all() {
        let stroke = BrowserKeyStroke::parse("Ctrl+A");
        assert_eq!(stroke.key, "A");
        assert_eq!(stroke.code.as_deref(), Some("KeyA"));
        assert_eq!(stroke.windows_virtual_key_code, 65);
        assert_eq!(stroke.modifiers, CTRL);
        // A non-Shift modifier clears the text so Blink runs the accelerator
        // (select-all) instead of typing "a".
        assert!(stroke.text.is_none());
        assert_eq!(stroke.key_down_type(), "rawKeyDown");
        assert!(stroke.event_params("keyDown").get("text").is_none());
    }

    #[test]
    fn bare_printable_key_types_text_via_key_down() {
        let stroke = BrowserKeyStroke::parse("a");
        assert_eq!(stroke.code.as_deref(), Some("KeyA"));
        assert_eq!(stroke.windows_virtual_key_code, 65);
        assert_eq!(stroke.text.as_deref(), Some("a"));
        assert_eq!(stroke.key_down_type(), "keyDown");
        let params = stroke.event_params("keyDown");
        assert_eq!(params["text"], "a");
        assert_eq!(params["unmodifiedText"], "a");
    }

    #[test]
    fn shift_alone_keeps_typed_text() {
        // Shift is the one modifier that still produces text.
        let stroke = BrowserKeyStroke::parse("Shift+A");
        assert_eq!(stroke.modifiers, SHIFT);
        assert_eq!(stroke.text.as_deref(), Some("A"));
        assert_eq!(stroke.key_down_type(), "keyDown");
    }

    #[test]
    fn enter_types_carriage_return() {
        let stroke = BrowserKeyStroke::parse("Enter");
        assert_eq!(stroke.code.as_deref(), Some("Enter"));
        assert_eq!(stroke.windows_virtual_key_code, 13);
        assert_eq!(stroke.text.as_deref(), Some("\r"));
        assert_eq!(stroke.key_down_type(), "keyDown");
    }

    #[test]
    fn named_space_types_a_space() {
        // The `Space`/`Spacebar` aliases are the only way to press space (a
        // literal " " is trimmed away), so they must insert a space rather than
        // dispatch a text-less rawKeyDown that types nothing.
        for name in ["Space", "Spacebar"] {
            let stroke = BrowserKeyStroke::parse(name);
            assert_eq!(stroke.code.as_deref(), Some("Space"), "code for {name:?}");
            assert_eq!(stroke.windows_virtual_key_code, 32, "vk for {name:?}");
            assert_eq!(stroke.text.as_deref(), Some(" "), "text for {name:?}");
            assert_eq!(stroke.key_down_type(), "keyDown", "type for {name:?}");
        }
        // A modifier still turns space into an accelerator with no typed text.
        let chord = BrowserKeyStroke::parse("Ctrl+Space");
        assert_eq!(chord.modifiers, CTRL);
        assert!(chord.text.is_none());
        assert_eq!(chord.key_down_type(), "rawKeyDown");
    }

    #[test]
    fn unrecognized_key_falls_back_to_key_and_modifiers() {
        // The zoom chords use keys with no layout entry; they must keep
        // dispatching by name without a virtual key code.
        let stroke = BrowserKeyStroke::parse("Ctrl+-");
        assert_eq!(stroke.key, "-");
        assert_eq!(stroke.modifiers, CTRL);
        assert!(stroke.code.is_none());
        assert_eq!(stroke.windows_virtual_key_code, 0);
        let params = stroke.event_params("rawKeyDown");
        assert!(params.get("windowsVirtualKeyCode").is_none());
        assert!(params.get("code").is_none());
    }
}
