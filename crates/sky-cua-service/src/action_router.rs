use sky_cua_platform::backend::DesktopBackend;
use sky_cua_platform::model::{ActionOutcome, ActionRequest};

pub async fn route_action(
    backend: &(impl DesktopBackend + ?Sized),
    request: ActionRequest,
) -> ActionOutcome {
    backend
        .execute_action(request)
        .await
        .unwrap_or_else(|error| {
            let diagnostic = error.diagnostic();
            ActionOutcome {
                success: false,
                message: error.message.clone(),
                code: error.code.to_string(),
                diagnostics: vec![diagnostic],
                agent_cursor: None,
            }
        })
}
