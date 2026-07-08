//! Browser-session activity tracking for the daemon idle gate.
//!
//! The daemon's 5-minute idle exit kills the heartbeat keepalive
//! (`browser/keepalive.rs`), and ~30s later the extension's driver-liveness
//! check detaches `chrome.debugger` from every tab. Agents routinely think for
//! longer than the idle timeout between browser actions, so every resumed
//! input operation landed on a freshly detached session (live incident,
//! 2026-07-08). The idle gate therefore must not fire while a browser session
//! is plausibly still in use: any browser bridge request marks activity here,
//! and the daemon stays alive for [`BROWSER_SESSION_LINGER`] past the last one.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long past the last browser bridge request the daemon keeps itself (and
/// with it the heartbeat keepalive, and so every tab's debugger attachment)
/// alive. Sized for agent think-time between actions; after it elapses the
/// normal idle exit applies and the extension detaches ~30s later.
const BROWSER_SESSION_LINGER: Duration = Duration::from_secs(30 * 60);

/// Milliseconds since [`anchor`] of the most recent browser bridge request;
/// 0 means no browser request was ever handled by this daemon.
static LAST_BRIDGE_ACTIVITY_MS: AtomicU64 = AtomicU64::new(0);

fn anchor() -> Instant {
    static ANCHOR: OnceLock<Instant> = OnceLock::new();
    *ANCHOR.get_or_init(Instant::now)
}

/// Record that a browser bridge request was handled just now.
pub(crate) fn mark_bridge_activity() {
    let elapsed_ms = u64::try_from(anchor().elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .max(1);
    LAST_BRIDGE_ACTIVITY_MS.store(elapsed_ms, Ordering::Relaxed);
}

/// Whether a browser session was active recently enough that the daemon must
/// not idle-exit (which would kill the keepalive and detach every tab).
pub(crate) fn browser_session_lingering() -> bool {
    let last_ms = LAST_BRIDGE_ACTIVITY_MS.load(Ordering::Relaxed);
    if last_ms == 0 {
        return false;
    }
    let now_ms = u64::try_from(anchor().elapsed().as_millis()).unwrap_or(u64::MAX);
    lingering(now_ms.saturating_sub(last_ms))
}

fn lingering(since_last_activity_ms: u64) -> bool {
    since_last_activity_ms < u64::try_from(BROWSER_SESSION_LINGER.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_active_daemon_does_not_linger() {
        // Uses the real statics: nothing has marked activity in this test
        // binary before this assertion runs only if the atomic is untouched,
        // so assert through the pure helper instead.
        assert!(lingering(0));
        assert!(lingering(BROWSER_SESSION_LINGER.as_millis() as u64 - 1));
        assert!(!lingering(BROWSER_SESSION_LINGER.as_millis() as u64));
        assert!(!lingering(u64::MAX));
    }

    #[test]
    fn marked_activity_lingers() {
        mark_bridge_activity();
        assert!(browser_session_lingering());
    }
}
