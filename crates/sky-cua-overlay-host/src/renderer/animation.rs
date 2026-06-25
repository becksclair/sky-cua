//! Animation clock abstraction for future effect phases.
//!
//! Phase 3 only renders the static cursor, so this module is intentionally a
//! thin boundary. Future phases (glow, wave, gesture trails) will use the clock
//! to drive uniform time values without coupling to `std::time` directly.

/// A source of monotonic millisecond time for the renderer.
pub trait AnimationClock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// Default clock backed by `std::time::SystemTime`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl AnimationClock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{AnimationClock, SystemClock};

    #[test]
    fn system_clock_returns_positive_ms() {
        let now = SystemClock.now_ms();
        assert!(now > 0);
    }
}
