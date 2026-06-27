pub mod actions;
pub mod snapshot;
pub mod tree;

use atspi::AccessibilityConnection;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

pub(crate) fn normalize_action(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
}

/// Whether an AT-SPI state set lets the element satisfy a state-driven semantic
/// action through its primary action.
///
/// The snapshot ([`tree::add_state_inferred_actions`]) advertises an action only
/// when this returns true, and the dispatch path ([`actions`]) falls back to the
/// element's primary action only when this returns true. Sharing one predicate
/// keeps advertisement and invocation consistent: a `desktop_toggle` aimed at a
/// non-checkable element (or an `expand` aimed at an already-expanded one) is
/// rejected with `ActionRequiresPhysicalInput` instead of silently firing the
/// element's primary action in the wrong direction. State strings are the
/// lowercase `State::to_string()` values AT-SPI reports.
pub(crate) fn state_supports_semantic_action(action: &str, state_flags: &[String]) -> bool {
    let has = |state: &str| state_flags.iter().any(|flag| flag == state);
    match action {
        "toggle" => has("checkable"),
        "select" => has("selectable"),
        "expand" => has("expandable") && !has("expanded"),
        "collapse" => has("expandable") && has("expanded"),
        _ => false,
    }
}

pub async fn connect() -> Result<AccessibilityConnection, BackendError> {
    AccessibilityConnection::new().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityUnavailable,
            format!("failed to connect to the AT-SPI accessibility bus: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::state_supports_semantic_action;

    fn flags(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn state_predicate_gates_each_semantic_action_on_its_state() {
        assert!(state_supports_semantic_action(
            "toggle",
            &flags(&["checkable", "focusable"])
        ));
        assert!(!state_supports_semantic_action(
            "toggle",
            &flags(&["focusable"])
        ));

        assert!(state_supports_semantic_action(
            "select",
            &flags(&["selectable"])
        ));
        assert!(!state_supports_semantic_action(
            "select",
            &flags(&["checkable"])
        ));

        // Direction is state-aware: only the transition the element can still make.
        assert!(state_supports_semantic_action(
            "expand",
            &flags(&["expandable"])
        ));
        assert!(!state_supports_semantic_action(
            "expand",
            &flags(&["expandable", "expanded"])
        ));
        assert!(state_supports_semantic_action(
            "collapse",
            &flags(&["expandable", "expanded"])
        ));
        assert!(!state_supports_semantic_action(
            "collapse",
            &flags(&["expandable"])
        ));

        // An expander that is neither expand- nor collapse-eligible without the
        // expandable flag, and unknown actions, are never supported.
        assert!(!state_supports_semantic_action(
            "collapse",
            &flags(&["expanded"])
        ));
        assert!(!state_supports_semantic_action(
            "activate",
            &flags(&["checkable"])
        ));
    }
}
