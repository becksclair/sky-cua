use sky_cua_platform::diagnostics::BackendError;

use crate::portal::session::portal_u32_property;

pub async fn version() -> Result<u32, BackendError> {
    portal_u32_property("org.freedesktop.portal.ScreenCast", "version").await
}

pub async fn available_source_types() -> Result<u32, BackendError> {
    portal_u32_property("org.freedesktop.portal.ScreenCast", "AvailableSourceTypes").await
}

pub async fn available_cursor_modes() -> Result<u32, BackendError> {
    portal_u32_property("org.freedesktop.portal.ScreenCast", "AvailableCursorModes").await
}
