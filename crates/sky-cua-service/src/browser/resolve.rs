//! Stateless re-resolution of a snapshot-referenced page element to its live
//! CSS-pixel center, so browser input can aim by element identity instead of
//! coordinates the agent computed and that may have gone stale.
//!
//! ## Contract (stable across Option 1 / Option 2 implementations)
//!
//! The element handle is an opaque token the browser snapshot emits per element
//! and the agent passes back verbatim. It is self-contained: this module needs
//! no server-held per-tab state to re-find the element (sky-cua drives the
//! browser over ephemeral per-operation bridge connections, so there is no
//! shared element cache). [`resolve_element_center`] decodes the token, runs a
//! resolution over the bridge, and returns the element's current center or a
//! structured diagnostic.
//!
//! The token (Option 1 encoding) is a base64url-encoded (no padding) compact
//! JSON object:
//!
//! ```text
//! { "v": 1, "sel": <string>, "i": <int>,
//!   "sig": { "tag": <string>, "role": <string|null>, "name": <string|null>,
//!            "href": <string|null> },
//!   "b": { "x": <num>, "y": <num>, "w": <num>, "h": <num> } }
//! ```
//!
//! `sel` is the selector base the snapshot enumerated with, `i` is the element's
//! index within that selector query at snapshot time, `sig` is the identifying
//! signature, and `b` is the element's CSS-pixel bounds at snapshot time (used
//! to disambiguate when the signature matches more than one live element). `v`
//! is a version integer so the encoding can evolve.
//!
//! ## Resolver return contract
//!
//! [`RESOLVER_EXPRESSION_TEMPLATE`] runs in the page via `Runtime.evaluate`
//! (`returnByValue`) with the decoded `sel`/`sig`/`b` injected as JSON literals,
//! and returns by value:
//!
//! ```text
//! { "found": <bool>, "center": { "x": <num>, "y": <num> } | null,
//!   "reason": "ok" | "not_found" | "ambiguous" | "zero_size" | "offscreen"
//!             | "covered",
//!   "scrolled": <bool> }
//! ```
//!
//! Reason mapping:
//! - `ok` -> [`Ok`] with the live CSS-pixel center.
//! - `not_found` / `ambiguous` -> [`element_unresolved_diagnostic`]
//!   (`BrowserElementUnresolved`): the token matches no single element; the
//!   caller must re-observe.
//! - `zero_size` / `offscreen` / `covered` ->
//!   [`element_not_actionable_diagnostic`] (`BrowserElementNotActionable`): the
//!   element was found but a click cannot reach it.

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::protocol::RESOLVE_ELEMENT_REQUEST_ID;
use super::snapshot::cdp_runtime_value;
use super::transport::execute_cdp_until;

/// The resolved live position of a snapshot-referenced element, in CSS pixels —
/// the same coordinate space as browser click coordinates and screenshot
/// pixels, so the center feeds the existing `Input.dispatchMouseEvent` path
/// with no scaling conversion.
#[derive(Debug)]
pub(super) struct ResolvedElementCenter {
    pub x: f64,
    pub y: f64,
}

/// The re-find recipe carried by a `v:1` ref token, after base64url decode and
/// JSON parse. The resolver injects `sel`/`sig`/`b` into the page expression as
/// JSON literals; the fields are re-serialized rather than interpreted in Rust.
#[derive(Debug)]
struct DecodedRef {
    sel: Value,
    sig: Value,
    bounds: Value,
}

/// Re-locate the element named by the opaque snapshot `element_ref` in the live
/// page (in the tab identified by `tab_id_value`) and return its current center.
/// Stateless; see the module docs for the token contract and the failure
/// diagnostic codes.
///
/// NOTE (Stream 1B integration): this signature gained `tab_id_value` relative
/// to the Milestone 0 stub — `execute_cdp_until` needs the tab id per call and
/// the resolver has no other source for it. See the report accompanying Stream
/// 1A.
pub(super) async fn resolve_element_center(
    stream: &mut UnixStream,
    socket: &Path,
    tab_id_value: &Value,
    element_ref: &str,
    deadline: TokioInstant,
) -> Result<ResolvedElementCenter, DiagnosticEntry> {
    let decoded = decode_ref(element_ref)?;
    let expression = resolver_expression(&decoded);
    let response = execute_cdp_until(
        stream,
        socket,
        RESOLVE_ELEMENT_REQUEST_ID,
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
        let description = exception
            .get("exception")
            .and_then(|value| value.get("description"))
            .and_then(Value::as_str)
            .or_else(|| exception.get("text").and_then(Value::as_str))
            .unwrap_or("resolver expression threw an exception");
        return Err(element_unresolved_diagnostic(format!(
            "browser element resolution failed while running in the page: {description}"
        )));
    }

    let value = cdp_runtime_value(&response).ok_or_else(|| {
        element_unresolved_diagnostic("browser element resolution returned no value from the page")
    })?;
    interpret_resolver_value(&value)
}

/// Turn the resolver's `{found, center, reason, scrolled}` payload into a
/// resolved center or the diagnostic its `reason` maps to.
fn interpret_resolver_value(value: &Value) -> Result<ResolvedElementCenter, DiagnosticEntry> {
    let reason = value
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("not_found");
    match reason {
        "ok" => {
            let center = value.get("center").ok_or_else(|| {
                element_unresolved_diagnostic(
                    "browser element resolution reported success without a center",
                )
            })?;
            let x = center.get("x").and_then(Value::as_f64);
            let y = center.get("y").and_then(Value::as_f64);
            match (x, y) {
                (Some(x), Some(y)) => Ok(ResolvedElementCenter { x, y }),
                _ => Err(element_unresolved_diagnostic(
                    "browser element resolution returned a malformed center",
                )),
            }
        }
        "not_found" => Err(element_unresolved_diagnostic(
            "browser element resolution failed (not_found): the referenced element is no \
             longer on the page",
        )),
        "ambiguous" => Err(element_unresolved_diagnostic(
            "browser element resolution failed (ambiguous): several elements now match the \
             reference and none is unambiguously the original",
        )),
        "zero_size" => Err(element_not_actionable_diagnostic(
            "browser element is not actionable (zero_size): the referenced element has \
             collapsed to zero width or height",
        )),
        "offscreen" => Err(element_not_actionable_diagnostic(
            "browser element is not actionable (offscreen): the referenced element stays \
             outside the viewport even after scrolling",
        )),
        "covered" => Err(element_not_actionable_diagnostic(
            "browser element is not actionable (covered): another element is stacked over the \
             referenced element's center",
        )),
        other => Err(element_unresolved_diagnostic(format!(
            "browser element resolution returned an unknown reason: {other}"
        ))),
    }
}

/// Decode a `v:1` ref token: base64url -> UTF-8 JSON -> the injectable fields.
/// Any malformed or unsupported token is a caller error (a stale or hand-forged
/// ref), reported as unresolved so the caller re-observes.
fn decode_ref(element_ref: &str) -> Result<DecodedRef, DiagnosticEntry> {
    let bytes = URL_SAFE_NO_PAD
        .decode(element_ref.as_bytes())
        .map_err(|_| {
            element_unresolved_diagnostic("browser element reference is not valid base64url")
        })?;
    let token: Value = serde_json::from_slice(&bytes).map_err(|_| {
        element_unresolved_diagnostic("browser element reference did not decode to JSON")
    })?;

    let version = token.get("v").and_then(Value::as_i64);
    if version != Some(1) {
        return Err(element_unresolved_diagnostic(format!(
            "browser element reference has an unsupported version: {}",
            version.map_or_else(|| "none".to_string(), |v| v.to_string())
        )));
    }

    let sel = token.get("sel").cloned().filter(Value::is_string);
    let sig = token.get("sig").cloned().filter(Value::is_object);
    let bounds = token.get("b").cloned().filter(Value::is_object);
    match (sel, sig, bounds) {
        (Some(sel), Some(sig), Some(bounds)) => Ok(DecodedRef { sel, sig, bounds }),
        _ => Err(element_unresolved_diagnostic(
            "browser element reference is missing its selector, signature, or bounds",
        )),
    }
}

/// Build the resolver `Runtime.evaluate` expression with the decoded ref fields
/// injected as JSON literals. `serde_json` output is valid JS literal syntax, so
/// interpolating it into the template is safe.
fn resolver_expression(decoded: &DecodedRef) -> String {
    RESOLVER_EXPRESSION_TEMPLATE
        .replace("__SELECTOR__", &decoded.sel.to_string())
        .replace("__SIGNATURE__", &decoded.sig.to_string())
        .replace("__BOUNDS__", &decoded.bounds.to_string())
}

/// The stateless resolver run in the page. Placeholders are replaced with JSON
/// literals for the selector string, the signature object, and the snapshot-time
/// bounds. Implements the algorithm documented in the module header.
const RESOLVER_EXPRESSION_TEMPLATE: &str = r#"
(() => {
  const sel = __SELECTOR__;
  const sig = __SIGNATURE__;
  const target = __BOUNDS__;
  const result = (reason, center, scrolled) => ({
    found: reason === 'ok',
    center: center || null,
    reason,
    scrolled: Boolean(scrolled)
  });
  let nodes;
  try {
    nodes = document.querySelectorAll(sel);
  } catch (e) {
    return result('not_found', null, false);
  }
  const nameOf = (el) => el.getAttribute('aria-label')
    || el.getAttribute('title')
    || (el.textContent ? el.textContent.trim().slice(0, 200) : '')
    || null;
  const sigMatches = (el) => {
    if (el.tagName.toLowerCase() !== sig.tag) return false;
    if (sig.role != null && (el.getAttribute('role') || null) !== sig.role) return false;
    if (sig.name != null && nameOf(el) !== sig.name) return false;
    if (sig.href != null && (el.href || null) !== sig.href) return false;
    return true;
  };
  const candidates = [];
  for (const el of nodes) {
    if (sigMatches(el)) candidates.push(el);
  }
  if (candidates.length === 0) return result('not_found', null, false);

  let chosen;
  if (candidates.length === 1) {
    chosen = candidates[0];
  } else {
    const targetCx = target.x + target.w / 2;
    const targetCy = target.y + target.h / 2;
    const scored = candidates.map((el) => {
      const r = el.getBoundingClientRect();
      const dx = (r.x + r.width / 2) - targetCx;
      const dy = (r.y + r.height / 2) - targetCy;
      return { el, d: dx * dx + dy * dy };
    }).sort((a, b) => a.d - b.d);
    // Accept only an unambiguous nearest: the runner-up must be meaningfully
    // farther (>1px^2 of squared-distance separation), else two live elements
    // sit equidistant from the snapshot center and the original is unknowable.
    if (scored.length > 1 && (scored[1].d - scored[0].d) <= 1) {
      return result('ambiguous', null, false);
    }
    chosen = scored[0].el;
  }

  let scrolled = false;
  const outsideViewport = (r) => r.top < 0 || r.left < 0 || r.bottom > innerHeight || r.right > innerWidth;
  let rect = chosen.getBoundingClientRect();
  if (outsideViewport(rect)) {
    chosen.scrollIntoView({ block: 'center', inline: 'center' });
    scrolled = true;
    rect = chosen.getBoundingClientRect();
  }
  if (rect.width < 1 || rect.height < 1) return result('zero_size', null, scrolled);
  const cx = rect.x + rect.width / 2;
  const cy = rect.y + rect.height / 2;
  if (cx < 0 || cy < 0 || cx > innerWidth || cy > innerHeight) {
    return result('offscreen', null, scrolled);
  }
  const hit = document.elementFromPoint(cx, cy);
  const actionable = hit && (hit === chosen || chosen.contains(hit) || hit.contains(chosen));
  if (!actionable) return result('covered', null, scrolled);
  return result('ok', { x: cx, y: cy }, scrolled);
})()
"#;

/// Build the `BrowserElementUnresolved` diagnostic used when a token matches no
/// element on the current page.
pub(super) fn element_unresolved_diagnostic(message: impl Into<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserElementUnresolved".to_string(),
        message: message.into(),
        details: Some(
            "The referenced element is no longer on the page. Re-run \
             observe(surface=\"browser\") to get fresh element references, then retry."
                .to_string(),
        ),
    }
}

/// Build the `BrowserElementNotActionable` diagnostic used when the element was
/// found but cannot receive a click (zero-size, off-screen, or covered).
pub(super) fn element_not_actionable_diagnostic(message: impl Into<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: "BrowserElementNotActionable".to_string(),
        message: message.into(),
        details: Some(
            "The referenced element is present but cannot be clicked right now (hidden, \
             off-screen, or covered by another element). Re-observe and retry, or use \
             coordinates if the target is a canvas/map region."
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a ref value the way the in-page `refEncode` does: JSON string then
    /// URL-safe base64 without padding. Mirrors the snapshot payload so the
    /// round-trip proves the Rust decoder accepts the JS encoder's output.
    fn encode_ref(value: &Value) -> String {
        URL_SAFE_NO_PAD.encode(value.to_string().as_bytes())
    }

    fn sample_ref() -> Value {
        json!({
            "v": 1,
            "sel": "a,button,input",
            "i": 3,
            "sig": { "tag": "button", "role": null, "name": "Add to cart", "href": null },
            "b": { "x": 10.0, "y": 20.0, "w": 100.0, "h": 40.0 }
        })
    }

    #[test]
    fn ref_encode_decode_round_trip() {
        let token = sample_ref();
        let encoded = encode_ref(&token);
        // base64url alphabet only, and no padding.
        assert!(!encoded.contains('+') && !encoded.contains('/') && !encoded.contains('='));

        let decoded = decode_ref(&encoded).expect("decode round-trips");
        assert_eq!(decoded.sel, token["sel"]);
        assert_eq!(decoded.sig, token["sig"]);
        assert_eq!(decoded.bounds, token["b"]);
    }

    #[test]
    fn ref_decode_round_trips_unicode_name() {
        let token = json!({
            "v": 1,
            "sel": "button",
            "i": 0,
            "sig": { "tag": "button", "role": null, "name": "Añadir al carrito ✓", "href": null },
            "b": { "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 }
        });
        let decoded = decode_ref(&encode_ref(&token)).expect("decode unicode ref");
        assert_eq!(decoded.sig["name"], "Añadir al carrito ✓");
    }

    #[test]
    fn ref_decode_rejects_non_base64url() {
        let err = decode_ref("not valid base64!!").expect_err("must reject");
        assert_eq!(err.code, "BrowserElementUnresolved");
    }

    #[test]
    fn ref_decode_rejects_unsupported_version() {
        let token = json!({
            "v": 2,
            "sel": "button",
            "i": 0,
            "sig": { "tag": "button" },
            "b": { "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0 }
        });
        let err = decode_ref(&encode_ref(&token)).expect_err("must reject v2");
        assert_eq!(err.code, "BrowserElementUnresolved");
        assert!(err.message.contains("version"));
    }

    #[test]
    fn ref_decode_rejects_missing_fields() {
        let token = json!({ "v": 1, "sel": "button" });
        let err = decode_ref(&encode_ref(&token)).expect_err("must reject incomplete");
        assert_eq!(err.code, "BrowserElementUnresolved");
    }

    #[test]
    fn resolver_expression_injects_decoded_fields() {
        let decoded = decode_ref(&encode_ref(&sample_ref())).expect("decode");
        let expression = resolver_expression(&decoded);
        assert!(!expression.contains("__SELECTOR__"));
        assert!(!expression.contains("__SIGNATURE__"));
        assert!(!expression.contains("__BOUNDS__"));
        // Injected as JSON literals the page can read directly.
        assert!(expression.contains("\"a,button,input\""));
        assert!(expression.contains("\"Add to cart\""));
    }

    #[test]
    fn interpret_ok_returns_center() {
        let center = interpret_resolver_value(&json!({
            "found": true,
            "center": { "x": 120.5, "y": 44.0 },
            "reason": "ok",
            "scrolled": true
        }))
        .expect("ok maps to a center");
        assert_eq!(center.x, 120.5);
        assert_eq!(center.y, 44.0);
    }

    #[test]
    fn interpret_ok_without_center_is_unresolved() {
        let err = interpret_resolver_value(&json!({
            "found": true,
            "center": null,
            "reason": "ok",
            "scrolled": false
        }))
        .expect_err("ok without a center is malformed");
        assert_eq!(err.code, "BrowserElementUnresolved");
    }

    #[test]
    fn interpret_unresolved_reasons_map_to_unresolved_code() {
        for reason in ["not_found", "ambiguous"] {
            let err = interpret_resolver_value(&json!({
                "found": false,
                "center": null,
                "reason": reason,
                "scrolled": false
            }))
            .expect_err("unresolved reason must be an error");
            assert_eq!(err.code, "BrowserElementUnresolved", "reason {reason}");
            assert!(err.message.contains(reason), "message names {reason}");
        }
    }

    #[test]
    fn interpret_not_actionable_reasons_map_to_not_actionable_code() {
        for reason in ["zero_size", "offscreen", "covered"] {
            let err = interpret_resolver_value(&json!({
                "found": false,
                "center": null,
                "reason": reason,
                "scrolled": true
            }))
            .expect_err("not-actionable reason must be an error");
            assert_eq!(err.code, "BrowserElementNotActionable", "reason {reason}");
            assert!(err.message.contains(reason), "message names {reason}");
        }
    }

    #[test]
    fn interpret_unknown_reason_is_unresolved() {
        let err = interpret_resolver_value(&json!({
            "found": false,
            "center": null,
            "reason": "meteor_strike",
            "scrolled": false
        }))
        .expect_err("unknown reason must be an error");
        assert_eq!(err.code, "BrowserElementUnresolved");
    }

    mod fake_bridge {
        use std::time::{Duration, SystemTime};

        use tokio::net::{UnixListener, UnixStream};

        use super::super::super::protocol::{read_frame, write_frame};
        use super::*;

        fn unique_socket_dir() -> std::path::PathBuf {
            std::env::temp_dir().join(format!(
                "sky-cua-browser-resolve-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
        }

        /// Drive [`resolve_element_center`] over a real `UnixStream` against a
        /// fake bridge that answers the resolver `Runtime.evaluate` with one
        /// canned payload carrying `reason`/`center`. Exercises the full decode
        /// -> `execute_cdp_until` -> reason-mapping path.
        async fn resolve_against(
            reason: &'static str,
            center: Value,
        ) -> Result<ResolvedElementCenter, DiagnosticEntry> {
            let dir = unique_socket_dir();
            std::fs::create_dir_all(&dir).unwrap();
            let socket_path = dir.join("extension-123-test.sock");
            let listener = UnixListener::bind(&socket_path).unwrap();

            let server_center = center.clone();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_frame(&mut stream).await.unwrap().unwrap();
                assert_eq!(
                    request.get("method").and_then(Value::as_str),
                    Some("executeCdp")
                );
                assert_eq!(
                    request.get("id").and_then(Value::as_str),
                    Some(RESOLVE_ELEMENT_REQUEST_ID)
                );
                assert_eq!(request["params"]["method"], "Runtime.evaluate");
                // The decoded selector must reach the page as a JSON literal
                // (not the unreplaced placeholder) so the resolver can re-query.
                let expression = request["params"]["commandParams"]["expression"]
                    .as_str()
                    .unwrap();
                assert!(expression.contains("\"a,button,input\""));
                assert!(!expression.contains("__SELECTOR__"));
                write_frame(
                    &mut stream,
                    &json!({
                        "jsonrpc": "2.0",
                        "id": request["id"],
                        "result": {
                            "result": {
                                "type": "object",
                                "value": {
                                    "found": reason == "ok",
                                    "center": server_center,
                                    "reason": reason,
                                    "scrolled": false
                                }
                            }
                        }
                    }),
                )
                .await
                .unwrap();
            });

            let mut client = UnixStream::connect(&socket_path).await.unwrap();
            let result = resolve_element_center(
                &mut client,
                &socket_path,
                &json!(515),
                &encode_ref(&sample_ref()),
                TokioInstant::now() + Duration::from_secs(5),
            )
            .await;

            server.await.unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
            result
        }

        #[tokio::test]
        async fn resolver_ok_yields_live_center() {
            let center = resolve_against("ok", json!({ "x": 120.5, "y": 44.0 }))
                .await
                .expect("ok reason resolves to a center");
            assert_eq!(center.x, 120.5);
            assert_eq!(center.y, 44.0);
        }

        #[tokio::test]
        async fn resolver_not_found_maps_to_unresolved() {
            let err = resolve_against("not_found", Value::Null)
                .await
                .expect_err("not_found is an error");
            assert_eq!(err.code, "BrowserElementUnresolved");
            assert!(err.message.contains("not_found"));
        }

        #[tokio::test]
        async fn resolver_ambiguous_maps_to_unresolved() {
            let err = resolve_against("ambiguous", Value::Null)
                .await
                .expect_err("ambiguous is an error");
            assert_eq!(err.code, "BrowserElementUnresolved");
            assert!(err.message.contains("ambiguous"));
        }

        #[tokio::test]
        async fn resolver_zero_size_maps_to_not_actionable() {
            let err = resolve_against("zero_size", Value::Null)
                .await
                .expect_err("zero_size is an error");
            assert_eq!(err.code, "BrowserElementNotActionable");
            assert!(err.message.contains("zero_size"));
        }

        #[tokio::test]
        async fn resolver_offscreen_maps_to_not_actionable() {
            let err = resolve_against("offscreen", Value::Null)
                .await
                .expect_err("offscreen is an error");
            assert_eq!(err.code, "BrowserElementNotActionable");
            assert!(err.message.contains("offscreen"));
        }

        #[tokio::test]
        async fn resolver_covered_maps_to_not_actionable() {
            let err = resolve_against("covered", Value::Null)
                .await
                .expect_err("covered is an error");
            assert_eq!(err.code, "BrowserElementNotActionable");
            assert!(err.message.contains("covered"));
        }
    }
}
