use atspi::AccessibilityConnection;
use atspi::CoordType;
use atspi::proxy::accessible::ObjectRefExt;
use atspi::proxy::proxy_ext::ProxyExt;
use sky_cua_platform::model::{CoordinateSpace, ElementNode, RectF};

use crate::apps::discovery::DiscoveredApp;

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
            }
            if proxies.component().await.is_ok() && !actions.iter().any(|name| name == "focus") {
                actions.push("focus".to_string());
            }
            if (proxies.editable_text().await.is_ok() || proxies.value().await.is_ok())
                && !actions.iter().any(|name| name == "set_value")
            {
                actions.push("set_value".to_string());
            }
            actions
        } else {
            Vec::new()
        };

        let keep = depth == 0
            || name.is_some()
            || description.is_some()
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
                value: None,
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
