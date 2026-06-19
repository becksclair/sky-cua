//! APK install and port-forward primitives.
//!
//! Split from the ADB lane root to keep `adb.rs` under the god-file threshold.
//! Install modes are modeled explicitly per the requirements matrix: single APK
//! (`install`), split APKs of one package (`install-multiple`), and several
//! packages at once (`install-multi-package`), each with explicit
//! reinstall/downgrade/test/grant flags and structured failure classification.
//! `install_replace`/`forward_tcp` are the companion-bootstrap primitives the
//! companion lane and integrator consume.

use super::parse::parse_install_failure;
use super::{InputOutcome, bound, command_line, input_outcome, serial_args};
use crate::phone::command::{CommandError, CommandOutput, CommandRunner, resolve_adb_path};

/// Structured classification of an install attempt outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::phone) struct InstallOutcome {
    pub(in crate::phone) success: bool,
    /// Stable failure class from [`parse_install_failure`] (e.g.
    /// `INSTALL_FAILED_VERSION_DOWNGRADE`), or `None` on success.
    pub(in crate::phone) failure_class: Option<String>,
    pub(in crate::phone) message: String,
}

/// Build the shared `install` flag tail from the explicit option set.
fn install_flags<'a>(
    reinstall: bool,
    allow_downgrade: bool,
    allow_test_apk: bool,
    grant_runtime_permissions: bool,
    extra: &mut Vec<&'a str>,
) {
    if reinstall {
        extra.push("-r");
    }
    if allow_downgrade {
        extra.push("-d");
    }
    if allow_test_apk {
        extra.push("-t");
    }
    if grant_runtime_permissions {
        extra.push("-g");
    }
}

fn classify_install(adb: &str, argv: &[&str], output: &CommandOutput) -> InstallOutcome {
    let combined = format!("{}{}", output.stdout_string(), output.stderr_string());
    if output.success() && combined.to_ascii_lowercase().contains("success") {
        return InstallOutcome {
            success: true,
            failure_class: None,
            message: String::new(),
        };
    }
    InstallOutcome {
        success: false,
        failure_class: parse_install_failure(&combined),
        message: format!(
            "{} -> {}",
            command_line(adb, argv),
            bound(combined.trim(), 400)
        ),
    }
}

/// Single-APK install/update: `adb -s S install [-r -d -t -g] <apk>`.
/// Reachable through [`install_replace`] (the companion auto-install path).
pub(in crate::phone) async fn install_single(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    apk_path: &str,
    reinstall: bool,
    allow_downgrade: bool,
    allow_test_apk: bool,
    grant_runtime_permissions: bool,
) -> Result<InstallOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let mut flags = Vec::new();
    install_flags(
        reinstall,
        allow_downgrade,
        allow_test_apk,
        grant_runtime_permissions,
        &mut flags,
    );
    let mut tail = vec!["install"];
    tail.extend_from_slice(&flags);
    tail.push(apk_path);
    let argv = serial_args(serial, &tail);
    let output = runner.run(&adb, &argv).await?;
    Ok(classify_install(&adb, &argv, &output))
}

/// Split-APK install of one package: `adb -s S install-multiple [...] <apks>`.
pub(in crate::phone) async fn install_multiple(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    apk_paths: &[String],
    reinstall: bool,
    allow_downgrade: bool,
    allow_test_apk: bool,
    grant_runtime_permissions: bool,
) -> Result<InstallOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let mut flags = Vec::new();
    install_flags(
        reinstall,
        allow_downgrade,
        allow_test_apk,
        grant_runtime_permissions,
        &mut flags,
    );
    let mut tail = vec!["install-multiple"];
    tail.extend_from_slice(&flags);
    for apk in apk_paths {
        tail.push(apk.as_str());
    }
    let argv = serial_args(serial, &tail);
    let output = runner.run(&adb, &argv).await?;
    Ok(classify_install(&adb, &argv, &output))
}

/// Several packages at once: `adb -s S install-multi-package [...] <apks>`.
pub(in crate::phone) async fn install_multi_package(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    apk_paths: &[String],
    reinstall: bool,
    allow_downgrade: bool,
    allow_test_apk: bool,
    grant_runtime_permissions: bool,
) -> Result<InstallOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let mut flags = Vec::new();
    install_flags(
        reinstall,
        allow_downgrade,
        allow_test_apk,
        grant_runtime_permissions,
        &mut flags,
    );
    let mut tail = vec!["install-multi-package"];
    tail.extend_from_slice(&flags);
    for apk in apk_paths {
        tail.push(apk.as_str());
    }
    let argv = serial_args(serial, &tail);
    let output = runner.run(&adb, &argv).await?;
    Ok(classify_install(&adb, &argv, &output))
}

/// Companion-install convenience: `adb -s S install -r <apk>`. Used by the
/// companion lane/integrator for the auto-install/update path; deliberately a
/// thin wrapper over [`install_single`] with `-r` set.
pub(in crate::phone) async fn install_replace(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    apk_path: &str,
    allow_downgrade: bool,
) -> Result<InstallOutcome, CommandError> {
    install_single(
        runner,
        configured_adb_path,
        serial,
        apk_path,
        true,
        allow_downgrade,
        false,
        false,
    )
    .await
}

/// Host-managed `adb -s S forward tcp:<host_port> tcp:<device_port>` used to
/// reach the companion RPC endpoint.
pub(in crate::phone) async fn forward_tcp(
    runner: &dyn CommandRunner,
    configured_adb_path: Option<&str>,
    serial: &str,
    host_port: u16,
    device_port: u16,
) -> Result<InputOutcome, CommandError> {
    let adb = resolve_adb_path(configured_adb_path);
    let host = format!("tcp:{host_port}");
    let device = format!("tcp:{device_port}");
    let argv = serial_args(serial, &["forward", &host, &device]);
    let output = runner.run(&adb, &argv).await?;
    Ok(input_outcome(&adb, &argv, &output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone::command::{CommandOutput, FakeCommandRunner};

    #[tokio::test]
    async fn install_single_sets_explicit_flags() {
        let runner = FakeCommandRunner::new();
        runner.set_stdout(
            "adb",
            &["-s", "S", "install", "-r", "-d", "-t", "-g", "/tmp/app.apk"],
            "Success",
        );
        let outcome = install_single(&runner, None, "S", "/tmp/app.apk", true, true, true, true)
            .await
            .expect("install");
        assert!(outcome.success);
        assert!(outcome.failure_class.is_none());
    }

    #[tokio::test]
    async fn install_single_classifies_downgrade_failure() {
        let runner = FakeCommandRunner::new();
        runner.set_output(
            "adb",
            &["-s", "S", "install", "/tmp/app.apk"],
            CommandOutput {
                status: Some(1),
                stdout: Vec::new(),
                stderr: b"Failure [INSTALL_FAILED_VERSION_DOWNGRADE]".to_vec(),
            },
        );
        let outcome = install_single(
            &runner,
            None,
            "S",
            "/tmp/app.apk",
            false,
            false,
            false,
            false,
        )
        .await
        .expect("install");
        assert!(!outcome.success);
        assert_eq!(
            outcome.failure_class.as_deref(),
            Some("INSTALL_FAILED_VERSION_DOWNGRADE")
        );
    }

    #[tokio::test]
    async fn install_multiple_passes_all_apks() {
        let runner = FakeCommandRunner::new();
        runner.set_stdout(
            "adb",
            &[
                "-s",
                "S",
                "install-multiple",
                "-r",
                "/a/base.apk",
                "/a/split.apk",
            ],
            "Success",
        );
        let apks = vec!["/a/base.apk".to_string(), "/a/split.apk".to_string()];
        let outcome = install_multiple(&runner, None, "S", &apks, true, false, false, false)
            .await
            .expect("install-multiple");
        assert!(outcome.success);
    }

    #[tokio::test]
    async fn install_multi_package_builds_argv() {
        let runner = FakeCommandRunner::new();
        runner.set_stdout(
            "adb",
            &["-s", "S", "install-multi-package", "/a/x.apk", "/a/y.apk"],
            "Success",
        );
        let apks = vec!["/a/x.apk".to_string(), "/a/y.apk".to_string()];
        let outcome = install_multi_package(&runner, None, "S", &apks, false, false, false, false)
            .await
            .expect("install-multi-package");
        assert!(outcome.success);
    }

    #[tokio::test]
    async fn install_replace_is_reinstall_shortcut() {
        let runner = FakeCommandRunner::new();
        runner.set_stdout(
            "adb",
            &["-s", "S", "install", "-r", "/tmp/companion.apk"],
            "Success",
        );
        let outcome = install_replace(&runner, None, "S", "/tmp/companion.apk", false)
            .await
            .expect("install -r");
        assert!(outcome.success);
    }

    #[tokio::test]
    async fn forward_tcp_builds_argv() {
        let runner = FakeCommandRunner::new();
        runner.set_stdout("adb", &["-s", "S", "forward", "tcp:47683", "tcp:47683"], "");
        let outcome = forward_tcp(&runner, None, "S", 47683, 47683)
            .await
            .expect("forward");
        assert!(outcome.success);
    }
}
