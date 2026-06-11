//! Snapshot expression privacy and bounds contract tests.

use crate::browser::snapshot::BROWSER_SNAPSHOT_EXPRESSION;

#[test]
fn browser_snapshot_expression_suppresses_sensitive_form_values() {
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("sensitiveField"));
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("api[-_ ]?key"));
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("password"));
    assert!(
        BROWSER_SNAPSHOT_EXPRESSION
            .contains("if (!('value' in el) || sensitiveField(el)) return null;")
    );
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("return String(el.value).slice"));
    // Element bounds stay in CSS pixels so they line up with screenshot
    // pixels and pointer coordinates.
    assert!(!BROWSER_SNAPSHOT_EXPRESSION.contains("rect.x * scale"));
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("x: rect.x"));
    assert!(BROWSER_SNAPSHOT_EXPRESSION.contains("elements.slice(0, 5000)"));
}
