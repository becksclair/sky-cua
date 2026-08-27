use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use zbus::Proxy;

pub const SCREENSAVER_DEST: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH: &str = "/org/freedesktop/ScreenSaver";
const SCREENSAVER_IFACE: &str = "org.freedesktop.ScreenSaver";

#[derive(Debug)]
pub struct LockInhibitor {
    pub connection: zbus::Connection,
    pub cookie: u32,
}

pub async fn session_connection() -> Result<zbus::Connection, BackendError> {
    zbus::Connection::session().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::ServiceUnavailable,
            format!("failed to connect to session bus for ScreenSaver inhibition: {error}"),
        )
    })
}

pub async fn name_has_owner(connection: &zbus::Connection) -> Result<bool, BackendError> {
    let proxy = Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .map_err(|error| {
        BackendError::new(
            BackendErrorCode::ServiceUnavailable,
            format!("failed to create session DBus proxy: {error}"),
        )
    })?;
    proxy
        .call("NameHasOwner", &(SCREENSAVER_DEST,))
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ServiceUnavailable,
                format!("failed to check {SCREENSAVER_DEST} owner: {error}"),
            )
        })
}

pub async fn inhibit_lock() -> Result<LockInhibitor, BackendError> {
    let connection = session_connection().await?;
    let cookie: u32 = screensaver_proxy(&connection)
        .await?
        .call("Inhibit", &("sky-cua", "automation session active"))
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ServiceUnavailable,
                format!("failed to acquire ScreenSaver inhibitor: {error}"),
            )
        })?;
    Ok(LockInhibitor { connection, cookie })
}

pub async fn uninhibit(inhibitor: &LockInhibitor) -> Result<(), BackendError> {
    let _: () = screensaver_proxy(&inhibitor.connection)
        .await?
        .call("UnInhibit", &(inhibitor.cookie,))
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ServiceUnavailable,
                format!(
                    "failed to release ScreenSaver inhibitor cookie {}: {error}",
                    inhibitor.cookie
                ),
            )
        })?;
    Ok(())
}

async fn screensaver_proxy(connection: &zbus::Connection) -> Result<Proxy<'_>, BackendError> {
    Proxy::new(
        connection,
        SCREENSAVER_DEST,
        SCREENSAVER_PATH,
        SCREENSAVER_IFACE,
    )
    .await
    .map_err(|error| {
        BackendError::new(
            BackendErrorCode::ServiceUnavailable,
            format!("failed to create ScreenSaver proxy: {error}"),
        )
    })
}
