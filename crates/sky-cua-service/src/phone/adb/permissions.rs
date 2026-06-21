//! Companion secure-settings service enablement.
//!
//! The companion declares no runtime permissions: its overlay rides a
//! `TYPE_ACCESSIBILITY_OVERLAY` window, so there is nothing to `pm grant` or
//! `appops set`. To be usable it needs exactly two secure-settings service
//! entries flipped on — its accessibility service must appear in
//! `enabled_accessibility_services` (with the global `accessibility_enabled`
//! flag set) and its notification listener in `enabled_notification_listeners`.
//! Both lists are writable by the adb `shell` user (`settings put secure ...`),
//! so an install-bearing bootstrap can enable them headlessly on the emulator
//! and on production devices alike.
//!
//! Every write is a read-merge-write: the existing colon-separated list is read,
//! our component appended only when absent, and the result written back. A blind
//! `settings put` would clobber a user's TalkBack/launcher entries, so it is
//! never used. Some OEM builds (notably Samsung One UI) ignore an adb-written
//! accessibility grant behind a manual "Restricted settings" confirmation; the
//! caller verifies the grant took via the companion health probe and surfaces an
//! actionable manual-setup diagnostic when it did not.

use super::{bound, command_line, serial_args, single_quote_for_shell};
use crate::phone::command::{CommandError, CommandRunner, resolve_adb_path};

/// Class suffix of the companion accessibility service, appended to the
/// companion package to form the flattened ComponentName written into
/// `enabled_accessibility_services`. The companion's namespace equals its
/// applicationId, so `<pkg>/<pkg>.service.SkyAccessibilityService` is the
/// installed component (verified against `dumpsys accessibility`).
pub(in crate::phone) const ACCESSIBILITY_SERVICE_CLASS_SUFFIX: &str =
    ".service.SkyAccessibilityService";

/// Class suffix of the companion notification listener service, appended to the
/// companion package to form the entry written into
/// `enabled_notification_listeners`.
pub(in crate::phone) const NOTIFICATION_LISTENER_CLASS_SUFFIX: &str =
    ".service.SkyNotificationListenerService";

/// The global secure setting that must be `1` for any accessibility service to
/// bind. Written alongside the accessibility-services list.
const ACCESSIBILITY_ENABLED_FLAG: &str = "accessibility_enabled";

/// Outcome of ensuring one companion service entry is present in its secure
/// settings list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) struct SecureServiceOutcome {
    pub(in crate::phone) state: SecureServiceState,
    /// Bounded, secret-free settings-command output for the `WriteRejected`
    /// path; empty otherwise.
    pub(in crate::phone) message: String,
}

/// Whether a companion service entry was already present, newly enabled, or
/// could not be enabled over adb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::phone) enum SecureServiceState {
    /// The component was already in the list; no write performed.
    AlreadyEnabled,
    /// The merged value (and the global flag, when applicable) was written.
    Enabled,
    /// A `settings get`/`put` exited non-zero (e.g. a `SecurityException` on a
    /// locked-down OEM build). The component could not be enabled over adb and
    /// needs a manual grant.
    WriteRejected,
}

/// Result of merging a component into a colon-separated secure-settings list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ColonListMerge {
    /// The component is already present; no write needed.
    AlreadyPresent,
    /// Write this value back (existing entries preserved, component appended).
    Write(String),
}

/// Ensure `component` is present in the `setting` secure-settings list, reading
/// the current value and appending only when absent so existing services are
/// never clobbered. When `enable_global_flag` is set, the global
/// `accessibility_enabled` switch is written to `1` after a successful list
/// write (a list entry alone does not bind a service without the global flag).
pub(in crate::phone) async fn ensure_secure_list_service(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    setting: &str,
    component: &str,
    enable_global_flag: bool,
) -> Result<SecureServiceOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);

    let get_argv = serial_args(serial, &["shell", "settings", "get", "secure", setting]);
    let get_output = runner.run(&adb, &get_argv).await?;
    if !get_output.success() {
        return Ok(write_rejected(&adb, &get_argv, &get_output.stderr_string()));
    }

    let merged = match merge_colon_list(&get_output.stdout_string(), component) {
        ColonListMerge::AlreadyPresent => {
            return Ok(SecureServiceOutcome {
                state: SecureServiceState::AlreadyEnabled,
                message: String::new(),
            });
        }
        ColonListMerge::Write(value) => value,
    };

    // The setting key is a constant and the merged value is well-formed
    // ComponentName text, but single-quote the value for the device shell to
    // match the lane's idiom and stay safe against any unexpected character in a
    // device-provided list.
    let put_command = format!(
        "settings put secure {setting} {}",
        single_quote_for_shell(&merged)
    );
    let put_argv = serial_args(serial, &["shell", &put_command]);
    let put_output = runner.run(&adb, &put_argv).await?;
    if !put_output.success() {
        return Ok(write_rejected(&adb, &put_argv, &put_output.stderr_string()));
    }

    if enable_global_flag {
        let flag_command = format!("settings put secure {ACCESSIBILITY_ENABLED_FLAG} 1");
        let flag_argv = serial_args(serial, &["shell", &flag_command]);
        let flag_output = runner.run(&adb, &flag_argv).await?;
        if !flag_output.success() {
            return Ok(write_rejected(
                &adb,
                &flag_argv,
                &flag_output.stderr_string(),
            ));
        }
    }

    Ok(SecureServiceOutcome {
        state: SecureServiceState::Enabled,
        message: String::new(),
    })
}

/// Build a `WriteRejected` outcome carrying the failing command and bounded
/// stderr so the caller can surface an actionable diagnostic.
fn write_rejected(adb: &str, argv: &[&str], stderr: &str) -> SecureServiceOutcome {
    SecureServiceOutcome {
        state: SecureServiceState::WriteRejected,
        message: format!(
            "{} -> {}",
            command_line(adb, argv),
            bound(stderr.trim(), 200)
        ),
    }
}

/// Merge `component` into a `:`-separated secure-settings value, preserving every
/// existing entry. `settings get` renders an unset value as the literal `null`
/// (often with trailing whitespace); both are treated as an empty list.
fn merge_colon_list(existing: &str, component: &str) -> ColonListMerge {
    if list_contains(existing, component) {
        return ColonListMerge::AlreadyPresent;
    }
    let trimmed = existing.trim();
    let current = if trimmed.eq_ignore_ascii_case("null") {
        ""
    } else {
        trimmed
    };
    let merged = if current.is_empty() {
        component.to_string()
    } else {
        format!("{current}:{component}")
    };
    ColonListMerge::Write(merged)
}

/// Whether `component` is already present in a `:`-separated secure-settings
/// value, tolerant of the framework's short (`pkg/.Cls`) spelling and of the
/// literal `null` an unset setting renders.
fn list_contains(existing: &str, component: &str) -> bool {
    let trimmed = existing.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return false;
    }
    trimmed
        .split(':')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| component_eq(entry, component))
}

/// Enable the companion notification listener, additive and bound immediately.
///
/// Unlike the accessibility list, a bare `settings put secure
/// enabled_notification_listeners` can leave the entry present but unbound until
/// the next NotificationManagerService reconcile, which would make the
/// post-install health probe spuriously report the listener off. `cmd
/// notification allow_listener` (available since the companion's minSdk 30) is
/// additive, idempotent, and binds the listener immediately, so it is the
/// authority here. A best-effort pre-read keeps the diagnostic quiet when the
/// listener was already enabled.
pub(in crate::phone) async fn ensure_notification_listener(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    component: &str,
) -> Result<SecureServiceOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);

    let get_argv = serial_args(
        serial,
        &[
            "shell",
            "settings",
            "get",
            "secure",
            "enabled_notification_listeners",
        ],
    );
    let already = match runner.run(&adb, &get_argv).await {
        Ok(output) if output.success() => list_contains(&output.stdout_string(), component),
        _ => false,
    };

    let allow_command = format!(
        "cmd notification allow_listener {}",
        single_quote_for_shell(component)
    );
    let allow_argv = serial_args(serial, &["shell", &allow_command]);
    let allow_output = runner.run(&adb, &allow_argv).await?;
    if !allow_output.success() {
        return Ok(write_rejected(
            &adb,
            &allow_argv,
            &allow_output.stderr_string(),
        ));
    }

    Ok(SecureServiceOutcome {
        state: if already {
            SecureServiceState::AlreadyEnabled
        } else {
            SecureServiceState::Enabled
        },
        message: String::new(),
    })
}

/// Compare two flattened ComponentNames for identity, expanding a leading-dot
/// short class (`pkg/.Cls`) to its package-qualified form so it matches the
/// fully-qualified spelling (`pkg/pkg.Cls`). This keeps the merge idempotent even
/// if the framework stored our component in the short form.
fn component_eq(left: &str, right: &str) -> bool {
    fn parts(value: &str) -> Option<(String, String)> {
        let (package, class) = value.split_once('/')?;
        let package = package.trim();
        let class = class.trim();
        let qualified = match class.strip_prefix('.') {
            Some(rest) => format!("{package}.{rest}"),
            None => class.to_string(),
        };
        Some((package.to_string(), qualified))
    }
    match (parts(left), parts(right)) {
        (Some(left), Some(right)) => left == right,
        _ => left.trim() == right.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone::command::{CommandOutput, FakeCommandRunner};

    const PKG: &str = "com.skycua.phonecompanion";

    fn a11y_component() -> String {
        format!("{PKG}/{PKG}{ACCESSIBILITY_SERVICE_CLASS_SUFFIX}")
    }

    #[test]
    fn merge_into_empty_or_null_writes_just_the_component() {
        let component = a11y_component();
        assert_eq!(
            merge_colon_list("null", &component),
            ColonListMerge::Write(component.clone())
        );
        assert_eq!(
            merge_colon_list("", &component),
            ColonListMerge::Write(component.clone())
        );
        assert_eq!(
            merge_colon_list("  \n", &component),
            ColonListMerge::Write(component)
        );
    }

    #[test]
    fn merge_appends_preserving_existing_entries() {
        let component = a11y_component();
        let existing = "com.example.a/com.example.a.SvcA:com.example.b/com.example.b.SvcB";
        let ColonListMerge::Write(merged) = merge_colon_list(existing, &component) else {
            panic!("expected a write, existing entries must be preserved");
        };
        assert_eq!(merged, format!("{existing}:{component}"));
    }

    #[test]
    fn merge_detects_already_present_full_form() {
        let component = a11y_component();
        let existing = format!("com.example.a/com.example.a.SvcA:{component}");
        assert_eq!(
            merge_colon_list(&existing, &component),
            ColonListMerge::AlreadyPresent
        );
    }

    #[test]
    fn merge_detects_already_present_short_form() {
        // The framework may store the component in short `pkg/.Class` form; it
        // must still be recognized so we never double-append.
        let component = a11y_component();
        let short = format!("{PKG}/{ACCESSIBILITY_SERVICE_CLASS_SUFFIX}");
        assert_eq!(
            merge_colon_list(&short, &component),
            ColonListMerge::AlreadyPresent
        );
    }

    #[tokio::test]
    async fn ensure_enables_when_absent_and_sets_global_flag() {
        let component = a11y_component();
        let runner = FakeCommandRunner::new();
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                "settings",
                "get",
                "secure",
                "enabled_accessibility_services",
            ],
            "null\n",
        );
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                &format!("settings put secure enabled_accessibility_services '{component}'"),
            ],
            "",
        );
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                "settings put secure accessibility_enabled 1",
            ],
            "",
        );
        let outcome = ensure_secure_list_service(
            &runner,
            None,
            "S",
            "enabled_accessibility_services",
            &component,
            true,
        )
        .await
        .expect("run");
        assert_eq!(outcome.state, SecureServiceState::Enabled);
        assert!(
            runner
                .recorded_calls()
                .iter()
                .any(|call| call.contains("accessibility_enabled 1")),
            "the global accessibility_enabled flag must be set after the list write"
        );
    }

    #[tokio::test]
    async fn ensure_is_noop_when_already_present() {
        let component = a11y_component();
        let runner = FakeCommandRunner::new();
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                "settings",
                "get",
                "secure",
                "enabled_accessibility_services",
            ],
            &format!("{component}\n"),
        );
        let outcome = ensure_secure_list_service(
            &runner,
            None,
            "S",
            "enabled_accessibility_services",
            &component,
            true,
        )
        .await
        .expect("run");
        assert_eq!(outcome.state, SecureServiceState::AlreadyEnabled);
        // Only the read ran; an already-present component performs no write.
        assert_eq!(runner.recorded_calls().len(), 1);
    }

    #[tokio::test]
    async fn ensure_reports_write_rejected_when_put_fails() {
        let component = format!("{PKG}/{PKG}{NOTIFICATION_LISTENER_CLASS_SUFFIX}");
        let runner = FakeCommandRunner::new();
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                "settings",
                "get",
                "secure",
                "enabled_notification_listeners",
            ],
            "null\n",
        );
        runner.set_output(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                &format!("settings put secure enabled_notification_listeners '{component}'"),
            ],
            CommandOutput {
                status: Some(255),
                stdout: Vec::new(),
                stderr: b"java.lang.SecurityException: not permitted".to_vec(),
            },
        );
        let outcome = ensure_secure_list_service(
            &runner,
            None,
            "S",
            "enabled_notification_listeners",
            &component,
            false,
        )
        .await
        .expect("run");
        assert_eq!(outcome.state, SecureServiceState::WriteRejected);
        assert!(outcome.message.contains("SecurityException"));
    }

    #[tokio::test]
    async fn notification_listener_uses_cmd_allow_listener_and_reports_enabled() {
        let component = format!("{PKG}/{PKG}{NOTIFICATION_LISTENER_CLASS_SUFFIX}");
        let runner = FakeCommandRunner::new();
        // Pre-read: listener not yet in the list -> newly Enabled.
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                "settings",
                "get",
                "secure",
                "enabled_notification_listeners",
            ],
            "null\n",
        );
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                &format!("cmd notification allow_listener '{component}'"),
            ],
            "",
        );
        let outcome = ensure_notification_listener(&runner, None, "S", &component)
            .await
            .expect("run");
        assert_eq!(outcome.state, SecureServiceState::Enabled);
        // The bind is forced through `cmd notification allow_listener`, not a bare
        // `settings put` that could leave the entry present but unbound.
        assert!(
            runner
                .recorded_calls()
                .iter()
                .any(|call| call.contains("cmd notification allow_listener")),
            "the notification listener must be bound via cmd notification allow_listener"
        );
        assert!(
            !runner
                .recorded_calls()
                .iter()
                .any(|call| call.contains("settings put secure enabled_notification_listeners")),
            "must not write the listener list directly"
        );
    }

    #[tokio::test]
    async fn notification_listener_already_present_stays_quiet() {
        let component = format!("{PKG}/{PKG}{NOTIFICATION_LISTENER_CLASS_SUFFIX}");
        let runner = FakeCommandRunner::new();
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                "settings",
                "get",
                "secure",
                "enabled_notification_listeners",
            ],
            &format!("com.example/com.example.Other:{component}\n"),
        );
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                &format!("cmd notification allow_listener '{component}'"),
            ],
            "",
        );
        let outcome = ensure_notification_listener(&runner, None, "S", &component)
            .await
            .expect("run");
        // Already enabled: reported quiet, but still re-asserted to force a bind.
        assert_eq!(outcome.state, SecureServiceState::AlreadyEnabled);
    }

    #[tokio::test]
    async fn ensure_reports_write_rejected_when_read_fails() {
        let component = a11y_component();
        let runner = FakeCommandRunner::new();
        runner.set_output(
            "adb",
            &[
                "-s",
                "S",
                "shell",
                "settings",
                "get",
                "secure",
                "enabled_accessibility_services",
            ],
            CommandOutput {
                status: Some(1),
                stdout: Vec::new(),
                stderr: b"settings: permission denied".to_vec(),
            },
        );
        let outcome = ensure_secure_list_service(
            &runner,
            None,
            "S",
            "enabled_accessibility_services",
            &component,
            true,
        )
        .await
        .expect("run");
        assert_eq!(outcome.state, SecureServiceState::WriteRejected);
        // A failed read must not attempt a write.
        assert_eq!(runner.recorded_calls().len(), 1);
    }
}
