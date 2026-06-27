use atspi::AccessibilityConnection;
use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use zbus::names::UniqueName;
use zbus::zvariant::ObjectPath;

use crate::atspi::normalize_action;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionInvocationResult {
    pub action_index: i32,
    pub action_name: Option<String>,
    pub ok: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SetValueResult {
    Numeric { value: f64 },
    EditableText,
}

pub async fn available_actions(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<Vec<String>, BackendError> {
    let object_ref = parse_backend_ref(backend_ref)?;
    let accessible = object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!(
                    "failed to resolve backend_ref {backend_ref} into an accessible object: {error}"
                ),
            )
        })?;
    let proxies: atspi::proxy::proxy_ext::Proxies<'_> =
        accessible.proxies().await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!("failed to enumerate accessibility proxies for {backend_ref}: {error}"),
            )
        })?;
    let action = proxies.action().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!("element {backend_ref} does not expose the AT-SPI Action interface: {error}"),
        )
    })?;

    action
        .get_actions()
        .await
        .map(|actions| actions.into_iter().map(|action| action.name).collect())
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!("failed to read AT-SPI action list for {backend_ref}: {error}"),
            )
        })
}

pub async fn invoke_default_action(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(
        connection,
        backend_ref,
        &[
            "click", "press", "activate", "open", "toggle", "jump", "invoke",
        ],
        true,
        None,
    )
    .await
}

pub async fn activate(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(
        connection,
        backend_ref,
        &["activate", "press", "click", "open", "jump", "invoke"],
        false,
        None,
    )
    .await
}

pub async fn select(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    // These semantic ops fall back to the element's primary action only when the
    // live element carries the matching state (`gated_action`), mirroring what
    // tree.rs advertises. So a `select` invoked on a non-selectable element
    // returns false (-> ActionRequiresPhysicalInput) instead of firing the
    // element's primary action, even though the literal `select` name is absent.
    invoke_preferred_action(
        connection,
        backend_ref,
        &["select", "choose"],
        true,
        Some("select"),
    )
    .await
}

pub async fn expand(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(
        connection,
        backend_ref,
        &["expand", "open"],
        true,
        Some("expand"),
    )
    .await
}

pub async fn collapse(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(
        connection,
        backend_ref,
        &["collapse", "close"],
        true,
        Some("collapse"),
    )
    .await
}

pub async fn toggle(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(
        connection,
        backend_ref,
        &["toggle", "check", "uncheck"],
        true,
        Some("toggle"),
    )
    .await
}

pub async fn invoke_secondary_action(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(
        connection,
        backend_ref,
        &["showmenu", "popup", "menu", "contextmenu", "openmenu"],
        false,
        None,
    )
    .await
}

pub async fn grab_focus(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    let object_ref = parse_backend_ref(backend_ref)?;
    let accessible = object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!(
                    "failed to resolve backend_ref {backend_ref} into an accessible object: {error}"
                ),
            )
        })?;
    let proxies: atspi::proxy::proxy_ext::Proxies<'_> =
        accessible.proxies().await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!("failed to enumerate accessibility proxies for {backend_ref}: {error}"),
            )
        })?;
    let component = proxies.component().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!(
                "element {backend_ref} does not expose the AT-SPI Component interface: {error}"
            ),
        )
    })?;

    component.grab_focus().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!("failed to focus AT-SPI element {backend_ref}: {error}"),
        )
    })
}

pub async fn invoke_action_by_index(
    connection: &AccessibilityConnection,
    backend_ref: &str,
    action_index: i32,
) -> Result<ActionInvocationResult, BackendError> {
    if action_index < 0 {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("AT-SPI action index must be non-negative, got {action_index}"),
        ));
    }

    let object_ref = parse_backend_ref(backend_ref)?;
    let accessible = object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!(
                    "failed to resolve backend_ref {backend_ref} into an accessible object: {error}"
                ),
            )
        })?;
    let proxies: atspi::proxy::proxy_ext::Proxies<'_> =
        accessible.proxies().await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!("failed to enumerate accessibility proxies for {backend_ref}: {error}"),
            )
        })?;
    let action = proxies.action().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!("element {backend_ref} does not expose the AT-SPI Action interface: {error}"),
        )
    })?;
    let actions = action.get_actions().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!("failed to fetch AT-SPI actions for {backend_ref}: {error}"),
        )
    })?;
    let action_name = usize::try_from(action_index)
        .ok()
        .and_then(|index| actions.get(index))
        .map(|action| action.name.clone());
    if action_name.is_none() {
        return Err(BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!(
                "element {backend_ref} has {} AT-SPI actions; action_index {action_index} is out of range",
                actions.len()
            ),
        ));
    }

    let ok = action.do_action(action_index).await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!("failed to invoke AT-SPI action {action_index} on {backend_ref}: {error}"),
        )
    })?;

    Ok(ActionInvocationResult {
        action_index,
        action_name,
        ok,
    })
}

pub async fn set_value(
    connection: &AccessibilityConnection,
    backend_ref: &str,
    value: &str,
) -> Result<SetValueResult, BackendError> {
    let object_ref = parse_backend_ref(backend_ref)?;
    let accessible = object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!(
                    "failed to resolve backend_ref {backend_ref} into an accessible object: {error}"
                ),
            )
        })?;
    let proxies: atspi::proxy::proxy_ext::Proxies<'_> =
        accessible.proxies().await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!("failed to enumerate accessibility proxies for {backend_ref}: {error}"),
            )
        })?;

    if let Ok(editable_text) = proxies.editable_text().await {
        editable_text
            .set_text_contents(value)
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!("failed to set editable text contents for {backend_ref}: {error}"),
                )
            })?;
        return Ok(SetValueResult::EditableText);
    }

    if let Ok(value_proxy) = proxies.value().await {
        let parsed = value.parse::<f64>().map_err(|error| {
            BackendError::new(
                BackendErrorCode::InvalidRequest,
                format!("set_value needs a numeric payload for AT-SPI Value targets: {error}"),
            )
        })?;
        value_proxy
            .set_current_value(parsed)
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!("failed to set numeric value for {backend_ref}: {error}"),
                )
            })?;
        return Ok(SetValueResult::Numeric { value: parsed });
    }

    Err(BackendError::new(
        BackendErrorCode::ActionRequiresPhysicalInput,
        format!(
            "element {backend_ref} does not expose EditableText or Value, so semantic set_value is unavailable"
        ),
    ))
}

async fn invoke_preferred_action(
    connection: &AccessibilityConnection,
    backend_ref: &str,
    preferred_names: &[&str],
    fallback_to_first: bool,
    gated_action: Option<&str>,
) -> Result<bool, BackendError> {
    let object_ref = parse_backend_ref(backend_ref)?;
    let accessible = object_ref
        .as_accessible_proxy(connection.connection())
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!(
                    "failed to resolve backend_ref {backend_ref} into an accessible object: {error}"
                ),
            )
        })?;
    let proxies: atspi::proxy::proxy_ext::Proxies<'_> =
        accessible.proxies().await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!("failed to enumerate accessibility proxies for {backend_ref}: {error}"),
            )
        })?;
    let action = proxies.action().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!("element {backend_ref} does not expose the AT-SPI Action interface: {error}"),
        )
    })?;
    let actions = action.get_actions().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!("failed to fetch AT-SPI actions for {backend_ref}: {error}"),
        )
    })?;

    // Resolve the literal action by name first; only consider the primary-action
    // fallback when no literal match exists.
    let preferred_index = match preferred_action_index(
        actions.iter().map(|candidate| candidate.name.as_str()),
        preferred_names,
    ) {
        Some(index) => index,
        None => {
            if !fallback_to_first || actions.is_empty() {
                return Ok(false);
            }
            // For state-driven semantic ops, only fall back to the primary action
            // when the live element still carries the matching state, so a
            // mistargeted (or stale-snapshot) op cannot fire the wrong primary
            // action. Fail closed if the state set cannot be read.
            if let Some(gate) = gated_action {
                let role = accessible.get_role_name().await.unwrap_or_default();
                let state_flags = accessible
                    .get_state()
                    .await
                    .map(|states| states.into_iter().map(|state| state.to_string()).collect())
                    .unwrap_or_else(|_| Vec::new());
                if !super::semantic_action_supported(gate, &role, &state_flags) {
                    return Ok(false);
                }
            }
            0
        }
    };

    let action_index = i32::try_from(preferred_index).map_err(|_| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("preferred action index {preferred_index} exceeds i32 range"),
        )
    })?;
    action.do_action(action_index).await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityCoverageLimited,
            format!("failed to invoke AT-SPI action on {backend_ref}: {error}"),
        )
    })
}

/// Index of the first action whose (normalized) name matches one of
/// `preferred_names`, or `None` when none match. The primary-action fallback is
/// the caller's decision (see [`invoke_preferred_action`]), not this lookup's.
fn preferred_action_index<'a>(
    action_names: impl IntoIterator<Item = &'a str>,
    preferred_names: &[&str],
) -> Option<usize> {
    action_names.into_iter().position(|action_name| {
        let normalized = normalize_action(action_name);
        preferred_names
            .iter()
            .any(|preferred| normalized == normalize_action(preferred))
    })
}

fn parse_backend_ref(backend_ref: &str) -> Result<atspi::ObjectRefOwned, BackendError> {
    let split_index = backend_ref.find(":/").ok_or_else(|| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("backend_ref {backend_ref:?} did not contain a unique-name/path separator"),
        )
    })?;
    let (name, path_with_separator) = backend_ref.split_at(split_index);
    let path = &path_with_separator[1..];
    let unique_name = UniqueName::try_from(name.to_string()).map_err(|error| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("backend_ref {backend_ref:?} had an invalid unique name: {error}"),
        )
    })?;
    let object_path = ObjectPath::try_from(path.to_string()).map_err(|error| {
        BackendError::new(
            BackendErrorCode::InvalidRequest,
            format!("backend_ref {backend_ref:?} had an invalid object path: {error}"),
        )
    })?;
    Ok(atspi::ObjectRef::new_owned(unique_name, object_path))
}

#[cfg(test)]
mod tests {
    use super::{parse_backend_ref, preferred_action_index};

    #[test]
    fn parses_backend_ref_format() {
        let parsed =
            parse_backend_ref(":1.7:/org/a11y/example/path").expect("backend ref should parse");
        assert_eq!(parsed.name().map(|name| name.as_str()), Some(":1.7"));
        assert_eq!(parsed.path().as_str(), "/org/a11y/example/path");
    }

    #[test]
    fn preferred_action_index_returns_none_without_a_literal_match() {
        assert_eq!(
            preferred_action_index(["press", "open"], &["select", "choose"]),
            None
        );
        // No fallback to the first action: the lookup is literal-only, and the
        // primary-action fallback is invoke_preferred_action's gated decision.
        assert_eq!(
            preferred_action_index(["custom"], &["click", "press"]),
            None
        );
    }

    #[test]
    fn preferred_action_index_normalizes_names() {
        assert_eq!(
            preferred_action_index(["Show Menu"], &["showmenu"]),
            Some(0)
        );
        assert_eq!(
            preferred_action_index(["context_menu"], &["contextmenu"]),
            Some(0)
        );
    }
}
