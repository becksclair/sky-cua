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
//! This module holds ONE long-lived connection to the bridge socket, identified
//! as a primary client, whose only job is to answer that heartbeat so the
//! debugger session stays attached across a browser-use session. It is started
//! lazily on first browser-bridge use and runs for the daemon lifetime,
//! reconnecting across host restarts (each restart yields a new
//! `extension-<pid>-<nonce>.sock`).
//!
//! Tradeoff (accepted): registering as the primary client means that if a real
//! Codex desktop app is also driving the same browser, one evicts the other.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::net::UnixStream;

use super::protocol::{read_frame, write_frame};
use super::sockets::newest_bridge_socket_path;

/// Session id that marks this connection as the primary/driving client. Any id
/// other than the ephemeral `sky-cua-mcp` used by per-op connections is treated
/// as primary by the native host, and the extension routes the heartbeat ping
/// only to the primary.
const KEEPALIVE_SESSION_ID: &str = "sky-cua-heartbeat-keepalive";
const KEEPALIVE_TURN_ID: &str = "heartbeat";

/// Poll interval while no bridge socket exists yet (extension not connected).
const SOCKET_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Backoff after a connection drops before re-establishing.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(1);
/// The extension pings every 30s; if no frame arrives within this window the
/// connection is treated as stale and re-established. Comfortably spans more
/// than one heartbeat interval so a healthy-but-quiet link is not churned.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(75);

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
    loop {
        match newest_bridge_socket_path() {
            Some(socket) => {
                serve_socket(&socket).await;
                tokio::time::sleep(RECONNECT_BACKOFF).await;
            }
            None => tokio::time::sleep(SOCKET_POLL_INTERVAL).await,
        }
    }
}

/// Connect to the bridge socket and answer heartbeat pings until the connection
/// drops or goes idle-stale, then return so the loop can reconnect.
async fn serve_socket(socket: &Path) {
    let Ok(stream) = UnixStream::connect(socket).await else {
        return;
    };
    serve_stream(stream).await;
}

async fn serve_stream(mut stream: UnixStream) {
    // Register as the driving (primary) client. The non-`sky-cua-mcp` session id
    // is what makes the host classify this connection as primary — the role the
    // extension routes the heartbeat ping to.
    if write_frame(&mut stream, &hello_frame()).await.is_err() {
        return;
    }

    loop {
        match tokio::time::timeout(READ_IDLE_TIMEOUT, read_frame(&mut stream)).await {
            Ok(Ok(Some(message))) => {
                if is_ping_request(&message) {
                    let id = message.get("id").cloned().unwrap_or(Value::Null);
                    if write_frame(&mut stream, &pong_frame(id)).await.is_err() {
                        return;
                    }
                }
                // Every other routed frame (the getInfo reply, CDP-event and
                // download notifications) needs no response from the keepalive.
            }
            // EOF, read/parse error, or idle-stale: drop and let the loop
            // reconnect against the current host socket.
            _ => return,
        }
    }
}

fn hello_frame() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": "sky-cua-browser-keepalive-info",
        "method": "getInfo",
        "params": { "session_id": KEEPALIVE_SESSION_ID, "turn_id": KEEPALIVE_TURN_ID },
    })
}

fn pong_frame(id: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": "pong" })
}

fn is_ping_request(message: &Value) -> bool {
    message.get("method").and_then(Value::as_str) == Some("ping") && message.get("id").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keepalive_registers_as_primary_not_ephemeral() {
        let hello = hello_frame();
        assert_eq!(hello["method"], "getInfo");
        // Must NOT use the ephemeral per-op session id, or the host will not
        // route the heartbeat to it.
        assert_ne!(hello["params"]["session_id"], "sky-cua-mcp");
        assert_eq!(hello["params"]["session_id"], KEEPALIVE_SESSION_ID);
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
}
