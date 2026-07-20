use super::*;

#[derive(Clone)]
pub(in crate::browser::control_plane) struct ActorEntry {
    pub(in crate::browser::control_plane) actor: BridgeActor,
    pub(in crate::browser::control_plane) socket: PathBuf,
    pub(in crate::browser::control_plane) browser_id: String,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::browser::control_plane) enum ServerRequestId {
    Number(String),
    String(String),
}

impl ServerRequestId {
    pub(super) fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Number(number) => Some(Self::Number(number.to_string())),
            Value::String(string) => Some(Self::String(string.clone())),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(in crate::browser::control_plane) struct Shared {
    pub(in crate::browser::control_plane) actors: RwLock<HashMap<PathBuf, ActorEntry>>,
    pub(super) groups: Mutex<HashMap<(String, String, String), GroupId>>,
    pub(in crate::browser::control_plane) tab_owners: Mutex<HashMap<TabKey, GroupId>>,
    pub(super) ownership_indexes_initialized: Mutex<bool>,
    pub(super) operation_browsers: Mutex<HashMap<OperationId, BrowserInstanceId>>,
    pub(super) operation_clients: Mutex<HashMap<OperationId, String>>,
    pub(super) settlement_fences: Mutex<HashMap<OperationId, SettlementFence>>,
    pub(super) handled_settlements: Mutex<VecDeque<SettlementAckIdentity>>,
    pub(in crate::browser::control_plane) terminal_settlement_operations:
        Mutex<VecDeque<TerminalSettlementOperation>>,
    pub(super) settlement_parents: Arc<Mutex<HashMap<OperationId, OperationId>>>,
    /// Top-level raw Codex operations currently executing on a tab. The
    /// upstream Browser client can synchronously answer extension CDP events
    /// (for example `Fetch.requestPaused`) while the initiating command is
    /// still waiting. Those continuation commands are bridge subrequests of
    /// the parent, not independent same-tab work that may wait in the FIFO.
    pub(in crate::browser::control_plane) raw_tab_parents:
        Mutex<HashMap<(String, TabKey), OperationId>>,
    pub(in crate::browser::control_plane) connections:
        Mutex<HashMap<String, (Principal, crate::codex_browser_compat::CodexOutbound)>>,
    pub(in crate::browser::control_plane) codex_connection_sessions:
        Mutex<HashMap<String, HashSet<String>>>,
    pub(in crate::browser::control_plane) connection_principals:
        Mutex<HashMap<String, HashMap<String, Principal>>>,
    pub(in crate::browser::control_plane) principal_connections:
        Mutex<HashMap<String, HashSet<String>>>,
    pub(in crate::browser::control_plane) mcp_connections: Mutex<McpConnectionLifecycle>,
    pub(in crate::browser::control_plane) operation_reservations:
        Mutex<HashMap<OperationId, OperationReservation>>,
    pub(in crate::browser::control_plane) codex_by_browser: Mutex<HashMap<String, HashSet<String>>>,
    pub(super) server_request_routes: Mutex<HashMap<(String, ServerRequestId), BridgeActor>>,
    pub(in crate::browser::control_plane) clients:
        RwLock<HashMap<String, BrowserControlClientSummary>>,
    pub(in crate::browser::control_plane) control: std::sync::OnceLock<ControlPlane>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::browser::control_plane) struct SettlementAckIdentity {
    pub(super) operation_id: String,
    pub(super) daemon_generation: String,
    pub(super) actor_generation: Value,
    pub(super) chrome_request_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::browser::control_plane) struct TerminalSettlementOperation {
    pub(in crate::browser::control_plane) operation_id: OperationId,
    pub(in crate::browser::control_plane) daemon_generation: String,
}

#[derive(Default)]
pub(in crate::browser::control_plane) struct McpConnectionLifecycle {
    pub(in crate::browser::control_plane) closed: HashSet<String>,
    pub(in crate::browser::control_plane) active_requests: HashMap<String, usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::browser::control_plane) struct OperationReservation {
    pub(in crate::browser::control_plane) tab: TabKey,
    pub(in crate::browser::control_plane) group_id: GroupId,
    pub(in crate::browser::control_plane) principal: Principal,
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::browser::control_plane) struct SettlementFence {
    pub(in crate::browser::control_plane) daemon_generation: String,
    pub(in crate::browser::control_plane) actor_generation: Value,
    pub(in crate::browser::control_plane) browser_instance_id: BrowserInstanceId,
    pub(in crate::browser::control_plane) target_lifetime_key: Value,
    pub(in crate::browser::control_plane) operation_class: &'static str,
}

#[derive(Clone)]
pub(in crate::browser::control_plane) struct IntegrationExecutor {
    pub(in crate::browser::control_plane) shared: Arc<Shared>,
}

pub(crate) struct BrowserControlRuntime {
    pub(super) generation: String,
    pub(super) owner_mode: BridgeOwnerMode,
    pub(in crate::browser::control_plane) control: ControlPlane,
    pub(in crate::browser::control_plane) shared: Arc<Shared>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::browser::control_plane) enum IntegrationPayload {
    HighLevel {
        request: BrowserRequest,
        identity: BrowserSessionIdentity,
    },
    Raw {
        method: String,
        params: Value,
        timeout_ms: u64,
        identity: BrowserSessionIdentity,
    },
}
