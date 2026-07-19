//! Raw upstream Browser-client compatibility ingress for Codex Desktop.
//!
//! Codex speaks the upstream framed JSON-RPC protocol here without a typed
//! BrowserControl hello. This module owns wire validation and normalization;
//! the eventual persistent bridge/control-plane adapter implements
//! [`CodexBrowserBackend`] without changing this compatibility surface.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc;

mod connection;
mod framing;
mod listener;
mod normalize;

pub(crate) use connection::serve_connection;
#[cfg(test)]
use connection::serve_stream;
use framing::{read_frame, write_frame};
#[cfg(test)]
use listener::configured_socket_path;
pub(crate) use listener::{CodexBrowserCompatListener, accept_configured};
use normalize::normalize_request;
#[cfg(test)]
use normalize::{canonical_fingerprint, classify_cdp};

pub(crate) const CODEX_BROWSER_SOCKET_PATH_ENV: &str = "SKY_CUA_CODEX_BROWSER_SOCKET_PATH";
const MAX_FRAME_BYTES: usize = 100 * 1024 * 1024;
const MAX_DEADLINE_MS: u64 = 120_000;
const DEFAULT_READ_DEADLINE_MS: u64 = 30_000;
const DEFAULT_MUTATION_DEADLINE_MS: u64 = 15_000;
const PROTOCOL_MISMATCH_CODE: i64 = -32070;
const GENERATION_CHANGED_CODE: i64 = -32071;
const PROTOCOL_MISMATCH_MESSAGE: &str = "sky-cua browser compatibility protocol mismatch";
const GENERATION_CHANGED_MESSAGE: &str =
    "sky-cua browser daemon generation changed; refresh browser backends";

static NEXT_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexConnectionContext {
    pub(crate) connection_id: String,
    pub(crate) provenance: &'static str,
    pub(crate) peer_uid: u32,
    pub(crate) codex_app_build_flavor: Option<String>,
    pub(crate) daemon_generation: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CodexLogicalIdentity {
    pub(crate) session_id: Option<String>,
    pub(crate) thread_id: Option<String>,
    pub(crate) turn_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexOperationClass {
    ReadOnly,
    AbsoluteSet,
    Mutation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexOperationScope {
    Tab(String),
    Bridge,
    Daemon,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexNormalizedRequest {
    pub(crate) operation_id: String,
    pub(crate) upstream_id: u64,
    pub(crate) method: String,
    pub(crate) params: Value,
    pub(crate) raw_request: Value,
    pub(crate) connection: CodexConnectionContext,
    pub(crate) logical_identity: CodexLogicalIdentity,
    pub(crate) class: CodexOperationClass,
    pub(crate) scope: CodexOperationScope,
    pub(crate) canonical_fingerprint: String,
    pub(crate) deadline: Duration,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)] // Result is produced by the forthcoming live bridge backend.
pub(crate) enum CodexBackendReply {
    Result(Value),
    Error(Value),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexCallerLifecycle {
    FinalizeTabs,
    TurnEnded,
}

/// Service-internal boundary between the raw compatibility wire and the
/// control-plane scheduler/bridge. Implementations retain dispatch state, so
/// `cancel_or_detach` can cancel queued admission or detach an in-flight waiter
/// while shared bridge execution drains.
#[async_trait]
pub(crate) trait CodexBrowserBackend: Send + Sync + 'static {
    fn daemon_generation(&self) -> String;

    async fn connection_opened(
        &self,
        connection: CodexConnectionContext,
        outbound: mpsc::UnboundedSender<Value>,
    ) -> Result<(), CodexBackendReply>;

    async fn request(&self, request: CodexNormalizedRequest) -> CodexBackendReply;

    async fn caller_lifecycle(
        &self,
        lifecycle: CodexCallerLifecycle,
        request: CodexNormalizedRequest,
    ) -> CodexBackendReply;

    async fn client_message(&self, connection_id: &str, message: Value);

    async fn cancel_or_detach(&self, connection_id: &str, operation_id: &str);

    async fn connection_closed(&self, connection_id: &str);
}

/// Startup placeholder until the persistent bridge package implements the
/// backend trait. It deliberately does not discover or connect to a legacy
/// native-host socket.
pub(crate) struct UnavailableCodexBrowserBackend {
    generation: String,
}

impl UnavailableCodexBrowserBackend {
    pub(crate) fn new() -> Self {
        Self {
            generation: format!("daemon-{}", std::process::id()),
        }
    }
}

#[async_trait]
impl CodexBrowserBackend for UnavailableCodexBrowserBackend {
    fn daemon_generation(&self) -> String {
        self.generation.clone()
    }

    async fn connection_opened(
        &self,
        _connection: CodexConnectionContext,
        _outbound: mpsc::UnboundedSender<Value>,
    ) -> Result<(), CodexBackendReply> {
        Ok(())
    }

    async fn request(&self, _request: CodexNormalizedRequest) -> CodexBackendReply {
        CodexBackendReply::Error(owner_error_value(
            PROTOCOL_MISMATCH_CODE,
            PROTOCOL_MISMATCH_MESSAGE,
        ))
    }

    async fn caller_lifecycle(
        &self,
        _lifecycle: CodexCallerLifecycle,
        _request: CodexNormalizedRequest,
    ) -> CodexBackendReply {
        self.request(_request).await
    }

    async fn client_message(&self, _connection_id: &str, _message: Value) {}

    async fn cancel_or_detach(&self, _connection_id: &str, _operation_id: &str) {}

    async fn connection_closed(&self, _connection_id: &str) {}
}

fn fresh_id(prefix: &str, counter: &AtomicU64) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        counter.fetch_add(1, Ordering::Relaxed)
    )
}

fn numeric_request_id(message: &Value) -> Option<u64> {
    message
        .get("id")
        .and_then(Value::as_u64)
        .filter(|_| message.get("jsonrpc").and_then(Value::as_str) == Some("2.0"))
}

fn numeric_id_for_error(message: &Value) -> u64 {
    message.get("id").and_then(Value::as_u64).unwrap_or(0)
}

fn owner_error_value(code: i64, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

fn reply_frame(id: u64, reply: CodexBackendReply) -> Value {
    match reply {
        CodexBackendReply::Result(result) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }),
        CodexBackendReply::Error(error) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": error,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, os::unix::fs::PermissionsExt, sync::Arc};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::UnixStream,
        sync::{Mutex, Notify},
    };

    #[derive(Default)]
    struct FakeState {
        opened: Vec<CodexConnectionContext>,
        requests: Vec<CodexNormalizedRequest>,
        lifecycle: Vec<(CodexCallerLifecycle, CodexNormalizedRequest)>,
        client_messages: Vec<Value>,
        cancelled: Vec<String>,
        closed: Vec<String>,
        replies: VecDeque<CodexBackendReply>,
        outbound: Option<mpsc::UnboundedSender<Value>>,
    }

    struct FakeBackend {
        generation: std::sync::RwLock<String>,
        state: Mutex<FakeState>,
        block: Option<Arc<Notify>>,
    }

    impl FakeBackend {
        fn new() -> Self {
            Self {
                generation: std::sync::RwLock::new("generation-1".to_owned()),
                state: Mutex::new(FakeState::default()),
                block: None,
            }
        }

        fn blocking() -> (Self, Arc<Notify>) {
            let notify = Arc::new(Notify::new());
            (
                Self {
                    generation: std::sync::RwLock::new("generation-1".to_owned()),
                    state: Mutex::new(FakeState::default()),
                    block: Some(notify.clone()),
                },
                notify,
            )
        }
    }

    #[async_trait]
    impl CodexBrowserBackend for FakeBackend {
        fn daemon_generation(&self) -> String {
            self.generation.read().unwrap().clone()
        }

        async fn connection_opened(
            &self,
            connection: CodexConnectionContext,
            outbound: mpsc::UnboundedSender<Value>,
        ) -> Result<(), CodexBackendReply> {
            let mut state = self.state.lock().await;
            state.opened.push(connection);
            state.outbound = Some(outbound);
            Ok(())
        }

        async fn request(&self, request: CodexNormalizedRequest) -> CodexBackendReply {
            let reply = {
                let mut state = self.state.lock().await;
                state.requests.push(request);
                state
                    .replies
                    .pop_front()
                    .unwrap_or_else(|| CodexBackendReply::Result(json!({ "ok": true })))
            };
            if let Some(block) = &self.block {
                block.notified().await;
            }
            reply
        }

        async fn caller_lifecycle(
            &self,
            lifecycle: CodexCallerLifecycle,
            request: CodexNormalizedRequest,
        ) -> CodexBackendReply {
            self.state.lock().await.lifecycle.push((lifecycle, request));
            CodexBackendReply::Result(Value::Null)
        }

        async fn client_message(&self, _connection_id: &str, message: Value) {
            self.state.lock().await.client_messages.push(message);
        }

        async fn cancel_or_detach(&self, _connection_id: &str, operation_id: &str) {
            self.state
                .lock()
                .await
                .cancelled
                .push(operation_id.to_owned());
        }

        async fn connection_closed(&self, connection_id: &str) {
            self.state
                .lock()
                .await
                .closed
                .push(connection_id.to_owned());
        }
    }

    async fn start_fake(
        backend: Arc<FakeBackend>,
    ) -> (
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
        tokio::io::ReadHalf<tokio::io::DuplexStream>,
        tokio::task::JoinHandle<Result<()>>,
    ) {
        let (client, server) = tokio::io::duplex(1024 * 1024);
        let handle = tokio::spawn(serve_stream(server, unsafe { libc::geteuid() }, backend));
        let (reader, writer) = tokio::io::split(client);
        (writer, reader, handle)
    }

    #[tokio::test]
    async fn native_endian_framing_and_cap_are_exact() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let value = json!({"jsonrpc":"2.0","id":7,"method":"getTabs","params":{}});
        write_frame(&mut client, &value).await.unwrap();
        let mut header = [0; 4];
        server.read_exact(&mut header).await.unwrap();
        let expected = serde_json::to_vec(&value).unwrap();
        assert_eq!(u32::from_ne_bytes(header) as usize, expected.len());
        let mut body = vec![0; expected.len()];
        server.read_exact(&mut body).await.unwrap();
        assert_eq!(body, expected);

        let (mut writer, mut reader) = tokio::io::duplex(16);
        writer
            .write_all(&((MAX_FRAME_BYTES + 1) as u32).to_ne_bytes())
            .await
            .unwrap();
        assert_eq!(
            read_frame(&mut reader).await.unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[tokio::test]
    async fn opaque_method_and_exact_params_result_and_error_are_forwarded() {
        let backend = Arc::new(FakeBackend::new());
        backend.state.lock().await.replies.extend([
            CodexBackendReply::Result(json!({"nested":[1,{"x":true}]})),
            CodexBackendReply::Error(json!({"code":-42,"message":"upstream","data":{"x":1}})),
        ]);
        let (mut writer, mut reader, handle) = start_fake(backend.clone()).await;
        let first = json!({"jsonrpc":"2.0","id":9,"method":"futureOpaqueMethod","params":{"z":1,"a":[2,3]}});
        write_frame(&mut writer, &first).await.unwrap();
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            json!({"jsonrpc":"2.0","id":9,"result":{"nested":[1,{"x":true}]}})
        );
        write_frame(
            &mut writer,
            &json!({"jsonrpc":"2.0","id":10,"method":"executeCdp","params":{"method":"Page.navigate","commandParams":{"url":"https://example.com"}}}),
        )
        .await
        .unwrap();
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            json!({"jsonrpc":"2.0","id":10,"error":{"code":-42,"message":"upstream","data":{"x":1}}})
        );
        drop(writer);
        drop(reader);
        handle.await.unwrap().unwrap();
        let state = backend.state.lock().await;
        assert_eq!(state.requests[0].method, "futureOpaqueMethod");
        assert_eq!(state.requests[0].params, first["params"]);
    }

    #[tokio::test]
    async fn reused_numeric_ids_allocate_distinct_operations_and_extract_identity() {
        let backend = Arc::new(FakeBackend::new());
        let (mut writer, mut reader, handle) = start_fake(backend.clone()).await;
        for turn in ["turn-1", "turn-2"] {
            write_frame(
                &mut writer,
                &json!({
                    "jsonrpc":"2.0","id":1,"method":"moveMouse",
                    "session_id":"top-level-session",
                    "params":{"target":{"tabId":44},"_meta":{"x-codex-turn-metadata":{
                        "sessionId":"session-a","thread_id":"thread-a","turnId":turn
                    }},"x":5,"y":6,"timeoutMs":999999}
                }),
            )
            .await
            .unwrap();
            let _ = read_frame(&mut reader).await.unwrap().unwrap();
        }
        drop(writer);
        drop(reader);
        handle.await.unwrap().unwrap();
        let state = backend.state.lock().await;
        assert_eq!(state.requests.len(), 2);
        assert_ne!(
            state.requests[0].operation_id,
            state.requests[1].operation_id
        );
        assert_eq!(state.requests[0].upstream_id, 1);
        assert_eq!(state.requests[0].connection.provenance, "codex_desktop");
        assert_eq!(
            state.requests[0].logical_identity.session_id.as_deref(),
            Some("top-level-session")
        );
        assert_eq!(
            state.requests[1].logical_identity.turn_id.as_deref(),
            Some("turn-2")
        );
        assert_eq!(state.requests[0].class, CodexOperationClass::AbsoluteSet);
        assert_eq!(
            state.requests[0].scope,
            CodexOperationScope::Tab("44".to_owned())
        );
        assert_eq!(
            state.requests[0].deadline,
            Duration::from_millis(DEFAULT_MUTATION_DEADLINE_MS)
        );
        assert_eq!(
            state.requests[0].logical_identity.thread_id.as_deref(),
            Some("thread-a")
        );
    }

    #[test]
    fn nested_payload_metadata_cannot_override_owner_policy_or_identity() {
        let connection = CodexConnectionContext {
            connection_id: "codex-1".to_owned(),
            provenance: "codex_desktop",
            peer_uid: 1000,
            codex_app_build_flavor: None,
            daemon_generation: "generation-1".to_owned(),
        };
        let request = normalize_request(
            json!({
                "jsonrpc":"2.0","id":1,"method":"getUserHistory",
                "params":{
                    "session_id":"real-session",
                    "turn_id":"real-turn",
                    "payload":{
                        "session_id":"nested-session",
                        "turn_id":"nested-turn",
                        "timeoutMs":1
                    }
                }
            }),
            1,
            &connection,
        )
        .unwrap();
        assert_eq!(request.class, CodexOperationClass::ReadOnly);
        assert_eq!(
            request.deadline,
            Duration::from_millis(DEFAULT_READ_DEADLINE_MS)
        );
        assert_eq!(
            request.logical_identity.session_id.as_deref(),
            Some("real-session")
        );
        assert_eq!(
            request.logical_identity.turn_id.as_deref(),
            Some("real-turn")
        );
    }

    #[tokio::test]
    async fn backend_requests_notifications_and_client_responses_pass_through() {
        let backend = Arc::new(FakeBackend::new());
        let (mut writer, mut reader, handle) = start_fake(backend.clone()).await;
        let outbound = loop {
            if let Some(outbound) = backend.state.lock().await.outbound.clone() {
                break outbound;
            }
            tokio::task::yield_now().await;
        };
        let request = json!({"jsonrpc":"2.0","id":"backend-7","method":"ping"});
        let notification = json!({"jsonrpc":"2.0","method":"onCDPEvent","params":{"x":1}});
        outbound.send(request.clone()).unwrap();
        outbound.send(notification.clone()).unwrap();
        assert_eq!(read_frame(&mut reader).await.unwrap().unwrap(), request);
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            notification
        );
        let response = json!({"jsonrpc":"2.0","id":"backend-7","result":"pong"});
        write_frame(&mut writer, &response).await.unwrap();
        write_frame(
            &mut writer,
            &json!({"jsonrpc":"2.0","id":99,"method":"ping"}),
        )
        .await
        .unwrap();
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            json!({"jsonrpc":"2.0","id":99,"result":"pong"})
        );
        tokio::task::yield_now().await;
        drop(writer);
        drop(reader);
        handle.await.unwrap().unwrap();
        assert_eq!(backend.state.lock().await.client_messages, vec![response]);
    }

    #[tokio::test]
    async fn eof_cancels_or_detaches_outstanding_operations_without_backend_close() {
        let (backend, release) = FakeBackend::blocking();
        let backend = Arc::new(backend);
        let (mut writer, reader, handle) = start_fake(backend.clone()).await;
        write_frame(
            &mut writer,
            &json!({"jsonrpc":"2.0","id":1,"method":"executeCdp","params":{"method":"Runtime.evaluate"}}),
        )
        .await
        .unwrap();
        loop {
            if !backend.state.lock().await.requests.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        drop(writer);
        drop(reader);
        handle.await.unwrap().unwrap();
        let state = backend.state.lock().await;
        assert_eq!(
            state.cancelled,
            vec![state.requests[0].operation_id.clone()]
        );
        assert_eq!(state.closed.len(), 1);
        drop(state);
        release.notify_waiters();
    }

    #[tokio::test]
    async fn generation_change_returns_stable_owner_error() {
        let (backend, release) = FakeBackend::blocking();
        let backend = Arc::new(backend);
        let (mut writer, mut reader, handle) = start_fake(backend.clone()).await;
        write_frame(
            &mut writer,
            &json!({"jsonrpc":"2.0","id":4,"method":"getTabs"}),
        )
        .await
        .unwrap();
        loop {
            if !backend.state.lock().await.requests.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        *backend.generation.write().unwrap() = "generation-2".to_owned();
        release.notify_waiters();
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap(),
            json!({"jsonrpc":"2.0","id":4,"error":{"code":-32071,"message":"sky-cua browser daemon generation changed; refresh browser backends"}})
        );
        drop(writer);
        drop(reader);
        handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn lifecycle_is_consumed_as_caller_callback() {
        let backend = Arc::new(FakeBackend::new());
        let (mut writer, mut reader, handle) = start_fake(backend.clone()).await;
        for (id, method) in [(1, "finalizeTabs"), (2, "turnEnded")] {
            write_frame(
                &mut writer,
                &json!({"jsonrpc":"2.0","id":id,"method":method,"params":{"session_id":"caller"}}),
            )
            .await
            .unwrap();
            let _ = read_frame(&mut reader).await.unwrap().unwrap();
        }
        drop(writer);
        drop(reader);
        handle.await.unwrap().unwrap();
        let state = backend.state.lock().await;
        assert!(state.requests.is_empty());
        assert_eq!(state.lifecycle.len(), 2);
    }

    #[tokio::test]
    async fn malformed_protocol_gets_stable_error_and_malformed_frame_closes() {
        let backend = Arc::new(FakeBackend::new());
        let (mut writer, mut reader, handle) = start_fake(backend.clone()).await;
        write_frame(
            &mut writer,
            &json!({"jsonrpc":"2.0","id":"not-numeric","method":"getTabs"}),
        )
        .await
        .unwrap();
        assert_eq!(
            read_frame(&mut reader).await.unwrap().unwrap()["error"],
            json!({"code":-32070,"message":"sky-cua browser compatibility protocol mismatch"})
        );
        let bad = b"not-json";
        writer
            .write_all(&(bad.len() as u32).to_ne_bytes())
            .await
            .unwrap();
        writer.write_all(bad).await.unwrap();
        writer.flush().await.unwrap();
        assert!(read_frame(&mut reader).await.unwrap().is_none());
        drop(writer);
        drop(reader);
        handle.await.unwrap().unwrap();
        assert!(backend.state.lock().await.requests.is_empty());
    }

    #[test]
    fn classification_and_fingerprint_are_server_owned_and_canonical() {
        assert_eq!(
            classify_cdp("Page.captureScreenshot"),
            CodexOperationClass::ReadOnly
        );
        assert_eq!(
            classify_cdp("Runtime.evaluate"),
            CodexOperationClass::Mutation
        );
        assert_eq!(
            classify_cdp("Emulation.setDeviceMetricsOverride"),
            CodexOperationClass::AbsoluteSet
        );
        assert_eq!(
            canonical_fingerprint("x", &json!({"b":2,"a":{"d":4,"c":3}})),
            canonical_fingerprint("x", &json!({"a":{"c":3,"d":4},"b":2}))
        );
    }

    #[tokio::test]
    async fn owner_only_socket_rebind_and_no_fallback_discovery() {
        let root = std::env::temp_dir().join(format!(
            "sky-cua-codex-compat-{}-{}",
            std::process::id(),
            NEXT_CONNECTION_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let path = root.join("compat.sock");
        let mut listener = CodexBrowserCompatListener::bind(path.clone()).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::remove_file(&path).unwrap();
        assert!(listener.rebind_if_unlinked().unwrap());
        assert!(path.exists());
        let client = UnixStream::connect(&path).await.unwrap();
        let server = listener.accept().await.unwrap();
        let backend = Arc::new(FakeBackend::new());
        let connection = tokio::spawn(serve_connection(server, backend.clone()));
        drop(client);
        connection.await.unwrap().unwrap();
        assert_eq!(backend.state.lock().await.opened[0].peer_uid, unsafe {
            libc::geteuid()
        });

        // Configuration is explicit. No native-host directory, default path,
        // or legacy discovery input exists in the resolver.
        let old = std::env::var_os(CODEX_BROWSER_SOCKET_PATH_ENV);
        let old_config = std::env::var_os(sky_cua_platform::config::MACHINE_CONFIG_PATH_ENV);
        unsafe {
            std::env::remove_var(CODEX_BROWSER_SOCKET_PATH_ENV);
            std::env::set_var(
                sky_cua_platform::config::MACHINE_CONFIG_PATH_ENV,
                root.join("missing-machine-config.toml"),
            );
        }
        assert_eq!(configured_socket_path().unwrap(), None);
        if let Some(old) = old {
            unsafe { std::env::set_var(CODEX_BROWSER_SOCKET_PATH_ENV, old) };
        }
        match old_config {
            Some(old) => unsafe {
                std::env::set_var(sky_cua_platform::config::MACHINE_CONFIG_PATH_ENV, old)
            },
            None => unsafe {
                std::env::remove_var(sky_cua_platform::config::MACHINE_CONFIG_PATH_ENV)
            },
        }
        drop(listener);
        let _ = std::fs::remove_dir_all(root);
    }
}
