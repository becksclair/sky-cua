use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};

pub async fn session_bus() -> Result<zbus::Connection, BackendError> {
    zbus::Connection::session().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::PortalUnavailable,
            format!("failed to connect to the session bus for portal probing: {error}"),
        )
    })
}

pub async fn portal_u32_property(
    interface: &str,
    property: &str,
) -> Result<u32, sky_cua_platform::diagnostics::BackendError> {
    let connection = session_bus().await?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        interface,
    )
    .await
    .map_err(|error| {
        sky_cua_platform::diagnostics::BackendError::new(
            BackendErrorCode::PortalUnavailable,
            format!("failed to create portal proxy for {interface}: {error}"),
        )
    })?;

    proxy.get_property::<u32>(property).await.map_err(|error| {
        sky_cua_platform::diagnostics::BackendError::new(
            BackendErrorCode::PortalCapabilityMissing,
            format!("failed to read {interface}.{property}: {error}"),
        )
    })
}
