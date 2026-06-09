use serde_json::{Value, json};
use sky_cua_platform::model::DiagnosticEntry;

pub(super) const BROWSER_SNAPSHOT_EXPRESSION: &str = r#"
(() => {
  const text = (document.body?.innerText || '').slice(0, 20000);
  const sensitiveField = (el) => {
    const attr = (name) => String(el.getAttribute(name) || '').toLowerCase();
    const haystack = [attr('type'), attr('name'), attr('id'), attr('autocomplete'), attr('aria-label'), attr('placeholder')].join(' ');
    return /password|passwd|passcode|secret|token|api[-_ ]?key|access[-_ ]?key|auth|credential|session|otp|code|pin|hidden/.test(haystack);
  };
  const safeValue = (el) => {
    if (!('value' in el) || sensitiveField(el)) return null;
    return String(el.value).slice(0, 500);
  };
  const elementFor = (el, index) => {
    const rect = el.getBoundingClientRect();
    return {
      index,
      tag: el.tagName.toLowerCase(),
      role: el.getAttribute('role') || null,
      name: el.getAttribute('aria-label') || el.getAttribute('title') || el.textContent?.trim()?.slice(0, 200) || null,
      value: safeValue(el),
      href: el.href || null,
      disabled: Boolean(el.disabled || el.getAttribute('aria-disabled') === 'true'),
      bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height }
    };
  };
  const selector = 'a,button,input,textarea,select,[role="button"],[role="link"],[contenteditable="true"]';
  return {
    title: document.title || '',
    url: location.href,
    viewport: { width: innerWidth, height: innerHeight, devicePixelRatio: devicePixelRatio || 1 },
    text,
    elements: Array.from(document.querySelectorAll(selector)).slice(0, 200).map(elementFor)
  };
})()
"#;

pub(super) fn snapshot_evaluate_params() -> Value {
    json!({
        "expression": BROWSER_SNAPSHOT_EXPRESSION,
        "awaitPromise": true,
        "returnByValue": true,
    })
}

pub(super) fn snapshot_from_cdp_response(
    response: &Value,
) -> Result<(Option<String>, Option<String>, Value), DiagnosticEntry> {
    let snapshot = cdp_runtime_value(response).ok_or_else(|| DiagnosticEntry {
        code: "BrowserBridgeRequestFailed".to_string(),
        message: "Browser snapshot CDP response did not include a runtime value.".to_string(),
        details: None,
    })?;
    let title = snapshot
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_string);
    let url = snapshot
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok((title, url, snapshot))
}

pub(super) fn cdp_runtime_value(response: &Value) -> Option<Value> {
    response.get("result")?.get("result")?.get("value").cloned()
}
