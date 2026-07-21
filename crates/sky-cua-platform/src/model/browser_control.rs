use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::browser::{
    BrowserCallerKind, BrowserCallerProvenance, BrowserLogicalIdentity, BrowserMcpClientInfo,
    BrowserProvenanceSource, BrowserRequest,
};

pub const BROWSER_CONTROL_PROTOCOL_VERSION: u32 = 1;
pub const BROWSER_CONTROL_CANONICAL_SESSION_ID: &str = "sky-cua-control-plane-v1";
pub const BROWSER_CONTROL_CANONICAL_TURN_ID: &str = "control-plane-lease-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserInstanceIdentity {
    pub browser_instance_id: String,
    pub browser_family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_discriminator: Option<String>,
    pub boot_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserBridgeConnectionIdentity {
    pub bridge_connection_id: String,
    pub browser_instance_id: String,
    pub host_instance_id: String,
    pub peer_pid: u32,
    pub peer_start_ticks: u64,
    pub actor_generation: u64,
    pub daemon_generation: String,
    pub role: BrowserBridgeRole,
    pub canonical_session_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserBridgeRole {
    ControlPlane,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserInstanceStability {
    Stable,
    ConnectionOnly,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BrowserTabKey {
    pub browser_instance_id: String,
    pub extension_tab_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserOperationScope {
    Tab,
    Bridge,
    Daemon,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserOperationClass {
    ReadOnly,
    AbsoluteSet,
    Mutation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserCompletionCertainty {
    PreDispatchRejected,
    DefinitiveSuccess,
    DefinitiveFailure,
    AmbiguousCompletion,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLeaseState {
    Active,
    OrphanedGrace,
    ExpiryPending,
    HandoffPending,
    Suspended,
    RecoveryRequired,
    Lost,
    Released,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserBridgeState {
    Discovered,
    Probing,
    Connecting,
    HostHandshake,
    ExtensionHandshake,
    Ready,
    Reconnecting,
    Quarantined,
    Lost,
}

/// Concrete browser transport identity.
///
/// Daemon-owned Chrome-family actors currently use the extension/native-host
/// relay. A host-provided IAB is a separate native transport whose lifecycle
/// remains owned by the host. Keeping transport separate from caller-facing
/// surface prevents compatibility adapters from conflating the two.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserBridgeTransport {
    ExtensionNativeHost,
    HostProvidedIab,
}

/// Browser API surface used by a control-plane client.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserClientSurface {
    McpTools,
    HostProvidedIab,
    NodeReplBrowserApi,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserMigrationMode {
    Legacy,
    Hybrid,
    Strict,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlPlaneSnapshot {
    pub protocol_version: u32,
    pub daemon_generation: String,
    pub migration_mode: BrowserMigrationMode,
    pub ready: bool,
    pub client_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clients: Vec<BrowserControlClientSummary>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub clients_omitted: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<BrowserControlActorSnapshot>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub actors_omitted: u32,
    pub scheduler: BrowserControlSchedulerSnapshot,
    pub events: BrowserControlEventWindow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserControlClientSummary {
    pub connection_id: String,
    pub ingress: String,
    pub surface: BrowserClientSurface,
    pub caller: BrowserCallerKind,
    pub provenance_source: BrowserProvenanceSource,
    pub declared_label: Option<String>,
    pub client_info_label: Option<String>,
    pub client_info: Option<BrowserMcpClientInfo>,
}

#[derive(Serialize)]
struct BrowserControlClientSummaryRef<'a> {
    connection_id: &'a str,
    ingress: &'a str,
    surface: BrowserClientSurface,
    caller: BrowserCallerKind,
    provenance_source: BrowserProvenanceSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance_source_detail: Option<BrowserProvenanceSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    declared_label: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_info_label: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_info: Option<&'a BrowserMcpClientInfo>,
}

#[derive(Deserialize)]
struct BrowserControlClientSummaryOwned {
    connection_id: String,
    ingress: String,
    #[serde(default)]
    surface: Option<BrowserClientSurface>,
    caller: BrowserCallerKind,
    provenance_source: BrowserProvenanceSource,
    #[serde(default)]
    provenance_source_detail: Option<BrowserProvenanceSource>,
    #[serde(default)]
    declared_label: Option<String>,
    #[serde(default)]
    client_info_label: Option<String>,
    #[serde(default)]
    client_info: Option<BrowserMcpClientInfo>,
}

impl Serialize for BrowserControlClientSummary {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        BrowserControlClientSummaryRef {
            connection_id: &self.connection_id,
            ingress: &self.ingress,
            surface: self.surface,
            caller: self.caller,
            provenance_source: self.provenance_source.v1_wire_fallback(),
            provenance_source_detail: self.provenance_source.v1_wire_detail(),
            declared_label: self.declared_label.as_deref(),
            client_info_label: self.client_info_label.as_deref(),
            client_info: self.client_info.as_ref(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BrowserControlClientSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BrowserControlClientSummaryOwned::deserialize(deserializer)?;
        let surface = wire.surface.unwrap_or(match wire.ingress.as_str() {
            "codex_compat" => BrowserClientSurface::HostProvidedIab,
            _ => BrowserClientSurface::McpTools,
        });
        Ok(Self {
            connection_id: wire.connection_id,
            ingress: wire.ingress,
            surface,
            caller: wire.caller,
            provenance_source: wire
                .provenance_source_detail
                .unwrap_or(wire.provenance_source),
            declared_label: wire.declared_label,
            client_info_label: wire.client_info_label,
            client_info: wire.client_info,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlActorSnapshot {
    pub state: BrowserBridgeState,
    #[serde(default = "default_browser_bridge_transport")]
    pub transport: BrowserBridgeTransport,
    pub socket_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_instance_id: Option<String>,
    pub browser_instance_stability: BrowserInstanceStability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_start_ticks: Option<u64>,
    pub actor_generation: u64,
    pub protocol_capable: bool,
    pub selected: bool,
    pub canonical: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_rtt_ms: Option<u64>,
    pub reconnect_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarantine_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlSchedulerSnapshot {
    pub queued_count: u32,
    pub in_flight_count: u32,
    pub settlement_pending_count: u32,
    pub settlement_unknown_count: u32,
    pub queued_client_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<BrowserControlGroupSummary>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub groups_omitted: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_operations: Vec<BrowserControlOperationSummary>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub recent_operations_omitted: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlGroupSummary {
    pub group_id: String,
    pub browser_instance_id: String,
    pub owner_principal_id: String,
    pub lease_id: String,
    pub lease_state: BrowserLeaseState,
    pub fence: u64,
    pub expires_at_ms: u64,
    pub admission_state: String,
    pub membership_revision: u64,
    pub member_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<BrowserTabKey>,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub members_omitted: u32,
    pub in_flight_count: u32,
    pub settlement_pending_count: u32,
    pub settlement_unknown_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlOperationSummary {
    pub operation_id: String,
    pub client_id: String,
    pub class: BrowserOperationClass,
    pub state: String,
    pub admitted_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_key: Option<BrowserTabKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<BrowserCompletionCertainty>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlEventWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_sequence: Option<u64>,
    pub dropped_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<BrowserControlEvent>,
}

const fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserLeaseReference {
    pub lease_id: String,
    pub group_id: String,
    pub fence: u64,
    pub membership_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlHello {
    pub protocol_version: u32,
    pub client_info: BrowserControlClientInfo,
    pub caller_provenance: BrowserCallerProvenance,
    pub logical_identity: BrowserLogicalIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlClientInfo {
    pub name: String,
    pub version: String,
    pub adapter_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlHelloOk {
    pub protocol_version: u32,
    pub client_instance_id: String,
    pub logical_session_id: String,
    pub principal_id: String,
    pub daemon_generation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserControlRequest {
    pub request_id: String,
    pub submission_id: String,
    pub upstream_correlation_id: String,
    pub daemon_generation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_identity_delta: Option<BrowserLogicalIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<BrowserLeaseReference>,
    pub operation: BrowserControlOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_deadline_ms: Option<u64>,
}

/// Policy-free operation submitted by a client. The daemon derives the
/// canonical fingerprint, scope, class, deadline, ownership checks, and retry
/// policy; callers cannot authoritatively supply those decisions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserControlOperation {
    HighLevel {
        request: BrowserRequest,
    },
    UpstreamJsonRpc {
        method: String,
        #[serde(default)]
        params: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BrowserControlResponse {
    pub request_id: String,
    pub submission_id: String,
    pub operation_id: String,
    pub completion: BrowserCompletionCertainty,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<BrowserControlDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlCancel {
    pub request_id: String,
    pub submission_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserControlEvent {
    pub event_sequence: u64,
    pub daemon_generation: String,
    pub timestamp_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_key: Option<BrowserTabKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub kind: BrowserControlEventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserControlEventKind {
    ClientState {
        state: String,
        #[serde(default = "legacy_browser_control_client_summary")]
        client: BrowserControlClientSummary,
    },
    BridgeState {
        state: BrowserBridgeState,
    },
    LeaseState {
        state: BrowserLeaseState,
    },
    QueueState {
        depth: u32,
    },
    OperationState {
        state: String,
    },
    Settlement {
        state: String,
    },
    Lifecycle {
        state: String,
    },
    Heartbeat {
        rtt_ms: u64,
    },
    Recovery {
        state: String,
    },
    Failover {
        state: String,
    },
    MigrationDiagnostic {
        code: String,
    },
}

const fn default_browser_bridge_transport() -> BrowserBridgeTransport {
    BrowserBridgeTransport::ExtensionNativeHost
}

fn legacy_browser_control_client_summary() -> BrowserControlClientSummary {
    BrowserControlClientSummary {
        connection_id: String::new(),
        ingress: "legacy_event".to_owned(),
        surface: BrowserClientSurface::McpTools,
        caller: BrowserCallerKind::LegacyUnknown,
        provenance_source: BrowserProvenanceSource::LegacyFallback,
        declared_label: None,
        client_info_label: None,
        client_info: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserControlClientFrame {
    Hello(BrowserControlHello),
    Request(BrowserControlRequest),
    Cancel(BrowserControlCancel),
    EventAck { through_sequence: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserControlServerFrame {
    HelloOk(BrowserControlHelloOk),
    Response(BrowserControlResponse),
    Event(BrowserControlEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserHostHello {
    pub protocol_version: u32,
    pub client_role: BrowserBridgeRole,
    pub daemon_generation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserHostHelloOk {
    pub protocol_version: u32,
    pub host_instance_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_instance_id: Option<String>,
    pub browser_instance_stability: BrowserInstanceStability,
    pub browser_family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_discriminator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;
    use crate::{BrowserCallerKind, BrowserCallerProvenance, BrowserProvenanceSource};

    #[allow(dead_code)]
    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    enum OldV1ProvenanceSource {
        InstallerDeclaration,
        ClientInfoInference,
        TrustedCodexMetadata,
        LegacyFallback,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct OldV1CallerProvenance {
        caller: BrowserCallerKind,
        source: OldV1ProvenanceSource,
        connection_id: String,
        declared_caller: Option<String>,
        client_info: Option<BrowserMcpClientInfo>,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct OldV1ClientSummary {
        connection_id: String,
        ingress: String,
        caller: BrowserCallerKind,
        provenance_source: OldV1ProvenanceSource,
        declared_label: Option<String>,
        client_info_label: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct OldV1ActorSnapshot {
        state: BrowserBridgeState,
        socket_path: String,
        bridge_connection_id: Option<String>,
        browser_instance_id: Option<String>,
        browser_instance_stability: BrowserInstanceStability,
        host_instance_id: Option<String>,
        peer_pid: Option<u32>,
        peer_start_ticks: Option<u64>,
        actor_generation: u64,
        protocol_capable: bool,
        selected: bool,
        canonical: bool,
        last_heartbeat_rtt_ms: Option<u64>,
        reconnect_count: u64,
        quarantine_reason: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    struct OldV1Event {
        event_sequence: u64,
        daemon_generation: String,
        timestamp_ms: u64,
        principal_id: Option<String>,
        group_id: Option<String>,
        tab_key: Option<BrowserTabKey>,
        operation_id: Option<String>,
        kind: OldV1EventKind,
    }

    #[allow(dead_code)]
    #[derive(Debug, Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum OldV1EventKind {
        ClientState { state: String },
    }

    #[test]
    fn host_hello_round_trips_with_explicit_connection_identity() {
        let hello = BrowserHostHello {
            protocol_version: BROWSER_CONTROL_PROTOCOL_VERSION,
            client_role: BrowserBridgeRole::ControlPlane,
            daemon_generation: "daemon-1".into(),
            capabilities: vec!["heartbeat".into(), "extension_events".into()],
        };
        let encoded = serde_json::to_value(&hello).expect("serialize host hello");
        assert_eq!(encoded["client_role"], "control_plane");
        assert_eq!(
            serde_json::from_value::<BrowserHostHello>(encoded).expect("deserialize host hello"),
            hello
        );
    }

    #[test]
    fn legacy_optional_fields_are_omitted() {
        let response = BrowserControlServerFrame::Response(BrowserControlResponse {
            request_id: "request-1".into(),
            submission_id: "submission-1".into(),
            operation_id: "operation-1".into(),
            completion: BrowserCompletionCertainty::DefinitiveSuccess,
            result: Some(json!({ "ok": true })),
            diagnostic: None,
        });
        let encoded = serde_json::to_value(response).expect("serialize response");
        assert!(encoded["diagnostic"].is_null());
    }

    #[test]
    fn typed_client_hello_round_trips_without_operation_identity() {
        let frame = BrowserControlClientFrame::Hello(BrowserControlHello {
            protocol_version: BROWSER_CONTROL_PROTOCOL_VERSION,
            client_info: BrowserControlClientInfo {
                name: "direct-mcp".into(),
                version: "1".into(),
                adapter_version: "1".into(),
            },
            caller_provenance: BrowserCallerProvenance {
                caller: BrowserCallerKind::DirectMcp,
                source: BrowserProvenanceSource::InstallerDeclaration,
                connection_id: "connection-1".into(),
                declared_caller: Some("direct_mcp".into()),
                client_info: None,
            },
            logical_identity: BrowserLogicalIdentity {
                session_id: "session-1".into(),
                thread_id: None,
                turn_id: None,
            },
            capabilities: vec!["events".into()],
            resume_token: None,
        });

        let encoded = serde_json::to_value(&frame).expect("serialize typed hello");
        assert_eq!(encoded["type"], "hello");
        assert_eq!(
            serde_json::from_value::<BrowserControlClientFrame>(encoded)
                .expect("deserialize typed hello"),
            frame
        );
    }

    #[test]
    fn browser_bridge_transport_vocabulary_distinguishes_native_backends() {
        assert_eq!(
            serde_json::to_value(BrowserBridgeTransport::ExtensionNativeHost)
                .expect("serialize extension transport"),
            json!("extension_native_host")
        );
        assert_eq!(
            serde_json::to_value(BrowserBridgeTransport::HostProvidedIab)
                .expect("serialize host IAB transport"),
            json!("host_provided_iab")
        );
        assert_eq!(
            serde_json::from_value::<BrowserBridgeTransport>(json!("host_provided_iab"))
                .expect("deserialize host IAB transport"),
            BrowserBridgeTransport::HostProvidedIab
        );
    }

    #[test]
    fn control_plane_snapshot_round_trips_with_bounded_health_state() {
        let fixture = json!({
            "protocol_version": 1,
            "daemon_generation": "daemon-1",
            "migration_mode": "strict",
            "ready": true,
            "client_count": 2,
            "clients": [{
                "connection_id": "client-1",
                "ingress": "mcp",
                "surface": "mcp_tools",
                "caller": "open_claw",
                "provenance_source": "installer_declaration",
                "declared_label": "openclaw",
                "client_info_label": "openclaw/1"
            }],
            "actors": [{
                "state": "ready",
                "transport": "extension_native_host",
                "socket_path": "/run/user/1000/sky-cua/extension.sock",
                "bridge_connection_id": "bridge-1",
                "browser_instance_id": "browser-1",
                "browser_instance_stability": "stable",
                "host_instance_id": "host-1",
                "peer_pid": 42,
                "peer_start_ticks": 99,
                "actor_generation": 3,
                "protocol_capable": true,
                "selected": true,
                "canonical": true,
                "last_heartbeat_rtt_ms": 7,
                "reconnect_count": 1
            }],
            "scheduler": {
                "queued_count": 1,
                "in_flight_count": 2,
                "settlement_pending_count": 1,
                "settlement_unknown_count": 0,
                "queued_client_count": 1,
                "groups": [{
                    "group_id": "group-1",
                    "browser_instance_id": "browser-1",
                    "owner_principal_id": "principal-1",
                    "lease_id": "lease-1",
                    "lease_state": "active",
                    "fence": 4,
                    "expires_at_ms": 1000,
                    "admission_state": "open",
                    "membership_revision": 2,
                    "member_count": 1,
                    "members": [{
                        "browser_instance_id": "browser-1",
                        "extension_tab_id": "7"
                    }],
                    "in_flight_count": 1,
                    "settlement_pending_count": 1,
                    "settlement_unknown_count": 0
                }],
                "recent_operations": [{
                    "operation_id": "operation-1",
                    "client_id": "client-1",
                    "class": "mutation",
                    "state": "settlement_pending",
                    "admitted_at_ms": 500,
                    "group_id": "group-1",
                    "completion": "ambiguous_completion"
                }]
            },
            "events": {
                "oldest_sequence": 8,
                "newest_sequence": 9,
                "dropped_count": 7,
                "events": [{
                    "event_sequence": 9,
                    "daemon_generation": "daemon-1",
                    "timestamp_ms": 600,
                    "operation_id": "operation-1",
                    "kind": {"type": "operation_state", "state": "settlement_pending"}
                }]
            }
        });

        let snapshot: BrowserControlPlaneSnapshot =
            serde_json::from_value(fixture.clone()).expect("deserialize control plane snapshot");
        assert_eq!(snapshot.events.dropped_count, 7);
        assert_eq!(snapshot.scheduler.groups[0].fence, 4);
        assert_eq!(
            serde_json::to_value(snapshot).expect("serialize control plane snapshot"),
            fixture
        );
    }

    #[test]
    fn new_reader_decodes_serialized_old_v1_client_actor_and_event_payloads() {
        let old_client = json!({
            "connection_id": "legacy-client",
            "ingress": "codex_compat",
            "caller": "codex_desktop",
            "provenance_source": "trusted_codex_metadata",
            "declared_label": "codex_desktop",
            "client_info_label": null
        });
        let client: BrowserControlClientSummary =
            serde_json::from_value(old_client).expect("decode old-v1 client");
        assert_eq!(client.surface, BrowserClientSurface::HostProvidedIab);
        assert_eq!(
            client.provenance_source,
            BrowserProvenanceSource::TrustedCodexMetadata
        );
        assert_eq!(client.client_info, None);

        let old_actor = json!({
            "state": "ready",
            "socket_path": "/run/user/1000/sky-cua/extension.sock",
            "bridge_connection_id": "bridge-1",
            "browser_instance_id": "browser-1",
            "browser_instance_stability": "stable",
            "host_instance_id": "host-1",
            "peer_pid": 42,
            "peer_start_ticks": 99,
            "actor_generation": 3,
            "protocol_capable": true,
            "selected": true,
            "canonical": true,
            "last_heartbeat_rtt_ms": 7,
            "reconnect_count": 1,
            "quarantine_reason": null
        });
        let actor: BrowserControlActorSnapshot =
            serde_json::from_value(old_actor).expect("decode old-v1 actor");
        assert_eq!(actor.transport, BrowserBridgeTransport::ExtensionNativeHost);

        let old_event = json!({
            "event_sequence": 4,
            "daemon_generation": "old-daemon",
            "timestamp_ms": 5,
            "kind": {"type": "client_state", "state": "mcp_connected"}
        });
        let event: BrowserControlEvent =
            serde_json::from_value(old_event).expect("decode old-v1 client-state event");
        let BrowserControlEventKind::ClientState { state, client } = event.kind else {
            panic!("expected client-state event");
        };
        assert_eq!(state, "mcp_connected");
        assert_eq!(client.ingress, "legacy_event");
        assert_eq!(client.caller, BrowserCallerKind::LegacyUnknown);
    }

    #[test]
    fn serialized_new_v1_payloads_remain_readable_by_old_v1_types() {
        let caller_provenance = BrowserCallerProvenance {
            caller: BrowserCallerKind::CodexDesktop,
            source: BrowserProvenanceSource::HostProvidedIab,
            connection_id: "iab-connection".to_owned(),
            declared_caller: None,
            client_info: None,
        };
        let caller_wire = serde_json::to_value(&caller_provenance).expect("encode provenance");
        assert_eq!(caller_wire["source"], "trusted_codex_metadata");
        assert_eq!(caller_wire["source_detail"], "host_provided_iab");
        let old_caller: OldV1CallerProvenance =
            serde_json::from_value(caller_wire.clone()).expect("old reader decodes provenance");
        assert_eq!(
            old_caller.source,
            OldV1ProvenanceSource::TrustedCodexMetadata
        );
        assert_eq!(
            serde_json::from_value::<BrowserCallerProvenance>(caller_wire)
                .expect("current reader decodes exact provenance")
                .source,
            BrowserProvenanceSource::HostProvidedIab
        );

        let client = BrowserControlClientSummary {
            connection_id: "raw-connection".to_owned(),
            ingress: "raw_native_pipe".to_owned(),
            surface: BrowserClientSurface::NodeReplBrowserApi,
            caller: BrowserCallerKind::OpenCode,
            provenance_source: BrowserProvenanceSource::RequestMetadataDeclaration,
            declared_label: Some("opencode".to_owned()),
            client_info_label: Some("opencode/1".to_owned()),
            client_info: Some(BrowserMcpClientInfo {
                name: "opencode".to_owned(),
                version: "1".to_owned(),
                title: None,
            }),
        };
        let client_wire = serde_json::to_value(&client).expect("encode client");
        assert_eq!(client_wire["provenance_source"], "installer_declaration");
        assert_eq!(
            client_wire["provenance_source_detail"],
            "request_metadata_declaration"
        );
        let old_client: OldV1ClientSummary =
            serde_json::from_value(client_wire.clone()).expect("old reader decodes client");
        assert_eq!(
            old_client.provenance_source,
            OldV1ProvenanceSource::InstallerDeclaration
        );
        assert_eq!(
            serde_json::from_value::<BrowserControlClientSummary>(client_wire)
                .expect("current reader decodes exact client")
                .provenance_source,
            BrowserProvenanceSource::RequestMetadataDeclaration
        );

        let actor = BrowserControlActorSnapshot {
            state: BrowserBridgeState::Ready,
            transport: BrowserBridgeTransport::ExtensionNativeHost,
            socket_path: "/tmp/browser.sock".to_owned(),
            bridge_connection_id: Some("bridge-1".to_owned()),
            browser_instance_id: Some("browser-1".to_owned()),
            browser_instance_stability: BrowserInstanceStability::Stable,
            host_instance_id: Some("host-1".to_owned()),
            peer_pid: Some(42),
            peer_start_ticks: Some(99),
            actor_generation: 3,
            protocol_capable: true,
            selected: true,
            canonical: true,
            last_heartbeat_rtt_ms: Some(7),
            reconnect_count: 1,
            quarantine_reason: None,
        };
        let actor_wire = serde_json::to_value(actor).expect("encode actor");
        let old_actor: OldV1ActorSnapshot =
            serde_json::from_value(actor_wire).expect("old reader decodes actor");
        assert_eq!(old_actor.state, BrowserBridgeState::Ready);

        let event = BrowserControlEvent {
            event_sequence: 6,
            daemon_generation: "new-daemon".to_owned(),
            timestamp_ms: 7,
            principal_id: None,
            group_id: None,
            tab_key: None,
            operation_id: None,
            kind: BrowserControlEventKind::ClientState {
                state: "raw_native_pipe_connected".to_owned(),
                client,
            },
        };
        let event_wire = serde_json::to_value(event).expect("encode event");
        let old_event: OldV1Event =
            serde_json::from_value(event_wire).expect("old reader decodes event");
        let OldV1EventKind::ClientState { state } = old_event.kind;
        assert_eq!(state, "raw_native_pipe_connected");
    }
}
