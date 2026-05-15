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
    let mut daemon = ServiceDaemon::new(socket_path.clone()).await?;
    info!("sky-cua-service listening on {}", socket_path.display());

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        if let Err(error) = handle_connection(stream, &mut daemon).await {
                            warn!("sky-cua IPC connection error: {error}");
                        }
                    }
                    Err(error) => {
                        warn!("sky-cua IPC accept error: {error}");
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                if daemon.idle_for().await >= IDLE_TIMEOUT {
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
    let mut daemon = ServiceDaemon::new(local_addr.clone().into()).await?;
    info!("sky-cua-service listening on {}", local_addr);

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, _)) => {
                        if let Err(error) = handle_connection(stream, &mut daemon).await {
                            warn!("sky-cua IPC connection error: {error}");
                        }
                    }
                    Err(error) => {
                        warn!("sky-cua IPC accept error: {error}");
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                if daemon.idle_for().await >= IDLE_TIMEOUT {
                    info!("sky-cua-service idle timeout reached; exiting");
                    break;
                }
            }
        }
    }

    Ok(())
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(unix)]
async fn handle_connection(stream: UnixStream, daemon: &mut ServiceDaemon) -> Result<()> {
    handle_stream(stream, daemon).await
}

#[cfg(windows)]
async fn handle_connection(stream: TcpStream, daemon: &mut ServiceDaemon) -> Result<()> {
    handle_stream(stream, daemon).await
}

async fn handle_stream<S>(stream: S, daemon: &mut ServiceDaemon) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let read = reader.read_line(&mut line).await?;
    if read == 0 {
        return Ok(());
    }
    let request: ServiceRequest = serde_json::from_str(line.trim_end())
        .map_err(|error| anyhow::anyhow!("failed to parse sky-cua IPC request as JSON: {error}"))?;
    let response = daemon.handle(request).await;
    let encoded = serde_json::to_vec(&response)?;
    writer.write_all(&encoded).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}
