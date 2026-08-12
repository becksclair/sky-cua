pub mod actions;
mod repair;
pub mod snapshot;
pub mod tree;

use atspi_connection::AccessibilityConnection;
pub(crate) use repair::{AccessibilityConnectFailure, RepairCoordinator, connect_with_repair};
use sky_cua_platform::diagnostics::BackendError;
use std::time::Duration;

#[cfg(not(test))]
const ACCESSIBILITY_CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
const ACCESSIBILITY_CONNECTION_TIMEOUT: Duration = Duration::from_millis(50);

pub(crate) fn normalize_action(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
}

/// Whether an element's role and state let it satisfy a state-driven semantic
/// action through its primary action.
///
/// The snapshot ([`tree::add_state_inferred_actions`]) advertises an action only
/// when this returns true, and the dispatch path ([`actions`]) falls back to the
/// element's primary action only when this returns true. Sharing one predicate
/// keeps advertisement and invocation consistent: a `desktop_toggle` aimed at a
/// non-toggleable element (or an `expand` aimed at an already-expanded one) is
/// rejected with `ActionRequiresPhysicalInput` instead of silently firing the
/// element's primary action in the wrong direction.
///
/// `toggle` keys on the role as well as state because real toolkits (e.g. GTK
/// check buttons reported as role "check box") expose the toggle affordance
/// through their role and primary action without ever setting the `checkable`
/// state. Role strings are the lowercase `get_role_name()` values AT-SPI reports
/// (e.g. "check box", "toggle button"); state strings are lowercase
/// `State::to_string()` values.
pub(crate) fn semantic_action_supported(action: &str, role: &str, state_flags: &[String]) -> bool {
    let has = |state: &str| state_flags.iter().any(|flag| flag == state);
    let is_toggle_role = || {
        matches!(
            role.trim().to_ascii_lowercase().as_str(),
            "check box"
                | "checkbox"
                | "radio button"
                | "check menu item"
                | "radio menu item"
                | "toggle button"
        )
    };
    match action {
        "toggle" => has("checkable") || is_toggle_role(),
        "select" => has("selectable"),
        "expand" => has("expandable") && !has("expanded"),
        "collapse" => has("expandable") && has("expanded"),
        _ => false,
    }
}

pub async fn connect() -> Result<AccessibilityConnection, BackendError> {
    connect_attempt()
        .await
        .map_err(AccessibilityConnectFailure::into_backend_error)
}

pub(crate) async fn connect_attempt() -> Result<AccessibilityConnection, AccessibilityConnectFailure>
{
    connect_with_timeout(AccessibilityConnection::new()).await
}

async fn connect_with_timeout<T, E>(
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, AccessibilityConnectFailure>
where
    E: std::fmt::Display,
{
    match tokio::time::timeout(ACCESSIBILITY_CONNECTION_TIMEOUT, future).await {
        Ok(Ok(connection)) => Ok(connection),
        Ok(Err(error)) => Err(AccessibilityConnectFailure::Error(error.to_string())),
        Err(_) => Err(AccessibilityConnectFailure::Timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::{connect_with_timeout, semantic_action_supported};

    fn flags(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn semantic_predicate_gates_each_action_on_role_and_state() {
        // toggle: by checkable state, or by toggle-ish role even without the
        // checkable state (GTK check buttons report role "check box" only).
        assert!(semantic_action_supported(
            "toggle",
            "push button",
            &flags(&["checkable", "focusable"])
        ));
        assert!(semantic_action_supported(
            "toggle",
            "check box",
            &flags(&["enabled", "focusable", "sensitive"])
        ));
        assert!(semantic_action_supported(
            "toggle",
            "toggle button",
            &flags(&[])
        ));
        // A plain button with no toggle role and no checkable state is rejected.
        assert!(!semantic_action_supported(
            "toggle",
            "push button",
            &flags(&["focusable"])
        ));

        assert!(semantic_action_supported(
            "select",
            "list item",
            &flags(&["selectable"])
        ));
        assert!(!semantic_action_supported(
            "select",
            "check box",
            &flags(&["checkable"])
        ));

        // Direction is state-aware: only the transition the element can still make.
        assert!(semantic_action_supported(
            "expand",
            "toggle button",
            &flags(&["expandable"])
        ));
        assert!(!semantic_action_supported(
            "expand",
            "toggle button",
            &flags(&["expandable", "expanded"])
        ));
        assert!(semantic_action_supported(
            "collapse",
            "toggle button",
            &flags(&["expandable", "expanded"])
        ));
        assert!(!semantic_action_supported(
            "collapse",
            "toggle button",
            &flags(&["expandable"])
        ));

        // Without the expandable flag, and for unknown actions, never supported.
        assert!(!semantic_action_supported(
            "collapse",
            "toggle button",
            &flags(&["expanded"])
        ));
        assert!(!semantic_action_supported(
            "activate",
            "check box",
            &flags(&["checkable"])
        ));
    }

    #[tokio::test]
    async fn accessibility_connection_attempt_is_deadline_bounded() {
        let result = connect_with_timeout(async {
            std::future::pending::<()>().await;
            Ok::<(), &str>(())
        })
        .await;
        let error = result.expect_err("a permanently pending connection must time out");
        assert!(matches!(error, super::AccessibilityConnectFailure::Timeout));
    }
}
