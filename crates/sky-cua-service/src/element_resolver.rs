use serde_json::Value;
use sky_cua_platform::model::{ActionName, AppStateSnapshot, ElementNode};

#[derive(Debug, Clone, Default)]
struct ElementSelector<'a> {
    role: Option<&'a str>,
    name: Option<&'a str>,
    text: Option<&'a str>,
    states: Vec<String>,
}

pub fn resolve_action_element(
    snapshot: &AppStateSnapshot,
    action: &ActionName,
    element_index: Option<usize>,
    arguments: &Value,
) -> Result<Option<ElementNode>, (&'static str, String)> {
    if direct_backend_ref(arguments).is_some() {
        return Ok(None);
    }

    if let Some(index) = element_index {
        return resolve_element(snapshot, index).map(Some);
    }

    let selector = selector_from_arguments(action, arguments);
    if selector.is_empty() {
        return Ok(None);
    }

    resolve_semantic_node(snapshot, &selector, action).map(Some)
}

pub fn resolve_target_element(
    snapshot: &AppStateSnapshot,
    arguments: &Value,
) -> Result<Option<ElementNode>, (&'static str, String)> {
    let Some(index) = arguments
        .get("to_element_index")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
    else {
        return Ok(None);
    };
    resolve_element(snapshot, index).map(Some)
}

pub fn direct_backend_ref(arguments: &Value) -> Option<&str> {
    arguments
        .get("element_identifier")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resolve_element(
    snapshot: &AppStateSnapshot,
    index: usize,
) -> Result<ElementNode, (&'static str, String)> {
    snapshot.elements.get(index).cloned().ok_or_else(|| {
        (
            "InvalidRequest",
            format!(
                "element_index {index} is out of range for snapshot {}",
                snapshot.snapshot_id
            ),
        )
    })
}

fn selector_from_arguments<'a>(action: &ActionName, arguments: &'a Value) -> ElementSelector<'a> {
    ElementSelector {
        role: arguments.get("role").and_then(Value::as_str),
        name: arguments.get("name").and_then(Value::as_str),
        text: (action != &ActionName::TypeText)
            .then(|| arguments.get("text").and_then(Value::as_str))
            .flatten(),
        states: arguments
            .get("states")
            .and_then(Value::as_array)
            .map(|states| {
                states
                    .iter()
                    .filter_map(|state| state.as_str().map(ToOwned::to_owned))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    }
}

impl ElementSelector<'_> {
    fn is_empty(&self) -> bool {
        [self.role, self.name, self.text]
            .into_iter()
            .all(|value| value.map(str::trim).is_none_or(str::is_empty))
            && self.states.iter().all(|value| value.trim().is_empty())
    }
}

fn resolve_semantic_node(
    snapshot: &AppStateSnapshot,
    selector: &ElementSelector<'_>,
    action: &ActionName,
) -> Result<ElementNode, (&'static str, String)> {
    let mut matches = snapshot
        .elements
        .iter()
        .filter(|node| node_matches_selector(node, selector))
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return Err((
            "InvalidRequest",
            format!(
                "No cached accessibility node matched semantic selector {}. Call get_app_state first or pass element_index.",
                describe_selector(selector)
            ),
        ));
    }

    if let Some(node) = unique_preferred_node(&matches, |node| node_matches_action(node, action)) {
        return Ok(node.clone());
    }

    let action_matches = matches
        .iter()
        .copied()
        .filter(|node| node_matches_action(node, action))
        .collect::<Vec<_>>();
    if !action_matches.is_empty() {
        matches = action_matches;
    }

    if let Some(node) = unique_preferred_node(&matches, node_is_showing) {
        return Ok(node.clone());
    }

    let visible_matches = matches
        .iter()
        .copied()
        .filter(|node| node_is_showing(node))
        .collect::<Vec<_>>();
    if !visible_matches.is_empty() {
        matches = visible_matches;
    }

    if matches.len() == 1 {
        return Ok(matches[0].clone());
    }

    Err((
        "InvalidRequest",
        format!(
            "Semantic selector {} matched multiple cached nodes: {}. Pass element_index or add more selector fields.",
            describe_selector(selector),
            describe_matching_nodes(&matches)
        ),
    ))
}

fn unique_preferred_node<'a>(
    nodes: &[&'a ElementNode],
    predicate: impl Fn(&ElementNode) -> bool,
) -> Option<&'a ElementNode> {
    let mut matches = nodes.iter().copied().filter(|node| predicate(node));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn node_matches_selector(node: &ElementNode, selector: &ElementSelector<'_>) -> bool {
    selector
        .role
        .is_none_or(|role| normalized_contains(Some(node.role.as_str()), role))
        && selector
            .name
            .is_none_or(|name| normalized_contains(node.name.as_deref(), name))
        && selector.text.is_none_or(|text| {
            normalized_contains(node.name.as_deref(), text)
                || normalized_contains(node.description.as_deref(), text)
                || normalized_contains(node.value.as_deref(), text)
        })
        && selector
            .states
            .iter()
            .filter(|state| !state.trim().is_empty())
            .all(|state| {
                node.state_flags
                    .iter()
                    .any(|node_state| normalized_equals(node_state, state))
            })
}

fn node_matches_action(node: &ElementNode, action: &ActionName) -> bool {
    let wanted = match action {
        ActionName::FocusElement => "focus",
        ActionName::ActivateElement => "activate",
        ActionName::SelectElement => "select",
        ActionName::ExpandElement => "expand",
        ActionName::CollapseElement => "collapse",
        ActionName::ToggleElement => "toggle",
        ActionName::SetValue => "set_value",
        ActionName::PerformSecondaryAction => "showmenu",
        ActionName::Click => "activate",
        ActionName::PerformAction
        | ActionName::Scroll
        | ActionName::Drag
        | ActionName::TypeText
        | ActionName::PressKey => return true,
    };
    node.semantic_actions
        .iter()
        .any(|action| normalized_equals(action, wanted))
}

fn node_is_showing(node: &ElementNode) -> bool {
    node.state_flags
        .iter()
        .any(|state| normalized_equals(state, "showing") || normalized_equals(state, "visible"))
}

fn normalized_contains(actual: Option<&str>, needle: &str) -> bool {
    let needle = normalize(needle);
    !needle.is_empty()
        && actual
            .map(normalize)
            .is_some_and(|actual| actual.contains(&needle))
}

fn normalized_equals(left: &str, right: &str) -> bool {
    let left_norm = normalize(left);
    let right_norm = normalize(right);
    !left_norm.is_empty() && left_norm == right_norm
}

fn normalize(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
}

fn describe_selector(selector: &ElementSelector<'_>) -> String {
    let mut parts = Vec::new();
    if let Some(role) = selector.role.filter(|value| !value.trim().is_empty()) {
        parts.push(format!("role={role:?}"));
    }
    if let Some(name) = selector.name.filter(|value| !value.trim().is_empty()) {
        parts.push(format!("name={name:?}"));
    }
    if let Some(text) = selector.text.filter(|value| !value.trim().is_empty()) {
        parts.push(format!("text={text:?}"));
    }
    let states = selector
        .states
        .iter()
        .filter(|state| !state.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>();
    if !states.is_empty() {
        parts.push(format!("states={states:?}"));
    }
    parts.join(", ")
}

fn describe_matching_nodes(nodes: &[&ElementNode]) -> String {
    nodes
        .iter()
        .take(8)
        .map(|node| {
            format!(
                "#{} role={} name={:?}",
                node.element_index, node.role, node.name
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use sky_cua_platform::model::{
        CaptureBackendKind, EnvironmentInfo, InputBackendKind, PortalCapabilities,
        SemanticBackendKind, SessionKind, ToolAvailability, ToolCapabilities,
    };

    #[test]
    fn semantic_selector_resolves_unique_node() {
        let snapshot = snapshot(vec![node(
            0,
            "button",
            Some("Save"),
            vec!["showing"],
            vec!["press"],
        )]);

        let resolved = resolve_action_element(
            &snapshot,
            &ActionName::ActivateElement,
            None,
            &serde_json::json!({"role": "button", "name": "save"}),
        )
        .unwrap()
        .unwrap();

        assert_eq!(resolved.element_index, 0);
    }

    #[test]
    fn semantic_selector_reports_ambiguous_matches() {
        let snapshot = snapshot(vec![
            node(0, "button", Some("OK"), vec!["showing"], vec!["press"]),
            node(1, "button", Some("OK"), vec!["showing"], vec!["press"]),
        ]);

        let error = resolve_action_element(
            &snapshot,
            &ActionName::ActivateElement,
            None,
            &serde_json::json!({"role": "button", "name": "ok"}),
        )
        .unwrap_err();

        assert_eq!(error.0, "InvalidRequest");
        assert!(error.1.contains("matched multiple"));
    }

    #[test]
    fn type_text_text_argument_is_not_a_semantic_selector() {
        let snapshot = snapshot(vec![node(
            0,
            "entry",
            Some("Address"),
            vec!["showing"],
            vec!["set_value"],
        )]);

        let resolved = resolve_action_element(
            &snapshot,
            &ActionName::TypeText,
            None,
            &serde_json::json!({"text": "chrome-extension://example"}),
        )
        .unwrap();

        assert!(resolved.is_none());
    }

    fn snapshot(elements: Vec<ElementNode>) -> AppStateSnapshot {
        AppStateSnapshot {
            snapshot_id: "snap".to_string(),
            created_at: Utc::now(),
            environment: EnvironmentInfo {
                session_kind: SessionKind::Unsupported,
                compositor: None,
                desktop_environment: None,
                capture_backend: CaptureBackendKind::None,
                input_backend: InputBackendKind::None,
                semantic_backend: SemanticBackendKind::None,
                portal_capabilities: PortalCapabilities {
                    screencast_version: None,
                    remote_desktop_version: None,
                    screenshot_version: None,
                    available_source_types: None,
                    available_cursor_modes: None,
                    available_device_types: None,
                },
                xdg_session_type: None,
                display: None,
                wayland_display: None,
            },
            capabilities: ToolCapabilities {
                list_apps: unavailable(),
                get_app_state: unavailable(),
                focus_element: unavailable(),
                activate_element: unavailable(),
                select_element: unavailable(),
                expand_element: unavailable(),
                collapse_element: unavailable(),
                toggle_element: unavailable(),
                click: unavailable(),
                perform_action: unavailable(),
                perform_secondary_action: unavailable(),
                scroll: unavailable(),
                supported_scroll_directions: Vec::new(),
                drag: unavailable(),
                type_text: unavailable(),
                press_key: unavailable(),
                set_value: unavailable(),
            },
            focused_app: None,
            capture: None,
            elements,
            diagnostics: Vec::new(),
            app_guidance: None,
            doctor_report: None,
            agent_cursor: None,
        }
    }

    fn node(
        index: usize,
        role: &str,
        name: Option<&str>,
        states: Vec<&str>,
        actions: Vec<&str>,
    ) -> ElementNode {
        ElementNode {
            element_index: index,
            parent_index: None,
            role: role.to_string(),
            name: name.map(ToOwned::to_owned),
            description: None,
            value: None,
            text: None,
            numeric_value: None,
            supports_editable_text: false,
            state_flags: states.into_iter().map(ToOwned::to_owned).collect(),
            semantic_actions: actions.into_iter().map(ToOwned::to_owned).collect(),
            bounds: None,
            backend_ref: Some(format!(":1.{index}:/node/{index}")),
        }
    }

    fn unavailable() -> ToolAvailability {
        ToolAvailability {
            available: false,
            reason: None,
        }
    }
}
