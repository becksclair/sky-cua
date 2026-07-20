use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sky_cua_platform::model::{
    BROWSER_CONTROL_PROTOCOL_VERSION, BrowserBridgeState, BrowserCallerKind,
    BrowserCallerProvenance, BrowserControlActorSnapshot, BrowserControlClientSummary,
    BrowserControlEventKind, BrowserControlPlaneSnapshot, BrowserIntegrationReport,
    BrowserMigrationMode, BrowserProvenanceSource, BrowserRequest, BrowserRequestContext,
    BrowserResponse, BrowserSessionIdentity, BrowserStatusReport, BrowserTabKey,
    BrowserTargetAvailability, BrowserTargetKind, DiagnosticEntry,
};
use tokio::sync::{Mutex, mpsc};

use super::{
    AdmissionError, BridgeActor, BridgeActorConfig, BridgeActorError, BridgeActorEvent,
    BridgeActorRequest, BridgeActorState, BridgeOwnerMode, BrowserInstanceId, CancelResult,
    ClientId, Completion, CompletionCertainty, CompletionDisposition, ControlPlane, Executor,
    ExecutorOutcome, GroupAdmission, GroupError, GroupId, GroupSnapshot, OperationClass,
    OperationId, OperationScope, Principal, QueueLimits, SettlementOutcome, SettlementResult,
    SubmitOperation, TabKey, UpstreamCorrelation, fixed_width_daemon_generation, persistence,
    persistent_proxy::{self, ChildTracker, ProxyContext},
};
use crate::{
    browser::sockets::{browser_socket_selection_from_env, find_bridge_sockets},
    codex_browser_compat::{
        CodexBackendReply, CodexBrowserBackend, CodexCallerLifecycle, CodexConnectionContext,
        CodexNormalizedRequest, CodexOperationClass, CodexOperationScope,
    },
};

use super::introspection;

mod actor_events;
mod codex;
mod executor;
mod runtime;
mod state;
mod support;

#[cfg(test)]
pub(in crate::browser::control_plane) use actor_events::spawn_actor_event_receiver_for_test;
pub(in crate::browser::control_plane) use actor_events::{
    settle_actor_message, settle_late_response, spawn_actor_events,
};
pub(crate) use state::BrowserControlRuntime;
pub(super) use state::{
    ActorEntry, IntegrationExecutor, IntegrationPayload, OperationReservation, ServerRequestId,
    SettlementAckIdentity, SettlementFence, Shared, TerminalSettlementOperation,
};
pub(in crate::browser::control_plane) use support::*;

const RUNTIME_ERROR_CODE: i64 = -32072;
const CLIENT_RESULT_LIMIT: usize = 64;
const CLIENT_LABEL_LIMIT: usize = 128;
const ACTOR_RESULT_LIMIT: usize = 32;
const LEASE_TICK_INTERVAL: Duration = Duration::from_secs(1);
