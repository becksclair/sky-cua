use std::collections::{HashMap, HashSet, VecDeque};

use super::super::{
    control::{AdmissionError, QueueLimits, Reply},
    group::GroupRegistry,
    introspection::EventRecorder,
    lease::LeaseProof,
    operation::{
        BrowserInstanceId, ClientId, Completion, DispatchOperation, GroupId, OperationClass,
        OperationId, Principal, SettlementOutcome, TabKey,
    },
    persistence::{JournalWriter, RecoveryJournal},
};

mod actor;
mod admission;
mod dispatch;
mod introspection;

pub(super) use actor::run_actor;

struct OperationRecord {
    dispatch: DispatchOperation,
    lease: Option<LeaseProof>,
    state: OperationState,
    waiters: Vec<Reply<Result<Completion, AdmissionError>>>,
    completion: Option<Completion>,
    admitted_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperationState {
    Queued,
    Dispatched,
    SettlementPending { deadline_ms: u64 },
    SettlementUnknown { deadline_ms: u64 },
    Terminal,
}

#[derive(Default)]
struct BridgeSchedule {
    globals: VecDeque<OperationId>,
    global_in_flight: bool,
    tab_round: Option<HashSet<TabKey>>,
}

struct State {
    generation: String,
    next_operation: u64,
    limits: QueueLimits,
    operations: HashMap<OperationId, OperationRecord>,
    recent: VecDeque<OperationId>,
    tab_queues: HashMap<TabKey, VecDeque<OperationId>>,
    tab_in_flight: HashSet<TabKey>,
    bridge_dispatch_in_flight: HashMap<BrowserInstanceId, usize>,
    bridges: HashMap<BrowserInstanceId, BridgeSchedule>,
    daemon_globals: VecDeque<OperationId>,
    daemon_global_in_flight: bool,
    daemon_tab_round: Option<HashSet<TabKey>>,
    queued_by_client: HashMap<ClientId, usize>,
    groups: GroupRegistry,
    in_flight_by_group: HashMap<GroupId, usize>,
    early_settlements: HashMap<OperationId, SettlementOutcome>,
    cancellation_intents: HashMap<OperationId, Option<ClientId>>,
    cancellation_intent_order: VecDeque<OperationId>,
    now_ms: u64,
    events: EventRecorder,
    persistence: Option<JournalWriter>,
    last_journal: Option<RecoveryJournal>,
}

impl State {
    fn new(
        generation: String,
        limits: QueueLimits,
        groups: GroupRegistry,
        events: EventRecorder,
        persistence: Option<JournalWriter>,
    ) -> Self {
        Self {
            generation,
            next_operation: 0,
            limits,
            operations: HashMap::new(),
            recent: VecDeque::new(),
            tab_queues: HashMap::new(),
            tab_in_flight: HashSet::new(),
            bridge_dispatch_in_flight: HashMap::new(),
            bridges: HashMap::new(),
            daemon_globals: VecDeque::new(),
            daemon_global_in_flight: false,
            daemon_tab_round: None,
            queued_by_client: HashMap::new(),
            groups,
            in_flight_by_group: HashMap::new(),
            early_settlements: HashMap::new(),
            cancellation_intents: HashMap::new(),
            cancellation_intent_order: VecDeque::new(),
            now_ms: 0,
            events,
            persistence,
            last_journal: None,
        }
    }
}
