use std::sync::Arc;

use tokio::sync::mpsc;

use super::operation;
use super::{
    control::{Command, QueueLimits, SubmitCommand},
    group::GroupRegistry,
    introspection::EventRecorder,
    operation::Executor,
    persistence::JournalWriter,
};

mod state;

pub(super) struct ActorConfig {
    pub(super) generation: String,
    pub(super) limits: QueueLimits,
    pub(super) groups: GroupRegistry,
    pub(super) events: EventRecorder,
    pub(super) persistence: Option<JournalWriter>,
}

pub(super) fn spawn_actor(
    receiver: mpsc::UnboundedReceiver<Command>,
    submit_receiver: mpsc::Receiver<SubmitCommand>,
    sender: mpsc::UnboundedSender<Command>,
    executor: Arc<dyn Executor>,
    config: ActorConfig,
) {
    tokio::spawn(state::run_actor(
        receiver,
        submit_receiver,
        sender,
        executor,
        config.generation,
        config.limits,
        config.groups,
        config.events,
        config.persistence,
    ));
}
