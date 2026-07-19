use std::{
    collections::{BTreeSet, HashSet},
    path::PathBuf,
    sync::Arc,
};

use tokio::sync::{mpsc, oneshot};

use sky_cua_platform::model::BrowserControlSchedulerSnapshot;

use super::{
    group::{GroupError, GroupRegistry, GroupSnapshot},
    introspection::EventRecorder,
    lease::{LeaseProof, LeaseSnapshot},
    operation::{
        BrowserInstanceId, ClientId, Completion, Executor, GroupId, OperationClass, OperationId,
        OperationScope, Principal, SettlementOutcome, SettlementResult, SettlementState, TabKey,
        UpstreamCorrelation,
    },
    persistence::{self, JournalWriter, RecoveryJournal},
    scheduler,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QueueLimits {
    pub(crate) per_client: usize,
    pub(crate) per_tab: usize,
    pub(crate) per_bridge_dispatch: usize,
    pub(crate) recent_operations: usize,
}

impl Default for QueueLimits {
    fn default() -> Self {
        Self {
            per_client: 128,
            per_tab: 32,
            per_bridge_dispatch: 2,
            recent_operations: 512,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SubmitOperation {
    pub(crate) operation_id: Option<OperationId>,
    pub(crate) canonical_fingerprint: String,
    pub(crate) upstream: UpstreamCorrelation,
    pub(crate) client_id: ClientId,
    pub(crate) principal: Principal,
    pub(crate) group_id: Option<GroupId>,
    pub(crate) lease: Option<LeaseProof>,
    pub(crate) scope: OperationScope,
    pub(crate) class: OperationClass,
    pub(crate) payload: String,
    pub(crate) now_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionError {
    Backpressure,
    OperationIdCollision,
    StaleGeneration,
    Group(GroupError),
    LeaseRequired,
    ActorStopped,
}

impl From<GroupError> for AdmissionError {
    fn from(value: GroupError) -> Self {
        Self::Group(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CancelResult {
    CancelledBeforeDispatch,
    WaiterDetached,
    AlreadyTerminal(Completion),
    UnknownOperation,
}

pub(super) type Reply<T> = oneshot::Sender<T>;

pub(super) enum Command {
    Submit(
        Box<SubmitOperation>,
        Reply<Result<Completion, AdmissionError>>,
    ),
    Cancel(OperationId, Reply<CancelResult>),
    Executed(OperationId, super::operation::ExecutorOutcome),
    Settle(OperationId, SettlementOutcome, Reply<SettlementResult>),
    SettlementState(OperationId, Reply<Option<SettlementState>>),
    CreateGroup {
        group_id: GroupId,
        browser: BrowserInstanceId,
        principal: Principal,
        now_ms: u64,
        reply: Reply<GroupSnapshot>,
    },
    AddMember {
        group_id: GroupId,
        principal: Principal,
        tab: TabKey,
        reply: Reply<Result<GroupSnapshot, GroupError>>,
    },
    Group(GroupId, Reply<Result<GroupSnapshot, GroupError>>),
    Groups(Reply<Vec<GroupSnapshot>>),
    BrowserLost(BrowserInstanceId, Reply<Vec<GroupId>>),
    Renew(
        LeaseProof,
        Principal,
        u64,
        Reply<Result<LeaseSnapshot, GroupError>>,
    ),
    Offer {
        group_id: GroupId,
        principal: Principal,
        target: Principal,
        revision: u64,
        reply: Reply<Result<GroupSnapshot, GroupError>>,
    },
    Accept {
        group_id: GroupId,
        target: Principal,
        revision: u64,
        now_ms: u64,
        reply: Reply<Result<GroupSnapshot, GroupError>>,
    },
    Force {
        group_id: GroupId,
        requester: Principal,
        target: Principal,
        revision: u64,
        now_ms: u64,
        reply: Reply<Result<GroupSnapshot, GroupError>>,
    },
    Disconnect(Principal, u64, Reply<()>),
    EndGroup(GroupId, Principal, Reply<Result<GroupSnapshot, GroupError>>),
    Tick(u64, Reply<()>),
    ResumeRecovered {
        group_id: GroupId,
        browser: BrowserInstanceId,
        principal: Principal,
        members: BTreeSet<TabKey>,
        revision: u64,
        now_ms: u64,
        reply: Reply<Result<GroupSnapshot, GroupError>>,
    },
    Snapshot(Reply<BrowserControlSchedulerSnapshot>),
}

#[derive(Clone)]
pub(crate) struct ControlPlane {
    sender: mpsc::UnboundedSender<Command>,
    generation: String,
    pub(super) events: EventRecorder,
    persistence: Option<JournalWriter>,
}

impl ControlPlane {
    pub(crate) fn start(
        generation: impl Into<String>,
        executor: Arc<dyn Executor>,
        limits: QueueLimits,
    ) -> Self {
        Self::start_with_groups(generation, executor, limits, GroupRegistry::default(), None)
    }

    pub(crate) fn recover(
        generation: impl Into<String>,
        executor: Arc<dyn Executor>,
        limits: QueueLimits,
        journal: &RecoveryJournal,
    ) -> Self {
        Self::start_with_groups(
            generation,
            executor,
            limits,
            journal.restore_suspended(),
            None,
        )
    }

    pub(crate) fn recover_persistent(
        generation: impl Into<String>,
        executor: Arc<dyn Executor>,
        limits: QueueLimits,
        path: PathBuf,
    ) -> Self {
        let generation = generation.into();
        let (journal, load_failure) = match persistence::load(&path) {
            Ok(journal) => (journal, None),
            Err(failure) => (RecoveryJournal::empty(), Some(failure)),
        };
        let groups = journal.restore_suspended();
        let checkpoint = RecoveryJournal::capture(&groups, &HashSet::new());
        let checkpoint_failure = persistence::write_atomic(&path, &checkpoint).err();
        let events = EventRecorder::new(generation.clone());
        let writer = JournalWriter::spawn(path, events.clone());
        let control = Self::start_with_groups_and_events(
            generation,
            executor,
            limits,
            groups,
            Some(writer),
            events,
        );
        if let Some(failure) = load_failure {
            control.events.record(
                sky_cua_platform::model::BrowserControlEventKind::Recovery {
                    state: failure.code.to_owned(),
                },
                super::introspection::EventContext::default(),
            );
            tracing::warn!(code = failure.code, detail = %failure.detail, "browser recovery journal ignored");
        } else if !journal.groups.is_empty() {
            control.events.record(
                sky_cua_platform::model::BrowserControlEventKind::Recovery {
                    state: "journal_loaded_suspended".to_owned(),
                },
                super::introspection::EventContext::default(),
            );
        }
        if let Some(error) = checkpoint_failure {
            control.events.record(
                sky_cua_platform::model::BrowserControlEventKind::Recovery {
                    state: format!("journal_write_failed:{}", error.kind()),
                },
                super::introspection::EventContext::default(),
            );
            tracing::warn!(detail = %error, "browser recovery journal checkpoint failed");
        }
        control
    }

    fn start_with_groups(
        generation: impl Into<String>,
        executor: Arc<dyn Executor>,
        limits: QueueLimits,
        groups: GroupRegistry,
        persistence: Option<JournalWriter>,
    ) -> Self {
        let generation = generation.into();
        let events = EventRecorder::new(generation.clone());
        Self::start_with_groups_and_events(
            generation,
            executor,
            limits,
            groups,
            persistence,
            events,
        )
    }

    fn start_with_groups_and_events(
        generation: String,
        executor: Arc<dyn Executor>,
        limits: QueueLimits,
        groups: GroupRegistry,
        persistence: Option<JournalWriter>,
        events: EventRecorder,
    ) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        scheduler::spawn_actor(
            receiver,
            sender.clone(),
            executor,
            scheduler::ActorConfig {
                generation: generation.clone(),
                limits,
                groups,
                events: events.clone(),
                persistence: persistence.clone(),
            },
        );
        Self {
            sender,
            generation,
            events,
            persistence,
        }
    }

    pub(crate) fn generation(&self) -> &str {
        &self.generation
    }

    pub(crate) async fn submit(
        &self,
        operation: SubmitOperation,
    ) -> Result<Completion, AdmissionError> {
        let (reply, receive) = oneshot::channel();
        self.sender
            .send(Command::Submit(Box::new(operation), reply))
            .map_err(|_| AdmissionError::ActorStopped)?;
        receive.await.unwrap_or(Err(AdmissionError::ActorStopped))
    }

    pub(crate) async fn cancel(&self, operation_id: OperationId) -> CancelResult {
        self.call(|reply| Command::Cancel(operation_id, reply))
            .await
            .unwrap_or(CancelResult::UnknownOperation)
    }

    pub(crate) async fn settle(
        &self,
        operation_id: OperationId,
        outcome: SettlementOutcome,
    ) -> SettlementResult {
        self.call(|reply| Command::Settle(operation_id, outcome, reply))
            .await
            .unwrap_or(SettlementResult::Ignored)
    }

    pub(crate) async fn settlement_state(
        &self,
        operation_id: OperationId,
    ) -> Option<SettlementState> {
        self.call(|reply| Command::SettlementState(operation_id, reply))
            .await
            .flatten()
    }

    pub(crate) async fn create_group(
        &self,
        group_id: GroupId,
        browser: BrowserInstanceId,
        principal: Principal,
        now_ms: u64,
    ) -> GroupSnapshot {
        self.call(|reply| Command::CreateGroup {
            group_id,
            browser,
            principal,
            now_ms,
            reply,
        })
        .await
        .expect("control plane stopped while creating group")
    }

    pub(crate) async fn add_member(
        &self,
        group_id: GroupId,
        principal: Principal,
        tab: TabKey,
    ) -> Result<GroupSnapshot, GroupError> {
        self.call(|reply| Command::AddMember {
            group_id,
            principal,
            tab,
            reply,
        })
        .await
        .unwrap_or(Err(GroupError::UnknownGroup))
    }

    pub(crate) async fn group(&self, group_id: GroupId) -> Result<GroupSnapshot, GroupError> {
        self.call(|reply| Command::Group(group_id, reply))
            .await
            .unwrap_or(Err(GroupError::UnknownGroup))
    }

    pub(crate) async fn groups(&self) -> Vec<GroupSnapshot> {
        self.call(Command::Groups).await.unwrap_or_default()
    }

    pub(crate) async fn browser_lost(&self, browser: BrowserInstanceId) -> Vec<GroupId> {
        self.call(|reply| Command::BrowserLost(browser, reply))
            .await
            .unwrap_or_default()
    }

    pub(crate) async fn renew(
        &self,
        proof: LeaseProof,
        principal: Principal,
        now_ms: u64,
    ) -> Result<LeaseSnapshot, GroupError> {
        self.call(|reply| Command::Renew(proof, principal, now_ms, reply))
            .await
            .unwrap_or(Err(GroupError::UnknownGroup))
    }

    pub(crate) async fn offer_handoff(
        &self,
        group_id: GroupId,
        principal: Principal,
        target: Principal,
        revision: u64,
    ) -> Result<GroupSnapshot, GroupError> {
        self.call(|reply| Command::Offer {
            group_id,
            principal,
            target,
            revision,
            reply,
        })
        .await
        .unwrap_or(Err(GroupError::UnknownGroup))
    }

    pub(crate) async fn accept_handoff(
        &self,
        group_id: GroupId,
        target: Principal,
        revision: u64,
        now_ms: u64,
    ) -> Result<GroupSnapshot, GroupError> {
        self.call(|reply| Command::Accept {
            group_id,
            target,
            revision,
            now_ms,
            reply,
        })
        .await
        .unwrap_or(Err(GroupError::UnknownGroup))
    }

    pub(crate) async fn force_handoff(
        &self,
        group_id: GroupId,
        requester: Principal,
        target: Principal,
        revision: u64,
        now_ms: u64,
    ) -> Result<GroupSnapshot, GroupError> {
        self.call(|reply| Command::Force {
            group_id,
            requester,
            target,
            revision,
            now_ms,
            reply,
        })
        .await
        .unwrap_or(Err(GroupError::UnknownGroup))
    }

    pub(crate) async fn disconnect(&self, principal: Principal, now_ms: u64) {
        let _ = self
            .call(|reply| Command::Disconnect(principal, now_ms, reply))
            .await;
    }

    pub(crate) async fn end_group(
        &self,
        group_id: GroupId,
        principal: Principal,
    ) -> Result<GroupSnapshot, GroupError> {
        self.call(|reply| Command::EndGroup(group_id, principal, reply))
            .await
            .unwrap_or(Err(GroupError::UnknownGroup))
    }

    pub(crate) async fn tick(&self, now_ms: u64) {
        let _ = self.call(|reply| Command::Tick(now_ms, reply)).await;
    }

    /// Resume a restart hint only after the caller has independently reconciled
    /// the exact browser identity and complete tab membership.
    pub(crate) async fn resume_recovered(
        &self,
        group_id: GroupId,
        browser: BrowserInstanceId,
        principal: Principal,
        members: BTreeSet<TabKey>,
        revision: u64,
        now_ms: u64,
    ) -> Result<GroupSnapshot, GroupError> {
        self.call(|reply| Command::ResumeRecovered {
            group_id,
            browser,
            principal,
            members,
            revision,
            now_ms,
            reply,
        })
        .await
        .unwrap_or(Err(GroupError::UnknownGroup))
    }

    #[cfg(test)]
    pub(crate) fn flush_persistence(&self) {
        if let Some(writer) = &self.persistence {
            writer.flush();
        }
    }

    pub(crate) async fn snapshot(&self) -> BrowserControlSchedulerSnapshot {
        self.call(Command::Snapshot).await.unwrap_or_default()
    }

    async fn call<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Option<T> {
        let (reply, receive) = oneshot::channel();
        self.sender.send(command(reply)).ok()?;
        receive.await.ok()
    }
}
