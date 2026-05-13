pub mod actions;
pub mod snapshot;
pub mod tree;

use atspi::AccessibilityConnection;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

pub(crate) fn normalize_action(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
}

pub async fn connect() -> Result<AccessibilityConnection, BackendError> {
    AccessibilityConnection::new().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::AccessibilityUnavailable,
            format!("failed to connect to the AT-SPI accessibility bus: {error}"),
        )
    })
}
