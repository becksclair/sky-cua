//! Request-scoped memo for read-only discovery that one request repeats.
//!
//! A desktop observe resolves its target, captures it and correlates
//! accessibility through separate backend calls, and each call used to
//! rediscover the same window list. The service wraps one request in
//! [`scope`]; backends store discovery results with [`put`] and read them back
//! with [`get`]. Outside a scope both are no-ops, so values can never outlive
//! the request that produced them and unscoped callers always see fresh state.

use std::any::Any;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;

#[derive(Default)]
struct RequestMemo {
    slots: Mutex<HashMap<&'static str, Box<dyn Any + Send>>>,
}

tokio::task_local! {
    static MEMO: RequestMemo;
}

/// Run `future` with a fresh request memo. The memo is visible only to code
/// polled within this task; work moved to another task does not see it.
pub async fn scope<F: Future>(future: F) -> F::Output {
    MEMO.scope(RequestMemo::default(), future).await
}

/// Whether the current task runs inside a request scope.
#[must_use]
pub fn is_active() -> bool {
    MEMO.try_with(|_| ()).is_ok()
}

/// Clone the value stored under `key`, if this request stored one of type `T`.
#[must_use]
pub fn get<T: Clone + 'static>(key: &'static str) -> Option<T> {
    MEMO.try_with(|memo| {
        lock(memo)
            .get(key)
            .and_then(|value| value.downcast_ref::<T>())
            .cloned()
    })
    .ok()
    .flatten()
}

/// Store `value` under `key` for the rest of the request; no-op outside a scope.
pub fn put<T: Send + 'static>(key: &'static str, value: T) {
    let _ = MEMO.try_with(|memo| {
        lock(memo).insert(key, Box::new(value));
    });
}

fn lock(
    memo: &RequestMemo,
) -> std::sync::MutexGuard<'_, HashMap<&'static str, Box<dyn Any + Send>>> {
    memo.slots
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn values_live_only_inside_their_scope() {
        put("k", 1_u32);
        assert_eq!(get::<u32>("k"), None, "no scope: put is a no-op");
        assert!(!is_active());

        let inside = scope(async {
            assert!(is_active());
            assert_eq!(get::<u32>("k"), None);
            put("k", 7_u32);
            assert_eq!(get::<String>("k"), None, "type mismatch reads as absent");
            get::<u32>("k")
        })
        .await;
        assert_eq!(inside, Some(7));

        let next = scope(async { get::<u32>("k") }).await;
        assert_eq!(next, None, "a new request starts empty");
    }
}
