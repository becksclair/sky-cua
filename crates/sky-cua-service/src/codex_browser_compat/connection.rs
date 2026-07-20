use std::{collections::HashSet, sync::Arc};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::UnixStream,
    sync::Mutex,
};

use super::{
    BACKPRESSURE_CODE, BACKPRESSURE_MESSAGE, CodexBackendReply, CodexBrowserBackend,
    CodexCallerLifecycle, CodexConnectionContext, GENERATION_CHANGED_CODE,
    GENERATION_CHANGED_MESSAGE, MAX_RETAINED_REQUESTS_PER_CONNECTION, NEXT_CONNECTION_ID,
    PROTOCOL_MISMATCH_CODE, PROTOCOL_MISMATCH_MESSAGE, fresh_id, normalize_request,
    numeric_id_for_error, numeric_request_id, owner_error_value, read_frame, reply_frame,
    write_frame,
};

const OUTBOUND_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

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
        ingress: "raw_native_pipe",
        peer_uid,
        codex_app_build_flavor,
        daemon_generation: generation.clone(),
    };
    let connection_id = connection.connection_id.clone();
    let (reader, mut writer) = tokio::io::split(stream);
    let (outbound, mut outbound_rx) = super::CodexOutbound::channel();
    if let Err(reply) = backend
        .connection_opened(connection.clone(), outbound.clone())
        .await
    {
        write_frame(&mut writer, &reply_frame(0, reply)).await?;
        return Ok(());
    }

    let writer_outbound = outbound.clone();
    let writer_task = tokio::spawn(async move {
        while let Some(message) = outbound_rx.recv().await {
            match write_outbound_frame(&mut writer, &message, OUTBOUND_WRITE_TIMEOUT).await {
                Ok(()) => {}
                Err(error) => {
                    writer_outbound.mark_stalled();
                    return Err(error);
                }
            }
        }
        Ok::<(), std::io::Error>(())
    });
    let outstanding = Arc::new(Mutex::new(HashSet::<String>::new()));
    let mut reader = reader;
    loop {
        let message = match tokio::select! {
            biased;
            _ = outbound.wait_stalled() => break,
            frame = read_frame(&mut reader) => frame,
        } {
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
                    let _ = outbound.try_send(reply_frame(
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
                    let _ = outbound.try_send(reply_frame(
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
                let _ = outbound.try_send(reply_frame(
                    upstream_id,
                    CodexBackendReply::Result(Value::String("pong".to_owned())),
                ));
                continue;
            }
            {
                let mut outstanding = outstanding.lock().await;
                if outstanding.len() >= MAX_RETAINED_REQUESTS_PER_CONNECTION {
                    drop(outstanding);
                    let _ = outbound.try_send(reply_frame(
                        upstream_id,
                        CodexBackendReply::Error(owner_error_value(
                            BACKPRESSURE_CODE,
                            BACKPRESSURE_MESSAGE,
                        )),
                    ));
                    continue;
                }
                outstanding.insert(operation_id.clone());
            }
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
                let _ = outbound.try_send(reply_frame(upstream_id, reply));
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

async fn write_outbound_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    message: &Value,
    timeout: std::time::Duration,
) -> std::io::Result<()> {
    tokio::time::timeout(timeout, write_frame(writer, message))
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Codex browser compatibility peer stopped reading",
            )
        })?
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
    use super::{codex_app_build_flavor_from_environ, write_outbound_frame};
    use serde_json::json;
    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    };
    use tokio::io::AsyncWrite;

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

    #[tokio::test]
    async fn stalled_outbound_write_has_a_deadline() {
        struct StalledWriter;
        impl AsyncWrite for StalledWriter {
            fn poll_write(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                _buffer: &[u8],
            ) -> Poll<io::Result<usize>> {
                Poll::Pending
            }
            fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Poll::Pending
            }
            fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }

        let error = write_outbound_frame(
            &mut StalledWriter,
            &json!({"jsonrpc":"2.0","method":"event"}),
            Duration::from_millis(10),
        )
        .await
        .expect_err("stalled writer must time out");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }
}
