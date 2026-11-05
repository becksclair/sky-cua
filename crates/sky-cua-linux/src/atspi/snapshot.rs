use atspi_connection::AccessibilityConnection;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::DiagnosticEntry;

use crate::apps::discovery::{AccessibleTopLevel, DiscoveredApp};
use crate::atspi::wake::walk_with_activation;

pub async fn snapshot_for_app(
    connection: &AccessibilityConnection,
    app: &DiscoveredApp,
) -> Result<
    (
        Vec<sky_cua_platform::model::ElementNode>,
        Vec<DiagnosticEntry>,
    ),
    BackendError,
> {
    let (elements, mut diagnostics) = walk_with_activation(connection, &app.object_ref, 256).await;
    if elements.is_empty() {
        diagnostics.push(DiagnosticEntry {
            code: BackendErrorCode::AccessibilityCoverageLimited
                .as_str()
                .to_string(),
            message: "Focused application exposed no meaningful accessible elements".to_string(),
            details: None,
        });
    }
    Ok((elements, diagnostics))
}

pub async fn snapshot_for_top_level(
    connection: &AccessibilityConnection,
    top_level: &AccessibleTopLevel,
) -> Result<
    (
        Vec<sky_cua_platform::model::ElementNode>,
        Vec<DiagnosticEntry>,
    ),
    BackendError,
> {
    let (elements, mut diagnostics) =
        walk_with_activation(connection, &top_level.object_ref, 256).await;
    if elements.is_empty() {
        diagnostics.push(DiagnosticEntry {
            code: BackendErrorCode::AccessibilityCoverageLimited
                .as_str()
                .to_string(),
            message: "Selected window exposed no meaningful accessible elements".to_string(),
            details: None,
        });
    }
    Ok((elements, diagnostics))
}
