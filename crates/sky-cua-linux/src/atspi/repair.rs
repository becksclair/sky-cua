use std::ffi::CString;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;
use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

use super::ACCESSIBILITY_CONNECTION_TIMEOUT;

const REPAIR_UNIT: &str = "at-spi-dbus-bus.service";
const SYSTEMCTL_PATH: &str = "/usr/bin/systemctl";
const REPAIR_COOLDOWN: i64 = 5 * 60;
const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(10);
const SYSTEMCTL_SHOW_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_COMMAND_OUTPUT: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccessibilityConnectFailure {
    Timeout,
    Error(String),
}

impl AccessibilityConnectFailure {
    pub(crate) fn into_backend_error(self) -> BackendError {
        match self {
            Self::Timeout => BackendError::new(
                BackendErrorCode::AccessibilityUnavailable,
                format!(
                    "AT-SPI accessibility bus connection exceeded the {:?} deadline",
                    ACCESSIBILITY_CONNECTION_TIMEOUT
                ),
            ),
            Self::Error(error) => BackendError::new(
                BackendErrorCode::AccessibilityUnavailable,
                format!("failed to connect to the AT-SPI accessibility bus: {error}"),
            ),
        }
    }

    fn summary(&self) -> String {
        match self {
            Self::Timeout => format!(
                "initial AT-SPI connection timed out after {:?}",
                ACCESSIBILITY_CONNECTION_TIMEOUT
            ),
            Self::Error(error) => format!("initial AT-SPI connection failed: {error}"),
        }
    }
}

#[derive(Debug, Clone)]
struct RepairAuthority {
    runtime_dir: PathBuf,
    expected_runtime_dir: PathBuf,
    bus_address: String,
    effective_uid: u32,
}

impl RepairAuthority {
    fn from_process() -> Result<Self, String> {
        let effective_uid = unsafe { libc::geteuid() };
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| "XDG_RUNTIME_DIR is unset".to_string())?;
        let bus_address = std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .map_err(|_| "DBUS_SESSION_BUS_ADDRESS is unset".to_string())?;
        Ok(Self {
            runtime_dir,
            expected_runtime_dir: PathBuf::from(format!("/run/user/{effective_uid}")),
            bus_address,
            effective_uid,
        })
    }

    #[cfg(test)]
    fn for_test(runtime_dir: PathBuf, effective_uid: u32) -> Self {
        Self {
            bus_address: format!("unix:path={}/bus", runtime_dir.display()),
            expected_runtime_dir: runtime_dir.clone(),
            runtime_dir,
            effective_uid,
        }
    }

    fn validate(&self) -> Result<ValidatedAuthority, String> {
        if !self.runtime_dir.is_absolute() {
            return Err("XDG_RUNTIME_DIR is not an absolute path".to_string());
        }
        if self.runtime_dir != self.expected_runtime_dir {
            return Err(format!(
                "XDG_RUNTIME_DIR is {}, expected {} for effective uid {}",
                self.runtime_dir.display(),
                self.expected_runtime_dir.display(),
                self.effective_uid
            ));
        }
        let canonical_runtime = fs::canonicalize(&self.runtime_dir)
            .map_err(|error| format!("cannot canonicalize XDG_RUNTIME_DIR: {error}"))?;
        if canonical_runtime != self.expected_runtime_dir {
            return Err(format!(
                "XDG_RUNTIME_DIR canonicalized to {}, expected {} for effective uid {}",
                canonical_runtime.display(),
                self.expected_runtime_dir.display(),
                self.effective_uid
            ));
        }
        let runtime_metadata = fs::symlink_metadata(&canonical_runtime)
            .map_err(|error| format!("cannot inspect canonical XDG_RUNTIME_DIR: {error}"))?;
        if !runtime_metadata.is_dir() || runtime_metadata.uid() != self.effective_uid {
            return Err(
                "canonical XDG_RUNTIME_DIR is not an effective-uid-owned directory".to_string(),
            );
        }

        let expected_bus = format!("unix:path={}/bus", self.runtime_dir.display());
        if self.bus_address != expected_bus {
            return Err(
                "DBUS_SESSION_BUS_ADDRESS is not the exact local XDG runtime bus address"
                    .to_string(),
            );
        }

        let bus_path = self.runtime_dir.join("bus");
        let bus_metadata = fs::symlink_metadata(&bus_path)
            .map_err(|error| format!("cannot inspect the session bus socket: {error}"))?;
        if !bus_metadata.file_type().is_socket() {
            return Err("session bus path is not a Unix socket".to_string());
        }
        if bus_metadata.uid() != self.effective_uid {
            return Err(format!(
                "session bus socket is owned by uid {}, not effective uid {}",
                bus_metadata.uid(),
                self.effective_uid
            ));
        }

        Ok(ValidatedAuthority {
            runtime_dir: self.runtime_dir.clone(),
            bus_address: self.bus_address.clone(),
            effective_uid: self.effective_uid,
        })
    }
}

#[derive(Debug, Clone)]
struct ValidatedAuthority {
    runtime_dir: PathBuf,
    bus_address: String,
    effective_uid: u32,
}

#[derive(Debug, Clone)]
struct CommandOutput {
    status: Option<i32>,
    stdout: String,
    stderr: String,
}

impl CommandOutput {
    fn success(&self) -> bool {
        self.status == Some(0)
    }
}

#[derive(Debug, Clone)]
enum CommandFailure {
    Spawn(String),
    Timeout,
}

impl std::fmt::Display for CommandFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "could not start systemctl: {error}"),
            Self::Timeout => write!(formatter, "systemctl exceeded its deadline"),
        }
    }
}

#[async_trait]
trait SystemctlRunner: Send + Sync {
    async fn run(
        &self,
        args: &[&str],
        authority: &ValidatedAuthority,
        deadline: Duration,
    ) -> Result<CommandOutput, CommandFailure>;
}

#[derive(Debug, Default)]
struct ProcessSystemctlRunner;

#[async_trait]
impl SystemctlRunner for ProcessSystemctlRunner {
    async fn run(
        &self,
        args: &[&str],
        authority: &ValidatedAuthority,
        deadline: Duration,
    ) -> Result<CommandOutput, CommandFailure> {
        let mut command = TokioCommand::new(SYSTEMCTL_PATH);
        command
            .args(args)
            .env_clear()
            .env("DBUS_SESSION_BUS_ADDRESS", &authority.bus_address)
            .env("XDG_RUNTIME_DIR", &authority.runtime_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let child = command
            .spawn()
            .map_err(|error| CommandFailure::Spawn(error.to_string()))?;
        collect_child_output(child, deadline).await
    }
}

async fn collect_child_output(
    mut child: Child,
    deadline: Duration,
) -> Result<CommandOutput, CommandFailure> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CommandFailure::Spawn("systemctl stdout was not piped".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CommandFailure::Spawn("systemctl stderr was not piped".to_string()))?;
    let collect = async {
        let stdout = read_bounded(stdout);
        let stderr = read_bounded(stderr);
        let wait = child.wait();
        let (stdout, stderr, status) = tokio::join!(stdout, stderr, wait);
        let status = status.map_err(|error| CommandFailure::Spawn(error.to_string()))?;
        Ok::<_, CommandFailure>(CommandOutput {
            status: status.code(),
            stdout,
            stderr,
        })
    };
    match timeout(deadline, collect).await {
        Ok(result) => result,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(CommandFailure::Timeout)
        }
    }
}

async fn read_bounded<R>(mut reader: R) -> String
where
    R: AsyncRead + Unpin,
{
    let mut captured = Vec::with_capacity(MAX_COMMAND_OUTPUT.min(4096));
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                if captured.len() < MAX_COMMAND_OUTPUT {
                    let remaining = MAX_COMMAND_OUTPUT - captured.len();
                    captured.extend_from_slice(&buffer[..read.min(remaining)]);
                }
            }
        }
    }
    String::from_utf8_lossy(&captured).into_owned()
}

trait EpochClock: Send + Sync {
    fn now(&self) -> i64;
}

#[derive(Debug, Default)]
struct SystemEpochClock;

impl EpochClock for SystemEpochClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
            .unwrap_or(0)
    }
}

#[derive(Clone)]
pub(crate) struct RepairCoordinator {
    authority: Option<RepairAuthority>,
    runner: Arc<dyn SystemctlRunner>,
    clock: Arc<dyn EpochClock>,
}

impl std::fmt::Debug for RepairCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RepairCoordinator")
            .field("authority", &self.authority)
            .finish_non_exhaustive()
    }
}

impl RepairCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            authority: None,
            runner: Arc::new(ProcessSystemctlRunner),
            clock: Arc::new(SystemEpochClock),
        }
    }

    async fn repair_and_retry<T, Retry, RetryFuture>(
        &self,
        original: &AccessibilityConnectFailure,
        retry: Retry,
    ) -> Result<T, BackendError>
    where
        Retry: FnOnce() -> RetryFuture,
        RetryFuture: std::future::Future<Output = Result<T, AccessibilityConnectFailure>>,
    {
        let authority = match self.authority.clone() {
            Some(authority) => Ok(authority),
            None => RepairAuthority::from_process(),
        }
        .map_err(|reason| {
            unavailable(
                original,
                format!("automatic repair not attempted: authority rejected ({reason})"),
            )
        })?;
        let validated = authority.validate().map_err(|reason| {
            unavailable(
                original,
                format!("automatic repair not attempted: authority rejected ({reason})"),
            )
        })?;

        let unit = self
            .runner
            .run(
                &[
                    "--user",
                    "show",
                    "--property=LoadState",
                    "--value",
                    REPAIR_UNIT,
                ],
                &validated,
                SYSTEMCTL_SHOW_TIMEOUT,
            )
            .await
            .map_err(|error| {
                unavailable(
                    original,
                    format!("automatic repair not attempted: unit check failed ({error})"),
                )
            })?;
        if !unit.success() || unit.stdout.trim() != "loaded" {
            return Err(unavailable(
                original,
                format!(
                    "automatic repair not attempted: {REPAIR_UNIT} is not loaded (status {:?}, output {:?})",
                    unit.status,
                    command_output_summary(&unit)
                ),
            ));
        }

        let mut lock = acquire_repair_lock(&validated).map_err(|outcome| {
            unavailable(
                original,
                format!("automatic repair not attempted: {outcome}"),
            )
        })?;
        let now = self.clock.now();
        if let Some(last_attempt) = read_last_attempt(&mut lock).map_err(|error| {
            unavailable(
                original,
                format!("automatic repair not attempted: cooldown state is unreadable ({error})"),
            )
        })? && now.saturating_sub(last_attempt) < REPAIR_COOLDOWN
        {
            return Err(unavailable(
                original,
                format!(
                    "automatic repair not attempted: cooldown active (last attempt epoch {last_attempt})"
                ),
            ));
        }

        write_last_attempt(&mut lock, now).map_err(|error| {
            unavailable(
                original,
                format!("automatic repair not attempted: could not record attempt ({error})"),
            )
        })?;

        let restart = self
            .runner
            .run(
                &["--user", "restart", REPAIR_UNIT],
                &validated,
                SYSTEMCTL_TIMEOUT,
            )
            .await;
        let restart = match restart {
            Ok(output) if output.success() => output,
            Ok(output) => {
                return Err(unavailable(
                    original,
                    format!(
                        "automatic repair failed: restart exited with status {:?} ({})",
                        output.status,
                        command_output_summary(&output)
                    ),
                ));
            }
            Err(error) => {
                return Err(unavailable(
                    original,
                    format!("automatic repair failed: {error}"),
                ));
            }
        };
        let _ = restart;

        match retry().await {
            Ok(connection) => Ok(connection),
            Err(error) => Err(unavailable(
                original,
                format!(
                    "automatic repair restarted {REPAIR_UNIT}, but the single retry failed: {}",
                    error.summary()
                ),
            )),
        }
    }

    #[cfg(test)]
    fn for_test(
        runtime_dir: PathBuf,
        effective_uid: u32,
        runner: Arc<dyn SystemctlRunner>,
        clock: Arc<dyn EpochClock>,
    ) -> Self {
        Self {
            authority: Some(RepairAuthority::for_test(runtime_dir, effective_uid)),
            runner,
            clock,
        }
    }
}

pub(crate) async fn connect_with_repair<T, Initial, InitialFuture, Retry, RetryFuture>(
    coordinator: &RepairCoordinator,
    initial: Initial,
    retry: Retry,
) -> Result<T, BackendError>
where
    Initial: FnOnce() -> InitialFuture,
    InitialFuture: std::future::Future<Output = Result<T, AccessibilityConnectFailure>>,
    Retry: FnOnce() -> RetryFuture,
    RetryFuture: std::future::Future<Output = Result<T, AccessibilityConnectFailure>>,
{
    match initial().await {
        Ok(connection) => Ok(connection),
        Err(AccessibilityConnectFailure::Error(error)) => {
            Err(AccessibilityConnectFailure::Error(error).into_backend_error())
        }
        Err(timeout @ AccessibilityConnectFailure::Timeout) => {
            coordinator.repair_and_retry(&timeout, retry).await
        }
    }
}

fn unavailable(original: &AccessibilityConnectFailure, outcome: String) -> BackendError {
    BackendError::new(
        BackendErrorCode::AccessibilityUnavailable,
        format!("{}; {outcome}", original.summary()),
    )
}

fn command_output_summary(output: &CommandOutput) -> String {
    format!("stdout={:?}, stderr={:?}", output.stdout, output.stderr)
}

struct RepairLock {
    file: File,
}

impl std::fmt::Debug for RepairLock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RepairLock(..)")
    }
}

fn acquire_repair_lock(authority: &ValidatedAuthority) -> Result<RepairLock, String> {
    let repair_dir = authority.runtime_dir.join("sky-cua");
    match fs::create_dir(&repair_dir) {
        Ok(()) => {
            fs::set_permissions(&repair_dir, fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("could not secure coordinator directory: {error}"))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("could not create coordinator directory: {error}")),
    }
    let directory_metadata = fs::symlink_metadata(&repair_dir)
        .map_err(|error| format!("could not inspect coordinator directory: {error}"))?;
    if !directory_metadata.is_dir()
        || directory_metadata.uid() != authority.effective_uid
        || directory_metadata.mode() & 0o777 != 0o700
    {
        return Err("coordinator directory is not an owner-only directory".to_string());
    }

    let directory_fd = open_directory_no_follow(&repair_dir)?;
    let lock_name = CString::new("atspi-repair.lock").expect("static lock filename has no NUL");
    let lock_fd = unsafe {
        libc::openat(
            directory_fd,
            lock_name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    let close_result = unsafe { libc::close(directory_fd) };
    if close_result != 0 {
        tracing::debug!(error = ?std::io::Error::last_os_error(), "failed to close coordinator directory fd");
    }
    if lock_fd < 0 {
        return Err(format!(
            "could not open coordinator lock: {}",
            std::io::Error::last_os_error()
        ));
    }

    let lock = unsafe { File::from_raw_fd(lock_fd) };
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(lock_fd, stat.as_mut_ptr()) } != 0 {
        return Err(format!(
            "could not inspect coordinator lock: {}",
            std::io::Error::last_os_error()
        ));
    }
    let stat = unsafe { stat.assume_init() };
    if stat.st_uid != authority.effective_uid
        || (stat.st_mode as libc::mode_t & libc::S_IFMT) != libc::S_IFREG
    {
        return Err("coordinator lock is not an owner-owned regular file".to_string());
    }
    if unsafe { libc::fchmod(lock_fd, 0o600) } != 0 {
        return Err(format!(
            "could not secure coordinator lock: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::flock(lock_fd, libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Err("coordinator lock is busy".to_string());
        }
        return Err(format!("could not acquire coordinator lock: {error}"));
    }
    Ok(RepairLock { file: lock })
}

fn open_directory_no_follow(path: &Path) -> Result<RawFd, String> {
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| "coordinator directory path contains NUL".to_string())?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(format!(
            "could not open coordinator directory: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(fd)
}

fn read_last_attempt(lock: &mut RepairLock) -> Result<Option<i64>, String> {
    lock.file
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut contents = String::new();
    lock.file
        .read_to_string(&mut contents)
        .map_err(|error| error.to_string())?;
    let contents = contents.trim();
    if contents.is_empty() {
        return Ok(None);
    }
    contents
        .parse::<i64>()
        .map(Some)
        .map_err(|error| format!("invalid last-attempt epoch: {error}"))
}

fn write_last_attempt(lock: &mut RepairLock, now: i64) -> Result<(), String> {
    lock.file.set_len(0).map_err(|error| error.to_string())?;
    lock.file
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    writeln!(lock.file, "{now}").map_err(|error| error.to_string())?;
    lock.file.sync_data().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use super::*;

    static TEST_ID: AtomicU64 = AtomicU64::new(0);

    #[derive(Clone)]
    struct FakeClock {
        now: i64,
    }

    impl EpochClock for FakeClock {
        fn now(&self) -> i64 {
            self.now
        }
    }

    #[derive(Default)]
    struct FakeRunner {
        invocations: Mutex<Vec<Vec<String>>>,
        responses: Mutex<Vec<Result<CommandOutput, CommandFailure>>>,
    }

    impl FakeRunner {
        fn with_responses(responses: Vec<Result<CommandOutput, CommandFailure>>) -> Arc<Self> {
            Arc::new(Self {
                invocations: Mutex::new(Vec::new()),
                responses: Mutex::new(responses),
            })
        }

        fn invocations(&self) -> Vec<Vec<String>> {
            self.invocations.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SystemctlRunner for FakeRunner {
        async fn run(
            &self,
            args: &[&str],
            _authority: &ValidatedAuthority,
            _deadline: Duration,
        ) -> Result<CommandOutput, CommandFailure> {
            self.invocations
                .lock()
                .unwrap()
                .push(args.iter().map(|arg| (*arg).to_string()).collect());
            self.responses.lock().unwrap().remove(0)
        }
    }

    fn output(stdout: &str) -> Result<CommandOutput, CommandFailure> {
        Ok(CommandOutput {
            status: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
    }

    fn test_authority() -> (PathBuf, UnixListener) {
        let id = TEST_ID.fetch_add(1, Ordering::Relaxed);
        let runtime =
            std::env::temp_dir().join(format!("sky-cua-atspi-repair-{}-{id}", std::process::id()));
        fs::create_dir(&runtime).unwrap();
        let listener = UnixListener::bind(runtime.join("bus")).unwrap();
        (runtime, listener)
    }

    fn coordinator(runtime: &Path, runner: Arc<dyn SystemctlRunner>) -> RepairCoordinator {
        RepairCoordinator::for_test(
            runtime.to_path_buf(),
            unsafe { libc::geteuid() },
            runner,
            Arc::new(FakeClock { now: 10_000 }),
        )
    }

    #[tokio::test]
    async fn timeout_restarts_once_then_retries_successfully() {
        let (runtime, _listener) = test_authority();
        let runner = FakeRunner::with_responses(vec![output("loaded\n"), output("")]);
        let repair = coordinator(&runtime, runner.clone());
        let initial_calls = AtomicUsize::new(0);
        let retry_calls = AtomicUsize::new(0);
        let result = connect_with_repair(
            &repair,
            || async {
                initial_calls.fetch_add(1, Ordering::SeqCst);
                Err(AccessibilityConnectFailure::Timeout)
            },
            || async {
                retry_calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, AccessibilityConnectFailure>("connected")
            },
        )
        .await
        .unwrap();
        assert_eq!(result, "connected");
        assert_eq!(initial_calls.load(Ordering::SeqCst), 1);
        assert_eq!(retry_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            runner.invocations(),
            vec![
                vec![
                    "--user".to_string(),
                    "show".to_string(),
                    "--property=LoadState".to_string(),
                    "--value".to_string(),
                    REPAIR_UNIT.to_string(),
                ],
                vec![
                    "--user".to_string(),
                    "restart".to_string(),
                    REPAIR_UNIT.to_string(),
                ],
            ]
        );
    }

    #[tokio::test]
    async fn ordinary_connection_error_does_not_restart() {
        let (runtime, _listener) = test_authority();
        let runner = FakeRunner::with_responses(Vec::new());
        let repair = coordinator(&runtime, runner.clone());
        let error = connect_with_repair(
            &repair,
            || async {
                Err::<(), _>(AccessibilityConnectFailure::Error(
                    "permission denied".to_string(),
                ))
            },
            || async { panic!("ordinary errors must not retry") },
        )
        .await
        .expect_err("ordinary errors should surface");
        assert!(error.message.contains("permission denied"));
        assert!(runner.invocations().is_empty());
    }

    #[tokio::test]
    async fn healthy_connection_does_not_inspect_or_restart_the_unit() {
        let (runtime, _listener) = test_authority();
        let runner = FakeRunner::with_responses(Vec::new());
        let repair = coordinator(&runtime, runner.clone());
        let result = connect_with_repair(
            &repair,
            || async { Ok::<_, AccessibilityConnectFailure>("connected") },
            || async { panic!("a healthy initial connection must not retry") },
        )
        .await
        .unwrap();
        assert_eq!(result, "connected");
        assert!(runner.invocations().is_empty());
    }

    #[tokio::test]
    async fn custom_bus_address_is_rejected_before_unit_check() {
        let (runtime, _listener) = test_authority();
        let runner = FakeRunner::with_responses(Vec::new());
        let repair = RepairCoordinator {
            authority: Some(RepairAuthority {
                bus_address: "unix:path=/tmp/custom-bus".to_string(),
                ..RepairAuthority::for_test(runtime, unsafe { libc::geteuid() })
            }),
            runner: runner.clone(),
            clock: Arc::new(FakeClock { now: 10_000 }),
        };
        let error = connect_with_repair(
            &repair,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async { panic!("authority rejection must not retry") },
        )
        .await
        .expect_err("custom bus should fail closed");
        assert!(error.message.contains("authority rejected"));
        assert!(runner.invocations().is_empty());
    }

    #[tokio::test]
    async fn restart_failure_does_not_retry_or_recurse() {
        let (runtime, _listener) = test_authority();
        let runner =
            FakeRunner::with_responses(vec![output("loaded\n"), Err(CommandFailure::Timeout)]);
        let repair = coordinator(&runtime, runner.clone());
        let retry_calls = AtomicUsize::new(0);
        let error = connect_with_repair(
            &repair,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async {
                retry_calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, AccessibilityConnectFailure>(())
            },
        )
        .await
        .expect_err("restart timeout should fail");
        assert!(error.message.contains("systemctl exceeded"));
        assert_eq!(retry_calls.load(Ordering::SeqCst), 0);
        assert_eq!(runner.invocations().len(), 2);
    }

    #[tokio::test]
    async fn unloaded_unit_does_not_restart_or_retry() {
        let (runtime, _listener) = test_authority();
        let runner = FakeRunner::with_responses(vec![output("not-found\n")]);
        let repair = coordinator(&runtime, runner.clone());
        let error = connect_with_repair(
            &repair,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async { panic!("an unloaded unit must not retry the connection") },
        )
        .await
        .expect_err("an unloaded repair unit should fail closed");
        assert!(error.message.contains("is not loaded"));
        assert_eq!(runner.invocations().len(), 1);
    }

    #[tokio::test]
    async fn failed_retry_is_reported_without_recursive_repair() {
        let (runtime, _listener) = test_authority();
        let runner = FakeRunner::with_responses(vec![output("loaded\n"), output("")]);
        let repair = coordinator(&runtime, runner.clone());
        let retry_calls = AtomicUsize::new(0);
        let error = connect_with_repair(
            &repair,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async {
                retry_calls.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(AccessibilityConnectFailure::Timeout)
            },
        )
        .await
        .expect_err("one failed retry should surface");
        assert!(error.message.contains("single retry failed"));
        assert_eq!(retry_calls.load(Ordering::SeqCst), 1);
        assert_eq!(runner.invocations().len(), 2);
    }

    #[tokio::test]
    async fn cooldown_is_shared_across_coordinator_instances() {
        let (runtime, _listener) = test_authority();
        let runner =
            FakeRunner::with_responses(vec![output("loaded\n"), output(""), output("loaded\n")]);
        let first = coordinator(&runtime, runner.clone());
        let second = coordinator(&runtime, runner.clone());
        connect_with_repair(
            &first,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async { Ok::<_, AccessibilityConnectFailure>(()) },
        )
        .await
        .unwrap();
        let error = connect_with_repair(
            &second,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async { panic!("cooldown must not retry") },
        )
        .await
        .expect_err("cooldown should suppress restart");
        assert!(error.message.contains("cooldown active"));
        assert_eq!(runner.invocations().len(), 3);
    }

    #[tokio::test]
    async fn expired_cooldown_allows_one_later_attempt() {
        let (runtime, _listener) = test_authority();
        let runner = FakeRunner::with_responses(vec![
            output("loaded\n"),
            output(""),
            output("loaded\n"),
            output(""),
        ]);
        let first = coordinator(&runtime, runner.clone());
        connect_with_repair(
            &first,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async { Ok::<_, AccessibilityConnectFailure>(()) },
        )
        .await
        .unwrap();

        let later = RepairCoordinator::for_test(
            runtime,
            unsafe { libc::geteuid() },
            runner.clone(),
            Arc::new(FakeClock {
                now: 10_000 + REPAIR_COOLDOWN + 1,
            }),
        );
        connect_with_repair(
            &later,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async { Ok::<_, AccessibilityConnectFailure>(()) },
        )
        .await
        .unwrap();
        assert_eq!(runner.invocations().len(), 4);
    }

    #[tokio::test]
    async fn lock_busy_is_shared_across_coordinator_instances() {
        let (runtime, _listener) = test_authority();
        let runner = FakeRunner::with_responses(vec![output("loaded\n")]);
        let first = coordinator(&runtime, runner.clone());
        let second = coordinator(&runtime, runner.clone());
        let validated = first.authority.as_ref().unwrap().validate().unwrap();
        let _held = acquire_repair_lock(&validated).unwrap();
        let error = connect_with_repair(
            &second,
            || async { Err::<(), _>(AccessibilityConnectFailure::Timeout) },
            || async { panic!("busy lock must not retry") },
        )
        .await
        .expect_err("busy lock should suppress restart");
        assert!(error.message.contains("lock is busy"));
        assert_eq!(runner.invocations().len(), 1);
    }
}
