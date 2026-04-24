use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct SessionStore {
    inner: Arc<RwLock<SessionState>>,
}

#[derive(Debug)]
struct SessionState {
    last_activity: Instant,
    request_count: u64,
}

impl SessionStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(SessionState {
                last_activity: Instant::now(),
                request_count: 0,
            })),
        }
    }

    pub async fn touch(&self) {
        let mut state = self.inner.write().await;
        state.last_activity = Instant::now();
        state.request_count += 1;
    }

    pub async fn idle_for(&self) -> Duration {
        let state = self.inner.read().await;
        state.last_activity.elapsed()
    }
}
