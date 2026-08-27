use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use zbus::Proxy;
use zbus::zvariant::{OwnedFd, OwnedObjectPath};

const LOGIND_DEST: &str = "org.freedesktop.login1";
const LOGIND_MANAGER_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER_IFACE: &str = "org.freedesktop.login1.Manager";
const LOGIND_SESSION_IFACE: &str = "org.freedesktop.login1.Session";

#[derive(Debug, Clone)]
pub struct ResolvedSession {
    pub id: String,
    pub path: OwnedObjectPath,
    pub locked: bool,
}

pub async fn system_connection() -> Result<zbus::Connection, BackendError> {
    zbus::Connection::system().await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::ServiceUnavailable,
            format!("failed to connect to system bus for logind session presence: {error}"),
        )
    })
}

pub async fn resolve_session(
    connection: &zbus::Connection,
) -> Result<ResolvedSession, BackendError> {
    let manager = manager_proxy(connection).await?;
    let path: OwnedObjectPath = match manager.call("GetSession", &("auto",)).await {
        Ok(path) => path,
        Err(auto_error) => {
            if let Some(session_id) = std::env::var("XDG_SESSION_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
            {
                manager
                    .call("GetSession", &(session_id.as_str(),))
                    .await
                    .map_err(|error| {
                        BackendError::new(
                            BackendErrorCode::ServiceUnavailable,
                            format!(
                                "GetSession(auto) failed with {auto_error}; GetSession({session_id}) failed with {error}"
                            ),
                        )
                    })?
            } else {
                let pid = std::process::id();
                manager
                    .call("GetSessionByPID", &(pid,))
                    .await
                    .map_err(|error| {
                        BackendError::new(
                            BackendErrorCode::ServiceUnavailable,
                            format!(
                                "GetSession(auto) failed with {auto_error}; GetSessionByPID({pid}) failed with {error}"
                            ),
                        )
                    })?
            }
        }
    };

    let session = session_proxy(connection, &path).await?;
    let id: String = session.get_property("Id").await.map_err(|error| {
        BackendError::new(
            BackendErrorCode::ServiceUnavailable,
            format!("failed to read logind session Id for {path}: {error}"),
        )
    })?;
    let locked = locked_hint(connection, &path).await?;
    Ok(ResolvedSession { id, path, locked })
}

pub async fn locked_hint(
    connection: &zbus::Connection,
    path: &OwnedObjectPath,
) -> Result<bool, BackendError> {
    session_proxy(connection, path)
        .await?
        .get_property("LockedHint")
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ServiceUnavailable,
                format!("failed to read logind LockedHint for {path}: {error}"),
            )
        })
}

pub async fn unlock_session(
    connection: &zbus::Connection,
    session_id: &str,
) -> Result<(), BackendError> {
    let _: () = manager_proxy(connection)
        .await?
        .call("UnlockSession", &(session_id,))
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ServiceUnavailable,
                format!("failed to call logind UnlockSession({session_id}): {error}"),
            )
        })?;
    Ok(())
}

pub async fn lock_session(
    connection: &zbus::Connection,
    session_id: &str,
) -> Result<(), BackendError> {
    let _: () = manager_proxy(connection)
        .await?
        .call("LockSession", &(session_id,))
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ServiceUnavailable,
                format!("failed to call logind LockSession({session_id}): {error}"),
            )
        })?;
    Ok(())
}

pub async fn inhibit_suspend(connection: &zbus::Connection) -> Result<OwnedFd, BackendError> {
    manager_proxy(connection)
        .await?
        .call(
            "Inhibit",
            &("sleep", "sky-cua", "automation session active", "block"),
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ServiceUnavailable,
                format!("failed to acquire logind sleep inhibitor: {error}"),
            )
        })
}

async fn manager_proxy(connection: &zbus::Connection) -> Result<Proxy<'_>, BackendError> {
    Proxy::new(
        connection,
        LOGIND_DEST,
        LOGIND_MANAGER_PATH,
        LOGIND_MANAGER_IFACE,
    )
    .await
    .map_err(|error| {
        BackendError::new(
            BackendErrorCode::ServiceUnavailable,
            format!("failed to create logind manager proxy: {error}"),
        )
    })
}

async fn session_proxy<'a>(
    connection: &'a zbus::Connection,
    path: &OwnedObjectPath,
) -> Result<Proxy<'a>, BackendError> {
    Proxy::new(connection, LOGIND_DEST, path.clone(), LOGIND_SESSION_IFACE)
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ServiceUnavailable,
                format!("failed to create logind session proxy for {path}: {error}"),
            )
        })
}
