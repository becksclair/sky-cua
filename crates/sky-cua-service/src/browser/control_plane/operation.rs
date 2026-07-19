use std::{fmt, future::Future, pin::Pin};

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub(crate) struct $name(pub(crate) String);

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(BrowserInstanceId);
string_id!(ClientId);
string_id!(GroupId);
string_id!(OperationId);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TabKey {
    pub(crate) browser_instance_id: BrowserInstanceId,
    pub(crate) tab_id: String,
}

impl TabKey {
    pub(crate) fn new(
        browser_instance_id: impl Into<BrowserInstanceId>,
        tab_id: impl Into<String>,
    ) -> Self {
        Self {
            browser_instance_id: browser_instance_id.into(),
            tab_id: tab_id.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Principal {
    pub(crate) id: String,
    pub(crate) uid: u32,
}

impl Principal {
    pub(crate) fn new(id: impl Into<String>, uid: u32) -> Self {
        Self { id: id.into(), uid }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UpstreamCorrelation {
    pub(crate) ingress: String,
    pub(crate) request_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OperationClass {
    ReadOnly,
    AbsoluteSet,
    Mutation,
    BrowserGlobal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OperationScope {
    Tab(TabKey),
    BridgeGlobal(BrowserInstanceId),
    DaemonGlobal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationIdentity {
    pub(crate) operation_id: OperationId,
    pub(crate) daemon_generation: String,
    pub(crate) canonical_fingerprint: String,
    pub(crate) upstream: UpstreamCorrelation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DispatchOperation {
    pub(crate) identity: OperationIdentity,
    pub(crate) client_id: ClientId,
    pub(crate) principal: Principal,
    pub(crate) group_id: Option<GroupId>,
    pub(crate) scope: OperationScope,
    pub(crate) class: OperationClass,
    pub(crate) payload: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExecutorOutcome {
    DefinitiveSuccess(String),
    DefinitiveFailure(String),
    Ambiguous(String),
}

pub(crate) const SETTLEMENT_DEADLINE_MS: u64 = 30_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SettlementOutcome {
    DefinitiveSuccess(String),
    ProvenPreDispatchFailure(String),
    Error(String),
    TargetLost(TabKey),
    BrowserLost(BrowserInstanceId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SettlementState {
    Pending { deadline_ms: u64 },
    Unknown { deadline_ms: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SettlementResult {
    Settled(Completion),
    RemainsAmbiguous,
    Ignored,
}

pub(crate) trait Executor: Send + Sync + 'static {
    fn execute(
        &self,
        operation: DispatchOperation,
    ) -> Pin<Box<dyn Future<Output = ExecutorOutcome> + Send + 'static>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CompletionCertainty {
    PreDispatchRejected,
    Definitive,
    Ambiguous,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CompletionDisposition {
    Success,
    Failure,
    CancelledBeforeDispatch,
    WaiterDetached,
    TargetLost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Completion {
    pub(crate) operation_id: OperationId,
    pub(crate) certainty: CompletionCertainty,
    pub(crate) disposition: CompletionDisposition,
    pub(crate) detail: String,
}

impl Completion {
    pub(crate) fn cancelled(operation_id: OperationId) -> Self {
        Self {
            operation_id,
            certainty: CompletionCertainty::PreDispatchRejected,
            disposition: CompletionDisposition::CancelledBeforeDispatch,
            detail: "cancelled before dispatch".to_owned(),
        }
    }

    pub(crate) fn detached(operation_id: OperationId) -> Self {
        Self {
            operation_id,
            certainty: CompletionCertainty::Ambiguous,
            disposition: CompletionDisposition::WaiterDetached,
            detail: "waiter detached after dispatch; shared execution continues".to_owned(),
        }
    }

    pub(crate) fn from_executor(operation_id: OperationId, outcome: ExecutorOutcome) -> Self {
        match outcome {
            ExecutorOutcome::DefinitiveSuccess(detail) => Self {
                operation_id,
                certainty: CompletionCertainty::Definitive,
                disposition: CompletionDisposition::Success,
                detail,
            },
            ExecutorOutcome::DefinitiveFailure(detail) => Self {
                operation_id,
                certainty: CompletionCertainty::Definitive,
                disposition: CompletionDisposition::Failure,
                detail,
            },
            ExecutorOutcome::Ambiguous(detail) => Self {
                operation_id,
                certainty: CompletionCertainty::Ambiguous,
                disposition: CompletionDisposition::Failure,
                detail,
            },
        }
    }

    pub(crate) fn settlement_success(operation_id: OperationId, detail: String) -> Self {
        Self {
            operation_id,
            certainty: CompletionCertainty::Definitive,
            disposition: CompletionDisposition::Success,
            detail,
        }
    }

    pub(crate) fn settlement_pre_dispatch_failure(
        operation_id: OperationId,
        detail: String,
    ) -> Self {
        Self {
            operation_id,
            certainty: CompletionCertainty::PreDispatchRejected,
            disposition: CompletionDisposition::Failure,
            detail,
        }
    }

    pub(crate) fn target_lost(operation_id: OperationId, detail: String) -> Self {
        Self {
            operation_id,
            certainty: CompletionCertainty::Definitive,
            disposition: CompletionDisposition::TargetLost,
            detail,
        }
    }
}
