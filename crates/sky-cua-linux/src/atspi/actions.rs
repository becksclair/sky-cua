use atspi::AccessibilityConnection;
use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use zbus::names::UniqueName;
use zbus::zvariant::ObjectPath;

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
    )
    .await
}

pub async fn select(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(connection, backend_ref, &["select", "choose"], false).await
}

pub async fn expand(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(connection, backend_ref, &["expand", "open"], false).await
}

pub async fn collapse(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(connection, backend_ref, &["collapse", "close"], false).await
}

pub async fn toggle(
    connection: &AccessibilityConnection,
    backend_ref: &str,
) -> Result<bool, BackendError> {
    invoke_preferred_action(
        connection,
        backend_ref,
        &["toggle", "check", "uncheck"],
        false,
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

pub async fn set_value(
    connection: &AccessibilityConnection,
    backend_ref: &str,
    value: &str,
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

    if let Ok(editable_text) = proxies.editable_text().await {
        return editable_text
            .set_text_contents(value)
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::AccessibilityCoverageLimited,
                    format!("failed to set editable text contents for {backend_ref}: {error}"),
                )
            });
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
        return Ok(true);
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

    let Some(preferred_index) = preferred_action_index(
        actions.iter().map(|candidate| candidate.name.as_str()),
        preferred_names,
        fallback_to_first,
    ) else {
        return Ok(false);
    };

    action
        .do_action(i32::try_from(preferred_index).unwrap_or(0))
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::AccessibilityCoverageLimited,
                format!("failed to invoke AT-SPI action on {backend_ref}: {error}"),
            )
        })
}

fn preferred_action_index<'a>(
    action_names: impl IntoIterator<Item = &'a str>,
    preferred_names: &[&str],
    fallback_to_first: bool,
) -> Option<usize> {
    let mut found_any = false;
    for (index, action_name) in action_names.into_iter().enumerate() {
        found_any = true;
        let normalized = normalize_action(action_name);
        if preferred_names
            .iter()
            .any(|preferred| normalized == normalize_action(preferred))
        {
            return Some(index);
        }
    }
    (fallback_to_first && found_any).then_some(0)
}

fn normalize_action(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
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
    fn strict_preferred_action_index_does_not_fall_back_to_first_action() {
        assert_eq!(
            preferred_action_index(["press", "open"], &["select", "choose"], false),
            None
        );
    }

    #[test]
    fn default_action_index_can_fall_back_to_first_action() {
        assert_eq!(
            preferred_action_index(["custom"], &["click", "press"], true),
            Some(0)
        );
    }

    #[test]
    fn preferred_action_index_normalizes_names() {
        assert_eq!(
            preferred_action_index(["Show Menu"], &["showmenu"], false),
            Some(0)
        );
        assert_eq!(
            preferred_action_index(["context_menu"], &["contextmenu"], false),
            Some(0)
        );
    }
}
