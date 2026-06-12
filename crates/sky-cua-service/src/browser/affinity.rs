//! Daemon-global tab-to-socket affinity.
//!
//! Bridge tab ids are plain per-browser integers, so the same id can name
//! unrelated tabs on two connected bridges (e.g. Chrome and Brave). Running a
//! tab-bound operation against the wrong bridge is at best a fast failure and
//! at worst a hijack: a recovery `claimUserTab` on a colliding id would adopt
//! and drive an unrelated tab. Every code path that learns which socket owns
//! a tab records it here, and bound-tab operations route to that socket only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex as StdMutex};

static TAB_SOCKET_AFFINITY: LazyLock<StdMutex<HashMap<String, PathBuf>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

/// Record that `tab_id` lives on `socket`. Overwrites a previous owner: tab
/// ids are stable for the life of a browser tab, so a new owner means the
/// old socket (browser instance) is gone or the old entry was wrong.
pub(super) fn record_tab_socket(tab_id: &str, socket: &Path) {
    if tab_id.is_empty() {
        return;
    }
    affinity().insert(tab_id.to_string(), socket.to_path_buf());
}

/// Drop the recorded owner for `tab_id`, e.g. when two sockets report the
/// same id in one listing and the mapping is genuinely ambiguous.
pub(super) fn forget_tab_socket(tab_id: &str) {
    affinity().remove(tab_id);
}

/// Drop the recorded owner for `tab_id` only when `socket` is that owner.
/// A `No tab with id` answer from any other socket says nothing about the
/// recorded owner — which may be transiently missing from the candidate set —
/// and must not erase a still-valid mapping.
pub(super) fn forget_tab_socket_if_owner(tab_id: &str, socket: &Path) {
    let mut map = affinity();
    if map.get(tab_id).is_some_and(|owner| owner == socket) {
        map.remove(tab_id);
    }
}

/// Reconcile the map against one socket's authoritative tab listing: drop
/// entries that name `socket` as owner but whose tab no longer appears in
/// its listing. This is the only prune that covers the common case of a tab
/// closed while its browser stays running — the socket still exists (so the
/// lookup-time prune never fires) and a closed tab is never looked up again
/// (so the owner's not-found never fires either).
pub(super) fn retain_socket_tabs(socket: &Path, live_tab_ids: &std::collections::HashSet<&str>) {
    affinity().retain(|tab_id, owner| owner != socket || live_tab_ids.contains(tab_id.as_str()));
}

/// The socket that owns `tab_id`, restricted to the current candidate set.
/// Entries whose socket file no longer exists are pruned (the owning browser
/// is gone, and a lingering mapping would block rediscovery); entries outside
/// `candidates` are ignored but kept, since socket-directory scans can be
/// transiently incomplete.
pub(super) fn tab_socket_affinity(tab_id: &str, candidates: &[PathBuf]) -> Option<PathBuf> {
    let mut map = affinity();
    let socket = map.get(tab_id)?.clone();
    if !socket.exists() {
        map.remove(tab_id);
        return None;
    }
    candidates.contains(&socket).then_some(socket)
}

fn affinity() -> std::sync::MutexGuard<'static, HashMap<String, PathBuf>> {
    TAB_SOCKET_AFFINITY
        .lock()
        .expect("tab socket affinity mutex poisoned")
}

#[cfg(test)]
pub(super) fn reset_tab_socket_affinity_for_tests() {
    affinity().clear();
}
