use atspi::AccessibilityConnection;
use atspi::CoordType;
use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
use sky_cua_platform::model::{
    CoordinateSpace, ElementNode, ElementNumericValueReadback, ElementTextReadback,
    ElementTextSelection, RectF,
};

use crate::apps::discovery::DiscoveredApp;
use crate::atspi::normalize_action;

pub async fn flatten_accessible_tree(
    connection: &AccessibilityConnection,
    app: &DiscoveredApp,
    max_nodes: usize,
) -> Vec<ElementNode> {
    let mut nodes = Vec::new();
    let mut stack = vec![(app.object_ref.clone(), None, 0usize)];

    while let Some((object_ref, parent_index, depth)) = stack.pop() {
        if nodes.len() >= max_nodes {
            break;
        }
        let Ok(accessible) = object_ref
            .as_accessible_proxy(connection.connection())
            .await
        else {
            continue;
        };
        let role = accessible
            .get_role_name()
            .await
            .unwrap_or_else(|_| "unknown".to_string());
        let name = accessible
            .name()
            .await
            .ok()
            .filter(|value| !value.trim().is_empty());
        let description = accessible
            .description()
            .await
            .ok()
            .filter(|value| !value.trim().is_empty());
        let state_flags = accessible
            .get_state()
            .await
            .map(|states| states.into_iter().map(|state| state.to_string()).collect())
            .unwrap_or_else(|_| Vec::new());
        let backend_ref = Some(format!(
            "{}:{}",
            accessible.inner().destination(),
            accessible.inner().path()
        ));
        let proxies = accessible.proxies().await.ok();
        let bounds =
            if let Some(proxies) = proxies.as_ref() {
                if let Ok(component) = proxies.component().await {
                    component.get_extents(CoordType::Screen).await.ok().map(
                        |(x, y, width, height)| RectF {
                            x: f64::from(x),
                            y: f64::from(y),
                            width: f64::from(width),
                            height: f64::from(height),
                            space: CoordinateSpace::DesktopLogical,
                        },
                    )
                } else {
                    None
                }
            } else {
                None
            };
        let mut supports_editable_text = false;
        let mut supports_numeric_value = false;
        let semantic_actions = if let Some(proxies) = proxies.as_ref() {
            let mut actions = Vec::new();
            if let Ok(action_proxy) = proxies.action().await
                && let Ok(available_actions) = action_proxy.get_actions().await
            {
                actions.extend(
                    available_actions
                        .into_iter()
                        .map(|action| action.name)
                        .filter(|name| !name.trim().is_empty()),
                );
                add_canonical_action_aliases(&mut actions);
            }
            if proxies.component().await.is_ok() && !actions.iter().any(|name| name == "focus") {
                actions.push("focus".to_string());
            }
            supports_editable_text = proxies.editable_text().await.is_ok();
            supports_numeric_value = proxies.value().await.is_ok();
            if (supports_editable_text || supports_numeric_value)
                && !actions.iter().any(|name| name == "set_value")
            {
                actions.push("set_value".to_string());
            }
            actions
        } else {
            Vec::new()
        };
        let is_focused = state_flags.iter().any(|state| state == "focused");
        let has_set_value = semantic_actions.iter().any(|action| action == "set_value");
        let should_read_text = supports_editable_text || is_focused || has_set_value;
        let sensitive = is_sensitive_text_node(&role, name.as_deref(), &state_flags);
        let text = if should_read_text {
            read_text_readback(proxies.as_ref(), sensitive).await
        } else {
            None
        };
        let numeric_value = if should_read_numeric_value(supports_numeric_value, sensitive) {
            read_numeric_value_readback(proxies.as_ref()).await
        } else {
            None
        };
        let value = readback_summary(text.as_ref(), numeric_value.as_ref());

        let keep = depth == 0
            || name.is_some()
            || description.is_some()
            || value.is_some()
            || text.is_some()
            || numeric_value.is_some()
            || !semantic_actions.is_empty()
            || state_flags
                .iter()
                .any(|state| state == "focused" || state == "active");
        let current_index = if keep {
            let index = nodes.len();
            nodes.push(ElementNode {
                element_index: index,
                parent_index,
                role,
                name,
                description,
                value,
                text,
                numeric_value,
                supports_editable_text,
                state_flags,
                semantic_actions,
                bounds,
                backend_ref,
            });
            Some(index)
        } else {
            parent_index
        };

        let children = accessible.get_children().await.unwrap_or_default();
        for child in children.into_iter().rev() {
            stack.push((child, current_index, depth + 1));
        }
    }

    nodes
}

async fn read_text_readback(
    proxies: Option<&atspi::proxy::proxy_ext::Proxies<'_>>,
    sensitive: bool,
) -> Option<ElementTextReadback> {
    let text = proxies?.text().await.ok()?;
    let character_count = text.character_count().await.ok()?.max(0);
    let caret_offset = text.caret_offset().await.ok();
    let capped_count = character_count.min(MAX_TEXT_READBACK_CHARS);
    let content = if sensitive {
        None
    } else if capped_count > 0 {
        text.get_text(0, capped_count).await.ok()
    } else {
        Some(String::new())
    };
    let selection_count = text
        .get_nselections()
        .await
        .unwrap_or_default()
        .clamp(0, MAX_TEXT_SELECTIONS);
    let mut selections = Vec::new();
    for index in 0..selection_count {
        if let Ok((start_offset, end_offset)) = text.get_selection(index).await {
            selections.push(ElementTextSelection {
                start_offset,
                end_offset,
            });
        }
    }

    Some(shape_text_readback(
        character_count,
        caret_offset,
        content,
        selections,
        sensitive,
    ))
}

async fn read_numeric_value_readback(
    proxies: Option<&atspi::proxy::proxy_ext::Proxies<'_>>,
) -> Option<ElementNumericValueReadback> {
    let value = proxies?.value().await.ok()?;
    Some(ElementNumericValueReadback {
        current: value.current_value().await.ok()?,
        minimum: value.minimum_value().await.ok()?,
        maximum: value.maximum_value().await.ok()?,
        minimum_increment: value.minimum_increment().await.ok()?,
        text: value
            .text()
            .await
            .ok()
            .filter(|value| !value.trim().is_empty()),
    })
}

fn shape_text_readback(
    character_count: i32,
    caret_offset: Option<i32>,
    content: Option<String>,
    selections: Vec<ElementTextSelection>,
    sensitive: bool,
) -> ElementTextReadback {
    let character_count = character_count.max(0);
    let truncated = character_count > MAX_TEXT_READBACK_CHARS;
    let selections = selections
        .into_iter()
        .take(MAX_TEXT_SELECTIONS as usize)
        .collect();
    ElementTextReadback {
        character_count,
        caret_offset,
        content: if sensitive { None } else { content },
        content_suppressed: sensitive,
        truncated,
        selections,
    }
}

fn readback_summary(
    text: Option<&ElementTextReadback>,
    numeric_value: Option<&ElementNumericValueReadback>,
) -> Option<String> {
    if let Some(text) = text {
        if text.content_suppressed {
            return None;
        }
        if let Some(content) = &text.content {
            return Some(content.clone());
        }
    }
    numeric_value.map(numeric_value_summary)
}

fn numeric_value_summary(value: &ElementNumericValueReadback) -> String {
    value
        .text
        .as_ref()
        .filter(|text| !text.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| value.current.to_string())
}

fn should_read_numeric_value(supports_numeric_value: bool, sensitive: bool) -> bool {
    supports_numeric_value && !sensitive
}

fn is_sensitive_text_node(role: &str, name: Option<&str>, state_flags: &[String]) -> bool {
    let role = role.to_ascii_lowercase();
    if role.contains("password") {
        return true;
    }
    if state_flags.iter().any(|state| {
        let normalized = state.to_ascii_lowercase();
        normalized.contains("password") || normalized.contains("protected")
    }) {
        return true;
    }
    name.is_some_and(|name| {
        let normalized = name.to_ascii_lowercase();
        normalized.contains("password")
            || normalized.contains("passphrase")
            || normalized.contains("secret")
            || normalized.contains("token")
    })
}

fn add_canonical_action_aliases(actions: &mut Vec<String>) {
    let normalized = actions
        .iter()
        .map(|action| normalize_action(action))
        .collect::<Vec<_>>();
    let mut maybe_push = |action: &str, aliases: &[&str]| {
        if normalized
            .iter()
            .any(|candidate| aliases.iter().any(|alias| candidate == alias))
            && !actions.iter().any(|existing| existing == action)
        {
            actions.push(action.to_string());
        }
    };

    maybe_push(
        "activate",
        &["activate", "press", "click", "open", "jump", "invoke"],
    );
    maybe_push("select", &["select", "choose"]);
    maybe_push("expand", &["expand", "open"]);
    maybe_push("collapse", &["collapse", "close"]);
    maybe_push("toggle", &["toggle", "check", "uncheck"]);
}

const MAX_TEXT_READBACK_CHARS: i32 = 4096;
const MAX_TEXT_SELECTIONS: i32 = 8;

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::{ElementNumericValueReadback, ElementTextSelection};

    use super::{
        add_canonical_action_aliases, is_sensitive_text_node, readback_summary,
        shape_text_readback, should_read_numeric_value,
    };

    #[test]
    fn canonical_aliases_match_semantic_action_fallbacks() {
        let mut actions = vec![
            "Press".to_string(),
            "Open".to_string(),
            "Close".to_string(),
            "Toggle".to_string(),
            "Choose".to_string(),
        ];

        add_canonical_action_aliases(&mut actions);

        for expected in ["activate", "expand", "collapse", "toggle", "select"] {
            assert!(
                actions.iter().any(|action| action == expected),
                "missing canonical action {expected} in {actions:?}"
            );
        }
    }

    #[test]
    fn canonical_aliases_do_not_duplicate_existing_actions() {
        let mut actions = vec!["activate".to_string(), "press".to_string()];

        add_canonical_action_aliases(&mut actions);

        assert_eq!(
            actions
                .iter()
                .filter(|action| action.as_str() == "activate")
                .count(),
            1
        );
    }

    #[test]
    fn editable_text_readback_keeps_empty_string_as_known_value() {
        let text = shape_text_readback(0, Some(0), Some(String::new()), Vec::new(), false);

        assert_eq!(text.content.as_deref(), Some(""));
        assert_eq!(readback_summary(Some(&text), None).as_deref(), Some(""));
        assert!(!text.truncated);
        assert!(!text.content_suppressed);
    }

    #[test]
    fn focused_readonly_text_can_be_summarized() {
        let text = shape_text_readback(
            12,
            Some(4),
            Some("visible text".to_string()),
            Vec::new(),
            false,
        );

        assert_eq!(
            readback_summary(Some(&text), None).as_deref(),
            Some("visible text")
        );
        assert_eq!(text.caret_offset, Some(4));
    }

    #[test]
    fn long_text_readback_marks_truncation_and_caps_selections() {
        let selections = (0..12)
            .map(|index| ElementTextSelection {
                start_offset: index,
                end_offset: index + 1,
            })
            .collect();

        let text =
            shape_text_readback(5000, None, Some("truncated".to_string()), selections, false);

        assert!(text.truncated);
        assert_eq!(text.selections.len(), 8);
    }

    #[test]
    fn sensitive_text_suppresses_content_and_summary() {
        let text = shape_text_readback(9, Some(9), Some("secret123".to_string()), Vec::new(), true);

        assert!(text.content_suppressed);
        assert_eq!(text.content, None);
        assert_eq!(readback_summary(Some(&text), None), None);
    }

    #[test]
    fn sensitive_text_does_not_fall_back_to_numeric_summary() {
        let text = shape_text_readback(9, Some(9), Some("secret123".to_string()), Vec::new(), true);
        let numeric = ElementNumericValueReadback {
            current: 123.0,
            minimum: 0.0,
            maximum: 999.0,
            minimum_increment: 1.0,
            text: Some("secret123".to_string()),
        };

        assert_eq!(readback_summary(Some(&text), Some(&numeric)), None);
    }

    #[test]
    fn non_sensitive_text_without_content_can_fall_back_to_numeric_summary() {
        let text = shape_text_readback(4, Some(0), None, Vec::new(), false);
        let numeric = ElementNumericValueReadback {
            current: 7.0,
            minimum: 0.0,
            maximum: 10.0,
            minimum_increment: 1.0,
            text: Some("7 percent".to_string()),
        };

        assert_eq!(
            readback_summary(Some(&text), Some(&numeric)),
            Some("7 percent".to_string())
        );
    }

    #[test]
    fn missing_readback_has_no_summary() {
        assert_eq!(readback_summary(None, None), None);
    }

    #[test]
    fn sensitive_nodes_do_not_read_numeric_values() {
        assert!(!should_read_numeric_value(true, true));
        assert!(should_read_numeric_value(true, false));
        assert!(!should_read_numeric_value(false, false));
    }

    #[test]
    fn numeric_value_prefers_text_summary_then_current_value() {
        let with_text = ElementNumericValueReadback {
            current: 42.0,
            minimum: 0.0,
            maximum: 100.0,
            minimum_increment: 1.0,
            text: Some("42 percent".to_string()),
        };
        let without_text = ElementNumericValueReadback {
            text: None,
            ..with_text.clone()
        };

        assert_eq!(
            readback_summary(None, Some(&with_text)),
            Some("42 percent".to_string())
        );
        assert_eq!(
            readback_summary(None, Some(&without_text)),
            Some("42".to_string())
        );
    }

    #[test]
    fn text_summary_takes_precedence_over_numeric_value() {
        let text = shape_text_readback(5, None, Some("hello".to_string()), Vec::new(), false);
        let numeric = ElementNumericValueReadback {
            current: 10.0,
            minimum: 0.0,
            maximum: 20.0,
            minimum_increment: 1.0,
            text: Some("10".to_string()),
        };

        assert_eq!(
            readback_summary(Some(&text), Some(&numeric)),
            Some("hello".to_string())
        );
    }

    #[test]
    fn password_like_nodes_are_sensitive() {
        assert!(is_sensitive_text_node("password text", None, &[]));
        assert!(is_sensitive_text_node("text", Some("API token"), &[]));
        assert!(is_sensitive_text_node(
            "text",
            None,
            &["protected".to_string()]
        ));
        assert!(!is_sensitive_text_node(
            "text",
            None,
            &["sensitive".to_string()]
        ));
        assert!(!is_sensitive_text_node("text", Some("Search"), &[]));
    }
}
