//! Timing lanes for the self-healing control-plane settlement pipeline.
//!
//! All constants are integer milliseconds with derived [`Duration`] values so
//! compile-time ordering assertions use plain integers:
//!
//! ```text
//! Transport:   frame write (2s) < heartbeat (3s) < liveness (7s)
//! Settlement:  retry (1s) < Original -> Unknown (15s) < hard evict (25s)
//! Unknown:     post-delivery grace (4s) < hard cap (10s, from conversion)
//! ```

use std::time::Duration;

pub(super) const CONTROL_PLANE_LIVENESS_DEADLINE_MS: u64 = 7_000;
pub(super) const CONTROL_PLANE_FRAME_WRITE_DEADLINE_MS: u64 = 2_000;
pub(super) const SETTLEMENT_ACK_RETRY_INTERVAL_MS: u64 = 1_000;
pub(super) const SETTLEMENT_ORIGINAL_TO_UNKNOWN_MS: u64 = 15_000;
pub(super) const SETTLEMENT_ENQUEUE_HARD_EVICT_MS: u64 = 25_000;
pub(super) const SETTLEMENT_UNKNOWN_MAX_POST_DELIVERY_GRACE_MS: u64 = 4_000;
pub(super) const SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP_MS: u64 = 10_000;

pub(super) const CONTROL_PLANE_LIVENESS_DEADLINE: Duration =
    Duration::from_millis(CONTROL_PLANE_LIVENESS_DEADLINE_MS);
pub(super) const CONTROL_PLANE_FRAME_WRITE_DEADLINE: Duration =
    Duration::from_millis(CONTROL_PLANE_FRAME_WRITE_DEADLINE_MS);
pub(super) const SETTLEMENT_ACK_RETRY_INTERVAL: Duration =
    Duration::from_millis(SETTLEMENT_ACK_RETRY_INTERVAL_MS);
pub(super) const SETTLEMENT_ORIGINAL_TO_UNKNOWN: Duration =
    Duration::from_millis(SETTLEMENT_ORIGINAL_TO_UNKNOWN_MS);
pub(super) const SETTLEMENT_ENQUEUE_HARD_EVICT: Duration =
    Duration::from_millis(SETTLEMENT_ENQUEUE_HARD_EVICT_MS);
pub(super) const SETTLEMENT_UNKNOWN_MAX_POST_DELIVERY_GRACE: Duration =
    Duration::from_millis(SETTLEMENT_UNKNOWN_MAX_POST_DELIVERY_GRACE_MS);
pub(super) const SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP: Duration =
    Duration::from_millis(SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP_MS);

const _: () = {
    assert!(CONTROL_PLANE_LIVENESS_DEADLINE_MS > SETTLEMENT_ACK_RETRY_INTERVAL_MS * 5);
    assert!(SETTLEMENT_ORIGINAL_TO_UNKNOWN_MS > CONTROL_PLANE_LIVENESS_DEADLINE_MS * 2);
    assert!(
        SETTLEMENT_UNKNOWN_PREEXISTING_HARD_CAP_MS > SETTLEMENT_UNKNOWN_MAX_POST_DELIVERY_GRACE_MS
    );
    assert!(CONTROL_PLANE_FRAME_WRITE_DEADLINE_MS < SETTLEMENT_ORIGINAL_TO_UNKNOWN_MS);
    assert!(SETTLEMENT_ORIGINAL_TO_UNKNOWN_MS <= SETTLEMENT_ENQUEUE_HARD_EVICT_MS);
};
