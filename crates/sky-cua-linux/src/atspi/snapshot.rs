use atspi::AccessibilityConnection;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::DiagnosticEntry;

use crate::apps::discovery::{AccessibleTopLevel, DiscoveredApp};
use crate::atspi::tree::{flatten_accessible_tree, flatten_accessible_tree_from_ref};

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
    let elements = flatten_accessible_tree(connection, app, 256).await;
    let diagnostics = if elements.is_empty() {
        vec![DiagnosticEntry {
            code: BackendErrorCode::AccessibilityCoverageLimited
                .as_str()
                .to_string(),
            message: "Focused application exposed no meaningful accessible elements".to_string(),
            details: None,
        }]
    } else {
        Vec::new()
    };
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
    let elements = flatten_accessible_tree_from_ref(connection, &top_level.object_ref, 256).await;
    let diagnostics = if elements.is_empty() {
        vec![DiagnosticEntry {
            code: BackendErrorCode::AccessibilityCoverageLimited
                .as_str()
                .to_string(),
            message: "Selected window exposed no meaningful accessible elements".to_string(),
            details: None,
        }]
    } else {
        Vec::new()
    };
    Ok((elements, diagnostics))
}
