use std::time::Duration;

use anyhow::Result;
use sky_cua_platform::{SERVICE_SOCKET_PATH_ENV, model::ServiceRequest, service_socket_path};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{info, warn};

use crate::daemon::ServiceDaemon;

const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
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
    let mut daemon = ServiceDaemon::new(socket_path.clone())?;
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
        }
    }

    let _ = tokio::fs::remove_file(&socket_path).await;
    Ok(())
}

fn set_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }

    Ok(())
}

async fn handle_connection(stream: UnixStream, daemon: &mut ServiceDaemon) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
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
