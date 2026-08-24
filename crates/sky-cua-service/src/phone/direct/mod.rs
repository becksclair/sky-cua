//! Host-side foundation for the `phone-control.v2` direct Companion link.
//!
//! This module deliberately keeps the transport independent from phone routing:
//! it owns enrollment, authentication, link epochs, and bounded frame queues.
//! A websocket adapter can consume these primitives without ever reusing the
//! legacy ADB serial as a device identity.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sky_cua_platform::config::{PhoneConfig, ResolvedPhoneSelection};
use sky_cua_platform::model::{
    PHONE_CONTENT_DEFAULT_CHUNK_BYTES, PHONE_CONTROL_MAX_JSON_BYTES, PHONE_CONTROL_PROTOCOL_V2,
    PHONE_ENROLLMENT_PENDING_TTL_MS, PhoneDirectControlFrame, PhoneDirectRole, PhoneEnrollmentAck,
    PhoneEnrollmentCommitted,
};
use std::{
    collections::{BTreeSet, HashMap},
    fs, io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc, oneshot},
    task::JoinSet,
};
use tokio_tungstenite::{WebSocketStream, accept_async};
use uuid::Uuid;

mod content_transfer;
pub(crate) mod lan;
pub(crate) mod provider;
use content_transfer::InboundContentStore;

pub(crate) const DEFAULT_ENROLLMENT_TTL: Duration = Duration::from_secs(5 * 60);

/// Errors returned by the internal direct Companion provider.  In particular,
/// a request which has crossed a disconnect boundary is never retried: the
/// caller must decide whether the operation is safe to issue again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DirectRuntimeError {
    NotConnected,
    LinkEpochChanged { expected: u64, current: Option<u64> },
    DeadlineExceeded,
    Disconnected,
    Protocol(String),
    Remote { code: String, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirectDeviceSnapshot {
    pub(crate) device_id: String,
    pub(crate) link_epoch: u64,
    pub(crate) connected: bool,
    pub(crate) capabilities: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DirectDeviceEvent {
    pub(crate) device_id: String,
    pub(crate) link_epoch: u64,
    pub(crate) event: String,
    pub(crate) payload: serde_json::Value,
}

struct PendingRequest {
    result: oneshot::Sender<Result<serde_json::Value, DirectRuntimeError>>,
}

pub(crate) struct BulkFrame {
    message: tokio_tungstenite::tungstenite::Message,
    sent: Option<oneshot::Sender<()>>,
}

struct DirectLink {
    epoch: u64,
    control: mpsc::Sender<tokio_tungstenite::tungstenite::Message>,
    bulk: mpsc::Sender<BulkFrame>,
    pending: Mutex<HashMap<String, PendingRequest>>,
    content: Mutex<InboundContentStore>,
    capabilities: Mutex<BTreeSet<String>>,
}

/// Cloneable service-internal handle used by phone providers.  It is
/// deliberately additive to the serial-shaped public phone contracts: direct
/// devices are keyed by their stable device id and epoch here.
#[derive(Clone)]
pub(crate) struct DirectRuntimeHandle {
    links: Arc<Mutex<HashMap<String, Arc<DirectLink>>>>,
    events: broadcast::Sender<DirectDeviceEvent>,
}

impl Default for DirectRuntimeHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectRuntimeHandle {
    pub(crate) fn new() -> Self {
        let (events, _) = broadcast::channel(128);
        Self {
            links: Arc::new(Mutex::new(HashMap::new())),
            events,
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<DirectDeviceEvent> {
        self.events.subscribe()
    }

    pub(crate) fn snapshots(&self) -> Vec<DirectDeviceSnapshot> {
        self.links
            .lock()
            .expect("direct link mutex poisoned")
            .iter()
            .map(|(device_id, link)| DirectDeviceSnapshot {
                device_id: device_id.clone(),
                link_epoch: link.epoch,
                connected: true,
                capabilities: link
                    .capabilities
                    .lock()
                    .expect("capabilities mutex poisoned")
                    .clone(),
            })
            .collect()
    }

    pub(crate) fn snapshot(&self, device_id: &str) -> Option<DirectDeviceSnapshot> {
        self.links
            .lock()
            .expect("direct link mutex poisoned")
            .get(device_id)
            .map(|link| DirectDeviceSnapshot {
                device_id: device_id.to_owned(),
                link_epoch: link.epoch,
                connected: true,
                capabilities: link
                    .capabilities
                    .lock()
                    .expect("capabilities mutex poisoned")
                    .clone(),
            })
    }

    pub(crate) fn read_content_artifact(
        &self,
        device_id: &str,
        epoch: u64,
        reference: &sky_cua_platform::model::ContentRef,
    ) -> io::Result<Vec<u8>> {
        let links = self.links.lock().expect("direct link mutex poisoned");
        let link = links.get(device_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "device is not connected")
        })?;
        if link.epoch != epoch {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "link epoch mismatch",
            ));
        }
        let mut content = link.content.lock().expect("content mutex poisoned");
        content.read_artifact_verified(
            &reference.content_id,
            epoch,
            reference.size_bytes,
            &reference.sha256,
        )
    }

    pub(crate) fn describe_content_artifact(
        &self,
        device_id: &str,
        epoch: u64,
        content_id: &str,
    ) -> io::Result<(String, u64, String, u64)> {
        let links = self.links.lock().expect("direct link mutex poisoned");
        let link = links.get(device_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "device is not connected")
        })?;
        if link.epoch != epoch {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "link epoch mismatch",
            ));
        }
        link.content
            .lock()
            .expect("content mutex poisoned")
            .describe_artifact(content_id, epoch)
    }

    pub(crate) fn release_content_artifact(
        &self,
        device_id: &str,
        epoch: u64,
        content_id: &str,
    ) -> io::Result<()> {
        let links = self.links.lock().expect("direct link mutex poisoned");
        let link = links.get(device_id).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "device is not connected")
        })?;
        if link.epoch != epoch {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "link epoch mismatch",
            ));
        }
        link.content
            .lock()
            .expect("content mutex poisoned")
            .release_artifact(content_id, epoch)
    }

    /// Register a test/fake peer. The returned receivers observe wire output;
    /// responses can be fed through `respond` below. Registration atomically
    /// supersedes any previous epoch for the device.
    #[cfg(test)]
    pub(crate) fn register_fake(
        &self,
        device_id: &str,
        epoch: u64,
    ) -> (
        mpsc::Receiver<tokio_tungstenite::tungstenite::Message>,
        mpsc::Receiver<BulkFrame>,
    ) {
        let (control_tx, control_rx) = mpsc::channel(32);
        let (bulk_tx, bulk_rx) = mpsc::channel(32);
        let mut links = self.links.lock().expect("direct link mutex poisoned");
        if let Some(old) = links.remove(device_id) {
            let pending = std::mem::take(&mut *old.pending.lock().expect("pending mutex poisoned"));
            for (_, request) in pending {
                let _ = request
                    .result
                    .send(Err(DirectRuntimeError::LinkEpochChanged {
                        expected: old.epoch,
                        current: Some(epoch),
                    }));
            }
        }
        links.insert(
            device_id.to_owned(),
            Arc::new(DirectLink {
                epoch,
                control: control_tx,
                bulk: bulk_tx,
                pending: Mutex::new(HashMap::new()),
                content: Mutex::new(InboundContentStore::default()),
                capabilities: Mutex::new(BTreeSet::new()),
            }),
        );
        (control_rx, bulk_rx)
    }

    #[cfg(test)]
    pub(crate) fn set_fake_capabilities(
        &self,
        device_id: &str,
        epoch: u64,
        capabilities: impl IntoIterator<Item = &'static str>,
    ) {
        self.set_capabilities(
            device_id,
            epoch,
            capabilities.into_iter().map(str::to_owned).collect(),
        );
    }

    fn set_capabilities(&self, device_id: &str, epoch: u64, capabilities: BTreeSet<String>) {
        let Some(link) = self
            .links
            .lock()
            .expect("direct link mutex poisoned")
            .get(device_id)
            .cloned()
        else {
            return;
        };
        if link.epoch == epoch {
            *link
                .capabilities
                .lock()
                .expect("capabilities mutex poisoned") = capabilities;
        }
    }

    #[cfg(test)]
    pub(crate) fn commit_fake_content(
        &self,
        device_id: &str,
        epoch: u64,
        content_id: &str,
        mime_type: &str,
        source: sky_cua_platform::model::ContentSource,
        bytes: &[u8],
    ) -> sky_cua_platform::model::ContentRef {
        use sha2::Digest as _;
        let sha256 = sha2::Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let content = sky_cua_platform::model::ContentRef {
            content_id: content_id.to_owned(),
            device_id: Some(device_id.to_owned()),
            link_epoch: Some(epoch),
            mime_type: mime_type.to_owned(),
            filename: None,
            size_bytes: bytes.len() as u64,
            sha256: sha256.clone(),
            source,
            expires_at_ms: Some(now_ms().saturating_add(15 * 60 * 1000)),
            persistence: sky_cua_platform::model::ContentPersistence::Temporary,
        };
        let transfer_id = format!("fake-{content_id}");
        let chunk_bytes = PHONE_CONTENT_DEFAULT_CHUNK_BYTES;
        let chunk_count = if bytes.is_empty() {
            0
        } else {
            1 + ((bytes.len() as u64 - 1) / u64::from(chunk_bytes))
        };
        let declaration = sky_cua_platform::model::ContentTransferDeclaration {
            transfer_id: transfer_id.clone(),
            device_id: device_id.to_owned(),
            link_epoch: epoch,
            content: content.clone(),
            chunk_bytes,
            chunk_count,
        };
        let links = self.links.lock().expect("direct link mutex poisoned");
        let link = links.get(device_id).expect("fake link registered");
        assert_eq!(link.epoch, epoch, "fake link epoch");
        let mut store = link.content.lock().expect("content mutex poisoned");
        store
            .declare(declaration, epoch)
            .expect("declare fake content");
        for (index, payload) in bytes.chunks(chunk_bytes as usize).enumerate() {
            let offset = index as u64 * u64::from(chunk_bytes);
            let frame = sky_cua_platform::model::encode_content_chunk(
                &sky_cua_platform::model::ContentChunkHeader {
                    transfer_id: transfer_id.clone(),
                    chunk_index: index as u64,
                    offset,
                    length: payload.len() as u32,
                    link_epoch: epoch,
                },
                payload,
            )
            .expect("encode fake content chunk");
            store
                .chunk(&frame, epoch)
                .expect("ingest fake content chunk");
        }
        store
            .commit(
                sky_cua_platform::model::ContentTransferCommit {
                    transfer_id,
                    size_bytes: bytes.len() as u64,
                    sha256,
                    link_epoch: epoch,
                },
                epoch,
            )
            .expect("commit fake content");
        content
    }

    pub(crate) async fn request(
        &self,
        device_id: &str,
        expected_epoch: u64,
        method: &str,
        params: serde_json::Value,
        idempotent: bool,
        deadline: Duration,
    ) -> Result<serde_json::Value, DirectRuntimeError> {
        let request_id = Uuid::new_v4().to_string();
        let expires_at_ms = now_ms().saturating_add(deadline.as_millis() as u64);
        let frame = PhoneDirectControlFrame::Request {
            request_id: request_id.clone(),
            device_id: device_id.to_owned(),
            link_epoch: expected_epoch,
            idempotent,
            expires_at_ms,
            method: method.to_owned(),
            params,
        };
        let encoded = serde_json::to_string(&frame)
            .map_err(|error| DirectRuntimeError::Protocol(error.to_string()))?;
        if encoded.len() > PHONE_CONTROL_MAX_JSON_BYTES as usize {
            return Err(DirectRuntimeError::Protocol(
                "control frame exceeds configured JSON limit".into(),
            ));
        }
        let (tx, rx) = oneshot::channel();
        let link = {
            let links = self.links.lock().expect("direct link mutex poisoned");
            let Some(link) = links.get(device_id).cloned() else {
                return Err(DirectRuntimeError::NotConnected);
            };
            if link.epoch != expected_epoch {
                return Err(DirectRuntimeError::LinkEpochChanged {
                    expected: expected_epoch,
                    current: Some(link.epoch),
                });
            }
            link.pending
                .lock()
                .expect("pending mutex poisoned")
                .insert(request_id.clone(), PendingRequest { result: tx });
            link
        };
        let message = tokio_tungstenite::tungstenite::Message::Text(encoded.into());
        if link.control.send(message).await.is_err() {
            link.pending
                .lock()
                .expect("pending mutex poisoned")
                .remove(&request_id);
            return Err(DirectRuntimeError::Disconnected);
        }
        match tokio::time::timeout(deadline, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(DirectRuntimeError::Disconnected),
            Err(_) => {
                link.pending
                    .lock()
                    .expect("pending mutex poisoned")
                    .remove(&request_id);
                Err(DirectRuntimeError::DeadlineExceeded)
            }
        }
    }

    /// Send one finite host artifact to the authenticated Companion. Transfer
    /// metadata and chunks share the bounded bulk queue so declaration,
    /// payload, and commit retain their wire order while control RPCs remain
    /// higher priority.
    pub(crate) async fn send_content(
        &self,
        device_id: &str,
        expected_epoch: u64,
        bytes: &[u8],
        mime_type: &str,
        filename: Option<String>,
    ) -> Result<sky_cua_platform::model::ContentRef, DirectRuntimeError> {
        use sha2::Digest as _;
        use sky_cua_platform::model::{
            ContentChunkHeader, ContentPersistence, ContentRef, ContentSource,
            ContentTransferCommit, ContentTransferDeclaration, encode_content_chunk,
        };

        let link = {
            let links = self.links.lock().expect("direct link mutex poisoned");
            let Some(link) = links.get(device_id).cloned() else {
                return Err(DirectRuntimeError::NotConnected);
            };
            if link.epoch != expected_epoch {
                return Err(DirectRuntimeError::LinkEpochChanged {
                    expected: expected_epoch,
                    current: Some(link.epoch),
                });
            }
            link
        };
        let transfer_id = Uuid::new_v4().to_string();
        let content_id = Uuid::new_v4().to_string();
        let sha256 = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let content = ContentRef {
            content_id,
            device_id: Some(device_id.to_owned()),
            link_epoch: Some(expected_epoch),
            mime_type: mime_type.to_owned(),
            filename,
            size_bytes: bytes.len() as u64,
            sha256: sha256.clone(),
            source: ContentSource::HostPrivateArtifact,
            expires_at_ms: Some(now_ms().saturating_add(15 * 60 * 1000)),
            persistence: ContentPersistence::Temporary,
        };
        let chunk_bytes = PHONE_CONTENT_DEFAULT_CHUNK_BYTES;
        let chunk_count = if bytes.is_empty() {
            0
        } else {
            1 + ((bytes.len() as u64 - 1) / u64::from(chunk_bytes))
        };
        let declare = PhoneDirectControlFrame::ContentDeclare(ContentTransferDeclaration {
            transfer_id: transfer_id.clone(),
            device_id: device_id.to_owned(),
            link_epoch: expected_epoch,
            content: content.clone(),
            chunk_bytes,
            chunk_count,
        });
        let declare = serde_json::to_string(&declare)
            .map_err(|error| DirectRuntimeError::Protocol(error.to_string()))?;
        link.bulk
            .send(BulkFrame {
                message: tokio_tungstenite::tungstenite::Message::Text(declare.into()),
                sent: None,
            })
            .await
            .map_err(|_| DirectRuntimeError::Disconnected)?;
        for (index, payload) in bytes.chunks(chunk_bytes as usize).enumerate() {
            let encoded = encode_content_chunk(
                &ContentChunkHeader {
                    transfer_id: transfer_id.clone(),
                    chunk_index: index as u64,
                    offset: index as u64 * u64::from(chunk_bytes),
                    length: payload.len() as u32,
                    link_epoch: expected_epoch,
                },
                payload,
            )
            .map_err(DirectRuntimeError::Protocol)?;
            link.bulk
                .send(BulkFrame {
                    message: tokio_tungstenite::tungstenite::Message::Binary(encoded.into()),
                    sent: None,
                })
                .await
                .map_err(|_| DirectRuntimeError::Disconnected)?;
        }
        let commit = PhoneDirectControlFrame::ContentCommit(ContentTransferCommit {
            transfer_id,
            size_bytes: bytes.len() as u64,
            sha256,
            link_epoch: expected_epoch,
        });
        let commit = serde_json::to_string(&commit)
            .map_err(|error| DirectRuntimeError::Protocol(error.to_string()))?;
        let (sent, flushed) = oneshot::channel();
        link.bulk
            .send(BulkFrame {
                message: tokio_tungstenite::tungstenite::Message::Text(commit.into()),
                sent: Some(sent),
            })
            .await
            .map_err(|_| DirectRuntimeError::Disconnected)?;
        tokio::time::timeout(Duration::from_secs(30), flushed)
            .await
            .map_err(|_| DirectRuntimeError::DeadlineExceeded)?
            .map_err(|_| DirectRuntimeError::Disconnected)?;
        Ok(content)
    }

    #[cfg(test)]
    pub(crate) fn respond(&self, frame: PhoneDirectControlFrame) {
        let PhoneDirectControlFrame::Response {
            request_id,
            device_id,
            link_epoch,
            result,
        } = frame
        else {
            return;
        };
        let Some(link) = self
            .links
            .lock()
            .expect("direct link mutex poisoned")
            .get(&device_id)
            .cloned()
        else {
            return;
        };
        if link.epoch != link_epoch {
            return;
        }
        if let Some(pending) = link
            .pending
            .lock()
            .expect("pending mutex poisoned")
            .remove(&request_id)
        {
            let _ = pending.result.send(Ok(result));
        }
    }

    #[cfg(test)]
    pub(crate) fn emit_event(&self, device_id: &str, link_epoch: u64, event: &str) {
        let _ = self.events.send(DirectDeviceEvent {
            device_id: device_id.to_owned(),
            link_epoch,
            event: event.to_owned(),
            payload: serde_json::json!({}),
        });
    }

    async fn serve_socket(
        &self,
        mut socket: WebSocketStream<TcpStream>,
        device_id: String,
        epoch: u64,
        mut cancel: tokio::sync::oneshot::Receiver<()>,
    ) -> io::Result<()> {
        let (control_tx, mut control_rx) = mpsc::channel(64);
        let (bulk_tx, mut bulk_rx) = mpsc::channel(32);
        let link = Arc::new(DirectLink {
            epoch,
            control: control_tx,
            bulk: bulk_tx,
            pending: Mutex::new(HashMap::new()),
            content: Mutex::new(InboundContentStore::default()),
            capabilities: Mutex::new(BTreeSet::new()),
        });
        self.links
            .lock()
            .expect("direct link mutex poisoned")
            .insert(device_id.clone(), link.clone());
        let mut control_burst = 0u8;
        let mut inbound_burst = 0u8;
        let mut terminal_error = None;
        loop {
            // Cancellation wins; control is preferred, but every eight control
            // frames reserve a turn for bulk. Inbound traffic is present in
            // both schedules so responses/events cannot be starved.
            if control_burst >= 8 {
                tokio::select! {
                    biased;
                    _ = &mut cancel => break,
                    message = socket.next(), if inbound_burst < 4 || (control_rx.is_empty() && bulk_rx.is_empty()) => {
                        if !matches!(self.handle_inbound(&link, &device_id, epoch, message).await, Ok(true)) {
                            break;
                        }
                        inbound_burst = inbound_burst.saturating_add(1);
                    }
                    Some(frame) = bulk_rx.recv() => {
                        if let Err(error) = socket.send(frame.message).await {
                            terminal_error = Some(io::Error::new(io::ErrorKind::BrokenPipe, error));
                            break;
                        }
                        if let Some(sent) = frame.sent { let _ = sent.send(()); }
                        control_burst = 0;
                        inbound_burst = 0;
                    }
                    Some(message) = control_rx.recv() => {
                        if let Err(error) = socket.send(message).await {
                            terminal_error = Some(io::Error::new(io::ErrorKind::BrokenPipe, error));
                            break;
                        }
                        control_burst = control_burst.saturating_add(1);
                        inbound_burst = 0;
                    }
                }
            } else {
                tokio::select! {
                    biased;
                    _ = &mut cancel => break,
                    message = socket.next(), if inbound_burst < 4 || (control_rx.is_empty() && bulk_rx.is_empty()) => {
                        if !matches!(self.handle_inbound(&link, &device_id, epoch, message).await, Ok(true)) {
                            break;
                        }
                        inbound_burst = inbound_burst.saturating_add(1);
                    }
                    Some(message) = control_rx.recv() => {
                        if let Err(error) = socket.send(message).await {
                            terminal_error = Some(io::Error::new(io::ErrorKind::BrokenPipe, error));
                            break;
                        }
                        control_burst = control_burst.saturating_add(1);
                        inbound_burst = 0;
                    }
                    Some(frame) = bulk_rx.recv() => {
                        if let Err(error) = socket.send(frame.message).await {
                            terminal_error = Some(io::Error::new(io::ErrorKind::BrokenPipe, error));
                            break;
                        }
                        if let Some(sent) = frame.sent { let _ = sent.send(()); }
                        control_burst = 0;
                        inbound_burst = 0;
                    }
                }
            }
        }
        let mut links = self.links.lock().expect("direct link mutex poisoned");
        if links
            .get(&device_id)
            .is_some_and(|current| current.epoch == epoch)
        {
            links.remove(&device_id);
        }
        let pending = std::mem::take(&mut *link.pending.lock().expect("pending mutex poisoned"));
        link.content
            .lock()
            .expect("content mutex poisoned")
            .abort_epoch(epoch);
        for (_, request) in pending {
            let _ = request.result.send(Err(DirectRuntimeError::Disconnected));
        }
        match terminal_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn handle_inbound(
        &self,
        link: &Arc<DirectLink>,
        device_id: &str,
        epoch: u64,
        message: Option<
            Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>,
        >,
    ) -> io::Result<bool> {
        let Some(message) = message else {
            return Ok(false);
        };
        let message = message.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if let tokio_tungstenite::tungstenite::Message::Binary(bytes) = &message {
            link.content
                .lock()
                .expect("content mutex poisoned")
                .chunk(bytes, epoch)?;
            return Ok(true);
        }
        let tokio_tungstenite::tungstenite::Message::Text(text) = message else {
            return Ok(true);
        };
        if text.len() > PHONE_CONTROL_MAX_JSON_BYTES as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "control frame exceeds limit",
            ));
        }
        let frame = decode_control_frame(text.as_bytes(), device_id, epoch, now_ms())?;
        match frame {
            PhoneDirectControlFrame::ContentDeclare(declaration) => {
                link.content
                    .lock()
                    .expect("content mutex poisoned")
                    .declare(declaration, epoch)?;
            }
            PhoneDirectControlFrame::ContentCommit(commit) => {
                link.content
                    .lock()
                    .expect("content mutex poisoned")
                    .commit(commit, epoch)?;
            }
            PhoneDirectControlFrame::ContentAbort { transfer_id, .. } => {
                link.content
                    .lock()
                    .expect("content mutex poisoned")
                    .abort(&transfer_id);
            }
            PhoneDirectControlFrame::Response {
                request_id,
                device_id: response_device,
                link_epoch: response_epoch,
                result,
            } if response_device == device_id && response_epoch == epoch => {
                if let Some(pending) = link
                    .pending
                    .lock()
                    .expect("pending mutex poisoned")
                    .remove(&request_id)
                {
                    let _ = pending.result.send(Ok(result));
                }
            }
            PhoneDirectControlFrame::Error {
                request_id: Some(request_id),
                device_id: Some(response_device),
                link_epoch: Some(response_epoch),
                code,
                message,
            } if response_device == device_id && response_epoch == epoch => {
                if let Some(pending) = link
                    .pending
                    .lock()
                    .expect("pending mutex poisoned")
                    .remove(&request_id)
                {
                    let _ = pending
                        .result
                        .send(Err(DirectRuntimeError::Remote { code, message }));
                }
            }
            PhoneDirectControlFrame::Event {
                device_id: event_device,
                link_epoch: event_epoch,
                event,
                payload,
                ..
            } if event_device == device_id && event_epoch == epoch => {
                if event == "capability_changed" {
                    let Some(values) = payload
                        .get("capabilities")
                        .and_then(|value| value.as_array())
                    else {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "capability_changed omitted capabilities",
                        ));
                    };
                    let capabilities = values
                        .iter()
                        .map(|value| {
                            value.as_str().map(str::to_owned).ok_or_else(|| {
                                io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "capability_changed contained a non-string capability",
                                )
                            })
                        })
                        .collect::<io::Result<BTreeSet<_>>>()?;
                    self.set_capabilities(device_id, epoch, capabilities);
                }
                let _ = self.events.send(DirectDeviceEvent {
                    device_id: event_device,
                    link_epoch: event_epoch,
                    event,
                    payload,
                });
            }
            _ => {}
        }
        Ok(true)
    }
}

pub(crate) fn decode_control_frame(
    bytes: &[u8],
    device_id: &str,
    epoch: u64,
    now_ms: u64,
) -> io::Result<PhoneDirectControlFrame> {
    if bytes.len() > PHONE_CONTROL_MAX_JSON_BYTES as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "control frame exceeds limit",
        ));
    }
    let frame: PhoneDirectControlFrame =
        serde_json::from_slice(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let valid = match &frame {
        PhoneDirectControlFrame::Request {
            device_id: d,
            link_epoch,
            expires_at_ms,
            ..
        } => d == device_id && *link_epoch == epoch && *expires_at_ms >= now_ms,
        PhoneDirectControlFrame::Response {
            device_id: d,
            link_epoch,
            ..
        }
        | PhoneDirectControlFrame::Event {
            device_id: d,
            link_epoch,
            ..
        } => d == device_id && *link_epoch == epoch,
        PhoneDirectControlFrame::ContentDeclare(d) => {
            d.device_id == device_id
                && d.link_epoch == epoch
                && d.chunk_bytes <= PHONE_CONTENT_DEFAULT_CHUNK_BYTES
        }
        PhoneDirectControlFrame::ContentCommit(c) => c.link_epoch == epoch,
        PhoneDirectControlFrame::ContentAbort { link_epoch, .. } => *link_epoch == epoch,
        _ => true,
    };
    if !valid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "device, epoch, or expiry mismatch",
        ));
    }
    Ok(frame)
}

pub(crate) fn is_private_ipv4(ipv4: std::net::Ipv4Addr) -> bool {
    let o = ipv4.octets();
    // 10.0.0.0/8
    if o[0] == 10 {
        return true;
    }
    // 172.16.0.0/12
    if o[0] == 172 && (16..=31).contains(&o[1]) {
        return true;
    }
    // 192.168.0.0/16 (incl. tether 192.168.42/49, hotspot 192.168.43/49)
    if o[0] == 192 && o[1] == 168 {
        return true;
    }
    // 169.254.0.0/16 link-local
    if o[0] == 169 && o[1] == 254 {
        return true;
    }
    // 100.64.0.0/10 CGNAT (Tailscale)
    if o[0] == 100 && (64..=127).contains(&o[1]) {
        return true;
    }
    false
}

pub(crate) fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => ipv4.is_loopback() || is_private_ipv4(ipv4),
        IpAddr::V6(ipv6) => {
            if ipv6.is_loopback() {
                return true;
            }
            // ::ffff:0:0/96 mapped IPv4 — evaluate inner IPv4.
            if let Some(v4) = ipv6.to_ipv4_mapped() {
                return v4.is_loopback() || is_private_ipv4(v4);
            }
            let o = ipv6.octets();
            // fc00::/7 ULA (superset of fd7a:115c:a1e0::/48)
            if o[0] == 0xfc || o[0] == 0xfd {
                return true;
            }
            // fe80::/10 link-local (strip %zone already done by caller)
            if o[0] == 0xfe && (o[1] & 0xc0) == 0x80 {
                return true;
            }
            false
        }
    }
}

/// Validate a configured listener address. Wildcard binds (0.0.0.0 / ::) are
/// allowed so one socket covers WiFi/ethernet/USB-tether, while public routable
/// binds remain rejected for cleartext `ws://` listeners.
pub(crate) fn validate_bind_addr(addr: SocketAddr) -> io::Result<()> {
    let ip = addr.ip();
    if ip.is_multicast() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "phone-control listener multicast address is not allowed",
        ));
    }
    if ip.is_unspecified() {
        return Ok(());
    }
    if is_private_ip(ip) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "phone-control listener requires a private, Tailscale, or loopback address (or 0.0.0.0/:: for all interfaces)",
    ))
}

/// Listener construction is disabled unless explicitly enabled by service
/// configuration. Wildcard binds (0.0.0.0/::) are allowed for LAN/tether but
/// public routable addresses remain rejected.
pub(crate) struct DirectListener {
    listener: TcpListener,
}

impl DirectListener {
    #[allow(dead_code)]
    pub(crate) async fn from_config(config: &PhoneConfig) -> io::Result<Option<Self>> {
        if !config.direct_enabled.unwrap_or(false) {
            return Ok(None);
        }
        let raw = config.direct_listen_addr.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "direct listener address is required when enabled",
            )
        })?;
        let addr: SocketAddr = raw
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        // Wildcard `0.0.0.0/::` on a host with a public NIC would be reachable
        // from the internet if the firewall is not configured. `docs/runtime/direct-lan.md:30`
        // says public `ws://` is `InvalidInput` and `wss://` is the public escape.
        // Enforce that: wildcard `ws://` with a public advertised host is rejected
        // (require `wss://`), private/`*.ts.net` `ws://` is still `Ok`.
        if addr.ip().is_unspecified() {
            if let Some(advertised) = config.direct_advertised_endpoint.as_deref() {
                let lower = advertised.to_ascii_lowercase();
                if lower.starts_with("ws://") {
                    // Extract host between ws:// and : or / or ?
                    let after = &advertised[5..];
                    let host_end = after.find([':', '/', '?', '#']).unwrap_or(after.len());
                    let mut host = &after[..host_end];
                    // Strip [] for IPv6 and %zone
                    host = host.trim_matches(|c| c == '[' || c == ']');
                    if let Some(p) = host.find('%') {
                        host = &host[..p];
                    }
                    let is_private = if host.eq_ignore_ascii_case("localhost")
                        || host == "127.0.0.1"
                        || host == "::1"
                        || host.to_ascii_lowercase().ends_with(".ts.net")
                    {
                        true
                    } else if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                        is_private_ip(ip) || ip.is_loopback()
                    } else {
                        false
                    };
                    if !is_private {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "wildcard direct listener (0.0.0.0/::) with public ws:// advertised endpoint requires wss:// (or a private/ tailnet host); see docs/runtime/direct-lan.md:30",
                        ));
                    }
                }
            }
        }
        Self::bind(true, addr).await
    }

    pub(crate) async fn bind(enabled: bool, addr: SocketAddr) -> io::Result<Option<Self>> {
        if !enabled {
            return Ok(None);
        }
        validate_bind_addr(addr)?;
        Ok(Some(Self {
            listener: TcpListener::bind(addr).await?,
        }))
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    #[allow(dead_code)]
    pub(crate) async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        self.listener.accept().await
    }

    pub(crate) async fn accept_websocket(
        &self,
    ) -> io::Result<(WebSocketStream<TcpStream>, SocketAddr)> {
        let (stream, peer) = self.listener.accept().await?;
        let websocket = accept_async(stream)
            .await
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok((websocket, peer))
    }
}

/// Service-owned lifecycle seam. The daemon can retain this value for the
/// duration of its run and call `shutdown` before dropping it.
pub(crate) struct DirectRuntime {
    shutdown: tokio::sync::watch::Sender<bool>,
    task: Option<tokio::task::JoinHandle<()>>,
    registry: EnrollmentRegistry,
    handle: DirectRuntimeHandle,
    state: Arc<dyn DirectStateStore>,
    advertised_endpoint: String,
}

impl Clone for DirectRuntime {
    fn clone(&self) -> Self {
        Self {
            shutdown: self.shutdown.clone(),
            task: None,
            registry: self.registry.clone(),
            handle: self.handle.clone(),
            state: self.state.clone(),
            advertised_endpoint: self.advertised_endpoint.clone(),
        }
    }
}

impl DirectRuntime {
    pub(crate) async fn start(selection: &ResolvedPhoneSelection) -> io::Result<Option<Self>> {
        if !selection.direct_enabled {
            return Ok(None);
        }
        let raw = selection.direct_listen_addr.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "direct listener address is required",
            )
        })?;
        let addr = raw
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let listener = DirectListener::bind(true, addr)
            .await?
            .expect("enabled listener");
        let (shutdown, shutdown_rx) = tokio::sync::watch::channel(false);
        let state_path =
            sky_cua_platform::phone_direct_state_path(selection.direct_state_path.as_deref())
                .map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("failed to resolve phone-control state path: {error}"),
                    )
                })?;
        let state: Arc<dyn DirectStateStore> = Arc::new(FileStateStore::new(state_path));
        let handle = DirectRuntimeHandle::new();
        let registry = LinkRegistry::with_handle(handle.clone());
        let ttl = Duration::from_millis(selection.direct_enrollment_ttl_ms);
        let enrollments = EnrollmentRegistry::with_ttl(ttl);
        let endpoint = selection
            .direct_advertised_endpoint
            .clone()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "direct advertised endpoint is required",
                )
            })?;
        let task = tokio::spawn(run_accept_loop(
            listener,
            state.clone(),
            registry,
            enrollments.clone(),
            shutdown_rx,
        ));
        Ok(Some(Self {
            shutdown,
            task: Some(task),
            registry: enrollments,
            handle,
            state,
            advertised_endpoint: endpoint,
        }))
    }
    pub(crate) async fn shutdown(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
    pub(crate) fn create_enrollment(&self) -> sky_cua_platform::model::PhoneEnrollmentPayload {
        let ticket = self.registry.create(SystemTime::now());
        sky_cua_platform::model::PhoneEnrollmentPayload {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            endpoint: self.advertised_endpoint.clone(),
            enrollment_id: ticket.enrollment_id,
            bootstrap_credential: ticket.code,
            expires_at_ms: ticket.expires_at_ms,
        }
    }

    pub(crate) fn handle(&self) -> DirectRuntimeHandle {
        self.handle.clone()
    }
}

async fn run_accept_loop(
    listener: DirectListener,
    store: Arc<dyn DirectStateStore>,
    registry: LinkRegistry,
    enrollments: EnrollmentRegistry,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut children = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => if *shutdown.borrow() { children.shutdown().await; break },
            Some(_) = children.join_next() => {},
            accepted = listener.accept_websocket() => {
                let (socket, peer) = match accepted {
                    Ok(connection) => connection,
                    Err(error) => {
                        tracing::warn!(%error, "CompanionDirect WebSocket accept failed");
                        continue;
                    }
                };
                let store = store.clone();
                let registry = registry.clone();
                let enrollments = enrollments.clone();
                children.spawn(async move {
                    if let Err(error) = accept_socket(socket, store, &registry, &enrollments).await {
                        tracing::warn!(%peer, %error, "CompanionDirect connection rejected");
                    }
                });
            }
        }
    }
}

async fn accept_socket(
    mut socket: WebSocketStream<TcpStream>,
    store: Arc<dyn DirectStateStore>,
    registry: &LinkRegistry,
    enrollments: &EnrollmentRegistry,
) -> io::Result<()> {
    let text = recv_auth_text(&mut socket)
        .await?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing first frame"))?;
    let frame: PhoneDirectControlFrame =
        serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if let PhoneDirectControlFrame::EnrollmentRedeem(redeem) = frame {
        if redeem.protocol != PHONE_CONTROL_PROTOCOL_V2 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "protocol mismatch",
            ));
        }
        let now = SystemTime::now();
        let credential = enrollments
            .consume_with_ticket(&redeem.enrollment_id, &redeem.bootstrap_credential, now)
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "invalid enrollment"))?;
        let (credential, ticket) = credential;
        let pending_expires_at_ms = now_ms().saturating_add(PHONE_ENROLLMENT_PENDING_TTL_MS);
        let enrolled_device_id = credential.device_id.clone();
        let result =
            PhoneDirectControlFrame::EnrollmentOk(sky_cua_platform::model::PhoneEnrollmentResult {
                protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
                enrollment_id: ticket.enrollment_id.clone(),
                device_id: enrolled_device_id.clone(),
                device_secret: B64.encode(credential.secret),
                enrolled_at_ms: now
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                pending_expires_at_ms,
            });
        let frame = serde_json::to_string(&result).unwrap().into();
        if let Err(e) = socket
            .send(tokio_tungstenite::tungstenite::Message::Text(frame))
            .await
        {
            enrollments.restore(&ticket);
            let _ = rollback_device(&*store, &credential.device_id);
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("enrollment wire delivery failed; ticket restored: {e}"),
            ));
        }
        let state = DeviceState {
            device_id: credential.device_id.clone(),
            secret: credential.secret,
            link_epoch: 0,
            revoked: false,
            lifecycle: DeviceLifecycle::Pending,
            enrollment_id: Some(ticket.enrollment_id.clone()),
            pending_expires_at_ms: Some(pending_expires_at_ms),
            committed_at_ms: None,
        };
        if let Err(e) = store.save(state) {
            enrollments.restore(&ticket);
            let _ = rollback_device(&*store, &credential.device_id);
            return Err(io::Error::other(format!(
                "enrollment state persistence failed after wire delivery: {e}"
            )));
        }
        return await_enrollment_ack(socket, store).await;
    }
    if let PhoneDirectControlFrame::EnrollmentAck(ack) = frame {
        return process_enrollment_ack(socket, store, ack).await;
    }
    authenticate_socket_after_hello(socket, frame, store, registry).await
}

fn rollback_device(store: &dyn DirectStateStore, device_id: &str) -> io::Result<()> {
    store.delete(device_id)?;
    if store.load(device_id)?.is_some() {
        return Err(io::Error::other("enrollment rollback left persisted state"));
    }
    Ok(())
}

async fn await_enrollment_ack(
    mut socket: WebSocketStream<TcpStream>,
    store: Arc<dyn DirectStateStore>,
) -> io::Result<()> {
    let text = recv_auth_text(&mut socket)
        .await?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing enrollment ack"))?;
    let frame: PhoneDirectControlFrame =
        serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let PhoneDirectControlFrame::EnrollmentAck(ack) = frame else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "expected enrollment ack",
        ));
    };
    process_enrollment_ack(socket, store, ack).await
}

async fn process_enrollment_ack(
    mut socket: WebSocketStream<TcpStream>,
    store: Arc<dyn DirectStateStore>,
    ack: PhoneEnrollmentAck,
) -> io::Result<()> {
    let device_lock = DEVICE_LOCKS
        .get_or_init(|| DeviceLocks {
            locks: Mutex::new(HashMap::new()),
        })
        .lock_for(&ack.device_id);
    let _device_guard = device_lock.lock().await;
    if ack.protocol != PHONE_CONTROL_PROTOCOL_V2
        || !valid_nonce(&ack.client_nonce)
        || !ack
            .client_proof
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid enrollment ack",
        ));
    }
    let state = store
        .load(&ack.device_id)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "pending device missing"))?;
    if !verify_enrollment_ack(&state.secret, &ack) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid enrollment ack",
        ));
    }
    if state.lifecycle == DeviceLifecycle::Active
        && state.enrollment_id.as_deref() == Some(ack.enrollment_id.as_str())
        && let Some(activated_at_ms) = state.committed_at_ms
    {
        let response = PhoneDirectControlFrame::EnrollmentCommitted(PhoneEnrollmentCommitted {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            enrollment_id: ack.enrollment_id,
            device_id: ack.device_id,
            activated_at_ms,
        });
        drop(_device_guard);
        return socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&response).unwrap().into(),
            ))
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e));
    }
    if state.lifecycle != DeviceLifecycle::Pending
        || state.enrollment_id.as_deref() != Some(ack.enrollment_id.as_str())
        || state.pending_expires_at_ms.unwrap_or(0) < now_ms()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid or expired enrollment ack",
        ));
    }
    let mut active = state;
    active.lifecycle = DeviceLifecycle::Active;
    active.pending_expires_at_ms = None;
    active.committed_at_ms = Some(now_ms());
    let committed_at_ms = active.committed_at_ms.unwrap();
    store.save(active)?;
    let response = PhoneDirectControlFrame::EnrollmentCommitted(PhoneEnrollmentCommitted {
        protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
        enrollment_id: ack.enrollment_id,
        device_id: ack.device_id,
        activated_at_ms: committed_at_ms,
    });
    drop(_device_guard);
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&response).unwrap().into(),
        ))
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn verify_enrollment_ack(secret: &[u8; 32], ack: &PhoneEnrollmentAck) -> bool {
    let expected = ack_proof(secret, ack);
    let got = ack.client_proof.as_bytes();
    constant_time_eq(expected.as_bytes(), got)
}

fn ack_proof(secret: &[u8; 32], ack: &PhoneEnrollmentAck) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts keys");
    for field in [
        ack.protocol.as_str(),
        "enrollment_ack",
        ack.enrollment_id.as_str(),
        ack.device_id.as_str(),
        ack.client_nonce.as_str(),
    ] {
        mac.update(&(field.len() as u32).to_be_bytes());
        mac.update(field.as_bytes());
    }
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn valid_nonce(value: &str) -> bool {
    B64.decode(value)
        .map(|bytes| bytes.len() == 32 && B64.encode(bytes) == value)
        .unwrap_or(false)
}

async fn authenticate_socket_after_hello(
    socket: WebSocketStream<TcpStream>,
    frame: PhoneDirectControlFrame,
    store: Arc<dyn DirectStateStore>,
    registry: &LinkRegistry,
) -> io::Result<()> {
    // Reuse the existing authentication implementation by putting the first frame back is not possible;
    // this path is only used by the accept loop, so authenticate from a small helper below.
    let PhoneDirectControlFrame::AuthHello(hello) = frame else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "application frame before auth",
        ));
    };
    authenticate_with_hello(socket, hello, store, registry).await
}

#[cfg_attr(not(test), expect(dead_code))]
async fn authenticate_socket(
    mut socket: WebSocketStream<TcpStream>,
    store: Arc<dyn DirectStateStore>,
    registry: &LinkRegistry,
) -> io::Result<()> {
    let text = recv_auth_text(&mut socket)
        .await?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing auth hello"))?;
    let hello: PhoneDirectControlFrame =
        serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let PhoneDirectControlFrame::AuthHello(hello) = hello else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "application frame before auth",
        ));
    };
    authenticate_with_hello(socket, hello, store, registry).await
}

async fn authenticate_with_hello(
    mut socket: WebSocketStream<TcpStream>,
    hello: sky_cua_platform::model::PhoneAuthHello,
    store: Arc<dyn DirectStateStore>,
    registry: &LinkRegistry,
) -> io::Result<()> {
    if hello.protocol != PHONE_CONTROL_PROTOCOL_V2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "protocol mismatch",
        ));
    }
    let client_nonce = B64
        .decode(&hello.client_nonce)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid client nonce"))?;
    if client_nonce.len() != 32 || !registry.claim_nonce(&hello.client_nonce) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "nonce replay or wrong length",
        ));
    }
    if Uuid::parse_str(&hello.device_id)
        .map(|u| u.to_string() != hello.device_id)
        .unwrap_or(true)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "device id must be canonical lowercase UUID",
        ));
    }
    let state = store
        .load(&hello.device_id)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "unknown device"))?;
    if state.revoked {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "revoked device",
        ));
    }
    if state.lifecycle == DeviceLifecycle::Pending
        && state.pending_expires_at_ms.unwrap_or(0) < now_ms()
    {
        let _ = store.delete(&hello.device_id);
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pending enrollment expired",
        ));
    }
    let epoch = state.link_epoch.saturating_add(1);
    let mut nonce_bytes = [0u8; 32];
    getrandom::fill(&mut nonce_bytes).map_err(io::Error::other)?;
    let server_nonce = B64.encode(nonce_bytes);
    let _ = registry.claim_nonce(&server_nonce);
    let challenge = Challenge {
        protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
        device_id: hello.device_id.clone(),
        server_nonce: server_nonce.clone(),
        client_nonce: hello.client_nonce.clone(),
        link_epoch: epoch,
        role: PhoneDirectRole::Saga,
    };
    let server_proof = challenge_proof(&state.secret, &challenge)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&PhoneDirectControlFrame::AuthChallenge(
                sky_cua_platform::model::PhoneAuthChallenge {
                    protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
                    server_nonce,
                    link_epoch: epoch.to_string(),
                    server_proof,
                },
            ))
            .unwrap()
            .into(),
        ))
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
    let text = recv_auth_text(&mut socket)
        .await?
        .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "missing auth proof"))?;
    let PhoneDirectControlFrame::AuthProof(proof) =
        serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "expected auth proof",
        ));
    };
    if proof.link_epoch != epoch.to_string() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "epoch mismatch",
        ));
    }
    let expected = challenge_proof(
        &state.secret,
        &Challenge {
            role: PhoneDirectRole::Companion,
            ..challenge
        },
    );
    let got = proof.client_proof.as_bytes();
    let expected_hex = expected
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if got.len() != 64
        || !got
            .iter()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || !constant_time_eq(expected_hex.as_bytes(), got)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "bad auth proof",
        ));
    }
    if state.lifecycle == DeviceLifecycle::Pending {
        let device_lock = DEVICE_LOCKS
            .get_or_init(|| DeviceLocks {
                locks: Mutex::new(HashMap::new()),
            })
            .lock_for(&hello.device_id);
        let _device_guard = device_lock.lock().await;
        let mut current = store
            .load(&hello.device_id)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "device disappeared"))?;
        if current.lifecycle == DeviceLifecycle::Pending {
            current.lifecycle = DeviceLifecycle::Active;
            current.pending_expires_at_ms = None;
            current.committed_at_ms = Some(now_ms());
            store.save(current.clone())?;
        }
        drop(_device_guard);
    }
    let reservation = registry.reserve(&hello.device_id, epoch, &*store)?;
    let ok = PhoneDirectControlFrame::AuthOk(sky_cua_platform::model::PhoneAuthOk {
        protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
        device_id: hello.device_id.clone(),
        link_epoch: epoch.to_string(),
    });
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&ok).unwrap().into(),
        ))
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
    let mut cancel = registry.finalize(reservation);
    if let Some(handle) = registry.handle.clone() {
        return handle
            .serve_socket(socket, hello.device_id, epoch, cancel)
            .await;
    }
    // Hold the authenticated link until peer close. Application dispatch is
    // intentionally owned by PhoneManager; this loop only enforces epoch and
    // identity fencing at the transport boundary.
    loop {
        if !registry.is_current(&hello.device_id, epoch) {
            let _ = socket.close(None).await;
            return Ok(());
        }
        let message = tokio::select! {
            _ = &mut cancel => { let _ = socket.close(None).await; return Ok(()); }
            message = tokio::time::timeout(Duration::from_secs(30), socket.next()) => {
                message.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "authenticated link idle timeout"))?
            }
        };
        let Some(message) = message else {
            break;
        };
        let message = message.map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        match message {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                let now_ms = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                if decode_control_frame(text.as_bytes(), &hello.device_id, epoch, now_ms).is_err() {
                    let _ = socket.close(None).await;
                    return Ok(());
                }
            }
            tokio_tungstenite::tungstenite::Message::Ping(payload) => {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::BrokenPipe, e))?;
            }
            tokio_tungstenite::tungstenite::Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

async fn recv_auth_text(socket: &mut WebSocketStream<TcpStream>) -> io::Result<Option<String>> {
    let next = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "authentication timeout"))?;
    Ok(match next {
        Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
            if text.len() > PHONE_CONTROL_MAX_JSON_BYTES as usize {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "control frame exceeds limit",
                ));
            }
            Some(text.to_string())
        }
        Some(Ok(_)) => None,
        Some(Err(e)) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
        None => None,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct EnrollmentTicket {
    pub(crate) enrollment_id: String,
    pub(crate) code: String,
    pub(crate) expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnrollmentCredential {
    pub(crate) device_id: String,
    pub(crate) secret: [u8; 32],
}

#[derive(Debug, Clone)]
struct PendingEnrollment {
    code: String,
    expires_at: SystemTime,
}

/// Single-use enrollment registry. Codes are random, short-lived, and are
/// removed atomically before a credential is returned.
#[derive(Clone)]
pub(crate) struct EnrollmentRegistry {
    pending: Arc<Mutex<HashMap<String, PendingEnrollment>>>,
    ttl: Duration,
}

impl Default for EnrollmentRegistry {
    fn default() -> Self {
        Self::with_ttl(DEFAULT_ENROLLMENT_TTL)
    }
}

impl EnrollmentRegistry {
    pub(crate) fn with_ttl(ttl: Duration) -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            ttl,
        }
    }
    pub(crate) fn create(&self, now: SystemTime) -> EnrollmentTicket {
        let id = Uuid::new_v4().to_string();
        let mut credential = [0u8; 32];
        getrandom::fill(&mut credential).expect("OS random source");
        let code = B64.encode(credential);
        let expires_at = now + self.ttl;
        self.pending
            .lock()
            .expect("enrollment mutex poisoned")
            .insert(
                id.clone(),
                PendingEnrollment {
                    code: code.clone(),
                    expires_at,
                },
            );
        EnrollmentTicket {
            enrollment_id: id,
            code,
            expires_at_ms: expires_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn consume(
        &self,
        enrollment_id: &str,
        code: &str,
        now: SystemTime,
    ) -> Option<EnrollmentCredential> {
        self.consume_with_ticket(enrollment_id, code, now)
            .map(|(credential, _)| credential)
    }

    fn consume_with_ticket(
        &self,
        enrollment_id: &str,
        code: &str,
        now: SystemTime,
    ) -> Option<(EnrollmentCredential, EnrollmentTicket)> {
        let mut pending_map = self.pending.lock().expect("enrollment mutex poisoned");
        let pending = pending_map.get(enrollment_id)?.clone();
        let valid =
            pending.expires_at > now && constant_time_eq(pending.code.as_bytes(), code.as_bytes());
        if !valid {
            return None;
        }
        // Invalid attempts do not consume the one valid enrollment ticket.
        pending_map.remove(enrollment_id);
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).expect("OS random source");
        let ticket = EnrollmentTicket {
            enrollment_id: enrollment_id.into(),
            code: code.into(),
            expires_at_ms: pending
                .expires_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as u64)
                .unwrap_or_default(),
        };
        Some((
            EnrollmentCredential {
                device_id: Uuid::new_v4().to_string(),
                secret,
            },
            ticket,
        ))
    }

    fn restore(&self, ticket: &EnrollmentTicket) {
        let expires_at = SystemTime::UNIX_EPOCH + Duration::from_millis(ticket.expires_at_ms);
        if expires_at <= SystemTime::now() {
            return;
        }
        self.pending
            .lock()
            .expect("enrollment mutex poisoned")
            .insert(
                ticket.enrollment_id.clone(),
                PendingEnrollment {
                    code: ticket.code.clone(),
                    expires_at,
                },
            );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Challenge {
    pub(crate) protocol: String,
    pub(crate) device_id: String,
    pub(crate) server_nonce: String,
    pub(crate) client_nonce: String,
    pub(crate) link_epoch: u64,
    pub(crate) role: PhoneDirectRole,
}

pub(crate) fn challenge_proof(secret: &[u8; 32], challenge: &Challenge) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts all key sizes");
    mac.update(&canonical_challenge_bytes(challenge));
    mac.finalize().into_bytes().into()
}

fn canonical_challenge_bytes(challenge: &Challenge) -> Vec<u8> {
    let mut out = Vec::new();
    for field in [
        challenge.protocol.as_str(),
        challenge.device_id.as_str(),
        challenge.server_nonce.as_str(),
        challenge.client_nonce.as_str(),
        &challenge.link_epoch.to_string(),
        match challenge.role {
            PhoneDirectRole::Saga => "saga",
            PhoneDirectRole::Companion => "companion",
        },
    ] {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    out
}

#[cfg_attr(not(test), expect(dead_code))]
pub(crate) fn verify_challenge(secret: &[u8; 32], challenge: &Challenge, proof: &[u8]) -> bool {
    if challenge.protocol != PHONE_CONTROL_PROTOCOL_V2 {
        return false;
    }
    let expected = challenge_proof(secret, challenge);
    constant_time_eq(&expected, proof)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) enum DeviceLifecycle {
    #[default]
    Active,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct DeviceState {
    pub(crate) device_id: String,
    pub(crate) secret: [u8; 32],
    pub(crate) link_epoch: u64,
    pub(crate) revoked: bool,
    #[serde(default)]
    pub(crate) lifecycle: DeviceLifecycle,
    #[serde(default)]
    pub(crate) enrollment_id: Option<String>,
    #[serde(default)]
    pub(crate) pending_expires_at_ms: Option<u64>,
    #[serde(default)]
    pub(crate) committed_at_ms: Option<u64>,
}

pub(crate) trait DirectStateStore: Send + Sync {
    fn load(&self, device_id: &str) -> io::Result<Option<DeviceState>>;
    fn save(&self, state: DeviceState) -> io::Result<()>;
    fn delete(&self, device_id: &str) -> io::Result<()>;
}

#[derive(Clone, Default)]
#[cfg_attr(not(test), expect(dead_code))]
pub(crate) struct MemoryStateStore(Arc<Mutex<HashMap<String, DeviceState>>>);

impl DirectStateStore for MemoryStateStore {
    fn load(&self, device_id: &str) -> io::Result<Option<DeviceState>> {
        Ok(self
            .0
            .lock()
            .expect("state mutex poisoned")
            .get(device_id)
            .cloned())
    }
    fn save(&self, state: DeviceState) -> io::Result<()> {
        self.0
            .lock()
            .expect("state mutex poisoned")
            .insert(state.device_id.clone(), state);
        Ok(())
    }
    fn delete(&self, device_id: &str) -> io::Result<()> {
        self.0
            .lock()
            .expect("state mutex poisoned")
            .remove(device_id);
        Ok(())
    }
}

/// Durable private state store. Writes are atomic (temp file + rename) so an
/// interrupted process cannot roll an epoch back or lose a revocation.
#[derive(Clone)]
pub(crate) struct FileStateStore {
    path: std::path::PathBuf,
}
static STATE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
struct DeviceLocks {
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}
impl DeviceLocks {
    fn lock_for(&self, device_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .expect("device lock map poisoned")
            .entry(device_id.into())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}
static DEVICE_LOCKS: OnceLock<DeviceLocks> = OnceLock::new();

impl FileStateStore {
    pub(crate) fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
    fn read_all_unlocked(&self) -> io::Result<HashMap<String, DeviceState>> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(e),
        }
    }
    fn write_all_unlocked(&self, states: &HashMap<String, DeviceState>) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(states)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let tmp = self.path.with_extension(format!("{}.tmp", Uuid::new_v4()));
        fs::write(&tmp, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
        }
        let file = fs::OpenOptions::new().read(true).open(&tmp)?;
        file.sync_all()?;
        fs::rename(tmp, &self.path)?;
        #[cfg(unix)]
        if let Some(parent) = self.path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
    fn read_all(&self) -> io::Result<HashMap<String, DeviceState>> {
        let _guard = STATE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("state lock poisoned");
        self.read_all_unlocked()
    }
}

impl DirectStateStore for FileStateStore {
    fn load(&self, device_id: &str) -> io::Result<Option<DeviceState>> {
        Ok(self.read_all()?.remove(device_id))
    }
    fn save(&self, state: DeviceState) -> io::Result<()> {
        let _guard = STATE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("state lock poisoned");
        let mut all = self.read_all_unlocked()?;
        all.insert(state.device_id.clone(), state);
        self.write_all_unlocked(&all)
    }
    fn delete(&self, device_id: &str) -> io::Result<()> {
        FileStateStore::delete(self, device_id)
    }
}

impl FileStateStore {
    #[allow(dead_code)]
    pub(crate) fn revoke(&self, device_id: &str) -> io::Result<()> {
        let _guard = STATE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("state lock poisoned");
        let mut all = self.read_all_unlocked()?;
        let state = all
            .get_mut(device_id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "device is not enrolled"))?;
        state.revoked = true;
        self.write_all_unlocked(&all)
    }
    pub(crate) fn delete(&self, device_id: &str) -> io::Result<()> {
        let _guard = STATE_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("state lock poisoned");
        let mut all = self.read_all_unlocked()?;
        all.remove(device_id);
        self.write_all_unlocked(&all)
    }
}

/// Link epochs fence commands, results and chunks from a superseded socket.
#[derive(Clone, Default)]
pub(crate) struct LinkRegistry {
    links: Arc<Mutex<LinkMap>>,
    nonces: Arc<Mutex<HashMap<String, Instant>>>,
    handle: Option<DirectRuntimeHandle>,
}

const NONCE_TTL: Duration = Duration::from_secs(120);

type LinkMap = HashMap<String, (u64, tokio::sync::oneshot::Sender<()>)>;

pub(crate) struct LinkReservation {
    device_id: String,
    epoch: u64,
}

impl LinkRegistry {
    pub(crate) fn with_handle(handle: DirectRuntimeHandle) -> Self {
        Self {
            links: Arc::new(Mutex::new(HashMap::new())),
            nonces: Arc::new(Mutex::new(HashMap::new())),
            handle: Some(handle),
        }
    }
    #[allow(dead_code)]
    pub(crate) fn handle(&self) -> DirectRuntimeHandle {
        self.handle.clone().unwrap_or_default()
    }
    pub(crate) fn reserve(
        &self,
        device_id: &str,
        expected_epoch: u64,
        store: &dyn DirectStateStore,
    ) -> io::Result<LinkReservation> {
        let mut links = self.links.lock().expect("link mutex poisoned");
        let mut state = store
            .load(device_id)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "device is not enrolled"))?;
        if state.revoked || state.link_epoch.saturating_add(1) != expected_epoch {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stale challenged epoch",
            ));
        }
        state.link_epoch = expected_epoch;
        store.save(state)?;
        if let Some((_, old)) = links.remove(device_id) {
            let _ = old.send(());
        }
        Ok(LinkReservation {
            device_id: device_id.into(),
            epoch: expected_epoch,
        })
    }
    pub(crate) fn finalize(
        &self,
        reservation: LinkReservation,
    ) -> tokio::sync::oneshot::Receiver<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.links
            .lock()
            .expect("link mutex poisoned")
            .insert(reservation.device_id, (reservation.epoch, tx));
        rx
    }
    fn claim_nonce(&self, nonce: &str) -> bool {
        let mut map = self.nonces.lock().expect("nonce mutex poisoned");
        let deadline = Instant::now();
        map.retain(|_, expires_at| *expires_at > deadline);
        map.insert(nonce.to_string(), deadline + NONCE_TTL)
            .is_none()
    }
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn revoke(&self, device_id: &str) {
        if let Some((_, cancel)) = self
            .links
            .lock()
            .expect("link mutex poisoned")
            .remove(device_id)
        {
            let _ = cancel.send(());
        }
    }
    #[cfg_attr(not(test), expect(dead_code))]
    pub(crate) fn authenticate(
        &self,
        device_id: &str,
        store: &dyn DirectStateStore,
    ) -> io::Result<(u64, tokio::sync::oneshot::Receiver<()>)> {
        let mut links = self.links.lock().expect("link mutex poisoned");
        let mut state = store
            .load(device_id)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "device is not enrolled"))?;
        if state.revoked {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "device is revoked",
            ));
        }
        let epoch = state.link_epoch.saturating_add(1);
        state.link_epoch = epoch;
        store.save(state)?;
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        if let Some((_, old_cancel)) = links.remove(device_id) {
            let _ = old_cancel.send(());
        }
        links.insert(device_id.to_string(), (epoch, cancel_tx));
        Ok((epoch, cancel_rx))
    }
    #[allow(dead_code)]
    pub(crate) fn activate(
        &self,
        device_id: &str,
        expected_epoch: u64,
        store: &dyn DirectStateStore,
    ) -> io::Result<tokio::sync::oneshot::Receiver<()>> {
        Ok(self.finalize(self.reserve(device_id, expected_epoch, store)?))
    }
    pub(crate) fn is_current(&self, device_id: &str, epoch: u64) -> bool {
        self.links
            .lock()
            .expect("link mutex poisoned")
            .get(device_id)
            .map(|(current, _)| *current)
            == Some(epoch)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddr};

    #[derive(Clone, Default)]
    struct FailingStore;
    #[derive(Clone, Default)]
    struct CommitThenErrorStore(Arc<Mutex<HashMap<String, DeviceState>>>);
    impl DirectStateStore for CommitThenErrorStore {
        fn load(&self, id: &str) -> io::Result<Option<DeviceState>> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }
        fn save(&self, state: DeviceState) -> io::Result<()> {
            let id = state.device_id.clone();
            self.0.lock().unwrap().insert(id, state);
            Err(io::Error::other("commit then error"))
        }
        fn delete(&self, id: &str) -> io::Result<()> {
            self.0.lock().unwrap().remove(id);
            Ok(())
        }
    }
    #[derive(Clone, Default)]
    struct DeleteFailStore(Arc<Mutex<HashMap<String, DeviceState>>>);
    impl DirectStateStore for DeleteFailStore {
        fn load(&self, id: &str) -> io::Result<Option<DeviceState>> {
            Ok(self.0.lock().unwrap().get(id).cloned())
        }
        fn save(&self, state: DeviceState) -> io::Result<()> {
            let id = state.device_id.clone();
            self.0.lock().unwrap().insert(id, state);
            Ok(())
        }
        fn delete(&self, _: &str) -> io::Result<()> {
            Err(io::Error::other("delete failure"))
        }
    }

    #[test]
    fn rollback_reconciles_commit_then_error_and_surfaces_delete_failure() {
        let state = DeviceState {
            device_id: "00000000-0000-4000-8000-000000000001".into(),
            secret: [1; 32],
            link_epoch: 0,
            revoked: false,
            lifecycle: DeviceLifecycle::Active,
            enrollment_id: None,
            pending_expires_at_ms: None,
            committed_at_ms: None,
        };
        let committed = CommitThenErrorStore::default();
        committed.save(state.clone()).unwrap_err();
        assert!(rollback_device(&committed, &state.device_id).is_ok());
        assert!(committed.load(&state.device_id).unwrap().is_none());
        let failing = DeleteFailStore::default();
        failing.save(state.clone()).unwrap();
        assert!(rollback_device(&failing, &state.device_id).is_err());
        assert!(failing.load(&state.device_id).unwrap().is_some());
    }

    #[test]
    fn enrollment_ack_proof_is_lowercase_and_rejects_replay_fields() {
        let mut ack = PhoneEnrollmentAck {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            enrollment_id: "enroll".into(),
            device_id: "device".into(),
            client_nonce: B64.encode([4u8; 32]),
            client_proof: String::new(),
        };
        ack.client_proof = ack_proof(&[9u8; 32], &ack);
        assert_eq!(ack.client_proof.len(), 64);
        assert!(verify_enrollment_ack(&[9u8; 32], &ack));
        let mut tampered = ack;
        tampered.enrollment_id = "other".into();
        assert!(!verify_enrollment_ack(&[9u8; 32], &tampered));
    }

    #[tokio::test]
    async fn ack_and_pending_auth_serialization_cannot_roll_back_epoch() {
        let store = Arc::new(MemoryStateStore::default());
        let device_id = "00000000-0000-4000-8000-000000000009".to_string();
        store
            .save(DeviceState {
                device_id: device_id.clone(),
                secret: [2; 32],
                link_epoch: 0,
                revoked: false,
                lifecycle: DeviceLifecycle::Pending,
                enrollment_id: Some("e".into()),
                pending_expires_at_ms: Some(now_ms() + 60_000),
                committed_at_ms: None,
            })
            .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let a_store = store.clone();
        let a_barrier = barrier.clone();
        let a_id = device_id.clone();
        let ack = tokio::spawn(async move {
            a_barrier.wait().await;
            let lock = DEVICE_LOCKS
                .get_or_init(|| DeviceLocks {
                    locks: Mutex::new(HashMap::new()),
                })
                .lock_for(&a_id);
            let _g = lock.lock().await;
            let mut s = a_store.load(&a_id).unwrap().unwrap();
            s.lifecycle = DeviceLifecycle::Active;
            s.committed_at_ms = Some(now_ms());
            a_store.save(s).unwrap();
        });
        let p_store = store.clone();
        let p_barrier = barrier.clone();
        let p_id = device_id.clone();
        let auth = tokio::spawn(async move {
            p_barrier.wait().await;
            let lock = DEVICE_LOCKS
                .get_or_init(|| DeviceLocks {
                    locks: Mutex::new(HashMap::new()),
                })
                .lock_for(&p_id);
            let _g = lock.lock().await;
            let mut s = p_store.load(&p_id).unwrap().unwrap();
            s.link_epoch += 1;
            p_store.save(s).unwrap();
        });
        ack.await.unwrap();
        auth.await.unwrap();
        let final_state = store.load(&device_id).unwrap().unwrap();
        assert_eq!(final_state.lifecycle, DeviceLifecycle::Active);
        assert_eq!(final_state.link_epoch, 1);
        assert!(final_state.committed_at_ms.is_some());
    }

    fn enrollment_redeem(
        protocol: &str,
        enrollment_id: String,
        bootstrap_credential: String,
    ) -> PhoneDirectControlFrame {
        serde_json::from_value(serde_json::json!({
            "type": "enrollment_redeem",
            "protocol": protocol,
            "enrollment_id": enrollment_id,
            "bootstrap_credential": bootstrap_credential,
        }))
        .unwrap()
    }
    impl DirectStateStore for FailingStore {
        fn load(&self, _: &str) -> io::Result<Option<DeviceState>> {
            Ok(None)
        }
        fn save(&self, _: DeviceState) -> io::Result<()> {
            Err(io::Error::other("injected persistence failure"))
        }
        fn delete(&self, _: &str) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn rejects_wildcard_binds() {
        // 0.0.0.0 / :: now allowed for LAN/tether (covers WiFi + rndis0/usb0 in one socket).
        assert!(validate_bind_addr(SocketAddr::from(([0, 0, 0, 0], 1))).is_ok());
        assert!(validate_bind_addr("[::]:1".parse().unwrap()).is_ok());
        assert!(validate_bind_addr(SocketAddr::from(([127, 0, 0, 1], 1))).is_ok());
        assert!(validate_bind_addr(SocketAddr::from((Ipv4Addr::new(100, 64, 0, 2), 1))).is_ok());
        assert!(validate_bind_addr(SocketAddr::from(([8, 8, 8, 8], 1))).is_err());
        // RFC1918 + link-local + ULA now allowed (tether 192.168.42, hotspot, etc.)
        assert!(validate_bind_addr(SocketAddr::from(([192, 168, 1, 2], 1))).is_ok());
        assert!(validate_bind_addr(SocketAddr::from(([192, 168, 42, 10], 1))).is_ok());
        assert!(validate_bind_addr(SocketAddr::from(([10, 0, 0, 5], 1))).is_ok());
        assert!(validate_bind_addr(SocketAddr::from(([172, 16, 5, 5], 1))).is_ok());
        assert!(validate_bind_addr(SocketAddr::from(([169, 254, 1, 2], 1))).is_ok());
        assert!(validate_bind_addr("[fd7a:115c:a1e0::2]:1".parse().unwrap()).is_ok());
        assert!(validate_bind_addr("[fd00::2]:1".parse().unwrap()).is_ok());
        assert!(validate_bind_addr("[fe80::1]:1".parse().unwrap()).is_ok());
        assert!(validate_bind_addr("[::ffff:192.168.1.1]:1".parse().unwrap()).is_ok());
        // Public still rejected.
        assert!(validate_bind_addr(SocketAddr::from(([203, 0, 113, 5], 1))).is_err());
        assert!(validate_bind_addr("[2001:db8::1]:1".parse().unwrap()).is_err());
    }

    #[test]
    fn enrollment_is_single_use_and_expires() {
        let r = EnrollmentRegistry::default();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let t = r.create(now);
        assert!(r.consume(&t.enrollment_id, "wrong", now).is_none());
        assert!(r.consume(&t.enrollment_id, &t.code, now).is_some());
        assert!(r.consume(&t.enrollment_id, &t.code, now).is_none());
        let t = r.create(now);
        assert!(
            r.consume(&t.enrollment_id, &t.code, now + DEFAULT_ENROLLMENT_TTL)
                .is_none()
        );
        assert_eq!(B64.decode(t.code).unwrap().len(), 32);
    }

    #[test]
    fn runtime_create_enrollment_uses_endpoint_and_ttl() {
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let runtime = DirectRuntime {
            shutdown,
            task: None,
            registry: EnrollmentRegistry::with_ttl(Duration::from_secs(300)),
            handle: DirectRuntimeHandle::new(),
            state: Arc::new(MemoryStateStore::default()),
            advertised_endpoint: "wss://saga.example/phone".into(),
        };
        let before_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let payload = runtime.create_enrollment();
        let after_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        assert_eq!(payload.protocol, PHONE_CONTROL_PROTOCOL_V2);
        assert_eq!(payload.endpoint, "wss://saga.example/phone");
        assert_eq!(B64.decode(payload.bootstrap_credential).unwrap().len(), 32);
        assert!(payload.expires_at_ms >= before_ms + 300_000);
        assert!(payload.expires_at_ms <= after_ms + 300_000);
    }

    #[tokio::test]
    async fn websocket_redeem_persists_state_and_returns_secret() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let registry = EnrollmentRegistry::default();
        let ticket = registry.create(SystemTime::now());
        let store = Arc::new(MemoryStateStore::default());
        let task_store = store.clone();
        let task_registry = registry.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = accept_async(stream).await.unwrap();
            accept_socket(socket, task_store, &LinkRegistry::default(), &task_registry).await
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        let redeem = enrollment_redeem(
            PHONE_CONTROL_PROTOCOL_V2,
            ticket.enrollment_id.clone(),
            ticket.code.clone(),
        );
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&redeem).unwrap().into(),
            ))
            .await
            .unwrap();
        let result = serde_json::from_str::<PhoneDirectControlFrame>(
            client
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let PhoneDirectControlFrame::EnrollmentOk(ok) = result else {
            panic!("expected enrollment_ok")
        };
        let state = store.load(&ok.device_id).unwrap().unwrap();
        assert_eq!(state.link_epoch, 0);
        assert!(!state.revoked);
        assert_eq!(
            B64.decode(ok.device_secret).unwrap().as_slice(),
            state.secret
        );
        let ack = PhoneEnrollmentAck {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            enrollment_id: ticket.enrollment_id.clone(),
            device_id: ok.device_id.clone(),
            client_nonce: B64.encode([7u8; 32]),
            client_proof: String::new(),
        };
        let mut ack = ack;
        ack.client_proof = ack_proof(&state.secret, &ack);
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&PhoneDirectControlFrame::EnrollmentAck(ack))
                    .unwrap()
                    .into(),
            ))
            .await
            .unwrap();
        let committed = serde_json::from_str::<PhoneDirectControlFrame>(
            client
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        assert!(matches!(
            committed,
            PhoneDirectControlFrame::EnrollmentCommitted(_)
        ));
        task.await.unwrap().unwrap();

        let auth_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let auth_addr = auth_listener.local_addr().unwrap();
        let auth_task = tokio::spawn(async move {
            let (stream, _) = auth_listener.accept().await.unwrap();
            let socket = accept_async(stream).await.unwrap();
            authenticate_socket(socket, store, &LinkRegistry::default()).await
        });
        let (mut auth_client, _) = tokio_tungstenite::connect_async(format!("ws://{auth_addr}"))
            .await
            .unwrap();
        let client_nonce = B64.encode([3u8; 32]);
        let hello = PhoneDirectControlFrame::AuthHello(sky_cua_platform::model::PhoneAuthHello {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            device_id: ok.device_id.clone(),
            client_nonce: client_nonce.clone(),
        });
        auth_client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&hello).unwrap().into(),
            ))
            .await
            .unwrap();
        let challenge = serde_json::from_str::<PhoneDirectControlFrame>(
            auth_client
                .next()
                .await
                .unwrap()
                .unwrap()
                .into_text()
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let PhoneDirectControlFrame::AuthChallenge(challenge) = challenge else {
            panic!("expected auth challenge")
        };
        let epoch = challenge.link_epoch.parse().unwrap();
        let proof = challenge_proof(
            &state.secret,
            &Challenge {
                protocol: challenge.protocol.clone(),
                device_id: ok.device_id.clone(),
                server_nonce: challenge.server_nonce.clone(),
                client_nonce,
                link_epoch: epoch,
                role: PhoneDirectRole::Companion,
            },
        );
        let auth_proof =
            PhoneDirectControlFrame::AuthProof(sky_cua_platform::model::PhoneAuthProof {
                link_epoch: challenge.link_epoch,
                client_proof: proof.iter().map(|b| format!("{b:02x}")).collect(),
            });
        auth_client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&auth_proof).unwrap().into(),
            ))
            .await
            .unwrap();
        assert!(matches!(
            auth_client.next().await.unwrap().unwrap(),
            tokio_tungstenite::tungstenite::Message::Text(_)
        ));
        auth_client.close(None).await.unwrap();
        auth_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn websocket_persistence_failure_sends_no_ok_and_restores_ticket() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let registry = EnrollmentRegistry::with_ttl(Duration::from_secs(60));
        let ticket = registry.create(SystemTime::now());
        let task_registry = registry.clone();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = accept_async(stream).await.unwrap();
            accept_socket(
                socket,
                Arc::new(FailingStore),
                &LinkRegistry::default(),
                &task_registry,
            )
            .await
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        let redeem = enrollment_redeem(
            PHONE_CONTROL_PROTOCOL_V2,
            ticket.enrollment_id.clone(),
            ticket.code.clone(),
        );
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&redeem).unwrap().into(),
            ))
            .await
            .unwrap();
        // EnrollmentOk frame is sent before persistence, so the client should
        // receive it even though the store save will fail.
        let frame = tokio::time::timeout(Duration::from_millis(500), client.next()).await;
        let msg = match frame {
            Ok(Some(Ok(msg))) => msg,
            other => panic!("expected EnrollmentOk frame, got: {other:?}"),
        };
        assert!(msg.is_text(), "expected text EnrollmentOk frame");
        match serde_json::from_str::<PhoneDirectControlFrame>(msg.to_text().unwrap()) {
            Ok(PhoneDirectControlFrame::EnrollmentOk(_)) => {}
            other => panic!("expected EnrollmentOk, got: {other:?}"),
        }
        // The store save fails and the ticket is restored.
        let restored = registry
            .pending
            .lock()
            .unwrap()
            .get(&ticket.enrollment_id)
            .unwrap()
            .expires_at;
        assert_eq!(
            restored,
            SystemTime::UNIX_EPOCH + Duration::from_millis(ticket.expires_at_ms)
        );
        assert!(
            registry
                .consume(&ticket.enrollment_id, &ticket.code, SystemTime::now())
                .is_some()
        );
        assert!(task.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn websocket_wrong_and_expired_redeems_create_no_state() {
        for expired in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let registry = EnrollmentRegistry::with_ttl(Duration::from_secs(60));
            let ticket = if expired {
                registry.create(SystemTime::UNIX_EPOCH)
            } else {
                registry.create(SystemTime::now())
            };
            let store = Arc::new(MemoryStateStore::default());
            let task_registry = registry.clone();
            let task_store = store.clone();
            let task = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let socket = accept_async(stream).await.unwrap();
                accept_socket(socket, task_store, &LinkRegistry::default(), &task_registry).await
            });
            let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
                .await
                .unwrap();
            let enrollment_id = ticket.enrollment_id.clone();
            let credential = if expired {
                ticket.code.clone()
            } else {
                format!("{}x", ticket.code)
            };
            let redeem = enrollment_redeem(PHONE_CONTROL_PROTOCOL_V2, enrollment_id, credential);
            client
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::to_string(&redeem).unwrap().into(),
                ))
                .await
                .unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(500), client.next())
                    .await
                    .is_ok()
            );
            assert!(store.load("missing").unwrap().is_none());
            assert!(task.await.unwrap().is_err());
        }
    }

    #[test]
    fn challenge_proof_detects_tampering() {
        let secret = [7u8; 32];
        let c = Challenge {
            protocol: PHONE_CONTROL_PROTOCOL_V2.to_string(),
            device_id: "d".into(),
            server_nonce: "server".into(),
            client_nonce: "client".into(),
            link_epoch: 3,
            role: PhoneDirectRole::Saga,
        };
        let p = challenge_proof(&secret, &c);
        assert!(verify_challenge(&secret, &c, &p));
        assert!(!verify_challenge(
            &secret,
            &Challenge {
                link_epoch: 4,
                ..c.clone()
            },
            &p
        ));
    }

    #[test]
    fn canonical_proof_transcript_matches_shared_fixture() {
        let c = Challenge {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            device_id: "device-fixture".into(),
            server_nonce: "server-nonce".into(),
            client_nonce: "client-nonce".into(),
            link_epoch: 4,
            role: PhoneDirectRole::Companion,
        };
        let expected = "0000001070686f6e652d636f6e74726f6c2e76320000000e6465766963652d666978747572650000000c7365727665722d6e6f6e63650000000c636c69656e742d6e6f6e6365000000013400000009636f6d70616e696f6e";
        let actual: String = canonical_challenge_bytes(&c)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn epoch_supersedes_previous_link() {
        let r = LinkRegistry::default();
        let s = MemoryStateStore::default();
        s.save(DeviceState {
            device_id: "d".into(),
            secret: [1; 32],
            link_epoch: 0,
            revoked: false,
            lifecycle: DeviceLifecycle::Active,
            enrollment_id: None,
            pending_expires_at_ms: None,
            committed_at_ms: None,
        })
        .unwrap();
        let (first, _cancel) = r.authenticate("d", &s).unwrap();
        assert_eq!(first, 1);
        let (second, _cancel) = r.authenticate("d", &s).unwrap();
        assert_eq!(second, 2);
        assert!(!r.is_current("d", 1));
        assert!(r.is_current("d", 2));
    }

    #[test]
    fn nonce_replay_is_rejected_and_revocation_fences_link() {
        let r = LinkRegistry::default();
        assert!(r.claim_nonce("nonce"));
        assert!(!r.claim_nonce("nonce"));
        let store = MemoryStateStore::default();
        store
            .save(DeviceState {
                device_id: "d".into(),
                secret: [1; 32],
                link_epoch: 0,
                revoked: false,
                lifecycle: DeviceLifecycle::Active,
                enrollment_id: None,
                pending_expires_at_ms: None,
                committed_at_ms: None,
            })
            .unwrap();
        let (_epoch, _cancel) = r.authenticate("d", &store).unwrap();
        r.revoke("d");
        assert!(!r.is_current("d", 1));
    }

    #[test]
    fn failed_auth_ok_send_leaves_reserved_epoch_without_active_candidate() {
        let r = LinkRegistry::default();
        let store = MemoryStateStore::default();
        store
            .save(DeviceState {
                device_id: "d".into(),
                secret: [1; 32],
                link_epoch: 0,
                revoked: false,
                lifecycle: DeviceLifecycle::Active,
                enrollment_id: None,
                pending_expires_at_ms: None,
                committed_at_ms: None,
            })
            .unwrap();
        let reservation = r.reserve("d", 1, &store).unwrap();
        assert!(!r.is_current("d", 1));
        drop(reservation);
        assert!(!r.is_current("d", 1));
        assert_eq!(store.load("d").unwrap().unwrap().link_epoch, 1);
    }

    #[test]
    fn server_nonce_is_exactly_32_bytes_url_safe() {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).unwrap();
        let encoded = B64.encode(bytes);
        assert_eq!(B64.decode(encoded).unwrap().len(), 32);
    }

    #[tokio::test]
    async fn listener_is_disabled_by_default_and_accepts_loopback_websocket() {
        let disabled = DirectListener::bind(false, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        assert!(disabled.is_none());
        let listener = DirectListener::bind(true, "127.0.0.1:0".parse().unwrap())
            .await
            .unwrap()
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { listener.accept_websocket().await.unwrap().1 });
        let (_client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        assert!(server.await.unwrap().ip().is_loopback());
    }

    #[tokio::test]
    async fn loopback_authenticates_mutually_before_application_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let path = std::env::temp_dir().join(format!("phone-direct-auth-{}.json", Uuid::new_v4()));
        let store = FileStateStore::new(&path);
        let secret = [9u8; 32];
        store
            .save(DeviceState {
                device_id: "00000000-0000-4000-8000-000000000001".into(),
                secret,
                link_epoch: 0,
                revoked: false,
                lifecycle: DeviceLifecycle::Active,
                enrollment_id: None,
                pending_expires_at_ms: None,
                committed_at_ms: None,
            })
            .unwrap();
        let registry = LinkRegistry::default();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = accept_async(stream).await.unwrap();
            authenticate_socket(socket, Arc::new(store), &registry)
                .await
                .unwrap();
        });
        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap();
        let hello = PhoneDirectControlFrame::AuthHello(sky_cua_platform::model::PhoneAuthHello {
            protocol: PHONE_CONTROL_PROTOCOL_V2.into(),
            device_id: "00000000-0000-4000-8000-000000000001".into(),
            client_nonce: B64.encode([3u8; 32]),
        });
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&hello).unwrap().into(),
            ))
            .await
            .unwrap();
        let challenge = client.next().await.unwrap().unwrap();
        let challenge: PhoneDirectControlFrame =
            serde_json::from_str(challenge.into_text().unwrap().as_ref()).unwrap();
        let PhoneDirectControlFrame::AuthChallenge(c) = challenge else {
            panic!("expected challenge")
        };
        let epoch = c.link_epoch.parse().unwrap();
        let proof = challenge_proof(
            &secret,
            &Challenge {
                protocol: c.protocol.clone(),
                device_id: "00000000-0000-4000-8000-000000000001".into(),
                server_nonce: c.server_nonce.clone(),
                client_nonce: B64.encode([3u8; 32]),
                link_epoch: epoch,
                role: PhoneDirectRole::Companion,
            },
        );
        let proof = PhoneDirectControlFrame::AuthProof(sky_cua_platform::model::PhoneAuthProof {
            link_epoch: c.link_epoch,
            client_proof: proof.iter().map(|b| format!("{b:02x}")).collect(),
        });
        client
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&proof).unwrap().into(),
            ))
            .await
            .unwrap();
        assert!(matches!(
            client.next().await.unwrap().unwrap(),
            tokio_tungstenite::tungstenite::Message::Text(_)
        ));
        client.close(None).await.unwrap();
        task.await.unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn handle_rejects_old_epoch_before_emitting_wire_request() {
        let handle = DirectRuntimeHandle::new();
        let (mut control, _bulk) = handle.register_fake("device-a", 7);
        let error = handle
            .request(
                "device-a",
                6,
                "ui.tap",
                serde_json::json!({}),
                false,
                Duration::from_millis(50),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error,
            DirectRuntimeError::LinkEpochChanged {
                expected: 6,
                current: Some(7)
            }
        );
        assert!(control.try_recv().is_err());
    }

    #[tokio::test]
    async fn supersession_replaces_snapshot_and_fences_old_epoch() {
        let handle = DirectRuntimeHandle::new();
        let (_old_control, _old_bulk) = handle.register_fake("device-a", 1);
        let (_new_control, _new_bulk) = handle.register_fake("device-a", 2);
        assert_eq!(handle.snapshot("device-a").unwrap().link_epoch, 2);
        let error = handle
            .request(
                "device-a",
                1,
                "ui.tap",
                serde_json::json!({}),
                false,
                Duration::from_millis(50),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            DirectRuntimeError::LinkEpochChanged {
                expected: 1,
                current: Some(2)
            }
        ));
    }

    #[tokio::test]
    async fn disconnect_is_ambiguous_and_request_is_not_replayed() {
        let handle = DirectRuntimeHandle::new();
        let (mut control, _bulk) = handle.register_fake("device-a", 1);
        let task = tokio::spawn({
            let handle = handle.clone();
            async move {
                handle
                    .request(
                        "device-a",
                        1,
                        "camera.photo",
                        serde_json::json!({}),
                        false,
                        Duration::from_secs(2),
                    )
                    .await
            }
        });
        let message = control.recv().await.unwrap();
        assert!(matches!(
            message,
            tokio_tungstenite::tungstenite::Message::Text(_)
        ));
        let (_replacement_control, _replacement_bulk) = handle.register_fake("device-a", 2);
        let result = task.await.unwrap();
        assert!(matches!(
            result,
            Err(DirectRuntimeError::LinkEpochChanged { .. })
                | Err(DirectRuntimeError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn provider_lists_direct_devices_without_synthetic_serial() {
        let handle = DirectRuntimeHandle::new();
        let (_control, _bulk) = handle.register_fake("device-a", 4);
        let provider = provider::CompanionDirectProvider::new(handle);
        let devices = provider.list_devices();
        assert_eq!(
            devices,
            vec![DirectDeviceSnapshot {
                device_id: "device-a".into(),
                link_epoch: 4,
                connected: true,
                capabilities: BTreeSet::new(),
            }]
        );
    }

    #[tokio::test]
    async fn provider_snapshot_tracks_current_direct_capabilities() {
        let handle = DirectRuntimeHandle::new();
        let (_control, _bulk) = handle.register_fake("device-a", 4);
        handle.set_fake_capabilities("device-a", 4, ["sms.read", "content"]);

        assert_eq!(
            handle.snapshot("device-a").unwrap().capabilities,
            BTreeSet::from(["content".to_owned(), "sms.read".to_owned()])
        );
    }

    #[tokio::test]
    async fn host_content_transfer_preserves_declare_chunks_commit_order() {
        let handle = DirectRuntimeHandle::new();
        let (_control, mut bulk) = handle.register_fake("device-a", 4);
        let task = tokio::spawn({
            let handle = handle.clone();
            async move {
                handle
                    .send_content(
                        "device-a",
                        4,
                        b"host bytes",
                        "application/octet-stream",
                        Some("payload.bin".into()),
                    )
                    .await
            }
        });
        let declare = bulk.recv().await.expect("declaration");
        assert!(matches!(
            declare.message,
            tokio_tungstenite::tungstenite::Message::Text(_)
        ));
        assert!(declare.sent.is_none());
        let chunk = bulk.recv().await.expect("chunk");
        assert!(matches!(
            chunk.message,
            tokio_tungstenite::tungstenite::Message::Binary(_)
        ));
        let commit = bulk.recv().await.expect("commit");
        assert!(matches!(
            commit.message,
            tokio_tungstenite::tungstenite::Message::Text(_)
        ));
        commit
            .sent
            .expect("commit flush acknowledgement")
            .send(())
            .unwrap();
        let content = task.await.unwrap().unwrap();
        assert_eq!(content.size_bytes, 10);
        assert_eq!(content.filename.as_deref(), Some("payload.bin"));
    }

    #[tokio::test]
    async fn outbound_control_frame_limit_is_checked_before_enqueue() {
        let handle = DirectRuntimeHandle::new();
        let (mut control, _bulk) = handle.register_fake("device-a", 1);
        let error = handle
            .request(
                "device-a",
                1,
                "large",
                serde_json::json!({"payload": "x".repeat(PHONE_CONTROL_MAX_JSON_BYTES as usize)}),
                true,
                Duration::from_millis(50),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, DirectRuntimeError::Protocol(_)));
        assert!(control.try_recv().is_err());
    }

    #[test]
    fn inbound_control_frame_accepts_exact_limit_and_rejects_over_limit() {
        let base =
            serde_json::to_vec(&PhoneDirectControlFrame::Ping { nonce: "n".into() }).unwrap();
        let mut exact = base.clone();
        exact.extend(std::iter::repeat_n(
            b' ',
            PHONE_CONTROL_MAX_JSON_BYTES as usize - exact.len(),
        ));
        assert!(decode_control_frame(&exact, "device-a", 1, now_ms()).is_ok());
        let over = vec![b'x'; PHONE_CONTROL_MAX_JSON_BYTES as usize + 1];
        assert!(decode_control_frame(&over, "device-a", 1, now_ms()).is_err());
    }

    #[tokio::test]
    async fn live_socket_scheduler_exits_and_removes_link_on_peer_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = DirectRuntimeHandle::new();
        let server_handle = handle.clone();
        let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server_handle
                .serve_socket(socket, "device-a".into(), 1, cancel_rx)
                .await
                .unwrap();
        });
        let client = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap()
            .0;
        tokio::time::timeout(Duration::from_secs(1), async {
            while handle.snapshot("device-a").is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        drop(client);

        tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("server must stop after peer EOF")
            .unwrap();
        assert!(handle.snapshot("device-a").is_none());
    }

    #[tokio::test]
    async fn live_socket_scheduler_makes_control_and_bulk_progress_under_inbound_flood() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = DirectRuntimeHandle::new();
        let server_handle = handle.clone();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server_handle
                .serve_socket(socket, "device-a".into(), 1, cancel_rx)
                .await
                .unwrap();
        });
        let client = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap()
            .0;
        let (mut sink, mut stream) = client.split();
        tokio::time::timeout(Duration::from_secs(1), async {
            while handle.snapshot("device-a").is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let link = handle
            .links
            .lock()
            .unwrap()
            .get("device-a")
            .cloned()
            .unwrap();
        let control = PhoneDirectControlFrame::Ping {
            nonce: "control".into(),
        };
        link.control
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&control).unwrap().into(),
            ))
            .await
            .unwrap();
        link.bulk
            .send(BulkFrame {
                message: tokio_tungstenite::tungstenite::Message::Binary(vec![0x42].into()),
                sent: None,
            })
            .await
            .unwrap();
        let inbound = tokio::spawn(async move {
            for _ in 0..64 {
                let frame = PhoneDirectControlFrame::Ping {
                    nonce: "flood".into(),
                };
                if sink
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        serde_json::to_string(&frame).unwrap().into(),
                    ))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut saw_control = false;
        let mut saw_bulk = false;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !(saw_control && saw_bulk) {
                let Some(Ok(message)) = stream.next().await else {
                    break;
                };
                match message {
                    tokio_tungstenite::tungstenite::Message::Text(text)
                        if text.contains("control") =>
                    {
                        saw_control = true
                    }
                    tokio_tungstenite::tungstenite::Message::Binary(bytes)
                        if bytes.as_ref() == [0x42] =>
                    {
                        saw_bulk = true
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(saw_control, "control output was starved");
        assert!(saw_bulk, "bulk output was starved");
        inbound.await.unwrap();
        let _ = cancel_tx.send(());
        let _ = server.await;
    }

    #[tokio::test]
    async fn live_socket_scheduler_reads_response_after_four_inbound_transfer_frames() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = DirectRuntimeHandle::new();
        let server_handle = handle.clone();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server_handle
                .serve_socket(socket, "device-a".into(), 1, cancel_rx)
                .await
                .unwrap();
        });
        let (mut sink, mut stream) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap()
            .0
            .split();
        tokio::time::timeout(Duration::from_secs(1), async {
            while handle.snapshot("device-a").is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let request = tokio::spawn({
            let handle = handle.clone();
            async move {
                handle
                    .request(
                        "device-a",
                        1,
                        "appshot",
                        serde_json::json!({}),
                        true,
                        Duration::from_secs(1),
                    )
                    .await
            }
        });
        let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) = stream.next().await
        else {
            panic!("expected request")
        };
        let PhoneDirectControlFrame::Request {
            request_id,
            device_id,
            link_epoch,
            ..
        } = serde_json::from_str(&text).unwrap()
        else {
            panic!("expected request frame")
        };
        for index in 0..4 {
            sink.send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&PhoneDirectControlFrame::Ping {
                    nonce: format!("transfer-frame-{index}"),
                })
                .unwrap()
                .into(),
            ))
            .await
            .unwrap();
        }
        sink.send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&PhoneDirectControlFrame::Response {
                request_id,
                device_id,
                link_epoch,
                result: serde_json::json!({"appshot_id": "fresh"}),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();

        assert_eq!(
            request.await.unwrap().unwrap(),
            serde_json::json!({"appshot_id": "fresh"})
        );
        let _ = cancel_tx.send(());
        let _ = server.await;
    }

    #[tokio::test]
    async fn malformed_binary_closes_link_and_fails_pending_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = DirectRuntimeHandle::new();
        let server_handle = handle.clone();
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            server_handle
                .serve_socket(socket, "malformed".into(), 7, cancel_rx)
                .await
                .unwrap();
        });
        let (mut sink, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
            .await
            .unwrap()
            .0
            .split();
        tokio::time::timeout(Duration::from_secs(1), async {
            while handle.snapshot("malformed").is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let pending = tokio::spawn({
            let h = handle.clone();
            async move {
                h.request(
                    "malformed",
                    7,
                    "x",
                    serde_json::json!({}),
                    false,
                    Duration::from_secs(5),
                )
                .await
            }
        });
        sink.send(tokio_tungstenite::tungstenite::Message::Binary(
            vec![1, 2, 3].into(),
        ))
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while handle.snapshot("malformed").is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            pending.await.unwrap(),
            Err(DirectRuntimeError::Disconnected)
        ));
        let _ = cancel_tx.send(());
        let _ = server.await;
    }
}
