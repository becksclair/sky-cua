//! Accessibility tree and notification tools, plus the bounded summaries
//! `phone_observe` embeds.
//!
//! Both families are companion-only in v1 (ADB has no accessibility-tree or
//! notification fallback). A missing or unreachable companion returns a
//! structured unavailable response rather than pretending success, and a
//! transport failure drops the session's companion runtime so later routing falls
//! back to ADB. Notification operations require explicit fresh event/action ids.

use sky_cua_platform::model::{
    DiagnosticEntry, PhoneAccessibilityNode, PhoneAccessibilitySummary,
    PhoneAccessibilityTreeRequest, PhoneAccessibilityTreeResponse, PhoneBackendKind,
    PhoneNotificationAction, PhoneNotificationActionRequest, PhoneNotificationDismissRequest,
    PhoneNotificationEvent, PhoneNotificationOpenRequest, PhoneNotificationRedaction,
    PhoneNotificationReplyRequest, PhoneNotificationsRequest, PhoneNotificationsResponse,
    PhoneSessionSelector, RectF,
};

use super::{PhoneManager, no_session_diagnostic, selector_ids};
use crate::phone::protocol::{NotificationOp, NotificationOpParams, NotificationRedactionDto};

impl PhoneManager {
    // ===================================================================
    // Accessibility
    // ===================================================================

    /// `phone_accessibility_tree`: bounded active-window node list from the
    /// companion. ADB has no accessibility-tree fallback, so a missing/unreachable
    /// companion returns a structured unavailable response.
    pub(super) async fn accessibility_tree(
        &mut self,
        request: PhoneAccessibilityTreeRequest,
    ) -> PhoneAccessibilityTreeResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            let (session_id, serial) = selector_ids(&request.session);
            return accessibility_unavailable(
                session_id,
                serial,
                no_session_diagnostic(&request.session),
            );
        };
        let serial = self.serial_of(&session_id);
        let max_nodes = request.node_limit.unwrap_or(200) as u32;

        let Some(entry) = self.sessions.get_mut(&session_id) else {
            return accessibility_unavailable(session_id, serial, companion_required_diagnostic());
        };
        let Some(runtime) = entry.companion.as_mut() else {
            return accessibility_unavailable(session_id, serial, companion_required_diagnostic());
        };
        match runtime.client.accessibility_tree(max_nodes).await {
            Ok(tree) => {
                let nodes = tree
                    .nodes
                    .iter()
                    .enumerate()
                    .map(|(index, node)| PhoneAccessibilityNode {
                        node_index: index,
                        parent_index: None,
                        class_name: node.class.clone(),
                        package_name: tree.package.clone(),
                        text: node.text.clone(),
                        content_description: node.content_desc.clone(),
                        bounds: node.bounds.map(bounds_to_rect),
                        clickable: node.clickable,
                        focusable: node.focusable,
                        enabled: node.enabled,
                        redacted: tree.redacted,
                    })
                    .collect();
                PhoneAccessibilityTreeResponse {
                    session_id,
                    serial,
                    backend: PhoneBackendKind::Companion,
                    package_name: tree.package.clone(),
                    activity: tree.activity.clone(),
                    nodes,
                    truncated: tree.truncated,
                    redacted: tree.redacted,
                    diagnostics: Vec::new(),
                }
            }
            Err(error) => {
                if error.is_fallback() {
                    // Drop the dead runtime, then invalidate the cached companion
                    // capability so the profile stops advertising a reachable
                    // companion (the `entry` borrow ends before the `self.` call).
                    entry.companion = None;
                    self.invalidate_companion(&session_id);
                }
                accessibility_unavailable(
                    session_id,
                    serial,
                    DiagnosticEntry {
                        code: error.code().to_string(),
                        message: format!("companion accessibility_tree failed: {error}"),
                        details: None,
                    },
                )
            }
        }
    }

    /// Bounded accessibility summary for `phone_observe`. Returns `None` when the
    /// companion cannot serve a tree.
    pub(super) async fn accessibility_summary(
        &mut self,
        session_id: &str,
    ) -> Option<PhoneAccessibilitySummary> {
        let entry = self.sessions.get_mut(session_id)?;
        let runtime = entry.companion.as_mut()?;
        let tree = runtime.client.accessibility_tree(60).await.ok()?;
        let headline_texts: Vec<String> = tree
            .nodes
            .iter()
            .filter_map(|node| node.text.clone())
            .filter(|text| !text.trim().is_empty())
            .take(8)
            .collect();
        Some(PhoneAccessibilitySummary {
            package_name: tree.package.clone(),
            activity: tree.activity.clone(),
            node_count: tree.nodes.len() as u32,
            headline_texts,
            truncated: tree.truncated,
            redacted: tree.redacted,
        })
    }

    // ===================================================================
    // Notifications
    // ===================================================================

    /// `phone_notifications`: bounded recent notification events from the
    /// companion. ADB has no notification fallback in v1.
    pub(super) async fn notifications(
        &mut self,
        request: PhoneNotificationsRequest,
    ) -> PhoneNotificationsResponse {
        let Some(session_id) = self.resolve_session_id(&request.session) else {
            let (session_id, serial) = selector_ids(&request.session);
            return notifications_unavailable(
                session_id,
                serial,
                no_session_diagnostic(&request.session),
            );
        };
        let serial = self.serial_of(&session_id);
        let max = request.limit.unwrap_or(20) as u32;

        let Some(entry) = self.sessions.get_mut(&session_id) else {
            return notifications_unavailable(session_id, serial, companion_required_diagnostic());
        };
        let Some(runtime) = entry.companion.as_mut() else {
            return notifications_unavailable(session_id, serial, companion_required_diagnostic());
        };
        match runtime.client.notifications(max).await {
            Ok(result) => PhoneNotificationsResponse {
                session_id,
                serial,
                backend: PhoneBackendKind::Companion,
                listener_enabled: result.listener_enabled,
                events: result.events.iter().map(notification_event).collect(),
                truncated: result.truncated,
                diagnostics: Vec::new(),
            },
            Err(error) => {
                if error.is_fallback() {
                    entry.companion = None;
                    self.invalidate_companion(&session_id);
                }
                notifications_unavailable(
                    session_id,
                    serial,
                    DiagnosticEntry {
                        code: error.code().to_string(),
                        message: format!("companion notifications failed: {error}"),
                        details: None,
                    },
                )
            }
        }
    }

    /// Recent notifications for `phone_observe`. Empty when unavailable.
    pub(super) async fn recent_notifications(
        &mut self,
        session_id: &str,
    ) -> Vec<PhoneNotificationEvent> {
        let Some(entry) = self.sessions.get_mut(session_id) else {
            return Vec::new();
        };
        let Some(runtime) = entry.companion.as_mut() else {
            return Vec::new();
        };
        match runtime.client.notifications(10).await {
            Ok(result) => result.events.iter().map(notification_event).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// `phone_notification_open`: requires an explicit fresh `event_id`.
    pub(super) async fn notification_open(
        &mut self,
        request: PhoneNotificationOpenRequest,
    ) -> PhoneNotificationsResponse {
        self.notification_op(
            &request.session,
            NotificationOpParams {
                event_id: request.event_id,
                op: NotificationOp::Open,
                action_id: None,
                reply_text: None,
            },
        )
        .await
    }

    /// `phone_notification_dismiss`.
    pub(super) async fn notification_dismiss(
        &mut self,
        request: PhoneNotificationDismissRequest,
    ) -> PhoneNotificationsResponse {
        self.notification_op(
            &request.session,
            NotificationOpParams {
                event_id: request.event_id,
                op: NotificationOp::Dismiss,
                action_id: None,
                reply_text: None,
            },
        )
        .await
    }

    /// `phone_notification_action`: invoke an action button by explicit id.
    pub(super) async fn notification_action(
        &mut self,
        request: PhoneNotificationActionRequest,
    ) -> PhoneNotificationsResponse {
        self.notification_op(
            &request.session,
            NotificationOpParams {
                event_id: request.event_id,
                op: NotificationOp::Action,
                action_id: Some(request.action_id),
                reply_text: None,
            },
        )
        .await
    }

    /// `phone_notification_reply`: inline reply by explicit id + text.
    pub(super) async fn notification_reply(
        &mut self,
        request: PhoneNotificationReplyRequest,
    ) -> PhoneNotificationsResponse {
        self.notification_op(
            &request.session,
            NotificationOpParams {
                event_id: request.event_id,
                op: NotificationOp::Reply,
                action_id: Some(request.action_id),
                reply_text: Some(request.text),
            },
        )
        .await
    }

    /// Shared notification-op dispatch: companion-only, returns a refreshed
    /// notification list on success (so the agent sees the result) or a structured
    /// unavailable response otherwise.
    async fn notification_op(
        &mut self,
        selector: &PhoneSessionSelector,
        params: NotificationOpParams,
    ) -> PhoneNotificationsResponse {
        let Some(session_id) = self.resolve_session_id(selector) else {
            let (session_id, serial) = selector_ids(selector);
            return notifications_unavailable(session_id, serial, no_session_diagnostic(selector));
        };
        let serial = self.serial_of(&session_id);

        let Some(entry) = self.sessions.get_mut(&session_id) else {
            return notifications_unavailable(session_id, serial, companion_required_diagnostic());
        };
        let Some(runtime) = entry.companion.as_mut() else {
            return notifications_unavailable(session_id, serial, companion_required_diagnostic());
        };
        match runtime.client.notification_op(params).await {
            Ok(result) if result.ok => {
                // Fetch a fresh list so the agent sees the post-op state.
                let events = match runtime.client.notifications(20).await {
                    Ok(list) => list.events.iter().map(notification_event).collect(),
                    Err(_) => Vec::new(),
                };
                PhoneNotificationsResponse {
                    session_id,
                    serial,
                    backend: PhoneBackendKind::Companion,
                    listener_enabled: true,
                    events,
                    truncated: false,
                    diagnostics: Vec::new(),
                }
            }
            Ok(_) => notifications_unavailable(
                session_id,
                serial,
                DiagnosticEntry {
                    code: "PhoneNotificationOpRejected".to_string(),
                    message: "companion rejected the notification operation".to_string(),
                    details: None,
                },
            ),
            Err(error) => {
                if error.is_fallback() {
                    entry.companion = None;
                    self.invalidate_companion(&session_id);
                }
                notifications_unavailable(
                    session_id,
                    serial,
                    DiagnosticEntry {
                        code: error.code().to_string(),
                        message: format!("companion notification op failed: {error}"),
                        details: None,
                    },
                )
            }
        }
    }
}

fn bounds_to_rect(bounds: [i32; 4]) -> RectF {
    let [left, top, right, bottom] = bounds;
    RectF {
        x: f64::from(left),
        y: f64::from(top),
        width: f64::from(right - left),
        height: f64::from(bottom - top),
        space: sky_cua_platform::model::CoordinateSpace::StreamPixels,
    }
}

/// Map a companion notification DTO to the model notification event.
fn notification_event(
    dto: &crate::phone::protocol::NotificationEventDto,
) -> PhoneNotificationEvent {
    let redaction = match dto.redaction {
        NotificationRedactionDto::None => PhoneNotificationRedaction::None,
        NotificationRedactionDto::Partial => PhoneNotificationRedaction::Partial,
        NotificationRedactionDto::Full => PhoneNotificationRedaction::Full,
    };
    let actions = dto
        .actions
        .iter()
        .map(|action| PhoneNotificationAction {
            action_id: action.action_id.clone(),
            title: action.title.clone(),
            supports_inline_reply: action.is_reply,
        })
        .collect();
    PhoneNotificationEvent {
        event_id: dto.event_id.clone(),
        key: None,
        package_name: dto.package.clone(),
        channel: dto.channel.clone(),
        title: dto.title.clone(),
        body: dto.body.clone(),
        redaction,
        rank: dto.ranking,
        // Conservative defaults for companions predating these wire fields: an
        // ongoing flag is assumed clear, and open/dismiss are assumed allowed so
        // the agent is not blocked from acting on an older companion's events.
        ongoing: dto.ongoing.unwrap_or(false),
        can_open: dto.can_open.unwrap_or(true),
        can_dismiss: dto.can_dismiss.unwrap_or(true),
        actions,
        posted_at_ms: dto.when_ms,
    }
}

fn companion_required_diagnostic() -> DiagnosticEntry {
    DiagnosticEntry {
        code: "PhoneCompanionRequired".to_string(),
        message: "this tool requires a reachable companion; none is active for this session"
            .to_string(),
        details: None,
    }
}

fn accessibility_unavailable(
    session_id: String,
    serial: String,
    diagnostic: DiagnosticEntry,
) -> PhoneAccessibilityTreeResponse {
    PhoneAccessibilityTreeResponse {
        session_id,
        serial,
        backend: PhoneBackendKind::None,
        package_name: None,
        activity: None,
        nodes: Vec::new(),
        truncated: false,
        redacted: false,
        diagnostics: vec![diagnostic],
    }
}

fn notifications_unavailable(
    session_id: String,
    serial: String,
    diagnostic: DiagnosticEntry,
) -> PhoneNotificationsResponse {
    PhoneNotificationsResponse {
        session_id,
        serial,
        backend: PhoneBackendKind::None,
        listener_enabled: false,
        events: Vec::new(),
        truncated: false,
        diagnostics: vec![diagnostic],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone::protocol::NotificationEventDto;

    fn base_dto() -> NotificationEventDto {
        NotificationEventDto {
            event_id: "evt-1".to_string(),
            package: "com.example".to_string(),
            channel: None,
            title: None,
            body: None,
            redaction: NotificationRedactionDto::None,
            ranking: Some(3),
            can_open: None,
            can_dismiss: None,
            ongoing: None,
            when_ms: 1_000,
            actions: Vec::new(),
        }
    }

    #[test]
    fn notification_event_passes_through_explicit_affordances() {
        // A companion that reports the affordances must have them honored
        // verbatim, not overwritten by host-side defaults.
        let mut dto = base_dto();
        dto.can_open = Some(false);
        dto.can_dismiss = Some(false);
        dto.ongoing = Some(true);
        let event = notification_event(&dto);
        assert!(!event.can_open);
        assert!(!event.can_dismiss);
        assert!(event.ongoing);
        // The ranking passthrough must remain intact.
        assert_eq!(event.rank, Some(3));
    }

    #[test]
    fn notification_event_defaults_absent_affordances_conservatively() {
        // Older companions omit these fields; the host fills open/dismiss as
        // permitted and ongoing as clear so behavior is unchanged for them.
        let event = notification_event(&base_dto());
        assert!(event.can_open);
        assert!(event.can_dismiss);
        assert!(!event.ongoing);
    }
}
