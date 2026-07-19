use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use sky_cua_platform::model::{
    BrowserControlEvent, BrowserControlEventKind, BrowserControlEventWindow, BrowserTabKey,
};

pub(super) const EVENT_RING_CAPACITY: usize = 256;
pub(super) const GROUP_RESULT_LIMIT: usize = 64;
pub(super) const GROUP_MEMBER_LIMIT: usize = 32;
pub(super) const RECENT_OPERATION_LIMIT: usize = 64;

#[derive(Clone, Debug, Default)]
pub(super) struct EventContext {
    pub(super) principal_id: Option<String>,
    pub(super) group_id: Option<String>,
    pub(super) tab_key: Option<BrowserTabKey>,
    pub(super) operation_id: Option<String>,
}

#[derive(Clone)]
pub(super) struct EventRecorder {
    generation: Arc<str>,
    inner: Arc<Mutex<EventRing>>,
}

struct EventRing {
    capacity: usize,
    next_sequence: u64,
    dropped_count: u64,
    events: VecDeque<BrowserControlEvent>,
}

impl EventRecorder {
    pub(super) fn new(generation: impl Into<Arc<str>>) -> Self {
        Self::with_capacity(generation, EVENT_RING_CAPACITY)
    }

    fn with_capacity(generation: impl Into<Arc<str>>, capacity: usize) -> Self {
        Self {
            generation: generation.into(),
            inner: Arc::new(Mutex::new(EventRing {
                capacity,
                next_sequence: 1,
                dropped_count: 0,
                events: VecDeque::with_capacity(capacity),
            })),
        }
    }

    pub(super) fn record(&self, kind: BrowserControlEventKind, context: EventContext) {
        let mut ring = self.inner.lock().expect("browser event ring poisoned");
        let sequence = ring.next_sequence;
        ring.next_sequence = ring.next_sequence.saturating_add(1);
        if ring.events.len() == ring.capacity {
            ring.events.pop_front();
            ring.dropped_count = ring.dropped_count.saturating_add(1);
        }
        if ring.capacity != 0 {
            ring.events.push_back(BrowserControlEvent {
                event_sequence: sequence,
                daemon_generation: self.generation.to_string(),
                timestamp_ms: unix_epoch_ms(),
                principal_id: context.principal_id,
                group_id: context.group_id,
                tab_key: context.tab_key,
                operation_id: context.operation_id,
                kind,
            });
        }
    }

    pub(super) fn snapshot(&self) -> BrowserControlEventWindow {
        let ring = self.inner.lock().expect("browser event ring poisoned");
        BrowserControlEventWindow {
            oldest_sequence: ring.events.front().map(|event| event.event_sequence),
            newest_sequence: ring.events.back().map(|event| event.event_sequence),
            dropped_count: ring.dropped_count,
            events: ring.events.iter().cloned().collect(),
        }
    }
}

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::model::BrowserControlEventKind;

    use super::*;

    #[test]
    fn event_sequences_are_monotonic_and_overflow_is_explicit() {
        let events = EventRecorder::with_capacity("generation", 3);
        for depth in 0..5 {
            events.record(
                BrowserControlEventKind::QueueState { depth },
                EventContext::default(),
            );
        }

        let snapshot = events.snapshot();
        assert_eq!(snapshot.oldest_sequence, Some(3));
        assert_eq!(snapshot.newest_sequence, Some(5));
        assert_eq!(snapshot.dropped_count, 2);
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(|event| event.event_sequence)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
        assert!(
            snapshot
                .events
                .iter()
                .all(|event| event.daemon_generation == "generation")
        );
    }
}
