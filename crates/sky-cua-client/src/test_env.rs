//! Process-wide environment lock for tests.
//!
//! Every test thread in this binary shares one process environment, so every
//! test that reads or mutates it must hold the same lock. Separate per-module
//! locks let a test in one module overwrite a variable another module's test is
//! relying on.

use std::sync::{Mutex, MutexGuard, PoisonError};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serialize a test that depends on process environment variables. A panic in
/// another test must not cascade, so a poisoned lock is recovered.
pub(crate) fn env_lock() -> MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}
