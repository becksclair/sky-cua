//! scrcpy process ownership and liveness tracking.
//!
//! A process-ownership model (managed / adopted / external). Only sky-cua
//! *managed* processes are ever stopped; adopted/external windows are mapped but
//! their lifetime stays with whoever launched them. Crash detection downgrades
//! the capability without corrupting the session.

use sky_cua_platform::model::PhoneScrcpyCapabilities;

use super::command::{ScrcpyCodec, scrcpy_window_title};
use super::{crashed_capabilities, scrcpy_version_placeholder};

/// Who owns a scrcpy process backing a session, deciding whether sky-cua may
/// stop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::phone) enum ScrcpyOwnership {
    /// Launched by this service. sky-cua may stop it on cleanup.
    Managed,
    /// Found running with a sky-cua window title and adopted into a session.
    /// sky-cua maps it but does not kill it; the title says we created it in a
    /// previous run, but ownership of the live process is uncertain, so adoption
    /// is conservative and non-destructive.
    Adopted,
    /// A scrcpy window the operator launched independently. Mapped if useful,
    /// never stopped by sky-cua.
    External,
}

impl ScrcpyOwnership {
    /// Whether sky-cua is allowed to terminate a process with this ownership.
    pub(in crate::phone) fn may_stop(self) -> bool {
        matches!(self, ScrcpyOwnership::Managed)
    }
}

/// Runtime liveness of a tracked scrcpy process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::phone) enum ScrcpyLiveness {
    /// Process is believed running and its window mapped.
    Running,
    /// Process exited unexpectedly (crash); capability must downgrade.
    Crashed,
    /// Process was stopped by sky-cua as part of normal cleanup.
    Stopped,
}

/// A tracked scrcpy process: PID, the window title we expect, the codec that
/// actually launched, ownership, and current liveness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) struct ScrcpyProcess {
    /// OS process id, when known. `Adopted`/`External` windows discovered by
    /// title may not expose a PID we own.
    pub(in crate::phone) pid: Option<u32>,
    /// The deterministic window title (`sky-cua-phone-<safe-serial>`).
    pub(in crate::phone) window_title: String,
    /// The serial this process mirrors.
    pub(in crate::phone) serial: String,
    /// The codec the process launched with (after any retry).
    pub(in crate::phone) codec: ScrcpyCodec,
    pub(in crate::phone) ownership: ScrcpyOwnership,
    pub(in crate::phone) liveness: ScrcpyLiveness,
}

impl ScrcpyProcess {
    /// Record a freshly launched, sky-cua-managed process.
    pub(in crate::phone) fn managed(pid: u32, serial: &str, codec: ScrcpyCodec) -> Self {
        Self {
            pid: Some(pid),
            window_title: scrcpy_window_title(serial),
            serial: serial.to_string(),
            codec,
            ownership: ScrcpyOwnership::Managed,
            liveness: ScrcpyLiveness::Running,
        }
    }

    /// Adopt an already-running window that carries a sky-cua title. Ownership is
    /// `Adopted`: it is mapped but never stopped.
    pub(in crate::phone) fn adopted(pid: Option<u32>, window_title: &str, serial: &str) -> Self {
        Self {
            pid,
            window_title: window_title.to_string(),
            serial: serial.to_string(),
            codec: ScrcpyCodec::Default,
            ownership: ScrcpyOwnership::Adopted,
            liveness: ScrcpyLiveness::Running,
        }
    }

    /// Track an operator-launched (external) window. Never stopped by sky-cua.
    #[cfg_attr(not(test), expect(dead_code))]
    pub(in crate::phone) fn external(pid: Option<u32>, window_title: &str, serial: &str) -> Self {
        Self {
            pid,
            window_title: window_title.to_string(),
            serial: serial.to_string(),
            codec: ScrcpyCodec::Default,
            ownership: ScrcpyOwnership::External,
            liveness: ScrcpyLiveness::Running,
        }
    }

    /// Whether sky-cua may stop this specific process right now: only managed,
    /// still-running processes.
    pub(in crate::phone) fn can_be_stopped_by_us(&self) -> bool {
        self.ownership.may_stop() && self.liveness == ScrcpyLiveness::Running
    }

    /// Mark this process crashed. Returns the downgraded capability so the caller
    /// can replace the session's scrcpy capability without losing the session
    /// itself.
    ///
    /// Driven by the manager's scrcpy liveness watchdog
    /// ([`crate::phone::PhoneManager::poll_scrcpy_liveness`]), which polls each
    /// managed mirror's child and calls this when it has exited mid-session.
    pub(in crate::phone) fn mark_crashed(&mut self) -> PhoneScrcpyCapabilities {
        self.liveness = ScrcpyLiveness::Crashed;
        crashed_capabilities(scrcpy_version_placeholder())
    }

    /// Mark this process stopped after sky-cua terminated it. No-op for windows
    /// we are not allowed to stop.
    pub(in crate::phone) fn mark_stopped(&mut self) {
        if self.ownership.may_stop() {
            self.liveness = ScrcpyLiveness::Stopped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::active_capabilities;
    use super::*;

    #[test]
    fn ownership_only_managed_may_stop() {
        assert!(ScrcpyOwnership::Managed.may_stop());
        assert!(!ScrcpyOwnership::Adopted.may_stop());
        assert!(!ScrcpyOwnership::External.may_stop());
    }

    #[test]
    fn managed_process_can_be_stopped_until_crash() {
        let mut proc = ScrcpyProcess::managed(1234, "emulator-5554", ScrcpyCodec::H265);
        assert!(proc.can_be_stopped_by_us());
        assert_eq!(proc.window_title, "sky-cua-phone-emulator-5554");

        let caps = proc.mark_crashed();
        // After a crash it is no longer stoppable and the capability downgrades.
        assert!(!proc.can_be_stopped_by_us());
        assert_eq!(proc.liveness, ScrcpyLiveness::Crashed);
        assert!(caps.installed);
        assert!(!caps.active);
        assert!(!caps.host_window_mapped);
        assert!(caps.reason.is_some());
    }

    #[test]
    fn adopted_and_external_are_never_stopped_by_us() {
        let adopted = ScrcpyProcess::adopted(Some(10), "sky-cua-phone-dev1", "dev1");
        assert!(!adopted.can_be_stopped_by_us());

        let mut external = ScrcpyProcess::external(None, "some-other-window", "dev1");
        assert!(!external.can_be_stopped_by_us());
        // mark_stopped is a no-op for non-managed windows.
        external.mark_stopped();
        assert_eq!(external.liveness, ScrcpyLiveness::Running);
    }

    #[test]
    fn managed_stop_transitions_liveness() {
        let mut proc = ScrcpyProcess::managed(7, "dev1", ScrcpyCodec::Default);
        proc.mark_stopped();
        assert_eq!(proc.liveness, ScrcpyLiveness::Stopped);
        assert!(!proc.can_be_stopped_by_us());
    }

    #[test]
    fn active_capabilities_track_process_state() {
        let proc = ScrcpyProcess::managed(1, "dev1", ScrcpyCodec::H265);
        let caps = active_capabilities(Some("4.0".to_string()), &proc, true);
        assert!(caps.active);
        assert!(caps.host_window_mapped);
        assert_eq!(caps.window_title.as_deref(), Some("sky-cua-phone-dev1"));
        assert_eq!(caps.video_codec.as_deref(), Some("h265"));
        assert!(super::super::host_overlay_enabled(&caps));

        // Mapping not current: overlay must not be enabled even while active.
        let unmapped = active_capabilities(Some("4.0".to_string()), &proc, false);
        assert!(unmapped.active);
        assert!(!unmapped.host_window_mapped);
        assert!(!super::super::host_overlay_enabled(&unmapped));
    }

    #[test]
    fn crashed_process_active_capabilities_are_inert() {
        let mut proc = ScrcpyProcess::managed(1, "dev1", ScrcpyCodec::H265);
        proc.mark_crashed();
        let caps = active_capabilities(Some("4.0".to_string()), &proc, true);
        assert!(!caps.active);
        assert!(!caps.host_window_mapped);
        assert!(!super::super::host_overlay_enabled(&caps));
    }
}
