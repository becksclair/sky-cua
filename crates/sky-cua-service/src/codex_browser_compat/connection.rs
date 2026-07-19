use std::{collections::HashSet, sync::Arc};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::UnixStream,
    sync::{Mutex, mpsc},
};

use super::{
    CodexBackendReply, CodexBrowserBackend, CodexCallerLifecycle, CodexConnectionContext,
    GENERATION_CHANGED_CODE, GENERATION_CHANGED_MESSAGE, NEXT_CONNECTION_ID,
    PROTOCOL_MISMATCH_CODE, PROTOCOL_MISMATCH_MESSAGE, fresh_id, normalize_request,
    numeric_id_for_error, numeric_request_id, owner_error_value, read_frame, reply_frame,
    write_frame,
};

pub(crate) async fn serve_connection(
    stream: UnixStream,
    backend: Arc<dyn CodexBrowserBackend>,
) -> Result<()> {
    let credentials = stream
        .peer_cred()
        .context("read Codex browser peer credentials")?;
    let peer_uid = credentials.uid();
    let owner_uid = unsafe { libc::geteuid() };
    if peer_uid != owner_uid {
        anyhow::bail!("rejected Codex browser peer uid {peer_uid}; service uid is {owner_uid}");
    }
    let codex_app_build_flavor = match credentials.pid() {
        Some(pid) => peer_codex_app_build_flavor(pid)?,
        None => None,
    };
    serve_stream_with_flavor(stream, peer_uid, codex_app_build_flavor, backend).await
}

#[cfg(test)]
pub(super) async fn serve_stream<S>(
    stream: S,
    peer_uid: u32,
    backend: Arc<dyn CodexBrowserBackend>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    serve_stream_with_flavor(stream, peer_uid, None, backend).await
}

async fn serve_stream_with_flavor<S>(
    stream: S,
    peer_uid: u32,
    codex_app_build_flavor: Option<String>,
    backend: Arc<dyn CodexBrowserBackend>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let generation = backend.daemon_generation();
    let connection = CodexConnectionContext {
        connection_id: fresh_id("codex-connection", &NEXT_CONNECTION_ID),
        provenance: "codex_desktop",
        peer_uid,
        codex_app_build_flavor,
        daemon_generation: generation.clone(),
    };
    let connection_id = connection.connection_id.clone();
    let (reader, mut writer) = tokio::io::split(stream);
    let (outbound, mut outbound_rx) = mpsc::unbounded_channel::<Value>();
    if let Err(reply) = backend
        .connection_opened(connection.clone(), outbound.clone())
        .await
    {
        write_frame(&mut writer, &reply_frame(0, reply)).await?;
        return Ok(());
    }

    let writer_task = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            write_frame(&mut writer, &message).await?;
        }
        Ok::<(), std::io::Error>(())
    });
    let outstanding = Arc::new(Mutex::new(HashSet::<String>::new()));
    let mut reader = reader;
    loop {
        let message = match read_frame(&mut reader).await {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!("Codex browser compatibility frame error: {error}");
                break;
            }
        };

        if message.get("method").and_then(Value::as_str).is_some() {
            let Some(upstream_id) = numeric_request_id(&message) else {
                if message.get("id").is_none() {
                    backend.client_message(&connection_id, message).await;
                } else {
                    let _ = outbound.send(reply_frame(
                        numeric_id_for_error(&message),
                        CodexBackendReply::Error(owner_error_value(
                            PROTOCOL_MISMATCH_CODE,
                            PROTOCOL_MISMATCH_MESSAGE,
                        )),
                    ));
                }
                continue;
            };
            let request = match normalize_request(message, upstream_id, &connection) {
                Ok(request) => request,
                Err(()) => {
                    let _ = outbound.send(reply_frame(
                        upstream_id,
                        CodexBackendReply::Error(owner_error_value(
                            PROTOCOL_MISMATCH_CODE,
                            PROTOCOL_MISMATCH_MESSAGE,
                        )),
                    ));
                    continue;
                }
            };
            let operation_id = request.operation_id.clone();
            if request.method == "ping" {
                let _ = outbound.send(reply_frame(
                    upstream_id,
                    CodexBackendReply::Result(Value::String("pong".to_owned())),
                ));
                continue;
            }
            outstanding.lock().await.insert(operation_id.clone());
            let backend = backend.clone();
            let outbound = outbound.clone();
            let outstanding = outstanding.clone();
            let accepted_generation = generation.clone();
            tokio::spawn(async move {
                let reply = if backend.daemon_generation() != accepted_generation {
                    CodexBackendReply::Error(owner_error_value(
                        GENERATION_CHANGED_CODE,
                        GENERATION_CHANGED_MESSAGE,
                    ))
                } else {
                    match request.method.as_str() {
                        "finalizeTabs" => {
                            backend
                                .caller_lifecycle(CodexCallerLifecycle::FinalizeTabs, request)
                                .await
                        }
                        "turnEnded" => {
                            backend
                                .caller_lifecycle(CodexCallerLifecycle::TurnEnded, request)
                                .await
                        }
                        _ => backend.request(request).await,
                    }
                };
                let reply = if backend.daemon_generation() != accepted_generation {
                    CodexBackendReply::Error(owner_error_value(
                        GENERATION_CHANGED_CODE,
                        GENERATION_CHANGED_MESSAGE,
                    ))
                } else {
                    reply
                };
                outstanding.lock().await.remove(&operation_id);
                let _ = outbound.send(reply_frame(upstream_id, reply));
            });
        } else {
            backend.client_message(&connection_id, message).await;
        }
    }

    let operation_ids = outstanding.lock().await.drain().collect::<Vec<_>>();
    for operation_id in operation_ids {
        backend
            .cancel_or_detach(&connection_id, &operation_id)
            .await;
    }
    backend.connection_closed(&connection_id).await;
    drop(outbound);
    writer_task.abort();
    let _ = writer_task.await;
    Ok(())
}

#[cfg(target_os = "linux")]
fn peer_codex_app_build_flavor(pid: i32) -> Result<Option<String>> {
    if pid <= 0 {
        return Ok(None);
    }
    let environment = std::fs::read(format!("/proc/{pid}/environ"))
        .with_context(|| format!("read Codex browser peer {pid} environment"))?;
    codex_app_build_flavor_from_environ(&environment)
}

#[cfg(not(target_os = "linux"))]
fn peer_codex_app_build_flavor(_pid: i32) -> Result<Option<String>> {
    Ok(None)
}

fn codex_app_build_flavor_from_environ(environment: &[u8]) -> Result<Option<String>> {
    const KEY: &[u8] = b"BROWSER_USE_CODEX_APP_BUILD_FLAVOR=";
    let Some(value) = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(KEY))
    else {
        return Ok(None);
    };
    if value.is_empty() || value.len() > 256 {
        anyhow::bail!("Codex browser build flavor must contain 1-256 bytes");
    }
    Ok(Some(
        std::str::from_utf8(value)
            .context("Codex browser build flavor must be UTF-8")?
            .to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::codex_app_build_flavor_from_environ;

    #[test]
    fn extracts_bounded_utf8_codex_build_flavor() {
        assert_eq!(
            codex_app_build_flavor_from_environ(
                b"A=1\0BROWSER_USE_CODEX_APP_BUILD_FLAVOR=production-linux\0C=3\0"
            )
            .unwrap()
            .as_deref(),
            Some("production-linux")
        );
        assert!(
            codex_app_build_flavor_from_environ(b"BROWSER_USE_CODEX_APP_BUILD_FLAVOR=\0").is_err()
        );
        assert_eq!(codex_app_build_flavor_from_environ(b"A=1\0").unwrap(), None);
    }
}
