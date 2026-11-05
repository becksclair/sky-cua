# Plan 012: Replace the three copies of the timed-subprocess helper with one that drains child output, so large output cannot stall into a false timeout

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-platform/src/lib.rs crates/sky-cua-platform/Cargo.toml crates/sky-cua-linux/src/displays.rs crates/sky-cua-linux/src/virtual_input.rs crates/sky-cua-overlay-host/src/system_cursor.rs`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts are verbatim unless marked abridged.

## Status

- **Priority**: P2
- **Effort**: S-M
- **Risk**: LOW
- **Depends on**: 001 (so `sky-cua-linux` tests run in the gate; not a code dependency)
- **Category**: bug
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

Three near-identical helpers run a child process with a deadline:
`command_output_with_timeout` in `crates/sky-cua-linux/src/displays.rs`,
another `command_output_with_timeout` in `crates/sky-cua-linux/src/virtual_input.rs`,
and `spawn_with_timeout` in `crates/sky-cua-overlay-host/src/system_cursor.rs`.
A fourth copy of the same loop is inlined in `run_ydotool`. All four spawn
the child with `stdout`/`stderr` piped, then poll `try_wait()` every 10 ms
and only call `wait_with_output()` after the child has exited. Nothing reads
the pipes while the child runs. A pipe buffer is 64 KiB on Linux; a child
that writes more than that blocks on `write(2)` and never exits, so after the
deadline it is killed and reported as "timed out". `kscreen-doctor -j`
(display discovery) is about 8 KB with one monitor and 22 modes (roughly 300
bytes per mode, measured 2026-11-03), so it crosses 64 KiB only around 200
modes in total; `xrandr`, `hyprctl monitors -j`, `setxkbmap -query` and
`localectl status` are far smaller. The failure is therefore latent rather
than likely on today's desktops (it gets closer when a pipe-heavy session
exhausts `pipe-user-pages-soft`, which shrinks new pipes). When it fires on
KDE, the `-j` probe stalls for `COMMAND_TIMEOUT` and falls back to the `-o`
text path (`displays.rs:274-288`); on Hyprland, COSMIC and xrandr, which have
no fallback, discovery returns no displays. The cheap, correct fix is one
shared helper that reads the pipes while it waits.

The helper drains both pipes on reader threads while polling, kills and
reaps on timeout, and returns the collected output on success. On the timeout
path it must NOT join the reader threads: a grandchild of the killed process
(for example a `sleep` forked by `sh -c`) can still hold the pipe's write
end, and joining would block past the deadline, which is the one promise the
old helpers did keep. It belongs in `sky-cua-platform` (std-only, both
consuming crates already depend on it), replacing all four copies.

## Current state

- `crates/sky-cua-linux/src/displays.rs`
  - Lines 30-33:

    ```rust
    struct CommandProbeOutput {
        output: Option<Output>,
        timed_out: bool,
    }
    ```

  - Lines 428-455 (verbatim):

    ```rust
    fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> CommandProbeOutput {
        let Ok(mut child) = command.spawn() else {
            return CommandProbeOutput {
                output: None,
                timed_out: false,
            };
        };
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    return CommandProbeOutput {
                        output: child.wait_with_output().ok(),
                        timed_out: false,
                    };
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return CommandProbeOutput {
                        output: None,
                        timed_out: true,
                    };
                }
            }
        }
    }
    ```

  - Callers at lines 257 (`kscreen-doctor -j`), 275 (`kscreen-doctor -o`),
    333 (`hyprctl monitors -j`), 351 (`cosmic-randr list`), 369 (`xrandr`); each builds a `Command`
    with `.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())`
    and passes `COMMAND_TIMEOUT`.

- `crates/sky-cua-linux/src/virtual_input.rs`
  - Lines 1498-1512 (verbatim):

    ```rust
    fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> Option<Output> {
        let mut child = command.spawn().ok()?;
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => return child.wait_with_output().ok(),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
    }
    ```

    Callers at 1125 (`setxkbmap -query`), 1141 (`localectl status`), 1615
    (`cosmic-randr list`); stdout piped, stderr `Stdio::null()`.

  - `run_ydotool_command` (lines 1373-1445; `run_ydotool` at 560-565 is a
    thin wrapper that calls it) spawns ydotool with both pipes (1393-1398) and
    runs the same `try_wait` loop inline (lines 1400-1422), mapping timeout
    to `BackendError::new(ActionUnsupportedForEnvironment, "ydotool command timed out: ydotool {args}")`
    and wait errors to `"failed to wait for ydotool: {error}"`, then
    `wait_with_output()` (1424-1429) and a non-zero-status error that includes
    trimmed stderr. Imports: `displays.rs` uses `thread` (line 3) and
    `Instant` (line 4) only inside the helper being replaced;
    `virtual_input.rs` uses `Instant` (line 9) only at 1400/1404/1500/1504
    (all removed) but `thread` elsewhere (line 324); `system_cursor.rs` keeps
    using `Instant` and `thread` (lines 215, 278, 285, 588).

- `crates/sky-cua-overlay-host/src/system_cursor.rs` lines 340-362 (verbatim):

  ```rust
  /// Spawn `command`, waiting up to `timeout` for it to finish. On timeout the
  /// child is killed and reaped before returning an error, so a wedged helper
  /// never leaks a zombie process or hangs the caller.
  fn spawn_with_timeout(command: &mut Command, timeout: Duration) -> Result<std::process::Output> {
      let mut child = command.spawn().context("failed to spawn subprocess")?;
      let deadline = Instant::now() + timeout;
      loop {
          match child.try_wait() {
              Ok(Some(_status)) => return child.wait_with_output().context("collect output"),
              Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
              Ok(None) => {
                  let _ = child.kill();
                  let _ = child.wait();
                  bail!("subprocess timed out after {timeout:?}");
              }
              Err(error) => {
                  let _ = child.kill();
                  let _ = child.wait();
                  return Err(error).context("failed to wait for subprocess");
              }
          }
      }
  }
  ```

  Callers at 326 (`qdbus ... ` with `QDBUS_TIMEOUT`) and 795 (`hyprctl` with
  `HYPRCTL_TIMEOUT`); both pipe stdout and stderr. This crate uses `anyhow`.

- `crates/sky-cua-platform`: `Cargo.toml` deps are `async-trait, chrono, libc, serde, serde_json, toml, thiserror, uuid` (no dev-deps). `src/lib.rs`
  lines 1-10 declare `pub mod app_instructions; backend; config; diagnostics; fd_hygiene; log_rotation; model; overlay_spec_generated; paths; snapshot;`.
  `fd_hygiene.rs` is the exemplar of a std-only, process-level utility module
  in this crate. `crates/sky-cua-platform/AGENTS.md` says the crate "stays
  small, serializable, and runtime-agnostic: never pull in Linux crates";
  `std::process` and `std::thread` are allowed (the crate already uses
  `libc`). Both `sky-cua-linux` and `sky-cua-overlay-host` depend on
  `sky-cua-platform` (`sky-cua-platform = { path = "../sky-cua-platform" }`
  in their manifests).

- `crates/sky-cua-service/src/overlay/host/lifecycle.rs` also calls
  `try_wait()` but for a long-lived child it does not collect output from;
  `crates/sky-cua-linux/src/actions/mod.rs:1941` calls it in the wl-copy
  paste path. Neither is a copy of this pattern; both are out of scope.

Conventions: `cargo nextest run`; conventional commits, e.g.
`fix(linux): make X11 display-liveness tests hermetic`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Platform tests | `cargo nextest run -p sky-cua-platform process::` | 3 new tests pass (the bare word `process` also matches an unrelated `model::tests` test) |
| Linux tests | `cargo nextest run -p sky-cua-linux displays virtual_input` | all pass |
| Overlay-host tests | `cargo nextest run -p sky-cua-overlay-host` | all pass (105 at `e928a40`) |
| Whole workspace | `cargo nextest run --workspace` | all pass (1,927 at `e928a40`, plus yours) |

## Scope

**In scope** (the only files you should modify):
- `crates/sky-cua-platform/src/process.rs` (create)
- `crates/sky-cua-platform/src/lib.rs` (one `pub mod process;` line)
- `crates/sky-cua-linux/src/displays.rs`
- `crates/sky-cua-linux/src/virtual_input.rs`
- `crates/sky-cua-overlay-host/src/system_cursor.rs`

**Out of scope** (do NOT touch):
- `crates/sky-cua-service/src/overlay/host/lifecycle.rs` (long-lived child
  supervision, different problem).
- The per-site timeouts (`COMMAND_TIMEOUT`, `QDBUS_TIMEOUT`, `HYPRCTL_TIMEOUT`)
  and the error codes/messages callers produce. Keep messages byte-identical
  where tests assert on them.
- Any `Command` construction at call sites (which program, which args,
  which `Stdio`).
- Adding dependencies to `sky-cua-platform`.

## Git workflow

- Branch: `bex/timed-subprocess-drain`
- Commit only if the operator authorised it; style:
  `fix(platform): drain child output in the shared timed-subprocess helper`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add the shared helper to `sky-cua-platform`

Create `crates/sky-cua-platform/src/process.rs`:

```rust
//! Bounded subprocess execution shared by the Linux backend and the overlay
//! host. The child's stdout and stderr are drained on reader threads while
//! the deadline is polled, so a child that writes more than a pipe buffer
//! (64 KiB on Linux) can still exit instead of blocking on `write(2)` until
//! the deadline kills it and misreports a timeout.

use std::io::Read;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Output};
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
```

Use `use std::process::{Child, Command, Output};` (no `ChildStdout`,
`ChildStderr`, `ExitStatus` imports are needed; `.map(spawn_reader)` infers
the reader type). This shape was compiled standalone with
`rustc --edition 2024 --test -D warnings` and clippy on 2026-11-03.

Add `pub mod process;` to `crates/sky-cua-platform/src/lib.rs`
(alphabetical position: after `paths`).

Add tests at the bottom of `process.rs`, Unix-only (they spawn `sh`):

```rust
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
```

The grandchild test is the important one: with a shell that forks the last
command (dash does; bash execs it), a naive implementation that joins the
readers on timeout would block for the full 10 s and fail the 5 s assertion.
If the large-output test is slow on a loaded CI runner, raise its timeout to
30 s; it completes in well under a second normally.

**Verify**: `cargo nextest run -p sky-cua-platform process::` → 3 pass;
`cargo clippy -p sky-cua-platform --all-targets -- -D warnings` → exit 0.

### Step 2: Switch `displays.rs` to the shared helper

Replace the body of `command_output_with_timeout` (lines 428-455) with a thin
adapter that preserves `CommandProbeOutput` semantics exactly:

```rust
fn command_output_with_timeout(command: &mut Command, timeout: Duration) -> CommandProbeOutput {
    match sky_cua_platform::process::run_with_timeout(command, timeout) {
        Ok(output) => CommandProbeOutput {
            output: Some(output),
            timed_out: false,
        },
        Err(sky_cua_platform::process::RunError::TimedOut { .. }) => CommandProbeOutput {
            output: None,
            timed_out: true,
        },
        // Spawn failure was `timed_out: false, output: None`; a wait failure
        // was `timed_out: true` in the old code. Keep both.
        Err(sky_cua_platform::process::RunError::Spawn(_)) => CommandProbeOutput {
            output: None,
            timed_out: false,
        },
        Err(sky_cua_platform::process::RunError::Wait(_)) => CommandProbeOutput {
            output: None,
            timed_out: true,
        },
    }
}
```

Delete `use std::thread;` (line 3) and change line 4 to
`use std::time::Duration;`: `Instant` and `thread` have no other use in
`displays.rs`, and `-D warnings` would fail on them. `Output` is still used by
`CommandProbeOutput`.

**Verify**: `cargo clippy -p sky-cua-linux --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-linux displays` → all pass.

### Step 3: Switch `virtual_input.rs` (both the helper and `run_ydotool`)

1. Replace `command_output_with_timeout` (lines 1498-1511) body with
   `sky_cua_platform::process::run_with_timeout(command, timeout).ok()`
   (old semantics: `None` on spawn failure, timeout, or wait error).

2. In `run_ydotool_command` (NOT the five-line `run_ydotool` wrapper at
   560-565), replace lines 1393-1429 (from `let mut child = command.spawn()`
   through the `wait_with_output()` ... `map_err(...)?;`) with:

   ```rust
   let output = match sky_cua_platform::process::run_with_timeout(&mut command, COMMAND_TIMEOUT) {
       Ok(output) => output,
       Err(sky_cua_platform::process::RunError::Spawn(error)) => {
           return Err(BackendError::new(
               BackendErrorCode::ActionUnsupportedForEnvironment,
               format!("failed to start ydotool: {error}"),
           ));
       }
       Err(sky_cua_platform::process::RunError::TimedOut { .. }) => {
           return Err(BackendError::new(
               BackendErrorCode::ActionUnsupportedForEnvironment,
               format!("ydotool command timed out: ydotool {}", args.join(" ")),
           ));
       }
       Err(sky_cua_platform::process::RunError::Wait(error)) => {
           return Err(BackendError::new(
               BackendErrorCode::ActionUnsupportedForEnvironment,
               format!("failed to wait for ydotool: {error}"),
           ));
       }
   };
   ```

   Keep the three error message strings exactly as they are today
   (`"failed to start ydotool: "`, `"ydotool command timed out: ydotool "`,
   `"failed to wait for ydotool: "`); plan 009's regression test and
   existing tests match on the first one. The `"failed to collect ydotool output"`
   message disappears (that path no longer exists; nothing in `crates/` or
   `scripts/` matches on it). Leave the non-zero-status handling after the
   match unchanged. Then change line 9 to `use std::time::Duration;` (keep
   `use std::thread;`, still used at line 324).

**Verify**: `cargo clippy -p sky-cua-linux --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-linux virtual_input` → all pass;
`grep -n 'try_wait' crates/sky-cua-linux/src/displays.rs crates/sky-cua-linux/src/virtual_input.rs` → no matches.

### Step 4: Switch `system_cursor.rs`

Replace the body of `spawn_with_timeout` (lines 343-362) with:

```rust
fn spawn_with_timeout(command: &mut Command, timeout: Duration) -> Result<std::process::Output> {
    use sky_cua_platform::process::RunError;
    match sky_cua_platform::process::run_with_timeout(command, timeout) {
        Ok(output) => Ok(output),
        Err(RunError::Spawn(error)) => Err(error).context("failed to spawn subprocess"),
        Err(RunError::TimedOut { .. }) => bail!("subprocess timed out after {timeout:?}"),
        Err(RunError::Wait(error)) => Err(error).context("failed to wait for subprocess"),
    }
}
```

Keep the doc comment (it is now true again: a wedged helper never hangs the
caller). No import changes: `Instant` and `thread` are still used at lines
215, 278, 285 and 588.

**Verify**: `cargo clippy -p sky-cua-overlay-host --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-overlay-host` → all pass (105);
`grep -n 'try_wait' crates/sky-cua-overlay-host/src/system_cursor.rs` → no matches.

### Step 5: Whole-workspace gate

Run `cargo fmt` (then `cargo fmt --check`), then
`cargo clippy --workspace --all-targets -- -D warnings`, then
`cargo nextest run --workspace`.

**Verify**: all exit 0; the workspace test count is 1,930 or more (1,927 at
`e928a40` plus 3).

## Test plan

- New: three tests in `crates/sky-cua-platform/src/process.rs` (large output
  drained before the deadline, timeout returns promptly despite a
  pipe-holding grandchild, spawn error).
- Existing `displays`, `virtual_input` and overlay-host tests unchanged and
  passing; they are the regression net for the adapters.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run --workspace` exits 0 with 3 new tests
- [ ] `grep -n 'try_wait' crates/sky-cua-linux/src/displays.rs crates/sky-cua-linux/src/virtual_input.rs crates/sky-cua-overlay-host/src/system_cursor.rs` prints nothing (`actions/mod.rs:1941` keeps its wl-copy `try_wait`; out of scope)
- [ ] `grep -n 'pub mod process;' crates/sky-cua-platform/src/lib.rs` matches
- [ ] `grep -c 'run_with_timeout' crates/sky-cua-linux/src/displays.rs crates/sky-cua-linux/src/virtual_input.rs crates/sky-cua-overlay-host/src/system_cursor.rs` shows ≥1 for each file (2 for `virtual_input.rs`)
- [ ] `git status --porcelain -- crates/` lists exactly the five in-scope files (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- Any existing test asserts on the removed `"failed to collect ydotool output"`
  message (grep the workspace for it; none at `e928a40`).
- The platform tests fail on the development host (for example `sh` or
  `head` missing, which would be very surprising); report rather than mark
  them `#[ignore]`.
- A caller depends on `wait_with_output` having been called (none do; the
  helper returns the same `Output` type).

## Maintenance notes

- `run_with_timeout` spawns up to two reader threads per call. The call
  sites are infrequent probes (display discovery, keymap probing, ydotool
  actions, qdbus/hyprctl cursor control), so thread cost is negligible; do
  not use it in a hot loop without measuring.
- On the timeout path the reader threads are detached, not joined, so a
  timed-out child's partial output is discarded by design. If a caller later
  wants it, add a bounded join (for example `recv_timeout` on a channel the
  reader writes to), never an unbounded `join()`.
- If plan 009 lands first, the drift check flags `virtual_input.rs`; drift in
  `press_key_sequence` is expected from 009, and only the two helper regions
  this plan edits must match the excerpts.
- `PERF-09` in the audit notes these probes run synchronously on tokio
  workers; this plan does not change that, it only removes the false-timeout
  failure mode. A reviewer should check that each adapter maps the three
  error variants to the same observable behaviour the old code had,
  especially `displays.rs`'s `timed_out` flag, which feeds doctor
  diagnostics.
