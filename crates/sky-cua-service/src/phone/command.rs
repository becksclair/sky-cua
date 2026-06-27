//! Backend seam: the external-process boundary every phone backend goes through.
//!
//! `adb`, `companion` (when it shells out for install/forward), and `scrcpy`
//! never call [`std::process::Command`] directly. They go through a
//! [`CommandRunner`] so the manager can inject a [`FakeCommandRunner`] in tests
//! and so real device control is concentrated behind one auditable trait.
//!
//! This module ships:
//! - [`CommandRunner`]: the trait the ADB lane (and later companion/scrcpy
//!   lanes) implements. One async method, `run`, taking a program plus args and
//!   returning a [`CommandOutput`].
//! - [`RealCommandRunner`]: spawns the program with [`tokio::process::Command`],
//!   captures stdout/stderr/exit, and maps spawn failures to
//!   [`CommandError::Spawn`].
//! - [`resolve_adb_path`]: resolves the `adb` binary from config, then the
//!   `SKY_CUA_ADB` environment override, then `PATH`.
//! - [`FakeCommandRunner`]: a deterministic, scripted runner used by the
//!   manager unit tests, ADB/device parser tests, and routing tests.

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::collections::VecDeque;
use std::process::Stdio;
#[cfg(test)]
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// Environment variable that overrides the configured `adb` binary path.
pub(crate) const SKY_CUA_ADB_ENV: &str = "SKY_CUA_ADB";
/// Environment variable that bounds real external phone backend commands.
pub(crate) const SKY_CUA_COMMAND_TIMEOUT_MS_ENV: &str = "SKY_CUA_PHONE_COMMAND_TIMEOUT_MS";
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// Captured result of an external command. Mirrors the relevant subset of
/// [`std::process::Output`] but owns its bytes so it can cross await points and
/// be cloned into diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandOutput {
    /// Process exit status code, or `None` when the process was terminated by a
    /// signal before producing a code.
    pub(crate) status: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

impl CommandOutput {
    /// Whether the process exited successfully (status code 0).
    pub(crate) fn success(&self) -> bool {
        self.status == Some(0)
    }

    /// Stdout decoded lossily as UTF-8. Convenience for the text-oriented `adb`
    /// subcommands the ADB lane parses (`adb devices -l`, `wm size`, etc.).
    pub(crate) fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Stderr decoded lossily as UTF-8. Used for structured failure
    /// classification (install/pair/connect errors report on stderr).
    pub(crate) fn stderr_string(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Why a command could not be run. The ADB lane maps these into structured
/// `DiagnosticEntry`s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandError {
    /// The backend that would run this command is not implemented yet. Retained
    /// for the unconfigured [`FakeCommandRunner`] path so tests stay honest; the
    /// real runner never constructs it.
    #[cfg_attr(not(test), expect(dead_code))]
    NotImplemented {
        /// The program that would have been invoked (e.g. `"adb"`).
        program: String,
    },
    /// The program could not be found or spawned. Constructed by the real
    /// [`RealCommandRunner`] when `tokio::process` fails to spawn or wait on the
    /// child.
    Spawn { program: String, message: String },
    /// The command exceeded the bounded real-runner timeout.
    Timeout { program: String, timeout_ms: u64 },
}

impl CommandError {
    /// Stable diagnostic code so callers map errors to structured fields rather
    /// than parsing prose. The ADB lane maps these into `DiagnosticEntry`s.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            CommandError::NotImplemented { .. } => "PhoneCommandNotImplemented",
            CommandError::Spawn { .. } => "PhoneCommandSpawnFailed",
            CommandError::Timeout { .. } => "PhoneCommandTimedOut",
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::NotImplemented { program } => {
                write!(f, "command runner has no scripted result for `{program}`")
            }
            CommandError::Spawn { program, message } => {
                write!(f, "failed to spawn `{program}`: {message}")
            }
            CommandError::Timeout {
                program,
                timeout_ms,
            } => {
                write!(f, "`{program}` timed out after {timeout_ms}ms")
            }
        }
    }
}

/// The external-process seam. The ADB lane implements this against a real
/// `adb` binary path; the companion and scrcpy lanes reuse it for their own
/// process invocations. Implementations must be `Send + Sync` because the
/// manager stores one behind an `Arc` and shares it across requests.
///
/// `#[async_trait]` makes this dyn-compatible so the manager can hold an
/// `Arc<dyn CommandRunner>`, matching the `DesktopBackend` trait in
/// `sky-cua-platform`.
#[async_trait::async_trait]
pub(crate) trait CommandRunner: Send + Sync {
    /// Run `program` with `args` and capture its output. `args` is borrowed as
    /// `&[&str]` so callers can pass slices of owned argv without allocating a
    /// `Vec<String>` per call.
    async fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError>;

    /// Run `program` with `args`, write `stdin` to the child's standard input,
    /// then capture output. Used for sensitive prompts such as wireless ADB
    /// pairing codes so secrets do not appear in host argv.
    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        _stdin: &[u8],
    ) -> Result<CommandOutput, CommandError> {
        self.run(program, args).await
    }
}

/// Resolve the `adb` binary to invoke.
///
/// Resolution order, highest precedence first:
/// 1. the explicit `configured_path` (from `PhoneConfig.adb_path`, already
///    overlaid with the `SKY_CUA_ADB` config-env value by the resolver in
///    `sky-cua-platform`),
/// 2. the `SKY_CUA_ADB` process environment override read here directly, so a
///    backend can honor it even when constructed without a resolved selection,
/// 3. the literal `"adb"`, which the OS resolves against `PATH` at spawn time.
///
/// This never probes the filesystem; it only decides which program string to
/// hand to [`CommandRunner::run`]. Availability is determined by actually
/// running `adb version` and observing success.
pub(crate) fn resolve_adb_path(configured_path: Option<&str>) -> String {
    if let Some(path) = configured_path {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(env_path) = std::env::var(SKY_CUA_ADB_ENV) {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "adb".to_string()
}

/// Real process runner.
///
/// Spawns `program` with `tokio::process::Command`, capturing stdout, stderr,
/// and the exit status. A failure to spawn or to collect output is mapped to
/// [`CommandError::Spawn`] keyed to the program, so callers route on the
/// structured code rather than parsing prose. The runner imposes a bounded
/// timeout so a stuck `adb`/`scrcpy` subprocess cannot hold the phone manager
/// lock indefinitely.
#[derive(Debug, Default, Clone)]
pub(crate) struct RealCommandRunner;

#[async_trait::async_trait]
impl CommandRunner for RealCommandRunner {
    async fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError> {
        run_real_command(program, args, None).await
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        stdin: &[u8],
    ) -> Result<CommandOutput, CommandError> {
        run_real_command(program, args, Some(stdin)).await
    }
}

async fn run_real_command(
    program: &str,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<CommandOutput, CommandError> {
    let timeout = command_timeout();
    let mut command = tokio::process::Command::new(program);
    command.args(args);
    command.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);

    let mut child = command.spawn().map_err(|error| CommandError::Spawn {
        program: program.to_string(),
        message: error.to_string(),
    })?;

    if let Some(stdin_bytes) = stdin
        && let Some(mut child_stdin) = child.stdin.take()
    {
        child_stdin
            .write_all(stdin_bytes)
            .await
            .map_err(|error| CommandError::Spawn {
                program: program.to_string(),
                message: error.to_string(),
            })?;
    }

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => result.map_err(|error| CommandError::Spawn {
            program: program.to_string(),
            message: error.to_string(),
        })?,
        Err(_) => {
            return Err(CommandError::Timeout {
                program: program.to_string(),
                timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
            });
        }
    };

    Ok(CommandOutput {
        status: output.status.code(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn command_timeout() -> Duration {
    std::env::var(SKY_CUA_COMMAND_TIMEOUT_MS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_COMMAND_TIMEOUT)
}

/// Deterministic command runner for tests. Supports two modes that can be mixed:
///
/// - a per-`(program, args)` canned response map (preferred for parser tests),
///   so a test can pin the exact output for, e.g., `adb -s S exec-out screencap
///   -p` without ordering coupling;
/// - a FIFO queue of scripted results for tests that only care about call order.
///
/// Every invocation is recorded so tests can assert the exact argv the backend
/// built without touching a real device. The keyed map is consulted first; on a
/// miss the FIFO queue is popped; on a miss there, an unconfigured call returns
/// [`CommandError::NotImplemented`], which keeps unscripted tests honest.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct FakeCommandRunner {
    keyed: Mutex<HashMap<String, Result<CommandOutput, CommandError>>>,
    scripted: Mutex<VecDeque<Result<CommandOutput, CommandError>>>,
    recorded: Mutex<Vec<String>>,
}

#[cfg(test)]
impl FakeCommandRunner {
    /// A runner with no scripted results. Any `run` call returns a
    /// `NotImplemented` error, which keeps unconfigured tests honest.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The canonical command-line key for a `(program, args)` pair: the program
    /// followed by space-joined args. Matches the recorded-call format.
    fn key(program: &str, args: &[&str]) -> String {
        let mut line = String::from(program);
        for arg in args {
            line.push(' ');
            line.push_str(arg);
        }
        line
    }

    /// Pin a successful response (status 0, given stdout, empty stderr) for an
    /// exact `(program, args)` command line.
    pub(crate) fn set_stdout(&self, program: &str, args: &[&str], stdout: &str) {
        self.set_output(
            program,
            args,
            CommandOutput {
                status: Some(0),
                stdout: stdout.as_bytes().to_vec(),
                stderr: Vec::new(),
            },
        );
    }

    /// Pin a full canned [`CommandOutput`] (any status/stdout/stderr) for an
    /// exact `(program, args)` command line.
    pub(crate) fn set_output(&self, program: &str, args: &[&str], output: CommandOutput) {
        self.keyed
            .lock()
            .expect("keyed lock")
            .insert(Self::key(program, args), Ok(output));
    }

    /// Pin a spawn failure for an exact `(program, args)` command line, so tests
    /// can drive the "adb missing" / "binary not found" path.
    pub(crate) fn set_error(&self, program: &str, args: &[&str], error: CommandError) {
        self.keyed
            .lock()
            .expect("keyed lock")
            .insert(Self::key(program, args), Err(error));
    }

    /// Queue a successful invocation returning the given stdout (FIFO mode).
    pub(crate) fn push_stdout(&self, stdout: &str) {
        self.scripted
            .lock()
            .expect("scripted lock")
            .push_back(Ok(CommandOutput {
                status: Some(0),
                stdout: stdout.as_bytes().to_vec(),
                stderr: Vec::new(),
            }));
    }

    /// The argv lines the backend actually ran, in order.
    pub(crate) fn recorded_calls(&self) -> Vec<String> {
        self.recorded.lock().expect("recorded lock").clone()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl CommandRunner for FakeCommandRunner {
    async fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, CommandError> {
        let line = Self::key(program, args);
        self.recorded
            .lock()
            .expect("recorded lock")
            .push(line.clone());

        if let Some(result) = self.keyed.lock().expect("keyed lock").get(&line) {
            return result.clone();
        }

        match self.scripted.lock().expect("scripted lock").pop_front() {
            Some(result) => result,
            None => Err(CommandError::NotImplemented {
                program: program.to_string(),
            }),
        }
    }

    async fn run_with_stdin(
        &self,
        program: &str,
        args: &[&str],
        _stdin: &[u8],
    ) -> Result<CommandOutput, CommandError> {
        self.run(program, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolve_adb_path_prefers_configured_path() {
        let resolved = resolve_adb_path(Some("/opt/platform-tools/adb"));
        assert_eq!(resolved, "/opt/platform-tools/adb");
    }

    #[test]
    fn resolve_adb_path_trims_and_ignores_blank_config() {
        // A blank/whitespace configured path must not shadow env/PATH resolution.
        let prior = std::env::var(SKY_CUA_ADB_ENV).ok();
        // SAFETY: env mutation is process-global; nextest runs each test in its
        // own process (see .config/nextest.toml), so no concurrent thread reads
        // or writes the environment here. Restored below.
        unsafe { std::env::remove_var(SKY_CUA_ADB_ENV) };
        assert_eq!(resolve_adb_path(Some("   ")), "adb");
        assert_eq!(resolve_adb_path(None), "adb");
        if let Some(value) = prior {
            unsafe { std::env::set_var(SKY_CUA_ADB_ENV, value) };
        }
    }

    #[tokio::test]
    async fn keyed_response_takes_precedence_over_fifo() {
        let runner = FakeCommandRunner::new();
        runner.set_stdout("adb", &["version"], "Android Debug Bridge version 1.0.41");
        runner.push_stdout("ignored fifo entry");

        let output = runner.run("adb", &["version"]).await.expect("keyed hit");
        assert!(output.success());
        assert!(output.stdout_string().contains("1.0.41"));
        assert_eq!(runner.recorded_calls(), vec!["adb version".to_string()]);
    }

    #[tokio::test]
    async fn set_error_drives_spawn_failure_path() {
        let runner = FakeCommandRunner::new();
        runner.set_error(
            "adb",
            &["version"],
            CommandError::Spawn {
                program: "adb".to_string(),
                message: "No such file or directory".to_string(),
            },
        );
        let error = runner
            .run("adb", &["version"])
            .await
            .expect_err("spawn err");
        assert_eq!(error.code(), "PhoneCommandSpawnFailed");
    }

    #[tokio::test]
    async fn real_command_runner_executes_a_real_process() {
        // `true` exits 0 on every supported CI/dev host; this proves the runner
        // actually spawns rather than returning a placeholder.
        let runner = RealCommandRunner;
        let output = runner.run("true", &[]).await.expect("spawn true");
        assert!(output.success());
    }

    #[tokio::test]
    async fn real_command_runner_maps_missing_binary_to_spawn_error() {
        let runner = RealCommandRunner;
        let error = runner
            .run("sky-cua-nonexistent-binary-xyz", &[])
            .await
            .expect_err("missing binary must not pretend success");
        assert_eq!(error.code(), "PhoneCommandSpawnFailed");
    }

    #[tokio::test]
    async fn real_command_runner_captures_stdout() {
        let runner = RealCommandRunner;
        let output = runner
            .run("printf", &["%s", "hello-adb"])
            .await
            .expect("spawn printf");
        assert_eq!(output.stdout_string(), "hello-adb");
    }

    #[tokio::test]
    async fn real_command_runner_times_out_stuck_processes() {
        let _serial = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let _guard = EnvGuard::set(SKY_CUA_COMMAND_TIMEOUT_MS_ENV, "1");
        let runner = RealCommandRunner;
        let error = runner
            .run("sh", &["-c", "sleep 1"])
            .await
            .expect_err("sleep must exceed the configured timeout");
        assert_eq!(error.code(), "PhoneCommandTimedOut");
    }

    #[tokio::test]
    async fn real_command_runner_writes_stdin() {
        let runner = RealCommandRunner;
        let output = runner
            .run_with_stdin("cat", &[], b"secret-on-stdin")
            .await
            .expect("cat should echo stdin");
        assert_eq!(output.stdout_string(), "secret-on-stdin");
    }

    struct EnvGuard {
        key: &'static str,
        prior: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prior = std::env::var(key).ok();
            unsafe { std::env::set_var(key, value) };
            Self { key, prior }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prior {
                Some(value) => unsafe { std::env::set_var(self.key, value) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }
}
