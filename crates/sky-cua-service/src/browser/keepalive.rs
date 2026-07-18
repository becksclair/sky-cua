//! Persistent primary-client keepalive for the browser extension heartbeat.
//!
//! The Codex Chrome extension pings its driving ("primary") native-host client
//! every 30 seconds and, if no truthy reply arrives within 3 seconds, detaches
//! `chrome.debugger` from every tab — a driver-liveness cleanup so a crashed
//! driver does not leave the browser debugged forever (see the extension's
//! `client-heartbeat-alarm`). sky-cua drives the browser through per-operation,
//! *ephemeral* connections (`session_id: "sky-cua-mcp"`) that the host does not
//! route the heartbeat to, so nothing answers the ping and the extension
//! detaches the debugger. That surfaces as intermittent "Detached while handling
//! command" wedges between (and within) browser operations.
//!
//! This module holds one long-lived heartbeat-fallback connection per native
//! host socket. The native host routes extension requests to a real primary
//! client first and to a fallback only when no primary exists, so Codex Browser
//! Use and sky-cua do not evict each other. The supervisor runs for the daemon
//! lifetime and tracks host restarts and concurrently running browser profiles.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::UnixStream;
use tokio::time::Instant as TokioInstant;

use super::protocol::{read_frame, write_frame};
use super::sockets::bridge_socket_paths_newest_first;

const KEEPALIVE_SESSION_ID: &str = "sky-cua-heartbeat-keepalive";
const KEEPALIVE_TURN_ID: &str = "heartbeat";
const KEEPALIVE_INFO_REQUEST_ID: &str = "sky-cua-browser-keepalive-info";

/// Poll interval while no bridge socket exists yet (extension not connected).
const SOCKET_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// A native-host socket is not usable merely because `connect(2)` succeeded.
/// Require one valid inbound protocol frame quickly enough to leave time for
/// the extension's three-second heartbeat deadline before committing to it.
const INITIAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Spawn the heartbeat keepalive task exactly once for the daemon lifetime.
/// Safe to call from any browser-bridge entry point; later calls are no-ops.
/// A no-op under test so the browser unit tests' fake servers are never claimed
/// by a background keepalive connection.
pub(super) fn ensure_spawned() {
    if cfg!(test) {
        return;
    }
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(keepalive_loop());
}

async fn keepalive_loop() {
    let mut tasks = HashMap::new();
    loop {
        tasks.retain(|_, task: &mut tokio::task::JoinHandle<()>| !task.is_finished());
        spawn_missing_keepalives(
            &mut tasks,
            bridge_socket_paths_newest_first(),
            INITIAL_RESPONSE_TIMEOUT,
        );
        tokio::time::sleep(SOCKET_POLL_INTERVAL).await;
    }
}

fn spawn_missing_keepalives(
    tasks: &mut HashMap<PathBuf, tokio::task::JoinHandle<()>>,
    sockets: impl IntoIterator<Item = PathBuf>,
    initial_response_timeout: Duration,
) {
    for socket in sockets {
        if tasks.contains_key(&socket) {
            continue;
        }
        let task_socket = socket.clone();
        let task = tokio::spawn(async move {
            let _ = serve_socket(&task_socket, initial_response_timeout).await;
        });
        tasks.insert(socket, task);
    }
}

#[cfg(test)]
async fn serve_first_available(sockets: &[PathBuf]) -> Option<PathBuf> {
    serve_first_available_with_timeout(sockets, INITIAL_RESPONSE_TIMEOUT).await
}

#[cfg(test)]
async fn serve_first_available_with_timeout(
    sockets: &[PathBuf],
    initial_response_timeout: Duration,
) -> Option<PathBuf> {
    for socket in sockets {
        if serve_socket(socket, initial_response_timeout).await {
            return Some(socket.clone());
        }
    }
    None
}

/// Connect to the bridge socket and answer heartbeat pings until the connection
/// drops, then return so the supervisor can reconnect.
async fn serve_socket(socket: &Path, initial_response_timeout: Duration) -> bool {
    let Ok(stream) = UnixStream::connect(socket).await else {
        tracing::debug!(socket = %socket.display(), "browser keepalive could not connect");
        return false;
    };
    tracing::info!(
        socket = %socket.display(),
        "browser keepalive connected; answering the extension heartbeat as fallback"
    );
    let exit = serve_stream_with_timeout(stream, initial_response_timeout).await;
    // Every exit here is a window in which a 30s heartbeat tick can go
    // unanswered and the extension detaches chrome.debugger from every tab —
    // agent-visible as "Debugger unattached". Keep this at info so incidents
    // are attributable from the daemon log alone.
    tracing::info!(
        socket = %socket.display(),
        exit = exit.reason,
        "browser keepalive disconnected; reconnecting"
    );
    exit.responsive
}

#[cfg(test)]
async fn serve_stream(stream: UnixStream) -> &'static str {
    serve_stream_with_timeout(stream, INITIAL_RESPONSE_TIMEOUT)
        .await
        .reason
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ServeExit {
    reason: &'static str,
    responsive: bool,
}

async fn serve_stream_with_timeout(
    mut stream: UnixStream,
    initial_response_timeout: Duration,
) -> ServeExit {
    // Register as the heartbeat fallback. The explicit role marker keeps this
    // connection from evicting a real Codex Browser Use primary.
    if write_frame(&mut stream, &hello_frame()).await.is_err() {
        return ServeExit {
            reason: "hello-write-failed",
            responsive: false,
        };
    }

    let mut responsive = false;
    let initial_response_deadline = TokioInstant::now() + initial_response_timeout;
    loop {
        let next_frame = if responsive {
            Ok(read_frame(&mut stream).await)
        } else {
            let read_timeout =
                initial_response_deadline.saturating_duration_since(TokioInstant::now());
            if read_timeout.is_zero() {
                return ServeExit {
                    reason: "initial-response-timeout",
                    responsive: false,
                };
            }
            tokio::time::timeout(read_timeout, read_frame(&mut stream)).await
        };
        match next_frame {
            Ok(Ok(Some(message))) => {
                if is_ping_request(&message) {
                    let id = message.get("id").cloned().unwrap_or(Value::Null);
                    if write_frame(&mut stream, &pong_frame(id)).await.is_err() {
                        return ServeExit {
                            reason: "pong-write-failed",
                            responsive,
                        };
                    }
                    tracing::debug!("browser keepalive answered heartbeat ping");
                }
                // JSON decoding alone is not a responsiveness proof: an orphan
                // or wrong-protocol listener may emit arbitrary JSON forever.
                // Only attributable registration or heartbeat frames may mask
                // older socket candidates.
                if is_keepalive_protocol_frame(&message) {
                    responsive = true;
                }
                // Every other routed frame (the getInfo reply, CDP-event and
                // download notifications) needs no response from the keepalive.
            }
            // EOF (host restart), read/parse error, or a failed initial
            // handshake: drop and let the supervisor reconnect.
            Ok(Ok(None)) => {
                return ServeExit {
                    reason: "eof",
                    responsive,
                };
            }
            Ok(Err(_)) => {
                return ServeExit {
                    reason: "read-error",
                    responsive,
                };
            }
            Err(_) => {
                return ServeExit {
                    reason: "initial-response-timeout",
                    responsive: false,
                };
            }
        }
    }
}

fn hello_frame() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": KEEPALIVE_INFO_REQUEST_ID,
        "method": "getInfo",
        "params": {
            "session_id": KEEPALIVE_SESSION_ID,
            "turn_id": KEEPALIVE_TURN_ID,
            "_sky_cua_client_role": "heartbeat"
        },
    })
}

fn pong_frame(id: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": "pong" })
}

fn is_ping_request(message: &Value) -> bool {
    message.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && message.get("method").and_then(Value::as_str) == Some("ping")
        && message.get("id").is_some()
}

fn is_keepalive_info_response(message: &Value) -> bool {
    message.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && message.get("id").and_then(Value::as_str) == Some(KEEPALIVE_INFO_REQUEST_ID)
        && (message.get("result").is_some() || message.get("error").is_some())
}

fn is_keepalive_protocol_frame(message: &Value) -> bool {
    is_ping_request(message) || is_keepalive_info_response(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keepalive_registers_as_heartbeat_fallback() {
        let hello = hello_frame();
        assert_eq!(hello["method"], "getInfo");
        assert_eq!(hello["params"]["session_id"], KEEPALIVE_SESSION_ID);
        assert_eq!(hello["params"]["_sky_cua_client_role"], "heartbeat");
    }

    #[test]
    fn only_ping_requests_are_answered() {
        assert!(is_ping_request(
            &json!({ "jsonrpc": "2.0", "method": "ping", "id": "chrome-1-1" })
        ));
        // A ping notification (no id) is not a request needing a response.
        assert!(!is_ping_request(
            &json!({ "jsonrpc": "2.0", "method": "ping" })
        ));
        // CDP-event notifications and other routed frames are ignored.
        assert!(!is_ping_request(
            &json!({ "jsonrpc": "2.0", "method": "onCDPEvent", "params": {} })
        ));
    }

    #[test]
    fn only_attributable_protocol_frames_prove_responsiveness() {
        assert!(!is_keepalive_protocol_frame(&json!({})));
        assert!(!is_keepalive_protocol_frame(&Value::Null));
        assert!(!is_keepalive_protocol_frame(&json!({
            "jsonrpc": "2.0",
            "id": "some-other-request",
            "result": {}
        })));
        assert!(is_keepalive_protocol_frame(&json!({
            "jsonrpc": "2.0",
            "id": KEEPALIVE_INFO_REQUEST_ID,
            "result": { "type": "extension" }
        })));
        assert!(is_keepalive_protocol_frame(&json!({
            "jsonrpc": "2.0",
            "method": "ping",
            "id": "chrome-heartbeat"
        })));
    }

    #[tokio::test]
    async fn answers_heartbeat_ping_with_pong_keyed_on_request_id() {
        let (server, client) = UnixStream::pair().unwrap();
        let keepalive = tokio::spawn(serve_stream(client));

        let mut server = server;
        // The keepalive first sends its primary-registering hello.
        let hello = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(hello["method"], "getInfo");
        assert_eq!(hello["params"]["session_id"], KEEPALIVE_SESSION_ID);

        // A routed heartbeat ping must come back as a pong on the same id.
        write_frame(
            &mut server,
            &json!({ "jsonrpc": "2.0", "method": "ping", "id": "chrome-424576-204" }),
        )
        .await
        .unwrap();
        let reply = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(reply["id"], "chrome-424576-204");
        assert_eq!(reply["result"], "pong");

        // A CDP-event notification is ignored (no reply), and closing the
        // connection lets the keepalive task unwind.
        write_frame(
            &mut server,
            &json!({ "jsonrpc": "2.0", "method": "onCDPEvent", "params": {} }),
        )
        .await
        .unwrap();
        drop(server);
        keepalive.await.unwrap();
    }

    #[tokio::test]
    async fn registered_fallback_stays_connected_while_primary_handles_heartbeats() {
        let (mut server, client) = UnixStream::pair().unwrap();
        let keepalive = tokio::spawn(serve_stream_with_timeout(client, Duration::from_millis(20)));

        let hello = read_frame(&mut server).await.unwrap().unwrap();
        assert_eq!(hello["id"], KEEPALIVE_INFO_REQUEST_ID);
        write_frame(
            &mut server,
            &json!({
                "jsonrpc": "2.0",
                "id": KEEPALIVE_INFO_REQUEST_ID,
                "result": { "type": "extension" }
            }),
        )
        .await
        .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!keepalive.is_finished());
        drop(server);
        assert_eq!(
            keepalive.await.unwrap(),
            ServeExit {
                reason: "eof",
                responsive: true,
            }
        );
    }

    #[tokio::test]
    async fn supervisor_keeps_every_responsive_host_socket_alive() {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-keepalive-multi-host-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let first_path = dir.join("extension-first.sock");
        let second_path = dir.join("extension-second.sock");
        let first_listener = tokio::net::UnixListener::bind(&first_path).unwrap();
        let second_listener = tokio::net::UnixListener::bind(&second_path).unwrap();
        let (connected_tx, mut connected_rx) = tokio::sync::mpsc::channel(2);

        let first_tx = connected_tx.clone();
        let first_server = tokio::spawn(async move {
            exercise_responsive_host(first_listener, "first", first_tx).await;
        });
        let second_server = tokio::spawn(async move {
            exercise_responsive_host(second_listener, "second", connected_tx).await;
        });

        let mut tasks = HashMap::new();
        spawn_missing_keepalives(
            &mut tasks,
            vec![first_path, second_path],
            Duration::from_millis(100),
        );
        let mut connected = vec![
            tokio::time::timeout(Duration::from_secs(1), connected_rx.recv())
                .await
                .unwrap()
                .unwrap(),
            tokio::time::timeout(Duration::from_secs(1), connected_rx.recv())
                .await
                .unwrap()
                .unwrap(),
        ];
        connected.sort_unstable();
        assert_eq!(connected, ["first", "second"]);

        first_server.await.unwrap();
        second_server.await.unwrap();
        for task in tasks.into_values() {
            task.await.unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    async fn exercise_responsive_host(
        listener: tokio::net::UnixListener,
        name: &'static str,
        connected: tokio::sync::mpsc::Sender<&'static str>,
    ) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let hello = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(hello["params"]["_sky_cua_client_role"], "heartbeat");
        write_frame(
            &mut stream,
            &json!({
                "jsonrpc": "2.0",
                "id": KEEPALIVE_INFO_REQUEST_ID,
                "result": { "type": "extension" }
            }),
        )
        .await
        .unwrap();
        write_frame(
            &mut stream,
            &json!({ "jsonrpc": "2.0", "method": "ping", "id": name }),
        )
        .await
        .unwrap();
        let pong = read_frame(&mut stream).await.unwrap().unwrap();
        assert_eq!(pong["id"], name);
        assert_eq!(pong["result"], "pong");
        connected.send(name).await.unwrap();
    }

    #[tokio::test]
    async fn stale_newest_socket_falls_back_to_older_responsive_socket() {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-keepalive-failover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let stale_path = dir.join("extension-newest-stale.sock");
        let stale_listener = tokio::net::UnixListener::bind(&stale_path).unwrap();
        drop(stale_listener);
        let live_path = dir.join("extension-older-live.sock");
        let live_listener = tokio::net::UnixListener::bind(&live_path).unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = live_listener.accept().await.unwrap();
            let hello = read_frame(&mut stream).await.unwrap().unwrap();
            assert_eq!(hello["params"]["_sky_cua_client_role"], "heartbeat");
            write_frame(
                &mut stream,
                &json!({
                    "jsonrpc": "2.0",
                    "id": KEEPALIVE_INFO_REQUEST_ID,
                    "result": { "type": "extension" }
                }),
            )
            .await
            .unwrap();
        });
        let selected = serve_first_available(&[stale_path, live_path.clone()]).await;
        server.await.unwrap();

        assert_eq!(selected.as_deref(), Some(live_path.as_path()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn accepting_but_silent_newest_socket_falls_back_to_older_responsive_socket() {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-keepalive-silent-failover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let silent_path = dir.join("extension-newest-silent.sock");
        let silent_listener = tokio::net::UnixListener::bind(&silent_path).unwrap();
        let live_path = dir.join("extension-older-live.sock");
        let live_listener = tokio::net::UnixListener::bind(&live_path).unwrap();

        let silent_server = tokio::spawn(async move {
            let (mut stream, _) = silent_listener.accept().await.unwrap();
            let hello = read_frame(&mut stream).await.unwrap().unwrap();
            assert_eq!(hello["params"]["_sky_cua_client_role"], "heartbeat");
            // Hold the accepted connection without answering until the client
            // rejects it and advances to the older candidate.
            assert!(read_frame(&mut stream).await.unwrap().is_none());
        });
        let live_server = tokio::spawn(async move {
            let (mut stream, _) = live_listener.accept().await.unwrap();
            let hello = read_frame(&mut stream).await.unwrap().unwrap();
            assert_eq!(hello["params"]["_sky_cua_client_role"], "heartbeat");
            write_frame(
                &mut stream,
                &json!({
                    "jsonrpc": "2.0",
                    "id": KEEPALIVE_INFO_REQUEST_ID,
                    "result": { "type": "extension" }
                }),
            )
            .await
            .unwrap();
        });

        let selected = serve_first_available_with_timeout(
            &[silent_path, live_path.clone()],
            Duration::from_millis(25),
        )
        .await;
        silent_server.await.unwrap();
        live_server.await.unwrap();

        assert_eq!(selected.as_deref(), Some(live_path.as_path()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn repeated_irrelevant_json_cannot_extend_initial_response_deadline() {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-keepalive-junk-failover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let junk_path = dir.join("extension-newest-junk.sock");
        let junk_listener = tokio::net::UnixListener::bind(&junk_path).unwrap();
        let live_path = dir.join("extension-older-live.sock");
        let live_listener = tokio::net::UnixListener::bind(&live_path).unwrap();

        let junk_server = tokio::spawn(async move {
            let (mut stream, _) = junk_listener.accept().await.unwrap();
            let _hello = read_frame(&mut stream).await.unwrap().unwrap();
            loop {
                if write_frame(&mut stream, &json!({})).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        let live_server = tokio::spawn(async move {
            let (mut stream, _) = live_listener.accept().await.unwrap();
            let _hello = read_frame(&mut stream).await.unwrap().unwrap();
            write_frame(
                &mut stream,
                &json!({
                    "jsonrpc": "2.0",
                    "id": KEEPALIVE_INFO_REQUEST_ID,
                    "result": { "type": "extension" }
                }),
            )
            .await
            .unwrap();
        });

        let selected = serve_first_available_with_timeout(
            &[junk_path, live_path.clone()],
            Duration::from_millis(25),
        )
        .await;
        junk_server.await.unwrap();
        live_server.await.unwrap();

        assert_eq!(selected.as_deref(), Some(live_path.as_path()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
