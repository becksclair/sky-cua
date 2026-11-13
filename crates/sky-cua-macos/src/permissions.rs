//! TCC permission status, System Settings guidance, and doctor reporting.
//!
//! macOS attributes the sky-cua daemon to its responsible process: the
//! application that launched it (a terminal, Codex, or a Claude host). Every
//! grant, prompt, and System Settings entry therefore belongs to that host
//! application, and the guidance here names it that way.

mod doctor;

use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;
use objc2_application_services::{AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFString};
use objc2_foundation::{NSString, NSURL};
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use sky_cua_platform::model::{
    AccessibilitySetupReport, DoctorReport, EnvironmentInfo, SetupCommandReport,
};

/// The TCC services the backend depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Accessibility: AX tree reads and semantic actions.
    Accessibility,
    /// Screen & System Audio Recording: ScreenCaptureKit and window titles.
    ScreenCapture,
    /// Event synthesis: CGEvent posting.
    PostEvent,
}

/// System Settings path of the list that holds Accessibility and event-posting
/// clients. macOS 27 labels the row "Device Control and Data Access"
/// (`ACCESSIBILITY` in the Privacy & Security extension's string table); earlier
/// releases label it "Accessibility".
const ACCESSIBILITY_SETTINGS_PATH: &str = "System Settings > Privacy & Security > Device Control and Data Access (named Accessibility on earlier macOS releases)";
const SCREEN_CAPTURE_SETTINGS_PATH: &str =
    "System Settings > Privacy & Security > Screen Recording";
const ACCESSIBILITY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
const SCREEN_CAPTURE_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
/// Who owns the grant, in the wording every message uses.
const HOST_APPLICATION: &str =
    "the application that launched sky-cua (your terminal, Codex, or Claude)";
/// The client-facing setup entry point.
const SETUP_HINT: &str = "Run setup_desktop with operation \"accessibility\" to add that application to the list and open the pane.";

impl Permission {
    /// Every permission, in the order setup requests them.
    pub const ALL: [Self; 3] = [Self::Accessibility, Self::ScreenCapture, Self::PostEvent];

    /// Human-readable permission name.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Accessibility => "Accessibility",
            Self::ScreenCapture => "Screen Recording",
            Self::PostEvent => "event posting",
        }
    }

    /// Stable `DoctorCheck` name for this permission.
    #[must_use]
    pub fn check_name(self) -> &'static str {
        match self {
            Self::Accessibility => "accessibility_permission",
            Self::ScreenCapture => "screen_capture_permission",
            Self::PostEvent => "post_event_permission",
        }
    }

    /// The System Settings path the user follows to grant this permission.
    /// Event posting is listed with Accessibility clients.
    #[must_use]
    pub fn settings_path(self) -> &'static str {
        match self {
            Self::Accessibility | Self::PostEvent => ACCESSIBILITY_SETTINGS_PATH,
            Self::ScreenCapture => SCREEN_CAPTURE_SETTINGS_PATH,
        }
    }

    /// The `x-apple.systempreferences` URL that opens this permission's
    /// Privacy & Security pane.
    #[must_use]
    pub fn settings_url(self) -> &'static str {
        match self {
            Self::Accessibility | Self::PostEvent => ACCESSIBILITY_SETTINGS_URL,
            Self::ScreenCapture => SCREEN_CAPTURE_SETTINGS_URL,
        }
    }

    /// Whether a new grant only takes effect after the host application (and
    /// the sky-cua daemon it launched) restarts. See `request_missing`.
    #[must_use]
    pub fn grant_requires_restart(self) -> bool {
        matches!(self, Self::ScreenCapture)
    }

    fn purpose(self) -> &'static str {
        match self {
            Self::Accessibility => "to read and act on application UI",
            Self::ScreenCapture => "to capture screenshots and read window titles",
            Self::PostEvent => "to send pointer and keyboard input",
        }
    }
}

/// Current grant state. Reads never prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionStatus {
    pub accessibility: bool,
    pub screen_capture: bool,
    pub post_event: bool,
}

impl PermissionStatus {
    #[must_use]
    pub fn granted(&self, permission: Permission) -> bool {
        match permission {
            Permission::Accessibility => self.accessibility,
            Permission::ScreenCapture => self.screen_capture,
            Permission::PostEvent => self.post_event,
        }
    }

    /// The permissions not granted, in `Permission::ALL` order.
    #[must_use]
    pub fn missing(&self) -> Vec<Permission> {
        Permission::ALL
            .into_iter()
            .filter(|permission| !self.granted(*permission))
            .collect()
    }
}

/// Reads the current grant state without prompting.
#[must_use]
pub fn status() -> PermissionStatus {
    // SAFETY: the three preflight calls take no arguments, never prompt, and
    // are callable from any thread.
    unsafe {
        PermissionStatus {
            accessibility: AXIsProcessTrusted(),
            screen_capture: CGPreflightScreenCaptureAccess(),
            post_event: CGPreflightPostEventAccess(),
        }
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
}

/// The structured error returned when an operation needs a missing grant. It
/// names the System Settings pane the user must open.
#[must_use]
pub fn missing_permission_error(permission: Permission) -> BackendError {
    BackendError::new(
        BackendErrorCode::PermissionRequired,
        missing_permission_message(permission),
    )
}

/// Builds the macOS doctor report for `environment`.
#[must_use]
pub fn doctor_report(environment: EnvironmentInfo, status: PermissionStatus) -> DoctorReport {
    doctor::build(environment, status)
}

/// What `request_missing` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequestOutcome {
    /// Grant state before any request.
    pub before: PermissionStatus,
    /// Grant state read immediately after the requests. Prompts are
    /// asynchronous, so a grant the user has yet to give still reads missing.
    pub after: PermissionStatus,
    /// Permissions whose system request was issued (those missing before).
    pub requested: Vec<Permission>,
    /// The permission whose Privacy & Security pane was opened, if any.
    pub opened_pane: Option<Permission>,
    /// Why opening the pane failed, when it did.
    pub open_error: Option<String>,
}

impl PermissionRequestOutcome {
    #[must_use]
    pub fn missing_after(&self) -> Vec<Permission> {
        self.after.missing()
    }

    /// Whether a requested grant only takes effect after the host application
    /// and the sky-cua daemon restart (Screen Recording was missing).
    #[must_use]
    pub fn requires_restart(&self) -> bool {
        self.requested
            .iter()
            .any(|permission| permission.grant_requires_restart())
    }

    /// The `accessibility_command` entry of an `AccessibilitySetupReport`.
    /// `ok` means every request was issued and the pane opened (or nothing was
    /// missing); it does not mean the user has granted anything yet.
    #[must_use]
    pub fn command_report(&self) -> SetupCommandReport {
        if self.requested.is_empty() {
            return SetupCommandReport {
                ok: true,
                detail: "All macOS permissions (Accessibility, Screen Recording, event posting) are already granted".to_string(),
            };
        }
        let mut detail = format!(
            "Requested {} for {HOST_APPLICATION}",
            join_names(&self.requested)
        );
        match (self.opened_pane, &self.open_error) {
            (_, Some(error)) => {
                detail.push_str(&format!("; could not open System Settings: {error}"));
            }
            (Some(permission), None) => {
                detail.push_str(&format!("; opened {}", permission.settings_path()));
            }
            (None, None) => {}
        }
        let still_missing = self.missing_after();
        if !still_missing.is_empty() {
            detail.push_str(&format!(
                ". Still missing: {}. Enable {} in {}",
                join_names(&still_missing),
                if still_missing.len() == 1 {
                    "it"
                } else {
                    "them"
                },
                settings_paths(&still_missing)
            ));
        }
        if self.requires_restart() {
            detail.push_str(". After granting Screen Recording, quit and reopen that application and restart sky-cua");
        }
        detail.push('.');
        SetupCommandReport {
            ok: self.open_error.is_none(),
            detail,
        }
    }
}

/// Requests every missing permission, then opens the Privacy & Security pane
/// of the first one still missing (Accessibility and event posting share a
/// pane). Granted permissions are never re-requested, so with nothing missing
/// this prompts nothing and opens nothing. Never blocks waiting for the user.
///
/// When grants take effect:
/// - Accessibility applies to the running process: `AXIsProcessTrusted` is
///   re-evaluated on every call, and System Settings offers no "Quit & Reopen"
///   for this list.
/// - Screen Recording needs the host application restarted: System Settings
///   warns that the application "may not be able to record the contents of
///   your screen until it is quit" and offers "Quit & Reopen". The daemon the
///   host launched must restart too.
/// - Event posting: neither the SDK header nor System Settings states a
///   restart requirement; it is treated as applying to the running process.
#[must_use]
pub fn request_missing() -> PermissionRequestOutcome {
    request_missing_with(status, register, open_settings_pane)
}

/// Builds the `setup_accessibility` report from doctor reports taken before
/// and after `request_missing`.
#[must_use]
pub fn setup_report(
    before: DoctorReport,
    outcome: &PermissionRequestOutcome,
    after: DoctorReport,
) -> AccessibilitySetupReport {
    AccessibilitySetupReport {
        changed: outcome.before != outcome.after,
        requires_restart: outcome.requires_restart(),
        accessibility_command: outcome.command_report(),
        before: Box::new(before),
        after: Box::new(after),
    }
}

fn request_missing_with(
    read_status: impl Fn() -> PermissionStatus,
    mut register: impl FnMut(Permission),
    open_pane: impl FnOnce(Permission) -> Result<(), BackendError>,
) -> PermissionRequestOutcome {
    let before = read_status();
    let requested = before.missing();
    for permission in &requested {
        register(*permission);
    }
    let after = read_status();
    let opened = after.missing().first().copied();
    let open_error = opened.and_then(|permission| open_pane(permission).err());
    PermissionRequestOutcome {
        before,
        after,
        requested,
        opened_pane: if open_error.is_some() { None } else { opened },
        open_error: open_error.map(|error| error.message),
    }
}

/// Issues the system request for `permission`, which adds the responsible
/// application to the System Settings list and shows the consent prompt when
/// the decision is undetermined.
fn register(permission: Permission) {
    match permission {
        Permission::Accessibility => {
            let key: &CFString = unsafe { kAXTrustedCheckOptionPrompt };
            let options =
                CFDictionary::<CFString, CFBoolean>::from_slices(&[key], &[CFBoolean::new(true)]);
            // SAFETY: the dictionary maps the documented CFString key to a
            // CFBoolean value; the header states prompting is asynchronous.
            let _ = unsafe { AXIsProcessTrustedWithOptions(Some(options.as_ref())) };
        }
        // SAFETY: argument-free CoreGraphics calls, callable from any thread.
        Permission::ScreenCapture => {
            let _ = unsafe { CGRequestScreenCaptureAccess() };
        }
        Permission::PostEvent => {
            let _ = unsafe { CGRequestPostEventAccess() };
        }
    }
}

fn open_settings_pane(permission: Permission) -> Result<(), BackendError> {
    let url = permission.settings_url();
    let opened = autoreleasepool(|_| {
        NSURL::URLWithString(&NSString::from_str(url))
            .is_some_and(|ns_url| NSWorkspace::sharedWorkspace().openURL(&ns_url))
    });
    if opened {
        Ok(())
    } else {
        Err(BackendError::new(
            BackendErrorCode::Internal,
            format!(
                "NSWorkspace could not open {url}; open {} manually",
                permission.settings_path()
            ),
        ))
    }
}

fn missing_permission_message(permission: Permission) -> String {
    let mut message = format!(
        "macOS {} permission is required {}. Enable it in {} for {HOST_APPLICATION}: macOS attributes the sky-cua daemon to the application that launched it, so the grant belongs to that application.",
        permission.display_name(),
        permission.purpose(),
        permission.settings_path(),
    );
    if permission.grant_requires_restart() {
        message
            .push_str(" Quit and reopen that application and restart sky-cua after granting it.");
    }
    message.push(' ');
    message.push_str(SETUP_HINT);
    message
}

fn join_names(permissions: &[Permission]) -> String {
    let names: Vec<&str> = permissions.iter().map(|p| p.display_name()).collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [first, second] => format!("{first} and {second}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// Distinct settings paths for `permissions`, in order.
fn settings_paths(permissions: &[Permission]) -> String {
    let mut paths: Vec<&str> = Vec::new();
    for permission in permissions {
        let path = permission.settings_path();
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths.join(" and ")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn status_from(missing: &[Permission]) -> PermissionStatus {
        PermissionStatus {
            accessibility: !missing.contains(&Permission::Accessibility),
            screen_capture: !missing.contains(&Permission::ScreenCapture),
            post_event: !missing.contains(&Permission::PostEvent),
        }
    }

    fn all_missing_combinations() -> Vec<Vec<Permission>> {
        (0..8u8)
            .map(|mask| {
                Permission::ALL
                    .into_iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, permission)| permission)
                    .collect()
            })
            .collect()
    }

    #[test]
    #[ignore = "live desktop probe"]
    fn live_status_reads_without_prompting() {
        let status = status();
        eprintln!("{status:?}");
    }

    #[test]
    fn missing_permission_error_is_structured_and_names_the_pane() {
        for permission in Permission::ALL {
            let error = missing_permission_error(permission);
            assert_eq!(error.code, "PermissionRequired");
            assert!(error.message.contains(permission.display_name()));
            assert!(error.message.contains(permission.settings_path()));
            assert!(
                error
                    .message
                    .contains("the application that launched sky-cua")
            );
            assert!(error.message.contains("setup_desktop"));
            assert_eq!(
                error.message.contains("Quit and reopen"),
                permission == Permission::ScreenCapture
            );
        }
    }

    #[test]
    fn settings_urls_use_the_privacy_anchors() {
        assert!(
            Permission::Accessibility
                .settings_url()
                .ends_with("?Privacy_Accessibility")
        );
        assert!(
            Permission::PostEvent
                .settings_url()
                .ends_with("?Privacy_Accessibility")
        );
        assert!(
            Permission::ScreenCapture
                .settings_url()
                .ends_with("?Privacy_ScreenCapture")
        );
    }

    #[test]
    fn request_missing_requests_only_missing_and_opens_first_missing_pane() {
        for missing in all_missing_combinations() {
            let registered = RefCell::new(Vec::new());
            let opened = RefCell::new(Vec::new());
            let outcome = request_missing_with(
                || status_from(&missing),
                |permission| registered.borrow_mut().push(permission),
                |permission| {
                    opened.borrow_mut().push(permission);
                    Ok(())
                },
            );
            assert_eq!(*registered.borrow(), missing);
            assert_eq!(outcome.requested, missing);
            assert_eq!(outcome.before.missing(), missing);
            assert_eq!(outcome.missing_after(), missing);
            assert_eq!(
                *opened.borrow(),
                missing.first().copied().into_iter().collect::<Vec<_>>()
            );
            assert_eq!(outcome.opened_pane, missing.first().copied());
            assert_eq!(
                outcome.requires_restart(),
                missing.contains(&Permission::ScreenCapture)
            );
            let command = outcome.command_report();
            assert!(command.ok);
            for permission in &missing {
                assert!(command.detail.contains(permission.display_name()));
            }
        }
    }

    #[test]
    fn request_missing_skips_the_pane_when_a_prompt_grant_lands_immediately() {
        let calls = RefCell::new(0);
        let outcome = request_missing_with(
            || {
                let mut calls = calls.borrow_mut();
                *calls += 1;
                if *calls == 1 {
                    status_from(&[Permission::PostEvent])
                } else {
                    status_from(&[])
                }
            },
            |_| {},
            |_| panic!("no pane should open once everything is granted"),
        );
        assert_eq!(outcome.requested, vec![Permission::PostEvent]);
        assert!(outcome.missing_after().is_empty());
        assert_eq!(outcome.opened_pane, None);
        assert!(outcome.command_report().ok);
    }

    #[test]
    fn open_failure_is_reported_without_claiming_the_pane_opened() {
        let outcome = request_missing_with(
            || status_from(&[Permission::ScreenCapture]),
            |_| {},
            |permission| {
                Err(BackendError::new(
                    BackendErrorCode::Internal,
                    format!("cannot open {}", permission.settings_url()),
                ))
            },
        );
        assert_eq!(outcome.opened_pane, None);
        let command = outcome.command_report();
        assert!(!command.ok);
        assert!(command.detail.contains("could not open System Settings"));
    }

    #[test]
    fn nothing_missing_reports_ready_command() {
        let outcome = request_missing_with(
            || status_from(&[]),
            |_| panic!("granted permissions must not be re-requested"),
            |_| panic!("no pane should open"),
        );
        let command = outcome.command_report();
        assert!(command.ok);
        assert!(command.detail.contains("already granted"));
        assert!(!outcome.requires_restart());
    }

    #[test]
    fn join_names_reads_naturally() {
        assert_eq!(join_names(&[Permission::Accessibility]), "Accessibility");
        assert_eq!(
            join_names(&[Permission::Accessibility, Permission::PostEvent]),
            "Accessibility and event posting"
        );
        assert_eq!(
            join_names(&Permission::ALL),
            "Accessibility, Screen Recording, and event posting"
        );
    }

    #[test]
    fn settings_paths_deduplicate_the_shared_pane() {
        assert_eq!(
            settings_paths(&[Permission::Accessibility, Permission::PostEvent]),
            ACCESSIBILITY_SETTINGS_PATH
        );
        assert_eq!(
            settings_paths(&Permission::ALL),
            format!("{ACCESSIBILITY_SETTINGS_PATH} and {SCREEN_CAPTURE_SETTINGS_PATH}")
        );
    }
}
