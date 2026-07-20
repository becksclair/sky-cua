use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use tokio::sync::{mpsc, oneshot};

use super::*;

struct FakeExecutor {
    started: mpsc::UnboundedSender<DispatchOperation>,
    finishes: Mutex<HashMap<String, oneshot::Receiver<ExecutorOutcome>>>,
}

impl FakeExecutor {
    fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<DispatchOperation>) {
        let (started, receive) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                started,
                finishes: Mutex::new(HashMap::new()),
            }),
            receive,
        )
    }

    fn hold(&self, payload: &str) -> oneshot::Sender<ExecutorOutcome> {
        let (send, receive) = oneshot::channel();
        self.finishes
            .lock()
            .expect("fake executor lock")
            .insert(payload.to_owned(), receive);
        send
    }
}

impl Executor for FakeExecutor {
    fn execute(
        &self,
        operation: DispatchOperation,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ExecutorOutcome> + Send + 'static>>
    {
        let _ = self.started.send(operation.clone());
        let finish = self
            .finishes
            .lock()
            .expect("fake executor lock")
            .remove(&operation.payload);
        Box::pin(async move {
            match finish {
                Some(finish) => finish
                    .await
                    .unwrap_or_else(|_| ExecutorOutcome::DefinitiveFailure("fake dropped".into())),
                None => ExecutorOutcome::DefinitiveSuccess(operation.payload),
            }
        })
    }
}

fn recovery_test_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!("sky-cua-control-recovery-{label}-{nonce}"))
        .join(persistence::RECOVERY_JOURNAL_FILE)
}

struct Fixture {
    control: ControlPlane,
    fake: Arc<FakeExecutor>,
    started: mpsc::UnboundedReceiver<DispatchOperation>,
    owner: Principal,
    group: GroupSnapshot,
    tab_a: TabKey,
    tab_b: TabKey,
}

async fn fixture() -> Fixture {
    let (fake, started) = FakeExecutor::new();
    let control = ControlPlane::start("g1", fake.clone(), QueueLimits::default());
    let owner = Principal::new("owner", 1000);
    let browser = BrowserInstanceId::from("browser-a");
    let group_id = GroupId::from("group-a");
    control
        .create_group(group_id.clone(), browser.clone(), owner.clone(), 0)
        .await;
    let tab_a = TabKey::new(browser.clone(), "1");
    let tab_b = TabKey::new(browser, "2");
    control
        .add_member(group_id.clone(), owner.clone(), tab_a.clone())
        .await
        .unwrap();
    let group = control
        .add_member(group_id, owner.clone(), tab_b.clone())
        .await
        .unwrap();
    Fixture {
        control,
        fake,
        started,
        owner,
        group,
        tab_a,
        tab_b,
    }
}

#[tokio::test]
async fn settlement_arriving_before_executor_ambiguity_is_buffered_and_consumed() {
    let mut f = fixture().await;
    let finish = f.fake.hold("early-settlement");
    let control = f.control.clone();
    let operation = tab_operation(&f, "early", &f.tab_a, "early-settlement");
    let submitted = tokio::spawn(async move { control.submit(operation).await.unwrap() });
    assert_eq!(f.started.recv().await.unwrap().payload, "early-settlement");

    assert_eq!(
        f.control
            .settle(
                OperationId::from("early"),
                SettlementOutcome::DefinitiveSuccess("late success".into()),
            )
            .await,
        SettlementResult::RemainsAmbiguous
    );
    finish
        .send(ExecutorOutcome::Ambiguous("connection lost".into()))
        .unwrap();
    let settled = submitted.await.unwrap();
    assert_eq!(settled.disposition, CompletionDisposition::Success);
    assert_eq!(settled.detail, "late success");
    assert_eq!(
        f.control.settlement_state(OperationId::from("early")).await,
        None
    );

    let next = tab_operation(&f, "after-early", &f.tab_a, "after-early");
    let control = f.control.clone();
    let next_submit = tokio::spawn(async move { control.submit(next).await.unwrap() });
    assert_eq!(f.started.recv().await.unwrap().payload, "after-early");
    assert_eq!(
        next_submit.await.unwrap().disposition,
        CompletionDisposition::Success
    );
}

#[tokio::test]
async fn scheduler_snapshot_reports_group_queue_and_settlement_counts() {
    let mut f = fixture().await;
    let finish_barrier = f.fake.hold("snapshot-barrier");
    let barrier = spawn_submit(
        &f.control,
        global_operation(
            "snapshot-barrier",
            OperationScope::BridgeGlobal(f.tab_a.browser_instance_id.clone()),
            "snapshot-barrier",
        ),
    );
    assert_eq!(f.started.recv().await.unwrap().payload, "snapshot-barrier");
    let queued = spawn_submit(
        &f.control,
        tab_operation(&f, "snapshot-queued", &f.tab_a, "snapshot-queued"),
    );
    tokio::task::yield_now().await;
    let snapshot = f.control.snapshot().await;
    assert_eq!(snapshot.queued_count, 1);
    assert_eq!(snapshot.in_flight_count, 1);
    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].member_count, 2);
    assert_eq!(snapshot.groups[0].membership_revision, 2);
    assert_eq!(snapshot.groups[0].fence, 1);
    finish_barrier
        .send(ExecutorOutcome::DefinitiveSuccess("done".into()))
        .unwrap();
    let _ = barrier.await.unwrap();
    assert_eq!(f.started.recv().await.unwrap().payload, "snapshot-queued");
    let _ = queued.await.unwrap();

    let finish = f.fake.hold("snapshot-ambiguous");
    let operation = tab_operation(&f, "snapshot-settlement", &f.tab_a, "snapshot-ambiguous");
    let submitted = spawn_submit(&f.control, operation);
    assert_eq!(
        f.started.recv().await.unwrap().payload,
        "snapshot-ambiguous"
    );
    finish
        .send(ExecutorOutcome::Ambiguous("connection lost".into()))
        .unwrap();
    let _ = submitted.await.unwrap();

    let snapshot = f.control.snapshot().await;
    assert_eq!(snapshot.queued_count, 0);
    assert_eq!(snapshot.in_flight_count, 0);
    assert_eq!(snapshot.settlement_pending_count, 1);
    assert_eq!(snapshot.settlement_unknown_count, 0);

    f.control.tick(SETTLEMENT_DEADLINE_MS + 1).await;
    let snapshot = f.control.snapshot().await;
    assert_eq!(snapshot.settlement_pending_count, 1);
    assert_eq!(snapshot.settlement_unknown_count, 1);
    assert_eq!(snapshot.groups[0].admission_state, "recovery_required");
}

fn tab_operation(f: &Fixture, id: &str, tab: &TabKey, payload: &str) -> SubmitOperation {
    SubmitOperation {
        operation_id: Some(OperationId::from(id)),
        canonical_fingerprint: format!("fingerprint:{payload}"),
        upstream: UpstreamCorrelation {
            ingress: "test".into(),
            request_id: Some(format!("upstream:{id}")),
        },
        client_id: ClientId::from("client-a"),
        principal: f.owner.clone(),
        group_id: Some(f.group.group_id.clone()),
        lease: Some(f.group.lease.proof()),
        scope: OperationScope::Tab(tab.clone()),
        class: OperationClass::Mutation,
        payload: payload.into(),
        now_ms: 1,
    }
}

fn global_operation(id: &str, scope: OperationScope, payload: &str) -> SubmitOperation {
    SubmitOperation {
        operation_id: Some(OperationId::from(id)),
        canonical_fingerprint: format!("fingerprint:{payload}"),
        upstream: UpstreamCorrelation {
            ingress: "test".into(),
            request_id: None,
        },
        client_id: ClientId::from("operator"),
        principal: Principal::new("operator", 1000),
        group_id: None,
        lease: None,
        scope,
        class: OperationClass::BrowserGlobal,
        payload: payload.into(),
        now_ms: 1,
    }
}

fn group_global_operation(f: &Fixture, id: &str, payload: &str) -> SubmitOperation {
    let mut operation = global_operation(
        id,
        OperationScope::BridgeGlobal(f.group.browser_instance_id.clone()),
        payload,
    );
    operation.principal = f.owner.clone();
    operation.group_id = Some(f.group.group_id.clone());
    operation
}

fn spawn_submit(
    control: &ControlPlane,
    operation: SubmitOperation,
) -> tokio::task::JoinHandle<Result<Completion, AdmissionError>> {
    let control = control.clone();
    tokio::spawn(async move { control.submit(operation).await })
}

async fn next_started(started: &mut mpsc::UnboundedReceiver<DispatchOperation>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(1), started.recv())
        .await
        .expect("operation did not dispatch")
        .expect("executor channel closed")
        .payload
}

#[tokio::test]
async fn same_tab_is_fifo_with_at_most_one_in_flight() {
    let mut f = fixture().await;
    let finish_a = f.fake.hold("a1");
    let finish_b = f.fake.hold("a2");
    let a = spawn_submit(&f.control, tab_operation(&f, "a1", &f.tab_a, "a1"));
    let b = spawn_submit(&f.control, tab_operation(&f, "a2", &f.tab_a, "a2"));

    assert_eq!(next_started(&mut f.started).await, "a1");
    assert!(f.started.try_recv().is_err());
    finish_a
        .send(ExecutorOutcome::DefinitiveSuccess("a1 done".into()))
        .unwrap();
    assert_eq!(next_started(&mut f.started).await, "a2");
    finish_b
        .send(ExecutorOutcome::DefinitiveSuccess("a2 done".into()))
        .unwrap();
    assert_eq!(
        a.await.unwrap().unwrap().disposition,
        CompletionDisposition::Success
    );
    assert_eq!(
        b.await.unwrap().unwrap().disposition,
        CompletionDisposition::Success
    );
}

#[tokio::test]
async fn independent_tabs_overlap_while_executor_awaits() {
    let mut f = fixture().await;
    let finish_a = f.fake.hold("a");
    let finish_b = f.fake.hold("b");
    let a = spawn_submit(&f.control, tab_operation(&f, "a", &f.tab_a, "a"));
    let b = spawn_submit(&f.control, tab_operation(&f, "b", &f.tab_b, "b"));

    let mut dispatched = BTreeSet::new();
    dispatched.insert(next_started(&mut f.started).await);
    dispatched.insert(next_started(&mut f.started).await);
    assert_eq!(dispatched, BTreeSet::from(["a".to_owned(), "b".to_owned()]));
    finish_a
        .send(ExecutorOutcome::DefinitiveSuccess("a".into()))
        .unwrap();
    finish_b
        .send(ExecutorOutcome::DefinitiveSuccess("b".into()))
        .unwrap();
    a.await.unwrap().unwrap();
    b.await.unwrap().unwrap();
}

#[tokio::test]
async fn per_bridge_dispatch_width_is_enforced_before_executor_spawn() {
    let (fake, mut started) = FakeExecutor::new();
    let mut finish_a = Some(fake.hold("bounded-a"));
    let mut finish_b = Some(fake.hold("bounded-b"));
    let control = ControlPlane::start(
        "bounded",
        fake,
        QueueLimits {
            per_bridge_dispatch: 1,
            ..QueueLimits::default()
        },
    );
    let owner = Principal::new("bounded-owner", 1000);
    let browser = BrowserInstanceId::from("bounded-browser");
    let group = control
        .create_group(
            GroupId::from("bounded-group"),
            browser.clone(),
            owner.clone(),
            0,
        )
        .await;
    let tab_a = TabKey::new(browser.clone(), "a");
    let tab_b = TabKey::new(browser, "b");
    control
        .add_member(group.group_id.clone(), owner.clone(), tab_a.clone())
        .await
        .unwrap();
    let group = control
        .add_member(group.group_id, owner.clone(), tab_b.clone())
        .await
        .unwrap();
    let operation = |id: &str, tab: TabKey| SubmitOperation {
        operation_id: Some(OperationId::from(id)),
        canonical_fingerprint: id.to_owned(),
        upstream: UpstreamCorrelation {
            ingress: "test".into(),
            request_id: None,
        },
        client_id: ClientId::from("bounded-client"),
        principal: owner.clone(),
        group_id: Some(group.group_id.clone()),
        lease: Some(group.lease.proof()),
        scope: OperationScope::Tab(tab),
        class: OperationClass::ReadOnly,
        payload: id.to_owned(),
        now_ms: 1,
    };
    let a = spawn_submit(&control, operation("bounded-a", tab_a));
    let b = spawn_submit(&control, operation("bounded-b", tab_b));
    let first = next_started(&mut started).await;
    assert!(started.try_recv().is_err());
    let expected_second = if first == "bounded-a" {
        finish_a
            .take()
            .unwrap()
            .send(ExecutorOutcome::DefinitiveSuccess("a".into()))
            .unwrap();
        "bounded-b"
    } else {
        finish_b
            .take()
            .unwrap()
            .send(ExecutorOutcome::DefinitiveSuccess("b".into()))
            .unwrap();
        "bounded-a"
    };
    assert_eq!(next_started(&mut started).await, expected_second);
    if expected_second == "bounded-a" {
        finish_a
            .take()
            .unwrap()
            .send(ExecutorOutcome::DefinitiveSuccess("a".into()))
            .unwrap();
    } else {
        finish_b
            .take()
            .unwrap()
            .send(ExecutorOutcome::DefinitiveSuccess("b".into()))
            .unwrap();
    }
    a.await.unwrap().unwrap();
    b.await.unwrap().unwrap();
}

#[tokio::test]
async fn bridge_and_daemon_globals_are_exclusive_barriers() {
    let mut f = fixture().await;
    let finish_a = f.fake.hold("a");
    let finish_bridge = f.fake.hold("bridge");
    let finish_b = f.fake.hold("b");
    let finish_daemon = f.fake.hold("daemon");
    let finish_c = f.fake.hold("c");

    let a = spawn_submit(&f.control, tab_operation(&f, "a", &f.tab_a, "a"));
    assert_eq!(next_started(&mut f.started).await, "a");
    let bridge = spawn_submit(
        &f.control,
        global_operation(
            "bridge",
            OperationScope::BridgeGlobal(f.tab_a.browser_instance_id.clone()),
            "bridge",
        ),
    );
    let b = spawn_submit(&f.control, tab_operation(&f, "b", &f.tab_b, "b"));
    assert!(f.started.try_recv().is_err());
    finish_a
        .send(ExecutorOutcome::DefinitiveSuccess("a".into()))
        .unwrap();
    assert_eq!(next_started(&mut f.started).await, "bridge");
    finish_bridge
        .send(ExecutorOutcome::DefinitiveSuccess("bridge".into()))
        .unwrap();
    assert_eq!(next_started(&mut f.started).await, "b");

    let daemon = spawn_submit(
        &f.control,
        global_operation("daemon", OperationScope::DaemonGlobal, "daemon"),
    );
    let c = spawn_submit(&f.control, tab_operation(&f, "c", &f.tab_a, "c"));
    assert!(f.started.try_recv().is_err());
    finish_b
        .send(ExecutorOutcome::DefinitiveSuccess("b".into()))
        .unwrap();
    assert_eq!(next_started(&mut f.started).await, "daemon");
    finish_daemon
        .send(ExecutorOutcome::DefinitiveSuccess("daemon".into()))
        .unwrap();
    assert_eq!(next_started(&mut f.started).await, "c");
    finish_c
        .send(ExecutorOutcome::DefinitiveSuccess("c".into()))
        .unwrap();

    for task in [a, bridge, b, daemon, c] {
        task.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn bridge_global_fairness_admits_one_tab_hol_round() {
    let mut f = fixture().await;
    let finish_g1 = f.fake.hold("g1");
    let finish_g2 = f.fake.hold("g2");
    let finish_a = f.fake.hold("a");
    let finish_b = f.fake.hold("b");
    let scope = OperationScope::BridgeGlobal(f.tab_a.browser_instance_id.clone());
    let g1 = spawn_submit(&f.control, global_operation("g1", scope.clone(), "g1"));
    assert_eq!(next_started(&mut f.started).await, "g1");
    let g2 = spawn_submit(&f.control, global_operation("g2", scope, "g2"));
    let a = spawn_submit(&f.control, tab_operation(&f, "a", &f.tab_a, "a"));
    let b = spawn_submit(&f.control, tab_operation(&f, "b", &f.tab_b, "b"));
    finish_g1
        .send(ExecutorOutcome::DefinitiveSuccess("g1".into()))
        .unwrap();

    let mut round = BTreeSet::new();
    round.insert(next_started(&mut f.started).await);
    round.insert(next_started(&mut f.started).await);
    assert_eq!(round, BTreeSet::from(["a".to_owned(), "b".to_owned()]));
    assert!(f.started.try_recv().is_err());
    finish_a
        .send(ExecutorOutcome::DefinitiveSuccess("a".into()))
        .unwrap();
    finish_b
        .send(ExecutorOutcome::DefinitiveSuccess("b".into()))
        .unwrap();
    assert_eq!(next_started(&mut f.started).await, "g2");
    finish_g2
        .send(ExecutorOutcome::DefinitiveSuccess("g2".into()))
        .unwrap();
    for task in [g1, g2, a, b] {
        task.await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn group_scoped_bridge_globals_validate_owner_and_admission_without_a_tab_lease() {
    let mut f = fixture().await;
    let admitted = spawn_submit(
        &f.control,
        group_global_operation(&f, "group-open", "group-open"),
    );
    assert_eq!(next_started(&mut f.started).await, "group-open");
    assert_eq!(
        admitted.await.unwrap().unwrap().disposition,
        CompletionDisposition::Success
    );

    let mut wrong_owner = group_global_operation(&f, "wrong-owner", "wrong-owner");
    wrong_owner.principal = Principal::new("other", 1000);
    assert_eq!(
        f.control.submit(wrong_owner).await,
        Err(AdmissionError::Group(GroupError::WrongPrincipal))
    );

    f.control
        .offer_handoff(
            f.group.group_id.clone(),
            f.owner.clone(),
            Principal::new("target", 1000),
            f.group.membership_revision,
        )
        .await
        .unwrap();
    assert_eq!(
        f.control
            .submit(group_global_operation(
                &f,
                "handoff-closed",
                "handoff-closed",
            ))
            .await,
        Err(AdmissionError::Group(GroupError::AdmissionClosed))
    );

    let released = fixture().await;
    released.control.tick(IDLE_LEASE_MS + 1).await;
    assert_eq!(
        released
            .control
            .submit(group_global_operation(
                &released,
                "released-closed",
                "released-closed",
            ))
            .await,
        Err(AdmissionError::Group(GroupError::AdmissionClosed))
    );
}

#[tokio::test]
async fn queued_group_global_is_revalidated_before_dispatch() {
    let mut f = fixture().await;
    let finish_barrier = f.fake.hold("global-barrier");
    let barrier = spawn_submit(
        &f.control,
        global_operation(
            "global-barrier",
            OperationScope::BridgeGlobal(f.group.browser_instance_id.clone()),
            "global-barrier",
        ),
    );
    assert_eq!(next_started(&mut f.started).await, "global-barrier");
    let queued = spawn_submit(
        &f.control,
        group_global_operation(&f, "stale-global", "stale-global"),
    );
    tokio::task::yield_now().await;

    f.control
        .force_handoff(
            f.group.group_id.clone(),
            Principal::new("operator", 1000),
            Principal::new("target", 1000),
            f.group.membership_revision,
            10,
        )
        .await
        .unwrap();
    finish_barrier
        .send(ExecutorOutcome::DefinitiveSuccess("barrier done".into()))
        .unwrap();
    barrier.await.unwrap().unwrap();

    let rejected = queued.await.unwrap().unwrap();
    assert_eq!(rejected.certainty, CompletionCertainty::PreDispatchRejected);
    assert_eq!(rejected.disposition, CompletionDisposition::Failure);
    assert!(rejected.detail.contains("WrongPrincipal"));
    assert!(f.started.try_recv().is_err());
}

#[tokio::test]
async fn handoff_checks_membership_revision_and_rejects_stale_fence() {
    let f = fixture().await;
    assert_eq!(f.group.membership_revision, 2);
    let target = Principal::new("target", 1000);
    assert_eq!(
        f.control
            .offer_handoff(f.group.group_id.clone(), f.owner.clone(), target.clone(), 1,)
            .await,
        Err(GroupError::StaleMembershipRevision)
    );
    f.control
        .offer_handoff(f.group.group_id.clone(), f.owner.clone(), target.clone(), 2)
        .await
        .unwrap();
    let moved = f
        .control
        .accept_handoff(f.group.group_id.clone(), target.clone(), 2, 10)
        .await
        .unwrap();
    assert_eq!(moved.membership_revision, 2);
    assert_eq!(moved.lease.fence, f.group.lease.fence + 1);

    let mut stale = tab_operation(&f, "stale", &f.tab_a, "stale");
    stale.principal = target;
    assert_eq!(
        f.control.submit(stale).await,
        Err(AdmissionError::Group(GroupError::StaleFence))
    );
}

#[tokio::test]
async fn failed_force_handoff_leaves_the_full_group_snapshot_unchanged() {
    let mut f = fixture().await;
    let finish = f.fake.hold("handoff-in-flight");
    let operation = spawn_submit(
        &f.control,
        tab_operation(&f, "handoff-in-flight", &f.tab_a, "handoff-in-flight"),
    );
    assert_eq!(next_started(&mut f.started).await, "handoff-in-flight");
    let before = f.control.group(f.group.group_id.clone()).await.unwrap();

    assert_eq!(
        f.control
            .force_handoff(
                f.group.group_id.clone(),
                Principal::new("operator", 1000),
                Principal::new("target", 1000),
                f.group.membership_revision,
                10,
            )
            .await,
        Err(GroupError::InFlight)
    );
    assert_eq!(
        f.control.group(f.group.group_id.clone()).await.unwrap(),
        before
    );

    finish
        .send(ExecutorOutcome::DefinitiveSuccess("done".into()))
        .unwrap();
    operation.await.unwrap().unwrap();
    let offered = f
        .control
        .offer_handoff(
            f.group.group_id.clone(),
            f.owner.clone(),
            Principal::new("target", 1000),
            f.group.membership_revision,
        )
        .await
        .unwrap();
    assert!(matches!(
        offered.admission,
        GroupAdmission::HandoffPending(_)
    ));
}

#[tokio::test]
async fn disconnect_grace_is_capped_and_idle_expiry_releases() {
    let f = fixture().await;
    f.control
        .disconnect(f.owner.clone(), IDLE_LEASE_MS - 1_000)
        .await;
    let orphaned = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_eq!(
        orphaned.lease.state,
        LeaseState::OrphanedGrace {
            grace_until_ms: IDLE_LEASE_MS
        }
    );
    f.control.tick(IDLE_LEASE_MS).await;
    let released = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_eq!(released.admission, GroupAdmission::Released);
    assert_eq!(released.lease.fence, f.group.lease.fence + 1);
}

#[tokio::test]
async fn reconnect_inside_disconnect_grace_restores_active_lease() {
    let f = fixture().await;
    f.control.disconnect(f.owner.clone(), 10).await;
    let renewed = f
        .control
        .renew(f.group.lease.proof(), f.owner.clone(), 20)
        .await
        .unwrap();
    assert_eq!(renewed.state, LeaseState::Active);
    assert_eq!(renewed.expires_at_ms, 20 + IDLE_LEASE_MS);
    f.control.tick(10 + super::lease::DISCONNECT_GRACE_MS).await;
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::Open
    );
}

#[tokio::test]
async fn reconnect_at_disconnect_grace_deadline_is_rejected() {
    let f = fixture().await;
    f.control.disconnect(f.owner.clone(), 10).await;
    assert_eq!(
        f.control
            .renew(
                f.group.lease.proof(),
                f.owner.clone(),
                10 + super::lease::DISCONNECT_GRACE_MS,
            )
            .await,
        Err(GroupError::AdmissionClosed)
    );
    f.control.tick(10 + super::lease::DISCONNECT_GRACE_MS).await;
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::Released
    );
}

#[tokio::test]
async fn disconnect_orphans_handoff_pending_without_preserving_authority_past_grace() {
    let f = fixture().await;
    let pending = f
        .control
        .offer_handoff(
            f.group.group_id.clone(),
            f.owner.clone(),
            Principal::new("target", 1000),
            f.group.membership_revision,
        )
        .await
        .unwrap();
    f.control.disconnect(f.owner.clone(), 10).await;
    let orphaned = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_eq!(orphaned.admission, pending.admission);
    assert_eq!(
        orphaned.lease.state,
        LeaseState::OrphanedGrace {
            grace_until_ms: 10 + super::lease::DISCONNECT_GRACE_MS
        }
    );
    let reconnected = f
        .control
        .renew(f.group.lease.proof(), f.owner.clone(), 20)
        .await
        .unwrap();
    assert_eq!(reconnected.state, LeaseState::Active);
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        pending.admission
    );
    f.control.disconnect(f.owner.clone(), 30).await;

    f.control.tick(30 + super::lease::DISCONNECT_GRACE_MS).await;
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::Released
    );
}

#[tokio::test]
async fn disconnect_grace_defers_settlement_pending_release_until_settlement() {
    let mut f = fixture().await;
    let finish = f.fake.hold("disconnect-settlement");
    let operation = spawn_submit(
        &f.control,
        tab_operation(
            &f,
            "disconnect-settlement",
            &f.tab_a,
            "disconnect-settlement",
        ),
    );
    assert_eq!(next_started(&mut f.started).await, "disconnect-settlement");
    finish
        .send(ExecutorOutcome::Ambiguous("uncertain".into()))
        .unwrap();
    operation.await.unwrap().unwrap();
    assert_eq!(
        f.control
            .submit(group_global_operation(
                &f,
                "settlement-closed",
                "settlement-closed",
            ))
            .await,
        Err(AdmissionError::Group(GroupError::SettlementRequired))
    );

    f.control.disconnect(f.owner.clone(), 10).await;
    let orphaned = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_eq!(orphaned.admission, GroupAdmission::SettlementPending);
    assert_eq!(
        orphaned.lease.state,
        LeaseState::OrphanedGrace {
            grace_until_ms: 10 + super::lease::DISCONNECT_GRACE_MS
        }
    );

    f.control.tick(10 + super::lease::DISCONNECT_GRACE_MS).await;
    let expired = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_eq!(expired.admission, GroupAdmission::RecoveryRequired);
    assert_eq!(expired.lease.state, LeaseState::ExpiryPending);
    f.control
        .settle(
            OperationId::from("disconnect-settlement"),
            SettlementOutcome::DefinitiveSuccess("settled".into()),
        )
        .await;
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::Released
    );
}

#[tokio::test]
async fn disconnect_orphans_expiry_pending_until_in_flight_work_finishes() {
    let mut f = fixture().await;
    let finish = f.fake.hold("disconnect-expiry");
    let operation = spawn_submit(
        &f.control,
        tab_operation(&f, "disconnect-expiry", &f.tab_a, "disconnect-expiry"),
    );
    assert_eq!(next_started(&mut f.started).await, "disconnect-expiry");
    let pending = f
        .control
        .end_group(f.group.group_id.clone(), f.owner.clone())
        .await
        .unwrap();
    assert_eq!(pending.admission, GroupAdmission::ExpiryPending);

    f.control.disconnect(f.owner.clone(), 10).await;
    let orphaned = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_eq!(orphaned.admission, GroupAdmission::ExpiryPending);
    assert_eq!(
        orphaned.lease.state,
        LeaseState::OrphanedGrace {
            grace_until_ms: 10 + super::lease::DISCONNECT_GRACE_MS
        }
    );
    assert_eq!(
        operation.await.unwrap().unwrap().disposition,
        CompletionDisposition::WaiterDetached
    );

    f.control.tick(10 + super::lease::DISCONNECT_GRACE_MS).await;
    let still_pending = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_eq!(still_pending.admission, GroupAdmission::ExpiryPending);
    assert_eq!(still_pending.lease.state, LeaseState::ExpiryPending);

    finish
        .send(ExecutorOutcome::DefinitiveSuccess("done".into()))
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if f.control
                .group(f.group.group_id.clone())
                .await
                .is_ok_and(|group| group.admission == GroupAdmission::Released)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("group did not release after in-flight completion");
}

#[tokio::test]
async fn cancellation_before_dispatch_and_after_dispatch_have_distinct_certainty() {
    let mut f = fixture().await;
    let finish_first = f.fake.hold("first");
    let finish_live = f.fake.hold("live");
    let first = spawn_submit(&f.control, tab_operation(&f, "first", &f.tab_a, "first"));
    assert_eq!(next_started(&mut f.started).await, "first");
    let queued = spawn_submit(&f.control, tab_operation(&f, "queued", &f.tab_a, "queued"));
    tokio::task::yield_now().await;
    assert_eq!(
        f.control.cancel(OperationId::from("queued")).await,
        CancelResult::CancelledBeforeDispatch
    );
    let cancelled = queued.await.unwrap().unwrap();
    assert_eq!(
        cancelled.certainty,
        CompletionCertainty::PreDispatchRejected
    );
    finish_first
        .send(ExecutorOutcome::DefinitiveSuccess("first".into()))
        .unwrap();
    first.await.unwrap().unwrap();

    let live = spawn_submit(&f.control, tab_operation(&f, "live", &f.tab_a, "live"));
    assert_eq!(next_started(&mut f.started).await, "live");
    assert_eq!(
        f.control.cancel(OperationId::from("live")).await,
        CancelResult::WaiterDetached
    );
    let detached = live.await.unwrap().unwrap();
    assert_eq!(detached.disposition, CompletionDisposition::WaiterDetached);
    finish_live
        .send(ExecutorOutcome::DefinitiveSuccess(
            "actually completed".into(),
        ))
        .unwrap();
    tokio::task::yield_now().await;
    let cached = f
        .control
        .submit(tab_operation(&f, "live", &f.tab_a, "live"))
        .await
        .unwrap();
    assert_eq!(cached.certainty, CompletionCertainty::Definitive);
}

#[tokio::test]
async fn cancellation_before_scheduler_registration_is_consumed_without_dispatch() {
    let (fake, mut started) = FakeExecutor::new();
    let control = ControlPlane::start("cancel-intent", fake, QueueLimits::default());
    let operation_id = OperationId::from("cancel-before-registration");

    assert_eq!(
        control
            .cancel_for_client(operation_id.clone(), ClientId::from("operator"))
            .await,
        CancelResult::UnknownOperation
    );
    let completion = control
        .submit(global_operation(
            &operation_id.0,
            OperationScope::DaemonGlobal,
            "must-not-dispatch",
        ))
        .await
        .unwrap();

    assert_eq!(
        completion.certainty,
        CompletionCertainty::PreDispatchRejected
    );
    assert_eq!(
        completion.disposition,
        CompletionDisposition::CancelledBeforeDispatch
    );
    assert!(started.try_recv().is_err());
}

#[tokio::test]
async fn pre_registration_cancellation_is_scoped_to_the_submitting_client() {
    let (fake, mut started) = FakeExecutor::new();
    let control = ControlPlane::start("scoped-cancel-intent", fake, QueueLimits::default());
    let operation_id = OperationId::from("scoped-cancel-before-registration");

    assert_eq!(
        control
            .cancel_for_client(operation_id.clone(), ClientId::from("different-client"))
            .await,
        CancelResult::UnknownOperation
    );
    let completion = control
        .submit(global_operation(
            &operation_id.0,
            OperationScope::DaemonGlobal,
            "dispatch-for-owner",
        ))
        .await
        .unwrap();

    assert_eq!(
        next_started(&mut started).await,
        "dispatch-for-owner".to_owned()
    );
    assert_eq!(completion.disposition, CompletionDisposition::Success);
}

#[tokio::test]
async fn concurrent_group_creation_preserves_the_first_lease_and_membership() {
    let (fake, _started) = FakeExecutor::new();
    let control = ControlPlane::start("atomic-groups", fake, QueueLimits::default());
    let group_id = GroupId::from("atomic-default-group");
    let browser = BrowserInstanceId::from("atomic-browser");
    let principal = Principal::new("atomic-owner", 1000);
    let tab = TabKey::new(browser.clone(), "member");
    let created = control
        .create_group(group_id.clone(), browser.clone(), principal.clone(), 1)
        .await;
    let with_member = control
        .add_member(group_id.clone(), principal.clone(), tab.clone())
        .await
        .unwrap();

    let mut creates = tokio::task::JoinSet::new();
    for now_ms in 2..18 {
        let control = control.clone();
        let group_id = group_id.clone();
        let browser = browser.clone();
        let principal = principal.clone();
        creates.spawn(async move {
            control
                .create_group(group_id, browser, principal, now_ms)
                .await
        });
    }
    while let Some(result) = creates.join_next().await {
        let group = result.unwrap();
        assert_eq!(group.lease.lease_id, created.lease.lease_id);
        assert_eq!(group.members, with_member.members);
        assert_eq!(group.membership_revision, with_member.membership_revision);
    }
}

#[tokio::test]
async fn bounded_submit_ingress_returns_backpressure_before_actor_admission() {
    let (fake, _started) = FakeExecutor::new();
    let control = ControlPlane::start(
        "bounded-ingress",
        fake,
        QueueLimits {
            submit_ingress: 1,
            ..QueueLimits::default()
        },
    );
    let first = control.submit(global_operation(
        "ingress-first",
        OperationScope::DaemonGlobal,
        "ingress-first",
    ));
    let second = control.submit(global_operation(
        "ingress-second",
        OperationScope::DaemonGlobal,
        "ingress-second",
    ));

    let (first, second) = tokio::join!(first, second);
    assert!(matches!(
        (&first, &second),
        (Ok(_), Err(AdmissionError::Backpressure)) | (Err(AdmissionError::Backpressure), Ok(_))
    ));
    assert_eq!(control.snapshot().await.queued_count, 0);
}

#[tokio::test]
async fn released_groups_are_pruned_only_after_in_flight_and_settlement_state_clear() {
    let (fake, _started) = FakeExecutor::new();
    let control = ControlPlane::start("prune-released", fake, QueueLimits::default());
    let principal = Principal::new("prune-owner", 1000);
    let group_id = GroupId::from("prune-group");
    control
        .create_group(
            group_id.clone(),
            BrowserInstanceId::from("prune-browser"),
            principal.clone(),
            0,
        )
        .await;
    let released = control
        .end_group(group_id.clone(), principal)
        .await
        .unwrap();
    assert_eq!(released.admission, GroupAdmission::Released);

    assert_eq!(control.prune_released().await, vec![group_id.clone()]);
    assert_eq!(control.group(group_id).await, Err(GroupError::UnknownGroup));
}

#[tokio::test]
async fn operation_ids_dedupe_in_generation_and_reject_collisions_and_old_generation() {
    let mut f = fixture().await;
    let finish = f.fake.hold("dedupe");
    let first = spawn_submit(&f.control, tab_operation(&f, "shared", &f.tab_a, "dedupe"));
    assert_eq!(next_started(&mut f.started).await, "dedupe");
    let duplicate = spawn_submit(&f.control, tab_operation(&f, "shared", &f.tab_a, "dedupe"));
    let collision = f
        .control
        .submit(tab_operation(&f, "shared", &f.tab_a, "different"))
        .await;
    assert_eq!(collision, Err(AdmissionError::OperationIdCollision));
    finish
        .send(ExecutorOutcome::DefinitiveSuccess("done".into()))
        .unwrap();
    assert_eq!(
        first.await.unwrap().unwrap(),
        duplicate.await.unwrap().unwrap()
    );
    assert!(f.started.try_recv().is_err());

    let client_owned = tab_operation(&f, "op-client-9", &f.tab_b, "client-owned");
    let client_owned = f.control.submit(client_owned).await.unwrap();
    assert_eq!(client_owned.certainty, CompletionCertainty::Definitive);

    let mut old = tab_operation(&f, "daemon-op-old-9", &f.tab_a, "old");
    old.operation_id = Some(OperationId::from("daemon-op-old-9"));
    assert_eq!(
        f.control.submit(old).await,
        Err(AdmissionError::StaleGeneration)
    );
}

#[tokio::test]
async fn ambiguous_mutation_waits_for_late_success_and_blocks_force_handoff() {
    let mut f = fixture().await;
    let finish = f.fake.hold("ambiguous");
    let finish_after = f.fake.hold("after-settlement");
    let operation = spawn_submit(
        &f.control,
        tab_operation(&f, "ambiguous", &f.tab_a, "ambiguous"),
    );
    assert_eq!(next_started(&mut f.started).await, "ambiguous");
    let after = spawn_submit(
        &f.control,
        tab_operation(&f, "after-settlement", &f.tab_a, "after-settlement"),
    );
    finish
        .send(ExecutorOutcome::Ambiguous(
            "transport lost after send".into(),
        ))
        .unwrap();
    let completion = operation.await.unwrap().unwrap();
    assert_eq!(completion.certainty, CompletionCertainty::Ambiguous);
    assert_eq!(
        completion.disposition,
        CompletionDisposition::WaiterDetached
    );
    assert_eq!(
        f.control
            .settlement_state(OperationId::from("ambiguous"))
            .await,
        Some(SettlementState::Pending {
            deadline_ms: 1 + SETTLEMENT_DEADLINE_MS
        })
    );
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::SettlementPending
    );
    let cached = f
        .control
        .submit(tab_operation(&f, "ambiguous", &f.tab_a, "ambiguous"))
        .await
        .unwrap();
    assert_eq!(cached, completion);
    assert_eq!(
        f.control
            .submit(tab_operation(
                &f,
                "new-during-settlement",
                &f.tab_b,
                "must-not-admit",
            ))
            .await,
        Err(AdmissionError::Group(GroupError::SettlementRequired))
    );
    assert!(f.started.try_recv().is_err());
    assert_eq!(
        f.control
            .force_handoff(
                f.group.group_id.clone(),
                Principal::new("operator", 1000),
                Principal::new("target", 1000),
                f.group.membership_revision,
                20,
            )
            .await,
        Err(GroupError::SettlementRequired)
    );
    let settled = f
        .control
        .settle(
            OperationId::from("ambiguous"),
            SettlementOutcome::DefinitiveSuccess("late host success".into()),
        )
        .await;
    assert!(matches!(
        settled,
        SettlementResult::Settled(Completion {
            disposition: CompletionDisposition::Success,
            ..
        })
    ));
    assert_eq!(next_started(&mut f.started).await, "after-settlement");
    finish_after
        .send(ExecutorOutcome::DefinitiveSuccess("after done".into()))
        .unwrap();
    after.await.unwrap().unwrap();
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::Open
    );
}

#[tokio::test]
async fn multiple_ambiguous_mutations_keep_group_blocked_until_all_settle() {
    let mut f = fixture().await;
    let finish_a = f.fake.hold("pending-a");
    let finish_b = f.fake.hold("pending-b");
    let a = spawn_submit(
        &f.control,
        tab_operation(&f, "pending-a", &f.tab_a, "pending-a"),
    );
    let b = spawn_submit(
        &f.control,
        tab_operation(&f, "pending-b", &f.tab_b, "pending-b"),
    );
    next_started(&mut f.started).await;
    next_started(&mut f.started).await;
    finish_a
        .send(ExecutorOutcome::Ambiguous("a uncertain".into()))
        .unwrap();
    finish_b
        .send(ExecutorOutcome::Ambiguous("b uncertain".into()))
        .unwrap();
    a.await.unwrap().unwrap();
    b.await.unwrap().unwrap();

    f.control
        .settle(
            OperationId::from("pending-a"),
            SettlementOutcome::DefinitiveSuccess("a done".into()),
        )
        .await;
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::SettlementPending
    );
    f.control
        .settle(
            OperationId::from("pending-b"),
            SettlementOutcome::ProvenPreDispatchFailure("b never dispatched".into()),
        )
        .await;
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::Open
    );
}

#[tokio::test]
async fn settlement_deadline_becomes_unknown_but_late_success_still_resolves() {
    let mut f = fixture().await;
    let finish = f.fake.hold("deadline");
    let operation = spawn_submit(
        &f.control,
        tab_operation(&f, "deadline", &f.tab_a, "deadline"),
    );
    next_started(&mut f.started).await;
    finish
        .send(ExecutorOutcome::Ambiguous("timeout".into()))
        .unwrap();
    operation.await.unwrap().unwrap();
    f.control.tick(1 + SETTLEMENT_DEADLINE_MS).await;
    assert!(matches!(
        f.control
            .settlement_state(OperationId::from("deadline"))
            .await,
        Some(SettlementState::Unknown { .. })
    ));
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::RecoveryRequired
    );
    assert_eq!(
        f.control
            .force_handoff(
                f.group.group_id.clone(),
                Principal::new("operator", 1000),
                Principal::new("target", 1000),
                f.group.membership_revision,
                1 + SETTLEMENT_DEADLINE_MS,
            )
            .await,
        Err(GroupError::SettlementRequired)
    );
    assert!(matches!(
        f.control
            .settle(
                OperationId::from("deadline"),
                SettlementOutcome::DefinitiveSuccess("eventually done".into()),
            )
            .await,
        SettlementResult::Settled(_)
    ));
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::Open
    );
}

#[tokio::test]
async fn matching_error_does_not_settle_ambiguous_mutation() {
    let mut f = fixture().await;
    let finish = f.fake.hold("error");
    let operation = spawn_submit(&f.control, tab_operation(&f, "error", &f.tab_a, "error"));
    next_started(&mut f.started).await;
    finish
        .send(ExecutorOutcome::Ambiguous("transport error".into()))
        .unwrap();
    operation.await.unwrap().unwrap();
    assert_eq!(
        f.control
            .settle(
                OperationId::from("error"),
                SettlementOutcome::Error("matching generic error".into()),
            )
            .await,
        SettlementResult::RemainsAmbiguous
    );
    assert!(matches!(
        f.control.settlement_state(OperationId::from("error")).await,
        Some(SettlementState::Pending { .. })
    ));
}

#[tokio::test]
async fn exact_target_loss_settles_without_retaining_old_tab_key() {
    let mut f = fixture().await;
    let finish = f.fake.hold("lost");
    let operation = spawn_submit(&f.control, tab_operation(&f, "lost", &f.tab_a, "lost"));
    next_started(&mut f.started).await;
    finish
        .send(ExecutorOutcome::Ambiguous("target vanished".into()))
        .unwrap();
    operation.await.unwrap().unwrap();
    let result = f
        .control
        .settle(
            OperationId::from("lost"),
            SettlementOutcome::TargetLost(f.tab_a.clone()),
        )
        .await;
    assert!(matches!(
        result,
        SettlementResult::Settled(Completion {
            disposition: CompletionDisposition::TargetLost,
            ..
        })
    ));
    let group = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert!(!group.members.contains(&f.tab_a));
    assert!(group.members.contains(&f.tab_b));
}

#[tokio::test]
async fn expiry_and_force_stay_blocked_until_settlement() {
    let mut f = fixture().await;
    let finish = f.fake.hold("expiry-pending");
    let operation = spawn_submit(
        &f.control,
        tab_operation(&f, "expiry-pending", &f.tab_a, "expiry-pending"),
    );
    next_started(&mut f.started).await;
    finish
        .send(ExecutorOutcome::Ambiguous("uncertain".into()))
        .unwrap();
    operation.await.unwrap().unwrap();
    f.control.tick(IDLE_LEASE_MS + 1).await;
    let blocked = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_ne!(blocked.admission, GroupAdmission::Released);
    assert_eq!(blocked.lease.fence, f.group.lease.fence);
    assert_eq!(
        f.control
            .force_handoff(
                f.group.group_id.clone(),
                Principal::new("operator", 1000),
                Principal::new("target", 1000),
                f.group.membership_revision,
                IDLE_LEASE_MS + 1,
            )
            .await,
        Err(GroupError::SettlementRequired)
    );
    f.control
        .settle(
            OperationId::from("expiry-pending"),
            SettlementOutcome::DefinitiveSuccess("late".into()),
        )
        .await;
    let released = f.control.group(f.group.group_id.clone()).await.unwrap();
    assert_eq!(released.admission, GroupAdmission::Released);
    assert_eq!(released.lease.fence, f.group.lease.fence + 1);
}

#[tokio::test]
async fn duplicate_and_foreign_settlement_events_are_ignored() {
    let mut f = fixture().await;
    let finish = f.fake.hold("correlated");
    let operation = spawn_submit(
        &f.control,
        tab_operation(&f, "correlated", &f.tab_a, "correlated"),
    );
    next_started(&mut f.started).await;
    finish
        .send(ExecutorOutcome::Ambiguous("uncertain".into()))
        .unwrap();
    operation.await.unwrap().unwrap();
    assert_eq!(
        f.control
            .settle(
                OperationId::from("foreign"),
                SettlementOutcome::DefinitiveSuccess("wrong op".into()),
            )
            .await,
        SettlementResult::Ignored
    );
    assert_eq!(
        f.control
            .settle(
                OperationId::from("correlated"),
                SettlementOutcome::TargetLost(f.tab_b.clone()),
            )
            .await,
        SettlementResult::Ignored
    );
    assert!(matches!(
        f.control
            .settle(
                OperationId::from("correlated"),
                SettlementOutcome::DefinitiveSuccess("done".into()),
            )
            .await,
        SettlementResult::Settled(_)
    ));
    assert_eq!(
        f.control
            .settle(
                OperationId::from("correlated"),
                SettlementOutcome::DefinitiveSuccess("duplicate".into()),
            )
            .await,
        SettlementResult::Ignored
    );
}

#[tokio::test]
async fn ordinary_definitive_completion_path_is_unchanged() {
    let mut f = fixture().await;
    let finish = f.fake.hold("ordinary");
    let operation = spawn_submit(
        &f.control,
        tab_operation(&f, "ordinary", &f.tab_a, "ordinary"),
    );
    assert_eq!(next_started(&mut f.started).await, "ordinary");
    finish
        .send(ExecutorOutcome::DefinitiveSuccess("done".into()))
        .unwrap();
    let completion = operation.await.unwrap().unwrap();
    assert_eq!(completion.certainty, CompletionCertainty::Definitive);
    assert_eq!(completion.disposition, CompletionDisposition::Success);
    assert_eq!(
        f.control
            .settlement_state(OperationId::from("ordinary"))
            .await,
        None
    );
    assert_eq!(
        f.control
            .group(f.group.group_id.clone())
            .await
            .unwrap()
            .admission,
        GroupAdmission::Open
    );
}

#[tokio::test]
async fn restart_recovers_only_suspended_hints_with_a_fresh_fence() {
    let (fake, _started) = FakeExecutor::new();
    let owner = Principal::new("owner", 1000);
    let group_id = GroupId::from("recovered");
    let browser = BrowserInstanceId::from("browser-a");
    let tab = TabKey::new(browser.clone(), "7");
    let journal = RecoveryJournal {
        version: 1,
        groups: vec![RecoveryGroupHint {
            group_id: group_id.clone(),
            browser_instance_id: browser,
            principal: owner.clone(),
            members: BTreeSet::from([tab.clone()]),
            membership_revision: 4,
            prior_fence: 8,
            unresolved_mutation: false,
        }],
    };
    let control = ControlPlane::recover("new", fake, QueueLimits::default(), &journal);
    let recovered = control.group(group_id.clone()).await.unwrap();
    assert_eq!(recovered.admission, GroupAdmission::Suspended);
    assert_eq!(recovered.lease.state, LeaseState::Suspended);
    assert_eq!(recovered.lease.fence, 9);
    control.tick(u64::MAX).await;
    let still_suspended = control.group(group_id.clone()).await.unwrap();
    assert_eq!(still_suspended.admission, GroupAdmission::Suspended);
    assert_eq!(still_suspended.lease.state, LeaseState::Suspended);
    let result = control
        .submit(SubmitOperation {
            operation_id: None,
            canonical_fingerprint: "read".into(),
            upstream: UpstreamCorrelation {
                ingress: "test".into(),
                request_id: None,
            },
            client_id: ClientId::from("client"),
            principal: owner,
            group_id: Some(group_id),
            lease: Some(recovered.lease.proof()),
            scope: OperationScope::Tab(tab),
            class: OperationClass::ReadOnly,
            payload: "must not run".into(),
            now_ms: 0,
        })
        .await;
    assert_eq!(
        result,
        Err(AdmissionError::Group(GroupError::AdmissionClosed))
    );
}

#[tokio::test]
async fn idle_lease_ticks_do_not_flood_the_bounded_event_ring() {
    let (fake, _started) = FakeExecutor::new();
    let control = ControlPlane::start("generation", fake, QueueLimits::default());
    let before = control.events.snapshot();
    control.tick(1).await;
    control.tick(2).await;
    let after = control.events.snapshot();
    assert_eq!(after.events, before.events);
    assert_eq!(after.dropped_count, before.dropped_count);
}

#[tokio::test]
async fn browser_loss_fences_group_and_invalidates_old_tab_ownership() {
    let (fake, mut started) = FakeExecutor::new();
    let control = ControlPlane::start("generation", fake, QueueLimits::default());
    let owner = Principal::new("owner", 1000);
    let browser = BrowserInstanceId::from("browser-before");
    let group_id = GroupId::from("browser-loss-group");
    let tab = TabKey::new(browser.clone(), "101");
    let created = control
        .create_group(group_id.clone(), browser.clone(), owner.clone(), 1)
        .await;
    control
        .add_member(group_id.clone(), owner.clone(), tab.clone())
        .await
        .unwrap();

    assert_eq!(control.browser_lost(browser).await, vec![group_id.clone()]);
    let lost = control.group(group_id.clone()).await.unwrap();
    assert_eq!(lost.admission, GroupAdmission::RecoveryRequired);
    assert_eq!(lost.lease.state, LeaseState::Suspended);
    assert_eq!(lost.lease.fence, created.lease.fence + 1);
    assert!(lost.members.is_empty());

    let rejected = control
        .submit(SubmitOperation {
            operation_id: Some(OperationId::from("old-browser-operation")),
            canonical_fingerprint: "old-browser-operation".into(),
            upstream: UpstreamCorrelation {
                ingress: "test".into(),
                request_id: None,
            },
            client_id: ClientId::from("client"),
            principal: owner,
            group_id: Some(group_id),
            lease: Some(created.lease.proof()),
            scope: OperationScope::Tab(tab),
            class: OperationClass::Mutation,
            payload: "must not dispatch".into(),
            now_ms: 2,
        })
        .await;
    assert!(matches!(rejected, Err(AdmissionError::Group(_))));
    assert!(
        started.try_recv().is_err(),
        "old-browser mutation dispatched"
    );
}

#[tokio::test]
async fn restart_preserves_unresolved_mutation_as_suspended_recovery_only() {
    let (fake, _started) = FakeExecutor::new();
    let group_id = GroupId::from("unresolved");
    let browser = BrowserInstanceId::from("surviving-browser");
    let owner = Principal::new("owner", 1000);
    let journal = RecoveryJournal {
        version: 1,
        groups: vec![RecoveryGroupHint {
            group_id: group_id.clone(),
            browser_instance_id: browser,
            principal: owner.clone(),
            members: BTreeSet::new(),
            membership_revision: 2,
            prior_fence: 4,
            unresolved_mutation: true,
        }],
    };
    let control = ControlPlane::recover("new", fake, QueueLimits::default(), &journal);
    let recovered = control.group(group_id.clone()).await.unwrap();
    assert_eq!(recovered.admission, GroupAdmission::RecoveryRequired);
    assert_eq!(recovered.lease.state, LeaseState::Suspended);
    assert_eq!(recovered.lease.fence, 5);
    assert_eq!(
        control
            .force_handoff(group_id, owner, Principal::new("target", 1000), 2, 0,)
            .await,
        Err(GroupError::SettlementRequired)
    );
}

#[tokio::test]
async fn persistent_restart_has_fresh_fence_no_authority_and_no_operation_replay() {
    let path = recovery_test_path("restart");
    let (first_executor, _first_started) = FakeExecutor::new();
    let first = ControlPlane::recover_persistent(
        "first",
        first_executor,
        QueueLimits::default(),
        path.clone(),
    );
    let owner = Principal::new("owner", 1000);
    let browser = BrowserInstanceId::from("browser-a");
    let group_id = GroupId::from("group-a");
    let created = first
        .create_group(group_id.clone(), browser.clone(), owner.clone(), 1)
        .await;
    let tab = TabKey::new(browser.clone(), "tab-a");
    let active = first
        .add_member(group_id.clone(), owner.clone(), tab.clone())
        .await
        .unwrap();
    first.flush_persistence();

    let (second_executor, mut second_started) = FakeExecutor::new();
    let second = ControlPlane::recover_persistent(
        "second",
        second_executor,
        QueueLimits::default(),
        path.clone(),
    );
    let recovered = second.group(group_id.clone()).await.unwrap();
    assert_eq!(recovered.admission, GroupAdmission::Suspended);
    assert_eq!(recovered.lease.state, LeaseState::Suspended);
    assert_eq!(recovered.lease.fence, active.lease.fence + 1);
    assert_ne!(recovered.lease.lease_id, created.lease.lease_id);
    assert!(
        second_started.try_recv().is_err(),
        "operation replayed on restart"
    );
    assert_eq!(
        second
            .renew(recovered.lease.proof(), owner.clone(), 2)
            .await,
        Err(GroupError::AdmissionClosed)
    );
    assert_eq!(
        second
            .resume_recovered(
                group_id.clone(),
                BrowserInstanceId::from("wrong-browser"),
                owner.clone(),
                BTreeSet::from([tab.clone()]),
                1,
                2,
            )
            .await,
        Err(GroupError::RecoveryIdentityMismatch)
    );
    let resumed = second
        .resume_recovered(group_id, browser, owner, BTreeSet::from([tab]), 1, 2)
        .await
        .unwrap();
    assert_eq!(resumed.admission, GroupAdmission::Open);
    second.flush_persistence();
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn in_flight_mutation_restarts_as_recovery_required() {
    let path = recovery_test_path("unresolved");
    let (executor, mut started) = FakeExecutor::new();
    let finish = executor.hold("mutation");
    let first =
        ControlPlane::recover_persistent("first", executor, QueueLimits::default(), path.clone());
    let owner = Principal::new("owner", 1000);
    let browser = BrowserInstanceId::from("browser-a");
    let group_id = GroupId::from("group-a");
    first
        .create_group(group_id.clone(), browser.clone(), owner.clone(), 0)
        .await;
    let tab = TabKey::new(browser, "tab-a");
    let group = first
        .add_member(group_id.clone(), owner.clone(), tab.clone())
        .await
        .unwrap();
    let operation = spawn_submit(
        &first,
        SubmitOperation {
            operation_id: Some(OperationId::from("mutation")),
            canonical_fingerprint: "mutation".into(),
            upstream: UpstreamCorrelation {
                ingress: "test".into(),
                request_id: None,
            },
            client_id: ClientId::from("client"),
            principal: owner,
            group_id: Some(group_id.clone()),
            lease: Some(group.lease.proof()),
            scope: OperationScope::Tab(tab),
            class: OperationClass::Mutation,
            payload: "mutation".into(),
            now_ms: 0,
        },
    );
    assert_eq!(next_started(&mut started).await, "mutation");
    first.flush_persistence();

    let (restart_executor, mut restart_started) = FakeExecutor::new();
    let restart = ControlPlane::recover_persistent(
        "restart",
        restart_executor,
        QueueLimits::default(),
        path.clone(),
    );
    let recovered = restart.group(group_id).await.unwrap();
    assert_eq!(recovered.admission, GroupAdmission::RecoveryRequired);
    assert!(
        restart_started.try_recv().is_err(),
        "mutation replayed on restart"
    );

    finish
        .send(ExecutorOutcome::DefinitiveSuccess("done".into()))
        .unwrap();
    operation.await.unwrap().unwrap();
    first.flush_persistence();
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn persistence_failures_emit_recovery_events_and_keep_empty_authority() {
    let path = recovery_test_path("failure");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"malformed").unwrap();
    let (executor, _started) = FakeExecutor::new();
    let control =
        ControlPlane::recover_persistent("restart", executor, QueueLimits::default(), path.clone());
    assert!(control.group(GroupId::from("missing")).await.is_err());
    assert!(control.events.snapshot().events.iter().any(|event| {
        matches!(
            &event.kind,
            sky_cua_platform::model::BrowserControlEventKind::Recovery { state }
                if state == "recovery_journal_malformed"
        )
    }));
    control.flush_persistence();
    assert!(!path.exists());
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn released_last_group_removes_persistent_journal() {
    let path = recovery_test_path("cleanup");
    let (executor, _started) = FakeExecutor::new();
    let control =
        ControlPlane::recover_persistent("first", executor, QueueLimits::default(), path.clone());
    let owner = Principal::new("owner", 1000);
    let group = control
        .create_group(
            GroupId::from("group-a"),
            BrowserInstanceId::from("browser-a"),
            owner.clone(),
            0,
        )
        .await;
    control.flush_persistence();
    assert!(path.exists());
    control.end_group(group.group_id, owner).await.unwrap();
    control.flush_persistence();
    assert!(!path.exists());
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

#[tokio::test]
async fn queue_limits_apply_per_tab_and_per_client() {
    let (fake, mut started) = FakeExecutor::new();
    let finish_barrier = fake.hold("barrier");
    let control = ControlPlane::start(
        "limited",
        fake,
        QueueLimits {
            submit_ingress: 128,
            per_client: 1,
            per_tab: 1,
            per_bridge_dispatch: 1,
            recent_operations: 8,
        },
    );
    let owner = Principal::new("limited-owner", 1000);
    let browser = BrowserInstanceId::from("limited-browser");
    let group = control
        .create_group(
            GroupId::from("limited-group"),
            browser.clone(),
            owner.clone(),
            0,
        )
        .await;
    let tab = TabKey::new(browser, "1");
    let group = control
        .add_member(group.group_id.clone(), owner.clone(), tab.clone())
        .await
        .unwrap();
    let barrier = spawn_submit(
        &control,
        global_operation(
            "barrier",
            OperationScope::BridgeGlobal(tab.browser_instance_id.clone()),
            "barrier",
        ),
    );
    assert_eq!(next_started(&mut started).await, "barrier");
    let base = SubmitOperation {
        operation_id: Some(OperationId::from("limit-a")),
        canonical_fingerprint: "limit-a".into(),
        upstream: UpstreamCorrelation {
            ingress: "test".into(),
            request_id: None,
        },
        client_id: ClientId::from("one-client"),
        principal: owner,
        group_id: Some(group.group_id.clone()),
        lease: Some(group.lease.proof()),
        scope: OperationScope::Tab(tab),
        class: OperationClass::ReadOnly,
        payload: "limit-a".into(),
        now_ms: 0,
    };
    let first = spawn_submit(&control, base.clone());
    tokio::task::yield_now().await;
    let mut second = base;
    second.operation_id = Some(OperationId::from("limit-b"));
    second.canonical_fingerprint = "limit-b".into();
    second.payload = "limit-b".into();
    assert_eq!(
        control.submit(second).await,
        Err(AdmissionError::Backpressure)
    );
    assert!(control.events.snapshot().events.iter().any(|event| {
        matches!(
            &event.kind,
            sky_cua_platform::model::BrowserControlEventKind::Lifecycle { state }
                if state == "admission_failed:backpressure"
        )
    }));
    finish_barrier
        .send(ExecutorOutcome::DefinitiveSuccess("barrier".into()))
        .unwrap();
    barrier.await.unwrap().unwrap();
    first.await.unwrap().unwrap();
}
