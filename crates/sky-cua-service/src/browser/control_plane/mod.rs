//! Daemon-local browser scheduling and ownership core.
//!
//! This module deliberately has no dependency on the current browser transport.
//! A later integration layer can implement [`Executor`] for the persistent bridge
//! actor without moving scheduling or lease policy into that actor.

#![allow(dead_code, unused_imports)] // Staged behind the fake executor until WP-04 routes ingress.

mod bridge_actor;
mod control;
mod group;
mod introspection;
mod lease;
mod operation;
mod persistence;
mod persistent_proxy;
mod scheduler;

mod integration;

#[cfg(test)]
mod bridge_actor_tests;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod tests;

pub(crate) use bridge_actor::{
    BridgeActor, BridgeActorConfig, BridgeActorError, BridgeActorEvent, BridgeActorHealth,
    BridgeActorRequest, BridgeActorState, BridgeOwnerMode, BridgeRequestSize,
    fixed_width_daemon_generation,
};
pub(crate) use control::{
    AdmissionError, CancelResult, ControlPlane, QueueLimits, SubmitOperation,
};
pub(crate) use group::{GroupAdmission, GroupError, GroupSnapshot};
pub(crate) use integration::BrowserControlRuntime;
pub(crate) use lease::{IDLE_LEASE_MS, LeaseState};
pub(crate) use operation::{
    BrowserInstanceId, ClientId, Completion, CompletionCertainty, CompletionDisposition,
    DispatchOperation, Executor, ExecutorOutcome, GroupId, OperationClass, OperationId,
    OperationScope, Principal, SETTLEMENT_DEADLINE_MS, SettlementOutcome, SettlementResult,
    SettlementState, TabKey, UpstreamCorrelation,
};
pub(crate) use persistence::{RecoveryGroupHint, RecoveryJournal};
pub(super) use persistent_proxy::connect as connect_persistent_proxy;
pub(super) use persistent_proxy::{
    capture as capture_persistent_proxy, scope_captured as scope_persistent_proxy,
};
