//! Bounded subprocess execution shared by the Linux backend and the overlay
//! host. The child's stdout and stderr are drained on reader threads while
//! the deadline is polled, so a child that writes more than a pipe buffer
//! (64 KiB on Linux) can still exit instead of blocking on `write(2)` until
//! the deadline kills it and misreports a timeout.

use std::io::Read;
use std::process::{Child, Command, Output};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Why a bounded run did not produce an [`Output`].
#[derive(Debug)]
pub enum RunError {
    /// `Command::spawn` failed (program missing, permissions, ...).
    Spawn(std::io::Error),
    /// The child was still running at the deadline; it has been killed and
    /// reaped.
    TimedOut { timeout: Duration },
    /// `try_wait` failed; the child has been killed and reaped.
    Wait(std::io::Error),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Spawn(error) => write!(f, "failed to spawn subprocess: {error}"),
            RunError::TimedOut { timeout } => write!(f, "subprocess timed out after {timeout:?}"),
            RunError::Wait(error) => write!(f, "failed to wait for subprocess: {error}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Polling interval between `try_wait` checks.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Run `command` to completion or until `timeout` elapses, draining stdout and
/// stderr concurrently. The caller decides which streams are piped; streams
/// left as `Stdio::null()`/`inherit()` are simply absent from the result.
pub fn run_with_timeout(command: &mut Command, timeout: Duration) -> Result<Output, RunError> {
    let mut child = command.spawn().map_err(RunError::Spawn)?;
    let stdout = child.stdout.take().map(spawn_reader);
    let stderr = child.stderr.take().map(spawn_reader);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                kill_and_reap(&mut child);
                return Err(RunError::TimedOut { timeout });
            }
            Err(error) => {
                kill_and_reap(&mut child);
                return Err(RunError::Wait(error));
            }
        }
    };
    Ok(Output {
        status,
        stdout: join_reader(stdout),
        stderr: join_reader(stderr),
    })
}

fn spawn_reader<R: Read + Send + 'static>(mut reader: R) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = reader.read_to_end(&mut buffer);
        buffer
    })
}

fn join_reader(handle: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

/// Kill and reap the child. Reader threads are intentionally not joined on
/// this path: a grandchild may still hold the pipe's write end, and joining
/// would block past the deadline. Dropping the handles detaches the threads;
/// they exit at EOF when the last writer goes away.
fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[test]
    fn drains_output_larger_than_a_pipe_buffer_before_the_deadline() {
        // 1 MiB on stdout and 128 KiB on stderr: both exceed the 64 KiB pipe
        // buffer, so without concurrent draining the child would block and
        // the run would time out.
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "head -c 1048576 /dev/zero; head -c 131072 /dev/zero 1>&2",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = run_with_timeout(&mut command, Duration::from_secs(10)).expect("completes");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 1_048_576);
        assert_eq!(output.stderr.len(), 131_072);
    }

    #[test]
    fn timeout_does_not_wait_for_a_pipe_holding_grandchild() {
        // The background `sleep 10 &` keeps stdout open after `sh` is killed;
        // the timeout path must return without waiting for it.
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 10 & echo started; sleep 10"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let started = Instant::now();
        let error = run_with_timeout(&mut command, Duration::from_millis(200)).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timeout must not wait for pipe holders"
        );
        assert!(matches!(error, RunError::TimedOut { .. }), "got {error:?}");
    }

    #[test]
    fn missing_program_is_a_spawn_error() {
        let mut command = Command::new("/nonexistent/sky-cua-test-binary");
        assert!(matches!(
            run_with_timeout(&mut command, Duration::from_secs(1)),
            Err(RunError::Spawn(_))
        ));
    }
}
