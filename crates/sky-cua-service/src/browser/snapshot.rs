use serde_json::{Value, json};
use sky_cua_platform::model::{
    BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT, BROWSER_SNAPSHOT_MAX_TEXT_LIMIT, DiagnosticEntry,
};

pub(super) const BROWSER_SNAPSHOT_EXPRESSION_TEMPLATE: &str = r#"
(() => {
  const textLimit = __TEXT_LIMIT__;
  const includeText = textLimit > 0;
  const fullText = includeText ? (document.body?.innerText || '') : '';
  const textChars = [];
  let textCharCount = null;
  let textTruncated = null;
  if (includeText) {
    textCharCount = 0;
    textTruncated = false;
    for (const char of fullText) {
      if (textChars.length < textLimit) {
        textChars.push(char);
        textCharCount += 1;
      } else {
        textTruncated = true;
        textCharCount = null;
        break;
      }
    }
  }
  const text = textChars.join('');
  const elementOffset = __ELEMENT_OFFSET__;
  const elementLimit = __ELEMENT_LIMIT__;
  const elementQuery = __ELEMENT_QUERY__;
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
  const elementName = (el) => el.getAttribute('aria-label') || el.getAttribute('title') || el.textContent?.trim()?.slice(0, 200) || '';
  const elementSearchText = (el) => [
    el.tagName.toLowerCase(),
    el.getAttribute('role') || '',
    elementName(el),
    safeValue(el) || '',
    el.href || ''
  ].join('\n').toLowerCase();
  const elementMatches = (el) => !elementQuery || elementSearchText(el).includes(elementQuery);
  const selector = 'a,button,input,textarea,select,[role="button"],[role="link"],[contenteditable="true"]';
  const elements = document.querySelectorAll(selector);
  const projectedElements = [];
  if (elementLimit > 0) {
    let matchedIndex = 0;
    for (let index = 0; index < elements.length; index += 1) {
      const el = elements[index];
      if (!elementMatches(el)) continue;
      if (matchedIndex >= elementOffset) {
        projectedElements.push(elementFor(el, index));
        if (projectedElements.length >= elementLimit) break;
      }
      matchedIndex += 1;
    }
  }
  return {
    title: document.title || '',
    url: location.href,
    viewport: { width: innerWidth, height: innerHeight, devicePixelRatio: devicePixelRatio || 1 },
    text,
    textCharCount,
    textLimit,
    textTruncated,
    elementCount: elements.length,
    elements: projectedElements
  };
})()
"#;

pub(super) fn snapshot_evaluate_params(
    text_limit: Option<usize>,
    element_offset: Option<usize>,
    element_limit: Option<usize>,
    element_query: Option<&str>,
) -> Value {
    let text_limit = text_limit
        .unwrap_or(BROWSER_SNAPSHOT_MAX_TEXT_LIMIT)
        .min(BROWSER_SNAPSHOT_MAX_TEXT_LIMIT);
    let element_offset = element_offset.unwrap_or(0);
    let element_limit = element_limit
        .unwrap_or(BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT)
        .min(BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT);
    let element_query = element_query.unwrap_or("").to_lowercase();
    json!({
        "expression": browser_snapshot_expression(
            text_limit,
            element_offset,
            element_limit,
            &element_query,
        ),
        "awaitPromise": true,
        "returnByValue": true,
    })
}

fn browser_snapshot_expression(
    text_limit: usize,
    element_offset: usize,
    element_limit: usize,
    element_query: &str,
) -> String {
    BROWSER_SNAPSHOT_EXPRESSION_TEMPLATE
        .replace("__TEXT_LIMIT__", &text_limit.to_string())
        .replace("__ELEMENT_OFFSET__", &element_offset.to_string())
        .replace("__ELEMENT_LIMIT__", &element_limit.to_string())
        .replace(
            "__ELEMENT_QUERY__",
            &serde_json::to_string(element_query).expect("serialize element query"),
        )
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
