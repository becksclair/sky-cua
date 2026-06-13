//! Snapshot expression privacy and bounds contract tests.

use sky_cua_platform::model::{
    BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT, BROWSER_SNAPSHOT_MAX_TEXT_LIMIT,
};

use crate::browser::snapshot::snapshot_evaluate_params;

#[test]
fn browser_snapshot_expression_suppresses_sensitive_form_values() {
    let params = snapshot_evaluate_params(Some(BROWSER_SNAPSHOT_MAX_TEXT_LIMIT), None, None, None);
    let expression = params["expression"].as_str().expect("snapshot expression");

    assert!(expression.contains("sensitiveField"));
    assert!(expression.contains("api[-_ ]?key"));
    assert!(expression.contains("password"));
    assert!(expression.contains("if (!('value' in el) || sensitiveField(el)) return null;"));
    assert!(expression.contains("return String(el.value).slice"));
    assert!(expression.contains(&format!(
        "const textLimit = {BROWSER_SNAPSHOT_MAX_TEXT_LIMIT};"
    )));
    assert!(expression.contains("for (const char of fullText)"));
    assert!(expression.contains("if (textChars.length < textLimit)"));
    assert!(expression.contains("textCharCount = null;"));
    assert!(expression.contains("break;"));
    assert!(expression.contains("const text = textChars.join('')"));
    assert!(expression.contains("textCharCount,"));
    assert!(expression.contains("textTruncated,"));
    assert!(!expression.contains("Array.from(fullText)"));
    assert!(!expression.contains("fullText.slice"));
    assert!(expression.contains("const elementOffset = 0;"));
    assert!(expression.contains(&format!(
        "const elementLimit = {BROWSER_SNAPSHOT_MAX_ELEMENT_LIMIT};"
    )));
    assert!(expression.contains("const elementQuery = \"\";"));
    assert!(expression.contains("const elements = document.querySelectorAll(selector);"));
    assert!(expression.contains("if (elementLimit > 0)"));
    assert!(expression.contains("for (let index = 0; index < elements.length; index += 1)"));
    assert!(expression.contains("if (!elementMatches(el)) continue;"));
    assert!(expression.contains("if (matchedIndex >= elementOffset)"));
    assert!(expression.contains("projectedElements.push(elementFor(el, index));"));
    assert!(expression.contains("if (projectedElements.length >= elementLimit) break;"));
    assert!(!expression.contains("Array.from(document.querySelectorAll(selector))"));
    // Element bounds stay in CSS pixels so they line up with screenshot
    // pixels and pointer coordinates.
    assert!(!expression.contains("rect.x * scale"));
    assert!(expression.contains("x: rect.x"));
    assert!(!expression.contains("elements.slice(0, 5000).map(elementFor)"));
}

#[test]
fn browser_snapshot_zero_text_limit_skips_full_text_extraction() {
    let params = snapshot_evaluate_params(Some(0), None, None, None);
    let expression = params["expression"].as_str().expect("snapshot expression");

    assert!(expression.contains("const textLimit = 0;"));
    assert!(expression.contains("const includeText = textLimit > 0;"));
    assert!(expression.contains("includeText ? (document.body?.innerText || '') : ''"));
    assert!(expression.contains("const textChars = [];"));
}

#[test]
fn browser_snapshot_expression_projects_elements_in_service() {
    let params = snapshot_evaluate_params(Some(4000), Some(7), Some(11), Some("Settings \"Menu\""));
    let expression = params["expression"].as_str().expect("snapshot expression");

    assert!(expression.contains("const elementOffset = 7;"));
    assert!(expression.contains("const elementLimit = 11;"));
    assert!(expression.contains("const elementQuery = \"settings \\\"menu\\\"\";"));
    assert!(expression.contains("const elementMatches = (el) => !elementQuery"));
    assert!(expression.contains("if (!elementMatches(el)) continue;"));
    assert!(expression.contains("if (matchedIndex >= elementOffset)"));
}
