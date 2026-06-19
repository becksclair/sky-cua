//! Phone-use service module root.
//!
//! Mirrors `src/browser.rs`: this file only declares the phone submodules and
//! re-exports the public surface the daemon dispatches through. The Phase 1
//! spine wires a [`PhoneManager`] that owns session state, a per-session
//! capability-profile cache, and a [`command::CommandRunner`] trait object that
//! later ADB/companion/scrcpy lanes implement. Every backend submodule
//! (`adb`, `device`, `snapshot`, `mapping`, `cursor`, `companion`, `scrcpy`) is
//! a deterministic stub today; routing and contract tests run against it.

mod adb;
mod command;
mod companion;
mod cursor;
mod device;
mod manager;
mod mapping;
mod scrcpy;
mod snapshot;

#[cfg(test)]
mod tests;

pub(crate) use manager::{PhoneManager, ScrcpyAdoptionCandidate};
pub(crate) use scrcpy::host_scrcpy_default_max_size;
