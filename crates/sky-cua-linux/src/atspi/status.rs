//! Session accessibility enablement (`org.a11y.Status.IsEnabled`).
//!
//! Toolkits only expose AT-SPI trees while the session reports accessibility as
//! enabled. Qt and at-spi2-atk watch the property and register dynamically, but
//! Chromium/Electron evaluate it once at startup: a browser started while it was
//! `false` never registers an application root and stays invisible to AT-SPI
//! until relaunched. On KDE, COSMIC, Hyprland and plain X11 nothing sets the
//! property after login, so sky-cua asserts it as an assistive technology
//! would. It is never restored: the property is session-scoped state that other
//! ATs may depend on, and the bus launcher resets it at session end.

use std::sync::Mutex;
use std::time::Duration;

use atspi::proxy::bus::StatusProxy;
use tokio::time::timeout;
use tracing::{debug, warn};

const STATUS_CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// What this process learned about `IsEnabled` and when it changed it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct StatusRecord {
    /// `IsEnabled` as first observed by this process, before any assertion.
    pub observed_before: Option<bool>,
    /// Boot-relative seconds of the first successful observation.
    pub first_observed_at: Option<f64>,
    /// Boot-relative seconds of the latest `false -> true` flip performed by
    /// this process.
    pub enabled_by_sky_cua_at: Option<f64>,
}

static RECORD: Mutex<StatusRecord> = Mutex::new(StatusRecord {
    observed_before: None,
    first_observed_at: None,
    enabled_by_sky_cua_at: None,
});

fn record() -> StatusRecord {
    *RECORD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn update_record(update: impl FnOnce(&mut StatusRecord)) {
    let mut guard = RECORD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    update(&mut guard);
}

async fn status_proxy() -> Result<StatusProxy<'static>, String> {
    let session = Box::pin(zbus::Connection::session())
        .await
        .map_err(|error| format!("session bus unavailable: {error}"))?;
    StatusProxy::new(&session)
        .await
        .map_err(|error| format!("org.a11y.Status proxy unavailable: {error}"))
}

async fn assert_enabled_inner() -> Result<bool, String> {
    let proxy = status_proxy().await?;
    let enabled = proxy
        .is_enabled()
        .await
        .map_err(|error| format!("reading IsEnabled failed: {error}"))?;
    let now = boot_seconds();
    update_record(|record| {
        if record.observed_before.is_none() {
            record.observed_before = Some(enabled);
            record.first_observed_at = now;
        }
    });
    if enabled {
        return Ok(true);
    }
    if let Err(error) = proxy.set_is_enabled(true).await {
        warn!(%error, "setting org.a11y.Status.IsEnabled failed; accessibility stays disabled");
        return Ok(false);
    }
    update_record(|record| record.enabled_by_sky_cua_at = boot_seconds());
    debug!("enabled session accessibility (org.a11y.Status.IsEnabled=true)");
    Ok(true)
}

/// Ensure `IsEnabled=true` on the session bus. Returns the resulting state:
/// `Ok(false)` means the property read `false` and could not be set; `Err`
/// means it could not be read at all.
pub(crate) async fn assert_enabled() -> Result<bool, String> {
    timeout(STATUS_CALL_TIMEOUT, assert_enabled_inner())
        .await
        .map_err(|_| format!("org.a11y.Status call exceeded {STATUS_CALL_TIMEOUT:?}"))?
}

/// Assert enablement on a plain thread so construction never blocks on D-Bus
/// and apps launched right after daemon start already see accessibility on.
pub(crate) fn spawn_startup_assertion() {
    std::thread::spawn(|| {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                warn!(%error, "could not build runtime to enable session accessibility");
                return;
            }
        };
        if let Err(error) = runtime.block_on(assert_enabled()) {
            warn!(%error, "could not enable session accessibility (org.a11y.Status.IsEnabled)");
        }
    });
}

/// `CLOCK_BOOTTIME` in seconds, the clock `/proc/<pid>/stat` start times use.
fn boot_seconds() -> Option<f64> {
    let mut spec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes only into the provided timespec.
    let status = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut spec) };
    (status == 0).then(|| spec.tv_sec as f64 + spec.tv_nsec as f64 / 1e9)
}

/// Process start time in boot-relative seconds from `/proc/<pid>/stat`.
fn process_start_seconds(pid: u32) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // SAFETY: sysconf has no preconditions.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    parse_start_seconds(&stat, ticks_per_second)
}

fn parse_start_seconds(stat: &str, ticks_per_second: i64) -> Option<f64> {
    if ticks_per_second <= 0 {
        return None;
    }
    // The command name may contain spaces or parentheses; fields resume after
    // the last ')'. Field 22 (starttime) is the 20th field after it.
    let rest = &stat[stat.rfind(')')? + 1..];
    let ticks: u64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    Some(ticks as f64 / ticks_per_second as f64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnabledBy {
    SkyCua,
    Other,
    Unknown,
}

impl EnabledBy {
    fn as_str(self) -> &'static str {
        match self {
            Self::SkyCua => "sky-cua",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnregisteredCause {
    /// The process started while accessibility was disabled; toolkits that only
    /// check at startup (Chromium/Electron) need a relaunch.
    StartedBeforeEnablement,
    /// Accessibility is still disabled and could not be enabled.
    AtSpiDisabled,
    /// Accessibility was enabled before the process started, yet it has no
    /// root: its toolkit does not speak AT-SPI. Relaunching will not help.
    NoAtSpiBridge,
    Unknown,
}

impl UnregisteredCause {
    fn as_str(self) -> &'static str {
        match self {
            Self::StartedBeforeEnablement => "started_before_enablement",
            Self::AtSpiDisabled => "at_spi_disabled",
            Self::NoAtSpiBridge => "no_at_spi_bridge",
            Self::Unknown => "unknown",
        }
    }

    fn remedy(self) -> &'static str {
        match self {
            Self::StartedBeforeEnablement => "relaunch_app",
            Self::AtSpiDisabled => "setup_accessibility",
            Self::NoAtSpiBridge => "none",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Classification {
    pub enabled_by: EnabledBy,
    pub started_before_enablement: Option<bool>,
    pub cause: UnregisteredCause,
}

/// Classify why a process with a compositor window has no AT-SPI root.
///
/// Sandboxed (Flatpak/Snap) processes register their AT-SPI root through a
/// proxy under a different PID, so a PID miss says nothing about their
/// toolkit or launch order: they classify as `Unknown`, never as needing a
/// relaunch or lacking a bridge.
pub(crate) fn classify(
    at_spi_enabled: Option<bool>,
    record: StatusRecord,
    process_start: Option<f64>,
    sandboxed: bool,
) -> Classification {
    let enabled_by = match (record.enabled_by_sky_cua_at, record.observed_before) {
        (Some(_), _) => EnabledBy::SkyCua,
        (None, Some(true)) => EnabledBy::Other,
        _ => EnabledBy::Unknown,
    };
    if at_spi_enabled == Some(false) {
        return Classification {
            enabled_by,
            started_before_enablement: None,
            cause: UnregisteredCause::AtSpiDisabled,
        };
    }
    // The latest moment accessibility is known to have been off is the
    // sky-cua flip; when someone else enabled it, only "it was already on when
    // this daemon first looked" is known.
    let started_before_enablement = match (enabled_by, process_start) {
        (EnabledBy::SkyCua, Some(start)) => {
            record.enabled_by_sky_cua_at.map(|enabled| start < enabled)
        }
        (EnabledBy::Other, Some(start)) => match record.first_observed_at {
            Some(first) if start >= first => Some(false),
            _ => None,
        },
        _ => None,
    };
    let cause = match (at_spi_enabled, started_before_enablement) {
        _ if sandboxed => UnregisteredCause::Unknown,
        (Some(true), Some(true)) => UnregisteredCause::StartedBeforeEnablement,
        (Some(true), Some(false)) => UnregisteredCause::NoAtSpiBridge,
        _ => UnregisteredCause::Unknown,
    };
    Classification {
        enabled_by,
        started_before_enablement,
        cause,
    }
}

pub(crate) const APP_NOT_REGISTERED_CODE: &str = "AccessibilityAppNotRegistered";

/// Structured evidence for a window whose process has no AT-SPI root.
/// Re-asserts enablement first, so a disabled session heals for future
/// launches while the current process is classified against the flip time.
pub(crate) async fn app_not_registered_diagnostic(pid: u32) -> (String, String) {
    let at_spi_enabled = match assert_enabled().await {
        Ok(enabled) => Some(enabled),
        Err(error) => {
            debug!(%error, "IsEnabled re-assertion failed during AT-SPI miss diagnosis");
            None
        }
    };
    let sandboxed = process_is_sandboxed(pid);
    let classification = classify(
        at_spi_enabled,
        record(),
        process_start_seconds(pid),
        sandboxed,
    );
    let message = match classification.cause {
        UnregisteredCause::Unknown if sandboxed => {
            "The sandboxed (Flatpak/Snap) application has no AT-SPI root under its compositor PID; sandboxed apps register through a proxy PID, so the cause is unknown."
        }
        UnregisteredCause::StartedBeforeEnablement => {
            "The application started while session accessibility was disabled; toolkits that only check at startup (Chromium, Electron) expose no AT-SPI tree until relaunched."
        }
        UnregisteredCause::AtSpiDisabled => {
            "Session accessibility (org.a11y.Status.IsEnabled) is disabled and could not be enabled, so applications expose no AT-SPI tree."
        }
        UnregisteredCause::NoAtSpiBridge => {
            "The application started after session accessibility was enabled but registered no AT-SPI root; its toolkit does not expose AT-SPI."
        }
        UnregisteredCause::Unknown => {
            "The application registered no AT-SPI root; whether it started before accessibility was enabled is unknown."
        }
    }
    .to_string();
    let details = format!(
        "pid={pid}; sandboxed={sandboxed}; at_spi_enabled={}; enabled_by={}; process_started_before_enablement={}; cause={}; remedy={}",
        optional_bool(at_spi_enabled),
        classification.enabled_by.as_str(),
        optional_bool(classification.started_before_enablement),
        classification.cause.as_str(),
        classification.cause.remedy(),
    );
    (message, details)
}

/// Flatpak apps run in `app-flatpak-*.scope` cgroups and snaps in `snap.*`
/// ones; both are readable without ptrace access to the process.
fn process_is_sandboxed(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .is_ok_and(|cgroup| cgroup_is_sandboxed(&cgroup))
}

fn cgroup_is_sandboxed(cgroup: &str) -> bool {
    cgroup.lines().any(|line| {
        line.rsplit('/')
            .next()
            .is_some_and(|unit| unit.starts_with("app-flatpak-") || unit.starts_with("snap."))
    })
}

fn optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flipped_at(at: f64) -> StatusRecord {
        StatusRecord {
            observed_before: Some(false),
            first_observed_at: Some(at),
            enabled_by_sky_cua_at: Some(at),
        }
    }

    #[test]
    fn process_started_before_sky_cua_flip_needs_relaunch() {
        let result = classify(Some(true), flipped_at(100.0), Some(40.0), false);
        assert_eq!(result.enabled_by, EnabledBy::SkyCua);
        assert_eq!(result.started_before_enablement, Some(true));
        assert_eq!(result.cause, UnregisteredCause::StartedBeforeEnablement);
        assert_eq!(result.cause.remedy(), "relaunch_app");
    }

    #[test]
    fn process_started_after_flip_has_no_bridge() {
        let result = classify(Some(true), flipped_at(100.0), Some(140.0), false);
        assert_eq!(result.cause, UnregisteredCause::NoAtSpiBridge);
        assert_eq!(result.cause.remedy(), "none");
    }

    #[test]
    fn enabled_by_other_is_unknown_for_older_processes() {
        let record = StatusRecord {
            observed_before: Some(true),
            first_observed_at: Some(100.0),
            enabled_by_sky_cua_at: None,
        };
        let older = classify(Some(true), record, Some(40.0), false);
        assert_eq!(older.enabled_by, EnabledBy::Other);
        assert_eq!(older.started_before_enablement, None);
        assert_eq!(older.cause, UnregisteredCause::Unknown);

        let newer = classify(Some(true), record, Some(140.0), false);
        assert_eq!(newer.started_before_enablement, Some(false));
        assert_eq!(newer.cause, UnregisteredCause::NoAtSpiBridge);
    }

    #[test]
    fn disabled_session_reports_setup_remedy() {
        let result = classify(Some(false), StatusRecord::default(), Some(10.0), false);
        assert_eq!(result.cause, UnregisteredCause::AtSpiDisabled);
        assert_eq!(result.cause.remedy(), "setup_accessibility");
    }

    #[test]
    fn unreadable_state_or_start_time_is_unknown() {
        assert_eq!(
            classify(None, flipped_at(100.0), Some(40.0), false).cause,
            UnregisteredCause::Unknown
        );
        assert_eq!(
            classify(Some(true), flipped_at(100.0), None, false).cause,
            UnregisteredCause::Unknown
        );
    }

    #[test]
    fn sandboxed_processes_never_claim_relaunch_or_missing_bridge() {
        let before = classify(Some(true), flipped_at(100.0), Some(40.0), true);
        let after = classify(Some(true), flipped_at(100.0), Some(140.0), true);
        assert_eq!(before.cause, UnregisteredCause::Unknown);
        assert_eq!(after.cause, UnregisteredCause::Unknown);
        assert_eq!(
            classify(Some(false), flipped_at(100.0), Some(40.0), true).cause,
            UnregisteredCause::AtSpiDisabled,
            "a disabled session is still the cause for sandboxed apps"
        );
    }

    #[test]
    fn detects_flatpak_and_snap_cgroups() {
        assert!(cgroup_is_sandboxed(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-org.mozilla.firefox-1234.scope\n"
        ));
        assert!(cgroup_is_sandboxed(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/snap.chromium.chromium-5f3a.scope"
        ));
        assert!(!cgroup_is_sandboxed(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-brave-browser@abc.service"
        ));
    }

    #[test]
    fn parses_start_time_after_parenthesised_comm() {
        // Field 22 (starttime) = 12345 ticks; comm contains spaces and ')'.
        let stat =
            "4242 (Web Content) x) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 12345 99 100";
        assert_eq!(parse_start_seconds(stat, 100), Some(123.45));
        assert_eq!(parse_start_seconds(stat, 0), None);
        assert_eq!(parse_start_seconds("garbage", 100), None);
    }

    #[test]
    fn own_process_start_precedes_now() {
        let start = process_start_seconds(std::process::id()).expect("own start time");
        let now = boot_seconds().expect("boot clock");
        assert!(start <= now + 1.0, "start={start} now={now}");
    }
}
