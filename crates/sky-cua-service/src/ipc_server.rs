use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use sky_cua_platform::model::ServiceRequest;
#[cfg(windows)]
use sky_cua_platform::service_tcp_addr;
#[cfg(unix)]
use sky_cua_platform::{SERVICE_SOCKET_PATH_ENV, service_socket_path};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::{TcpListener, TcpStream};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::daemon::ServiceDaemon;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

#[cfg(unix)]
pub async fn run_service() -> Result<()> {
    let socket_path = service_socket_path();
    let socket_path_overridden = std::env::var_os(SERVICE_SOCKET_PATH_ENV).is_some();
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
        if !socket_path_overridden {
            set_owner_only_permissions(parent)?;
        }
    }
    if socket_path.exists() {
        let _ = tokio::fs::remove_file(&socket_path).await;
    }

    let listener = UnixListener::bind(&socket_path)?;
    let daemon = Arc::new(tokio::sync::Mutex::new(
        ServiceDaemon::new(socket_path.clone()).await?,
    ));
    let active_connections = Arc::new(AtomicUsize::new(0));
    info!("sky-cua-service listening on {}", socket_path.display());

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        let daemon = daemon.clone();
                        let active_connections = active_connections.clone();
                        active_connections.fetch_add(1, Ordering::SeqCst);
                        tokio::spawn(async move {
                            if let Err(error) = handle_connection(stream, daemon.clone()).await {
                                warn!("sky-cua IPC connection error: {error}");
                            }
                            if active_connections.fetch_sub(1, Ordering::SeqCst) == 1 {
                                hide_agent_cursor_if_idle(daemon, active_connections).await;
                            }
                        });
                    }
                    Err(error) => {
                        warn!("sky-cua IPC accept error: {error}");
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                if service_idle_timed_out(&daemon, &active_connections).await {
                    info!("sky-cua-service idle timeout reached; exiting");
                    break;
                }
            }
            _ = shutdown_signal() => {
                info!("sky-cua-service shutdown signal received; exiting");
                break;
            }
        }
    }

    let _ = tokio::fs::remove_file(&socket_path).await;
    Ok(())
}

#[cfg(unix)]
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    match signal(SignalKind::terminate()) {
        Ok(mut signal) => {
            signal.recv().await;
        }
        Err(error) => {
            warn!("failed to install SIGTERM handler: {error}");
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(windows)]
pub async fn run_service() -> Result<()> {
    let bind_addr = service_tcp_addr();
    let listener = TcpListener::bind(&bind_addr).await?;
    let local_addr = listener.local_addr()?.to_string();
    let daemon = Arc::new(tokio::sync::Mutex::new(
        ServiceDaemon::new(local_addr.clone().into()).await?,
    ));
    let active_connections = Arc::new(AtomicUsize::new(0));
    info!("sky-cua-service listening on {}", local_addr);

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        let daemon = daemon.clone();
                        let active_connections = active_connections.clone();
                        active_connections.fetch_add(1, Ordering::SeqCst);
                        tokio::spawn(async move {
                            if let Err(error) = handle_connection(stream, daemon.clone()).await {
                                warn!("sky-cua IPC connection error: {error}");
                            }
                            if active_connections.fetch_sub(1, Ordering::SeqCst) == 1 {
                                hide_agent_cursor_if_idle(daemon, active_connections).await;
                            }
                        });
                    }
                    Err(error) => {
                        warn!("sky-cua IPC accept error: {error}");
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                if service_idle_timed_out(&daemon, &active_connections).await {
                    info!("sky-cua-service idle timeout reached; exiting");
                    break;
                }
            }
        }
    }

    Ok(())
}

async fn hide_agent_cursor_if_idle(
    daemon: Arc<tokio::sync::Mutex<ServiceDaemon>>,
    active_connections: Arc<AtomicUsize>,
) {
    if active_connections.load(Ordering::SeqCst) != 0 {
        return;
    }
    let mut daemon = daemon.lock().await;
    if active_connections.load(Ordering::SeqCst) != 0 {
        return;
    }
    daemon.hide_agent_cursor_after_last_client();
}

async fn service_idle_timed_out(
    daemon: &tokio::sync::Mutex<ServiceDaemon>,
    active_connections: &AtomicUsize,
) -> bool {
    active_connections.load(Ordering::SeqCst) == 0
        && daemon.lock().await.idle_for().await >= IDLE_TIMEOUT
}

#[cfg(test)]
fn idle_timed_out(idle_for: Duration, active_connections: usize) -> bool {
    active_connections == 0 && idle_for >= IDLE_TIMEOUT
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(unix)]
async fn handle_connection(
    stream: UnixStream,
    daemon: Arc<tokio::sync::Mutex<ServiceDaemon>>,
) -> Result<()> {
    handle_stream(stream, daemon).await
}

#[cfg(windows)]
async fn handle_connection(
    stream: TcpStream,
    daemon: Arc<tokio::sync::Mutex<ServiceDaemon>>,
) -> Result<()> {
    handle_stream(stream, daemon).await
}

async fn handle_stream<S>(stream: S, daemon: Arc<tokio::sync::Mutex<ServiceDaemon>>) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            return Ok(());
        }
        let request: ServiceRequest = serde_json::from_str(line.trim_end()).map_err(|error| {
            anyhow::anyhow!("failed to parse sky-cua IPC request as JSON: {error}")
        })?;
        let response = daemon.lock().await.handle(request).await;
        let encoded = serde_json::to_vec(&response)?;
        writer.write_all(&encoded).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_timeout_requires_no_active_connections() {
        assert!(!idle_timed_out(IDLE_TIMEOUT + Duration::from_secs(1), 1));
        assert!(idle_timed_out(IDLE_TIMEOUT, 0));
    }

    #[tokio::test]
    async fn cursor_hide_rechecks_active_connections_after_lock() {
        let daemon = Arc::new(tokio::sync::Mutex::new(
            ServiceDaemon::new_for_tests().expect("daemon"),
        ));
        let active_connections = Arc::new(AtomicUsize::new(0));
        let state = sky_cua_platform::model::AgentCursorState {
            visible: true,
            sequence: 0,
            model_point: None,
            native_point: Some(sky_cua_platform::model::AgentCursorPoint {
                x: 10.0,
                y: 20.0,
                coordinate_space: sky_cua_platform::model::CoordinateSpace::DesktopLogical,
                mapping_id: None,
            }),
            snapshot_id: None,
            source_action: Some(sky_cua_platform::model::ActionName::Click),
            updated_at_ms: 0,
        };
        let _ = daemon
            .lock()
            .await
            .handle(ServiceRequest::SetAgentCursor { state })
            .await;
        let lock = daemon.lock().await;

        let hide_task = tokio::spawn(hide_agent_cursor_if_idle(
            daemon.clone(),
            active_connections.clone(),
        ));
        active_connections.store(1, Ordering::SeqCst);
        drop(lock);
        hide_task.await.expect("hide task");

        match daemon
            .lock()
            .await
            .handle(ServiceRequest::AgentCursorStatus)
            .await
        {
            sky_cua_platform::model::ServiceResponse::AgentCursorStatus {
                state: Some(state),
                ..
            } => assert!(state.visible),
            other => panic!("unexpected response: {other:?}"),
        }
    }
}
