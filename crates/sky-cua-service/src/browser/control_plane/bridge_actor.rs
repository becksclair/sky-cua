use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use sky_cua_platform::model::{
    BROWSER_CONTROL_CANONICAL_SESSION_ID, BROWSER_CONTROL_CANONICAL_TURN_ID,
    BrowserInstanceStability,
};
use tokio::io::AsyncWrite;
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::{Instant, MissedTickBehavior};

use super::{
    DispatchOperation, Executor, ExecutorOutcome, OperationClass, OperationScope,
    SETTLEMENT_DEADLINE_MS,
};
use crate::browser::protocol::read_frame;
use crate::browser::sockets::{
    SocketPeerIdentity, record_persistent_actor_health, socket_peer_identity,
};

mod runtime;
mod wire;

use runtime::{PendingRequest, QueuedRequest, Runtime};
use wire::{
    HEARTBEAT_DEADLINE, HOST_RELEASE_METHOD, HOST_SETTLEMENT_ACK_METHOD, Handshake, is_ping,
    operation_class_name, perform_handshake, request_size, requires_settlement, route_notification,
    settlement_ack_frame, unix_epoch_ms, write_frame_bounded, write_pong,
};

static DAEMON_GENERATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A lexically sortable, fixed-width daemon generation. Nanoseconds establish
/// startup order and the process-local suffix prevents clock-resolution
/// collisions during rapid replacement or deterministic tests.
pub(crate) fn fixed_width_daemon_generation() -> String {
    let epoch_ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64;
    let sequence = DAEMON_GENERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{epoch_ns:020}{:010}{sequence:020}", std::process::id())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BridgeRequestSize {
    Ordinary,
    LargeFrame,
}

#[derive(Clone, Debug)]
pub(crate) struct BridgeActorRequest {
    pub(crate) method: String,
    pub(crate) params: Value,
    pub(crate) operation_id: String,
    pub(crate) operation_class: OperationClass,
    pub(crate) target_lifetime_key: Option<Value>,
    pub(crate) timeout: Duration,
    pub(crate) size: BridgeRequestSize,
}

impl BridgeActorRequest {
    pub(crate) fn new(
        method: impl Into<String>,
        params: Value,
        operation_id: impl Into<String>,
        operation_class: OperationClass,
    ) -> Self {
        let method = method.into();
        let size = request_size(&method, &params);
        Self {
            method,
            params,
            operation_id: operation_id.into(),
            operation_class,
            target_lifetime_key: None,
            timeout: Duration::from_secs(60),
            size,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_size(mut self, size: BridgeRequestSize) -> Self {
        self.size = size;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BridgeActorState {
    Connecting,
    HostHandshake,
    Ready,
    Reconnecting,
    Quarantined,
    Lost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BridgeActorHealth {
    pub(crate) state: BridgeActorState,
    pub(crate) socket_path: PathBuf,
    pub(crate) bridge_connection_id: Option<String>,
    pub(crate) browser_instance_id: Option<String>,
    pub(crate) browser_instance_stability: BrowserInstanceStability,
    pub(crate) host_instance_id: Option<String>,
    pub(crate) peer_pid: Option<u32>,
    pub(crate) peer_start_ticks: Option<u64>,
    pub(crate) boot_id: Option<String>,
    pub(crate) actor_generation: u64,
    pub(crate) daemon_generation: String,
    pub(crate) reconnect_count: u64,
    pub(crate) last_heartbeat_rtt_ms: Option<u64>,
    pub(crate) quarantine_reason: Option<String>,
}

impl BridgeActorHealth {
    fn initial(config: &BridgeActorConfig) -> Self {
        Self {
            state: BridgeActorState::Connecting,
            socket_path: config.socket_path.clone(),
            bridge_connection_id: None,
            browser_instance_id: None,
            browser_instance_stability: BrowserInstanceStability::Unavailable,
            host_instance_id: None,
            peer_pid: None,
            peer_start_ticks: None,
            boot_id: None,
            actor_generation: config.actor_generation,
            daemon_generation: config.daemon_generation.clone(),
            reconnect_count: 0,
            last_heartbeat_rtt_ms: None,
            quarantine_reason: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BridgeActorEvent {
    State(BridgeActorHealth),
    Extension(Value),
    Settlement(Value),
    SettlementUnknown(Value),
    LateResponse {
        request_id: String,
        operation_id: Option<String>,
        response: Value,
    },
    BrowserLost {
        browser_instance_id: String,
        reason: String,
        stable_recovery: bool,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct BridgeActorConfig {
    pub(crate) socket_path: PathBuf,
    pub(crate) daemon_generation: String,
    pub(crate) actor_generation: u64,
    pub(crate) owner_mode: BridgeOwnerMode,
    pub(crate) connect_timeout: Duration,
    pub(crate) handshake_timeout: Duration,
    pub(crate) write_timeout: Duration,
    pub(crate) heartbeat_interval: Duration,
    pub(crate) reconnect_min: Duration,
    pub(crate) reconnect_max: Duration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BridgeOwnerMode {
    #[default]
    Hybrid,
    Strict,
}

impl BridgeOwnerMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::Strict => "strict",
        }
    }
}

impl BridgeActorConfig {
    pub(crate) fn new(socket_path: PathBuf, actor_generation: u64) -> Self {
        Self {
            socket_path,
            daemon_generation: fixed_width_daemon_generation(),
            actor_generation,
            owner_mode: BridgeOwnerMode::default(),
            connect_timeout: Duration::from_secs(3),
            handshake_timeout: Duration::from_secs(3),
            write_timeout: Duration::from_secs(3),
            heartbeat_interval: Duration::from_secs(1),
            reconnect_min: Duration::from_millis(100),
            reconnect_max: Duration::from_secs(3),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BridgeActorError {
    Stopped,
    Unavailable(String),
    RequestFailed(String),
    UpstreamError(Value),
    TimedOut,
    Disconnected,
    Ambiguous,
    InvalidPayload(String),
}

enum Command {
    Request(
        BridgeActorRequest,
        oneshot::Sender<Result<Value, BridgeActorError>>,
    ),
    ServerMessage(Value),
    #[cfg(test)]
    Barrier(oneshot::Sender<()>),
    Shutdown,
}

#[derive(Clone)]
pub(crate) struct BridgeActor {
    sender: mpsc::Sender<Command>,
    events: broadcast::Sender<BridgeActorEvent>,
    health: watch::Receiver<BridgeActorHealth>,
}

impl BridgeActor {
    pub(crate) fn spawn(config: BridgeActorConfig) -> Self {
        let (sender, receiver) = mpsc::channel(128);
        let (events, _) = broadcast::channel(256);
        let (health_sender, health) = watch::channel(BridgeActorHealth::initial(&config));
        tokio::spawn(run_actor(config, receiver, events.clone(), health_sender));
        Self {
            sender,
            events,
            health,
        }
    }

    pub(crate) async fn request(
        &self,
        request: BridgeActorRequest,
    ) -> Result<Value, BridgeActorError> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(Command::Request(request, reply))
            .await
            .map_err(|_| BridgeActorError::Stopped)?;
        receive.await.unwrap_or(Err(BridgeActorError::Stopped))
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<BridgeActorEvent> {
        self.events.subscribe()
    }

    pub(crate) async fn send_server_message(&self, message: Value) -> Result<(), BridgeActorError> {
        self.sender
            .send(Command::ServerMessage(message))
            .await
            .map_err(|_| BridgeActorError::Stopped)
    }

    pub(crate) async fn acknowledge_settlement(
        &self,
        settlement: &Value,
    ) -> Result<(), BridgeActorError> {
        let frame = settlement_ack_frame(settlement, &self.health().daemon_generation)?;
        self.send_server_message(frame).await
    }

    #[cfg(test)]
    pub(super) async fn enqueue_request_for_test(
        &self,
        request: BridgeActorRequest,
    ) -> oneshot::Receiver<Result<Value, BridgeActorError>> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(Command::Request(request, reply))
            .await
            .expect("test actor remains active");
        receive
    }

    #[cfg(test)]
    pub(super) async fn barrier_for_test(&self) {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(Command::Barrier(reply))
            .await
            .expect("test actor remains active");
        receive.await.expect("test actor acknowledges barrier");
    }

    pub(crate) fn health(&self) -> BridgeActorHealth {
        self.health.borrow().clone()
    }

    pub(crate) async fn wait_until_ready(&mut self) -> Result<BridgeActorHealth, BridgeActorError> {
        loop {
            let health = self.health();
            if health.state == BridgeActorState::Ready {
                return Ok(health);
            }
            if health.state == BridgeActorState::Quarantined {
                return Err(BridgeActorError::Unavailable(
                    health
                        .quarantine_reason
                        .unwrap_or_else(|| "bridge quarantined".to_owned()),
                ));
            }
            self.health
                .changed()
                .await
                .map_err(|_| BridgeActorError::Stopped)?;
        }
    }

    pub(crate) async fn shutdown(&self) {
        let _ = self.sender.send(Command::Shutdown).await;
    }

    pub(crate) async fn wait_closed(&self) {
        self.sender.closed().await;
    }
}

impl Executor for BridgeActor {
    fn execute(
        &self,
        operation: DispatchOperation,
    ) -> Pin<Box<dyn Future<Output = ExecutorOutcome> + Send + 'static>> {
        let actor = self.clone();
        Box::pin(async move {
            let request = match request_from_dispatch(&operation) {
                Ok(request) => request,
                Err(error) => return ExecutorOutcome::DefinitiveFailure(error),
            };
            match actor.request(request).await {
                Ok(response) => {
                    let result = response.get("result").unwrap_or(&response);
                    ExecutorOutcome::DefinitiveSuccess(result.to_string())
                }
                Err(BridgeActorError::Ambiguous) => ExecutorOutcome::Ambiguous(format!(
                    "bridge disconnected before operation {} settled",
                    operation.identity.operation_id
                )),
                Err(error) => ExecutorOutcome::DefinitiveFailure(format!("{error:?}")),
            }
        })
    }
}

fn request_from_dispatch(operation: &DispatchOperation) -> Result<BridgeActorRequest, String> {
    let payload: Value = serde_json::from_str(&operation.payload)
        .map_err(|error| format!("invalid bridge JSON payload: {error}"))?;
    let method = payload
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.is_empty())
        .ok_or_else(|| "bridge payload requires method".to_owned())?;
    let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
    let mut request = BridgeActorRequest::new(
        method,
        params,
        operation.identity.operation_id.0.clone(),
        operation.class,
    );
    request.target_lifetime_key = match &operation.scope {
        OperationScope::Tab(tab) => Some(json!({
            "browser_instance_id": tab.browser_instance_id.0,
            "tab_id": tab.tab_id,
        })),
        OperationScope::BridgeGlobal(browser) => Some(json!({ "browser_instance_id": browser.0 })),
        OperationScope::DaemonGlobal => None,
    };
    Ok(request)
}

async fn run_actor(
    config: BridgeActorConfig,
    mut receiver: mpsc::Receiver<Command>,
    events: broadcast::Sender<BridgeActorEvent>,
    health_sender: watch::Sender<BridgeActorHealth>,
) {
    let mut runtime = Runtime::new(config.actor_generation);
    let mut health = BridgeActorHealth::initial(&config);
    let mut backoff = config.reconnect_min;
    let mut ever_ready = false;
    let mut prior_host_instance_id: Option<String> = None;

    loop {
        set_state(
            &config,
            &events,
            &health_sender,
            &mut health,
            if ever_ready {
                BridgeActorState::Reconnecting
            } else {
                BridgeActorState::Connecting
            },
            None,
        );
        let connected = tokio::time::timeout(
            config.connect_timeout,
            UnixStream::connect(&config.socket_path),
        )
        .await;
        let mut stream = match connected {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                if delay_or_shutdown(
                    backoff,
                    &mut receiver,
                    &mut runtime,
                    BridgeActorError::Unavailable(error.to_string()),
                )
                .await
                {
                    break;
                }
                backoff = doubled(backoff, config.reconnect_max);
                continue;
            }
            Err(_) => {
                if delay_or_shutdown(
                    backoff,
                    &mut receiver,
                    &mut runtime,
                    BridgeActorError::Unavailable("bridge connect timed out".to_owned()),
                )
                .await
                {
                    break;
                }
                backoff = doubled(backoff, config.reconnect_max);
                continue;
            }
        };
        let peer = socket_peer_identity(&stream);
        set_state(
            &config,
            &events,
            &health_sender,
            &mut health,
            BridgeActorState::HostHandshake,
            None,
        );
        let handshake = perform_handshake(&mut stream, &config, &mut runtime, &events).await;
        let handshake = match handshake {
            Ok(handshake) => handshake,
            Err(error) => {
                let quarantine = matches!(error, BridgeActorError::Unavailable(_));
                set_state(
                    &config,
                    &events,
                    &health_sender,
                    &mut health,
                    if quarantine {
                        BridgeActorState::Quarantined
                    } else {
                        BridgeActorState::Reconnecting
                    },
                    Some(format!("{error:?}")),
                );
                if delay_or_shutdown(backoff, &mut receiver, &mut runtime, error).await {
                    break;
                }
                backoff = doubled(backoff, config.reconnect_max);
                continue;
            }
        };

        let browser_instance_id = if handshake.browser_instance_stability
            == BrowserInstanceStability::Stable
        {
            handshake.browser_instance_id.unwrap_or_else(|| {
                format!(
                    "connection-{}-{}-{}-{}",
                    handshake.host_instance_id,
                    peer.pid,
                    peer.start_ticks,
                    runtime.actor_generation
                )
            })
        } else {
            // A connection-only identifier cannot name browser lifetime across a
            // reconnect, even when a legacy host happens to repeat its value.
            format!(
                "connection-{}-{}-{}-{}",
                handshake.host_instance_id, peer.pid, peer.start_ticks, runtime.actor_generation
            )
        };
        let reconnected_without_stable_identity =
            ever_ready && handshake.browser_instance_stability != BrowserInstanceStability::Stable;
        let host_replaced = prior_host_instance_id
            .as_ref()
            .is_some_and(|prior| prior != &handshake.host_instance_id);
        let browser_replaced = ever_ready
            && health
                .browser_instance_id
                .as_ref()
                .is_some_and(|prior| prior != &browser_instance_id);
        if reconnected_without_stable_identity || host_replaced || browser_replaced {
            let _ = events.send(BridgeActorEvent::BrowserLost {
                browser_instance_id: health
                    .browser_instance_id
                    .clone()
                    .unwrap_or_else(|| browser_instance_id.clone()),
                reason: if host_replaced {
                    "native host restarted".to_owned()
                } else if browser_replaced {
                    "browser instance changed across reconnect".to_owned()
                } else {
                    "connection-only browser identity cannot survive reconnect".to_owned()
                },
                stable_recovery: false,
            });
        }
        prior_host_instance_id = Some(handshake.host_instance_id.clone());
        health.state = BridgeActorState::Ready;
        health.bridge_connection_id = Some(format!(
            "bridge-{}-{:020}-{}",
            config.daemon_generation, runtime.actor_generation, handshake.host_instance_id
        ));
        health.browser_instance_id = Some(browser_instance_id);
        health.browser_instance_stability =
            if handshake.browser_instance_stability == BrowserInstanceStability::Unavailable {
                BrowserInstanceStability::ConnectionOnly
            } else {
                handshake.browser_instance_stability
            };
        health.host_instance_id = Some(handshake.host_instance_id);
        health.actor_generation = runtime.actor_generation;
        apply_peer_identity(&mut health, &peer);
        health.quarantine_reason = None;
        let _ = health_sender.send(health.clone());
        let _ = events.send(BridgeActorEvent::State(health.clone()));
        record_persistent_actor_health(&config.socket_path, true);
        ever_ready = true;
        backoff = config.reconnect_min;

        let exit = serve_ready_connection(
            &mut stream,
            &config,
            &mut receiver,
            &events,
            &health_sender,
            &mut health,
            &mut runtime,
        )
        .await;
        record_persistent_actor_health(&config.socket_path, false);
        runtime.fail_dispatched();
        if !exit {
            // Requests still in the actor-private queue were never written to
            // the old bridge. Fail them definitively instead of carrying
            // target-lifetime work into a possibly different browser.
            runtime.fail_queued(BridgeActorError::Disconnected);
        }
        runtime.advance_actor_generation();
        health.reconnect_count = health.reconnect_count.saturating_add(1);
        if exit {
            break;
        }
    }

    record_persistent_actor_health(&config.socket_path, false);
    runtime.fail_dispatched();
    for queued in runtime.queued {
        let _ = queued.reply.send(Err(BridgeActorError::Stopped));
    }
    health.state = BridgeActorState::Lost;
    let _ = health_sender.send(health.clone());
    let _ = events.send(BridgeActorEvent::State(health));
}

#[allow(clippy::too_many_arguments)]
async fn serve_ready_connection(
    stream: &mut UnixStream,
    config: &BridgeActorConfig,
    receiver: &mut mpsc::Receiver<Command>,
    events: &broadcast::Sender<BridgeActorEvent>,
    health_sender: &watch::Sender<BridgeActorHealth>,
    health: &mut BridgeActorHealth,
    runtime: &mut Runtime,
) -> bool {
    let mut heartbeat_tick = tokio::time::interval_at(
        Instant::now() + config.heartbeat_interval,
        config.heartbeat_interval,
    );
    heartbeat_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut maintenance_tick = tokio::time::interval(Duration::from_millis(25));
    maintenance_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        if dispatch_queued(stream, config, runtime).await.is_err() {
            return false;
        }
        tokio::select! {
            command = receiver.recv() => match command {
                Some(Command::Request(request, reply)) => {
                    runtime.queued.push_back(QueuedRequest { request, reply });
                }
                Some(Command::ServerMessage(message)) => {
                    let settled_operation = (message.get("method").and_then(Value::as_str)
                        == Some(HOST_SETTLEMENT_ACK_METHOD))
                    .then(|| {
                        message
                            .pointer("/params/operation_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .flatten();
                    if write_frame_bounded(stream, &message, config.write_timeout).await.is_err() {
                        return false;
                    }
                    if let Some(operation_id) = settled_operation {
                        runtime.resolve_settlement(&operation_id);
                    }
                }
                #[cfg(test)]
                Some(Command::Barrier(reply)) => {
                    let _ = reply.send(());
                }
                Some(Command::Shutdown) | None => {
                    if config.owner_mode == BridgeOwnerMode::Strict
                        && runtime.pending.is_empty()
                        && runtime.queued.is_empty()
                        && !runtime.has_unresolved_settlements()
                    {
                        let _ = release_strict_owner(stream, config, runtime, events).await;
                    }
                    return true;
                },
            },
            frame = read_frame(stream) => match frame {
                Ok(Some(frame)) => {
                    record_persistent_actor_health(&config.socket_path, true);
                    if handle_frame(stream, frame, events, health_sender, health, runtime).await.is_err() {
                        return false;
                    }
                }
                Ok(None) | Err(_) => return false,
            },
            _ = heartbeat_tick.tick() => {
                if let Some((_, sent_at)) = &runtime.heartbeat
                    && sent_at.elapsed() >= HEARTBEAT_DEADLINE
                {
                    return false;
                }
                if runtime.heartbeat.is_none() {
                    let id = runtime.allocate_request_id(config);
                    if write_frame_bounded(stream, &json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "method": "ping",
                        "params": {},
                    }), config.write_timeout).await.is_err() {
                        return false;
                    }
                    runtime.heartbeat = Some((id, Instant::now()));
                }
            },
            _ = maintenance_tick.tick() => {
                expire_pending(runtime);
            },
        }
    }
}

async fn dispatch_queued(
    stream: &mut (impl AsyncWrite + Unpin),
    config: &BridgeActorConfig,
    runtime: &mut Runtime,
) -> Result<(), BridgeActorError> {
    loop {
        let Some(front) = runtime.queued.front() else {
            return Ok(());
        };
        if !runtime.can_dispatch(front.request.size) {
            return Ok(());
        }
        let queued = runtime.queued.pop_front().expect("front exists");
        let request_id = runtime.allocate_request_id(config);
        let frame = request_frame(
            config,
            runtime.actor_generation,
            &request_id,
            &queued.request,
        )?;
        runtime.pending.insert(
            request_id,
            PendingRequest {
                reply: queued.reply,
                operation_class: queued.request.operation_class,
                operation_id: queued.request.operation_id,
                deadline: Instant::now() + queued.request.timeout,
                size: queued.request.size,
            },
        );
        write_frame_bounded(stream, &frame, config.write_timeout).await?;
    }
}

async fn release_strict_owner(
    stream: &mut UnixStream,
    config: &BridgeActorConfig,
    runtime: &mut Runtime,
    events: &broadcast::Sender<BridgeActorEvent>,
) -> Result<(), BridgeActorError> {
    let request_id = runtime.allocate_request_id(config);
    write_frame_bounded(
        stream,
        &json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": HOST_RELEASE_METHOD,
            "params": {
                "daemon_generation": config.daemon_generation,
                "owner_mode": "hybrid",
            },
        }),
        config.write_timeout,
    )
    .await?;

    let deadline = Instant::now() + config.handshake_timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let frame = tokio::time::timeout(remaining, read_frame(stream))
            .await
            .map_err(|_| BridgeActorError::TimedOut)?
            .map_err(|error| BridgeActorError::RequestFailed(error.to_string()))?
            .ok_or(BridgeActorError::Disconnected)?;
        if is_ping(&frame) {
            write_pong(stream, &frame).await?;
            continue;
        }
        if frame.get("id").and_then(Value::as_str) != Some(request_id.as_str()) {
            route_notification(events, frame);
            continue;
        }
        if let Some(error) = frame.get("error") {
            return Err(BridgeActorError::Unavailable(format!(
                "strict owner release rejected: {error}"
            )));
        }
        if frame.pointer("/result/owner_mode").and_then(Value::as_str) != Some("hybrid") {
            return Err(BridgeActorError::Unavailable(
                "strict owner release response omitted hybrid owner mode".into(),
            ));
        }
        return Ok(());
    }
}

#[cfg(test)]
pub(super) async fn exercise_write_failure(
    config: &BridgeActorConfig,
    request: BridgeActorRequest,
    bytes_before_failure: usize,
) -> (
    BridgeActorError,
    usize,
    Option<String>,
    Option<OperationClass>,
    usize,
) {
    use std::io;
    use std::task::{Context, Poll};

    struct FailingWriter {
        bytes_before_failure: usize,
        bytes_written: usize,
    }

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            let remaining = self.bytes_before_failure.saturating_sub(self.bytes_written);
            if remaining == 0 {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "deterministic write failure",
                )));
            }
            let written = remaining.min(buffer.len());
            self.bytes_written += written;
            Poll::Ready(Ok(written))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    let mut runtime = Runtime::new(config.actor_generation);
    let (reply, receive) = oneshot::channel();
    runtime.queued.push_back(QueuedRequest { request, reply });
    let mut writer = FailingWriter {
        bytes_before_failure,
        bytes_written: 0,
    };
    dispatch_queued(&mut writer, config, &mut runtime)
        .await
        .expect_err("scripted writer must fail");
    let pending_before_failure = runtime.pending.len();
    let request_id = runtime.pending.keys().next().cloned();
    runtime.fail_dispatched();
    let error = receive
        .await
        .expect("actor retains reply sender")
        .expect_err("failed dispatch cannot succeed");
    let tombstone = request_id.and_then(|request_id| runtime.tombstones.get(&request_id));
    (
        error,
        pending_before_failure,
        tombstone.and_then(|entry| entry.operation_id.clone()),
        tombstone.and_then(|entry| entry.operation_class),
        writer.bytes_written,
    )
}

#[cfg(test)]
pub(super) async fn exercise_stalled_write(
    config: &BridgeActorConfig,
    request: BridgeActorRequest,
) -> BridgeActorError {
    use std::io;
    use std::task::{Context, Poll};

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

    let mut runtime = Runtime::new(config.actor_generation);
    let (reply, receive) = oneshot::channel();
    runtime.queued.push_back(QueuedRequest { request, reply });
    dispatch_queued(&mut StalledWriter, config, &mut runtime)
        .await
        .expect_err("stalled writer must time out");
    runtime.fail_dispatched();
    receive
        .await
        .expect("actor retains reply sender")
        .expect_err("timed-out dispatch cannot succeed")
}

fn request_frame(
    config: &BridgeActorConfig,
    actor_generation: u64,
    request_id: &str,
    request: &BridgeActorRequest,
) -> Result<Value, BridgeActorError> {
    let mut params = match request.params.clone() {
        Value::Object(params) => params,
        Value::Null => Map::new(),
        _ => {
            return Err(BridgeActorError::InvalidPayload(
                "bridge request params must be an object".into(),
            ));
        }
    };
    params.insert(
        "session_id".into(),
        Value::String(BROWSER_CONTROL_CANONICAL_SESSION_ID.into()),
    );
    params.insert(
        "turn_id".into(),
        Value::String(BROWSER_CONTROL_CANONICAL_TURN_ID.into()),
    );
    params.insert(
        "_sky_cua_client_role".into(),
        Value::String("control_plane".into()),
    );
    params.insert("_sky_cua_observe_turns".into(), Value::Bool(false));
    let deadline_ms = unix_epoch_ms()
        .saturating_add(u64::try_from(request.timeout.as_millis()).unwrap_or(u64::MAX))
        .saturating_add(if requires_settlement(request.operation_class) {
            SETTLEMENT_DEADLINE_MS
        } else {
            0
        });
    params.insert(
        "_sky_cua_host_request".into(),
        json!({
            "operation_id": request.operation_id,
            "daemon_generation": config.daemon_generation,
            "actor_generation": actor_generation,
            "target_lifetime_key": request.target_lifetime_key,
            "operation_class": operation_class_name(request.operation_class),
            "settlement_deadline_ms": deadline_ms,
        }),
    );
    Ok(json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": request.method,
        "params": params,
    }))
}

#[cfg(test)]
pub(super) fn request_frame_for_test(
    config: &BridgeActorConfig,
    request: &BridgeActorRequest,
) -> Value {
    request_frame(config, config.actor_generation, "test-request", request)
        .expect("test request frame is valid")
}

async fn handle_frame(
    stream: &mut UnixStream,
    frame: Value,
    events: &broadcast::Sender<BridgeActorEvent>,
    health_sender: &watch::Sender<BridgeActorHealth>,
    health: &mut BridgeActorHealth,
    runtime: &mut Runtime,
) -> Result<(), BridgeActorError> {
    if is_ping(&frame) {
        return write_pong(stream, &frame).await;
    }
    if frame.get("method").and_then(Value::as_str).is_some() {
        route_notification(events, frame);
        return Ok(());
    }
    let Some(request_id) = frame.get("id").and_then(Value::as_str).map(str::to_owned) else {
        route_notification(events, frame);
        return Ok(());
    };
    if runtime
        .heartbeat
        .as_ref()
        .is_some_and(|(id, _)| id == &request_id)
    {
        let (_, sent_at) = runtime.heartbeat.take().expect("heartbeat exists");
        health.last_heartbeat_rtt_ms =
            Some(u64::try_from(sent_at.elapsed().as_millis()).unwrap_or(u64::MAX));
        let _ = health_sender.send(health.clone());
        return Ok(());
    }
    if let Some(pending) = runtime.pending.remove(&request_id) {
        let result = if let Some(error) = frame.get("error") {
            Err(BridgeActorError::UpstreamError(error.clone()))
        } else {
            Ok(frame)
        };
        let _ = pending.reply.send(result);
        runtime.tombstone(request_id, Instant::now());
    } else if let Some(tombstone) = runtime.tombstones.get(&request_id) {
        let operation_id = tombstone
            .operation_class
            .is_some_and(requires_settlement)
            .then(|| tombstone.operation_id.clone())
            .flatten();
        let _ = events.send(BridgeActorEvent::LateResponse {
            request_id,
            operation_id,
            response: frame,
        });
    } else {
        route_notification(events, frame);
    }
    Ok(())
}

fn expire_pending(runtime: &mut Runtime) {
    let now = Instant::now();
    let expired = runtime
        .pending
        .iter()
        .filter_map(|(id, pending)| (pending.deadline <= now).then_some(id.clone()))
        .collect::<Vec<_>>();
    for id in expired {
        if let Some(pending) = runtime.pending.remove(&id) {
            let result = if requires_settlement(pending.operation_class) {
                Err(BridgeActorError::Ambiguous)
            } else {
                Err(BridgeActorError::TimedOut)
            };
            let operation_id = pending.operation_id.clone();
            let operation_class = pending.operation_class;
            let _ = pending.reply.send(result);
            runtime.tombstone_pending(id, now, operation_id, operation_class);
        }
    }
    runtime.prune_tombstones(now);
}

async fn delay_or_shutdown(
    delay: Duration,
    receiver: &mut mpsc::Receiver<Command>,
    runtime: &mut Runtime,
    error: BridgeActorError,
) -> bool {
    let sleep = tokio::time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return false,
            command = receiver.recv() => match command {
                Some(Command::Request(_, reply)) => {
                    let _ = reply.send(Err(error.clone()));
                }
                Some(Command::ServerMessage(_)) => {}
                #[cfg(test)]
                Some(Command::Barrier(reply)) => {
                    let _ = reply.send(());
                }
                Some(Command::Shutdown) | None => return true,
            }
        }
        runtime.prune_tombstones(Instant::now());
    }
}

fn set_state(
    config: &BridgeActorConfig,
    events: &broadcast::Sender<BridgeActorEvent>,
    health_sender: &watch::Sender<BridgeActorHealth>,
    health: &mut BridgeActorHealth,
    state: BridgeActorState,
    quarantine_reason: Option<String>,
) {
    health.state = state;
    health.quarantine_reason = quarantine_reason;
    if state != BridgeActorState::Ready {
        record_persistent_actor_health(&config.socket_path, false);
    }
    let _ = health_sender.send(health.clone());
    let _ = events.send(BridgeActorEvent::State(health.clone()));
}

fn apply_peer_identity(health: &mut BridgeActorHealth, peer: &SocketPeerIdentity) {
    health.peer_pid = Some(peer.pid);
    health.peer_start_ticks = Some(peer.start_ticks);
    health.boot_id = Some(peer.boot_id.clone());
}

fn doubled(value: Duration, ceiling: Duration) -> Duration {
    value.saturating_mul(2).min(ceiling)
}
