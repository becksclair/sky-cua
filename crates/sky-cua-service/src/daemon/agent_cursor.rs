use super::*;

pub(super) enum AgentCursorResponseKind {
    Status,
    Set,
    Hide,
    Show,
}

pub(super) fn agent_cursor_status_response(
    status: AgentCursorStatus,
    kind: AgentCursorResponseKind,
) -> ServiceResponse {
    match kind {
        AgentCursorResponseKind::Status => ServiceResponse::AgentCursorStatus {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Set => ServiceResponse::SetAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Hide => ServiceResponse::HideAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
        AgentCursorResponseKind::Show => ServiceResponse::ShowAgentCursor {
            capabilities: status.capabilities,
            state: status.state,
            diagnostics: status.diagnostics,
        },
    }
}
