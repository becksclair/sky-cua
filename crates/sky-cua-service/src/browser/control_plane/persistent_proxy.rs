use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::net::UnixStream;

use super::{
    BridgeActor, BridgeActorError, BridgeActorRequest, ControlPlane, OperationClass, OperationId,
    SettlementOutcome,
};
use crate::browser::protocol::{read_frame, write_frame};

#[derive(Clone)]
pub(crate) struct ProxyContext {
    actors: HashMap<PathBuf, BridgeActor>,
    operation_id: String,
    operation_class: OperationClass,
    timeout: Duration,
    target_lifetime_key: Option<Value>,
    next_subrequest: Arc<AtomicU64>,
    settlement_parents: Arc<tokio::sync::Mutex<HashMap<OperationId, OperationId>>>,
    tracker: Arc<ChildTracker>,
}

pub(crate) struct ChildTracker {
    parent: OperationId,
    pending_mutations: AtomicUsize,
    ambiguous: AtomicBool,
    parent_detached: AtomicBool,
    late_outcome: tokio::sync::Mutex<Option<SettlementOutcome>>,
    control: ControlPlane,
}

impl ChildTracker {
    pub(crate) fn new(parent: OperationId, control: ControlPlane) -> Arc<Self> {
        Arc::new(Self {
            parent,
            pending_mutations: AtomicUsize::new(0),
            ambiguous: AtomicBool::new(false),
            parent_detached: AtomicBool::new(false),
            late_outcome: tokio::sync::Mutex::new(None),
            control,
        })
    }

    fn begin(&self) {
        self.pending_mutations.fetch_add(1, Ordering::AcqRel);
    }

    async fn finish(&self, result: &Result<Value, BridgeActorError>) {
        match result {
            Err(BridgeActorError::Ambiguous) => {
                self.ambiguous.store(true, Ordering::Release);
            }
            Ok(response) => {
                *self.late_outcome.lock().await = Some(SettlementOutcome::DefinitiveSuccess(
                    response.get("result").unwrap_or(response).to_string(),
                ));
            }
            Err(error) => {
                *self.late_outcome.lock().await = Some(SettlementOutcome::Error(format!(
                    "bridge child error without pre-dispatch proof: {error:?}"
                )));
            }
        }
        let remaining = self.pending_mutations.fetch_sub(1, Ordering::AcqRel) - 1;
        if remaining == 0
            && self.parent_detached.load(Ordering::Acquire)
            && !self.ambiguous.load(Ordering::Acquire)
            && let Some(outcome) = self.late_outcome.lock().await.take()
        {
            let _ = self.control.settle(self.parent.clone(), outcome).await;
        }
    }

    pub(crate) async fn detach_requires_settlement(&self) -> bool {
        self.parent_detached.store(true, Ordering::Release);
        let pending = self.pending_mutations.load(Ordering::Acquire);
        if pending == 0
            && !self.ambiguous.load(Ordering::Acquire)
            && let Some(outcome) = self.late_outcome.lock().await.take()
        {
            let _ = self.control.settle(self.parent.clone(), outcome).await;
        }
        pending > 0 || self.ambiguous.load(Ordering::Acquire)
    }
}

impl ProxyContext {
    pub(crate) fn new(
        actors: impl IntoIterator<Item = (PathBuf, BridgeActor)>,
        operation_id: String,
        operation_class: OperationClass,
        timeout: Duration,
        target_lifetime_key: Option<Value>,
        settlement_parents: Arc<tokio::sync::Mutex<HashMap<OperationId, OperationId>>>,
        tracker: Arc<ChildTracker>,
    ) -> Self {
        Self {
            actors: actors.into_iter().collect(),
            operation_id,
            operation_class,
            timeout,
            target_lifetime_key,
            next_subrequest: Arc::new(AtomicU64::new(1)),
            settlement_parents,
            tracker,
        }
    }

    pub(crate) fn tracker(&self) -> Arc<ChildTracker> {
        Arc::clone(&self.tracker)
    }
}

tokio::task_local! {
    static ACTIVE_PROXY: ProxyContext;
}

pub(crate) async fn scope<F: Future>(context: ProxyContext, future: F) -> F::Output {
    ACTIVE_PROXY.scope(context, future).await
}

pub(crate) fn capture() -> Option<ProxyContext> {
    ACTIVE_PROXY.try_with(Clone::clone).ok()
}

pub(crate) async fn scope_captured<F: Future>(
    context: Option<ProxyContext>,
    future: F,
) -> F::Output {
    match context {
        Some(context) => scope(context, future).await,
        None => future.await,
    }
}

pub(crate) async fn connect(socket: &Path) -> Option<Result<UnixStream, std::io::Error>> {
    let actor = match ACTIVE_PROXY.try_with(|context| context.actors.get(socket).cloned()) {
        Err(_) => return None,
        Ok(Some(actor)) => actor,
        Ok(None) => {
            return Some(Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "persistent control dispatch forbids direct native-host connections",
            )));
        }
    };
    let context = ACTIVE_PROXY.with(Clone::clone);
    let (client, server) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(error) => return Some(Err(error)),
    };
    tokio::spawn(run_proxy(server, actor, context));
    Some(Ok(client))
}

async fn run_proxy(mut stream: UnixStream, actor: BridgeActor, context: ProxyContext) {
    while let Ok(Some(frame)) = read_frame(&mut stream).await {
        let Some(method) = frame.get("method").and_then(Value::as_str) else {
            continue;
        };
        let id = frame.get("id").cloned().unwrap_or(Value::Null);
        if matches!(method, "finalizeTabs" | "turnEnded") {
            let _ = write_frame(
                &mut stream,
                &json!({"jsonrpc":"2.0", "id":id, "result":true}),
            )
            .await;
            continue;
        }
        let params = frame.get("params").cloned().unwrap_or_else(|| json!({}));
        let class = match method {
            "getInfo" | "getTabs" | "getUserTabs" => OperationClass::ReadOnly,
            "moveMouse" => OperationClass::AbsoluteSet,
            _ => context.operation_class,
        };
        let tracks_mutation = matches!(
            class,
            OperationClass::Mutation | OperationClass::BrowserGlobal
        );
        if tracks_mutation {
            context.tracker.begin();
        }
        let child_operation_id = format!(
            "{}:bridge-subrequest:{}",
            context.operation_id,
            context.next_subrequest.fetch_add(1, Ordering::Relaxed),
        );
        context.settlement_parents.lock().await.insert(
            OperationId(child_operation_id.clone()),
            OperationId(context.operation_id.clone()),
        );
        let mut request =
            BridgeActorRequest::new(method, params, child_operation_id.clone(), class);
        request.timeout = context.timeout;
        request.target_lifetime_key = context.target_lifetime_key.clone();
        let actor_result = actor.request(request).await;
        if tracks_mutation {
            context.tracker.finish(&actor_result).await;
        }
        let response = match actor_result {
            Ok(mut response) => {
                context
                    .settlement_parents
                    .lock()
                    .await
                    .remove(&OperationId(child_operation_id.clone()));
                if let Some(object) = response.as_object_mut() {
                    object.insert("id".to_owned(), id.clone());
                }
                response
            }
            Err(error) => {
                if matches!(error, super::BridgeActorError::Ambiguous) {
                    // Retain child-to-parent correlation for the host's late
                    // settlement event.
                } else {
                    context
                        .settlement_parents
                        .lock()
                        .await
                        .remove(&OperationId(child_operation_id.clone()));
                }
                json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "error":{"code":-32072, "message":format!("persistent browser bridge failed: {error:?}")},
                })
            }
        };
        if write_frame(&mut stream, &response).await.is_err() {
            break;
        }
    }
}
