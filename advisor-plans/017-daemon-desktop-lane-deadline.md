# Plan 017: Stop the daemon wedge — bound the shared desktop lane so one hung request can't freeze the whole daemon

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving on. If a
> STOP condition occurs, stop and report. When done, update the status row in
> `advisor-plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 06eda65..HEAD -- crates/sky-cua-service/src/daemon.rs crates/sky-cua-service/src/ipc_server.rs crates/sky-cua-linux/src/portal/pipewire.rs crates/sky-cua-linux/src/backend.rs`
> If any in-scope file changed since this plan was written, re-verify the
> "Current state" excerpts before proceeding.

## Status

- **Priority**: P1 (real reliability defect — total daemon unresponsiveness)
- **Status**: DONE — merged 2026-07-08 at `032c027`, live-proven
- **Effort**: M
- **Risk**: MED (touches the daemon's core request lane; a wrong deadline or a
  cache-reset-on-timeout mistake could abort healthy slow requests or leave
  stale state)
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `06eda65`, 2026-07-08

## Why this matters

On a live KDE Plasma 6 Wayland host, after a long session of repeated
`observe`/`capture` calls, the `sky-cua-service` daemon **wedges**: `observe`,
`list_resources`, AND `doctor` all time out together, permanently, until the
daemon is restarted. A freshly restarted daemon is instantly healthy. This is
a real reliability defect — one unlucky hung operation takes down the entire
desktop tool surface for the rest of the process lifetime.

**Root cause (confirmed by code analysis + live evidence):** every desktop
request runs under one shared `desktop_lane` mutex held across the *entire*
handler, and the daemon runs each request with **no server-side timeout and
no cancellation when the client disconnects**. So when any single request
hangs on a blocking operation, it holds the lane forever and every subsequent
desktop request queues behind it and times out. The "observe + doctor +
list_resources all wedge together" symptom is the signature of a shared lock,
not shared portal/AT-SPI state (doctor does neither).

The terminal blocker that hangs a request is one of two unbounded awaits on
the observe path:
- an **AT-SPI zbus call with no method timeout** (zbus 5.14 default
  `method_timeout = None`) — a GUI app whose a11y thread freezes (modal grab,
  hung Electron renderer, an app under `SIGSTOP`, sync IO stall) makes the
  registry walk pend forever; or
- a **GStreamer/PipeWire teardown deadlock** — `pipeline.set_state(Null)`
  joining stalled `pipewiresrc` streaming threads, awaited with no timeout.

Both are fixed by the same primary change: a server-side deadline that frees
the lane no matter what the underlying operation does.

**Ruled out (live evidence):** a 40-call rapid `observe` loop on one daemon
showed **flat fds (21) and flat threads (34)** with steady ~0.5–1.0s response
times — no fd/session/thread leak, no gradual slowdown. The wedge needs a
*hung* request, not accumulation. (Refuted hypotheses recorded in the
investigation: portal/PipeWire session leak, AT-SPI proxy accumulation,
blocking-pool exhaustion, snapshot-map growth.)

## Current state

- `crates/sky-cua-service/src/daemon.rs:202-205` — the catch-all desktop lane:
  ```rust
  request => {
      let _desktop_lane = self.desktop_lane.lock().await;
      self.handle_desktop_request(request).await
  }
  ```
  `Doctor`, `ListWindows`/`ListDisplays` (what `list_resources` maps to),
  `GetAppState` (== `observe`), and `Screenshot` all fall into this branch and
  share `desktop_lane`. The guard is held across the whole `.await`.
- `crates/sky-cua-service/src/ipc_server.rs:399` — `daemon.handle(request).await`
  has no `tokio::time::timeout` wrapper and no disconnect-triggered
  cancellation. When the client gives up, the server-side future keeps running
  and keeps holding the lane. `tokio::sync::Mutex` is FIFO, so later requests
  queue in order behind the stuck head.
- `crates/sky-cua-linux/src/portal/pipewire.rs:48-57` — the capture
  `spawn_blocking` JoinHandle is `.await`ed with no timeout; inside,
  `pipeline.set_state(gst::State::Null)` (~line 183) can deadlock on
  `pipewiresrc` teardown.
- AT-SPI connection built with no method timeout
  (`crates/sky-cua-linux/src/backend.rs:198-207`,
  `crates/sky-cua-linux/src/atspi/mod.rs:54`); the walk makes hundreds of
  unbounded calls (`crates/sky-cua-linux/src/apps/discovery.rs:20-100`,
  `crates/sky-cua-linux/src/atspi/tree.rs:13-159`).
- The overlay/snapshot locks (`daemon.rs:874/881/907`) are NOT held across the
  backend await — they are innocent; do not touch them.

Confirm each excerpt against live code before editing; line numbers may drift.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Build | `cargo build -p sky-cua-service -p sky-cua-linux` | exit 0 |
| Format | `cargo fmt --check` | exit 0 |
| Clippy (gate is -D warnings) | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo nextest run` | all pass |

## Scope

**In scope:**
- `crates/sky-cua-service/src/daemon.rs` — the desktop-lane deadline (primary fix)
- `crates/sky-cua-service/src/ipc_server.rs` — optional: outer request deadline / disconnect handling (see Step 4)
- `crates/sky-cua-linux/src/portal/pipewire.rs` — bound the capture join
- `crates/sky-cua-linux/src/backend.rs` — optional per-call AT-SPI walk timeout + a `reset_accessibility_connection`-style hook if one does not already exist
- The structured error contract for a new `DeadlineExceeded`-style diagnostic
  (check `crates/sky-cua-platform/src/diagnostics.rs` / model for the existing
  `BackendErrorCode` set; reuse or add one additively)
- Tests

**Out of scope:**
- The overlay/snapshot locks (innocent).
- Rewriting the AT-SPI walk or the portal capture pipeline.
- Capture PNG disk accumulation (real but separate — note it, don't fix here).
- The browser/phone request lanes (different handlers; this is the desktop lane).

## Steps

### Step 1: Add a server-side deadline on the desktop lane (primary fix)

In `daemon.rs`, wrap the desktop handler in `tokio::time::timeout` while
holding the lane, so a hung handler is cancelled (its future dropped) and the
lane is released:

```rust
const DESKTOP_REQUEST_DEADLINE: Duration = Duration::from_secs(50);

request => {
    let _desktop_lane = self.desktop_lane.lock().await;
    match tokio::time::timeout(
        DESKTOP_REQUEST_DEADLINE,
        self.handle_desktop_request(request),
    ).await {
        Ok(response) => response,
        Err(_elapsed) => {
            // The handler was cancelled (future dropped) so the lane frees
            // here. Reset the portal + AT-SPI session state so the next
            // request does not immediately re-hang on the same wedged peer.
            self.reset_desktop_backend_state().await;
            ServiceResponse::Error {
                code: "desktop_request_deadline_exceeded".to_string(),
                message: format!(
                    "desktop request exceeded {}s and was cancelled to keep the \
                     daemon responsive; backend session state was reset",
                    DESKTOP_REQUEST_DEADLINE.as_secs()
                ),
            }
        }
    }
}
```

- Pick the exact `ServiceResponse` error shape from the real enum
  (`crates/sky-cua-platform/src/model/service.rs` — reuse the existing
  `Error { code, message }` variant if present; do NOT change the wire
  contract).
- `reset_desktop_backend_state()`: implement (or reuse) a backend method that
  drops/closes the portal RemoteDesktop/ScreenCast session and resets the
  cached AT-SPI connection, so the next request re-establishes clean state.
  There is prior art: `reset_persisted_tokens` /
  `ensure_session_started` in `crates/sky-cua-linux/src/portal/remote_desktop.rs`
  and the AT-SPI reset-on-retryable-error path in `backend.rs`. Wire a
  lightweight "reset sessions, keep tokens" variant. If no such method exists
  and adding one is more than ~40 lines, STOP and report — the reset scope is
  a judgement call worth a checkpoint.

**Verify**: `cargo build -p sky-cua-service` → exit 0; `cargo nextest run -p sky-cua-service` → all pass.

### Step 2: Bound the capture spawn_blocking join

In `crates/sky-cua-linux/src/portal/pipewire.rs:48-57`, wrap the
`spawn_blocking(...).await` in `tokio::time::timeout(Duration::from_secs(15), ...)`.
Map a timeout to the existing `PipeWireStreamFailed` error (which already
triggers the session-rebuild path in
`crates/sky-cua-linux/src/portal/remote_desktop.rs:130`). A gst teardown
deadlock then strands one bounded, observable blocking thread instead of the
whole daemon.

**Verify**: `cargo build -p sky-cua-linux` → exit 0.

### Step 3: Per-call timeout on the AT-SPI walk (defense in depth)

Wrap the AT-SPI discovery/snapshot entry points
(`backend.rs` `discover_accessible_apps` ~:959 and `snapshot_for_app` ~:1174)
in a per-call `tokio::time::timeout` (~10s). A frozen a11y peer then fails
that one walk in 10s instead of burning the full 50s desktop deadline every
time. On timeout, surface a structured diagnostic and reset the AT-SPI
connection so a dead peer does not poison every later observe.

**Verify**: `cargo nextest run -p sky-cua-linux` → all pass.

### Step 4 (optional, judgement call): outer IPC deadline / disconnect cancel

Consider a belt-and-suspenders outer `tokio::time::timeout` at
`ipc_server.rs:399` (slightly larger than the desktop deadline, e.g. 60s) so
any handler — not just the desktop lane — cannot run unbounded, and/or drop
the request future when the client connection closes. Only do this if it does
not complicate the browser/phone lanes; if it does, note it as deferred.

### Step 5: Regression test

Add a test that proves the daemon stays responsive when a desktop request
would otherwise hang. Two options — pick what fits the test harness:

- **Preferred (unit-level):** with the daemon's fakeable backend, inject a
  desktop handler that sleeps longer than the deadline; assert the request
  returns the `desktop_request_deadline_exceeded` error within ~deadline and
  that a *subsequent* `Doctor` on the same daemon returns promptly (the lane
  freed). This is the core invariant and needs no live desktop.
- **Optional (gated live smoke):** a Python smoke that launches an AT-SPI app
  (zenity), `SIGSTOP`s it, calls `observe`, and asserts it fails within the
  deadline while a fresh `doctor` still answers, then `SIGCONT`s and asserts
  recovery. Gate it behind an env flag (needs a real a11y bus); model it after
  the existing `scripts/live_*_smoke.py`. NOTE: the advisor confirmed this
  scenario is the exact field-repro (a frozen AT-SPI app wedges the walk); the
  sandbox blocked running it live during investigation, so treat first
  execution as unverified.

**Verify**: `cargo nextest run` → all pass, including the new deadline test.

## Done criteria

- [ ] Desktop handler runs under a `tokio::time::timeout`; on elapse it returns a structured error AND resets backend session state (lane provably freed)
- [ ] Capture `spawn_blocking` join is bounded → `PipeWireStreamFailed` on timeout
- [ ] AT-SPI walk entry points have a per-call timeout
- [ ] Regression test: a would-hang desktop request returns the deadline error and a subsequent doctor on the same daemon responds promptly
- [ ] `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run` all green
- [ ] No wire-contract change (reused existing `ServiceResponse::Error`); no out-of-scope files modified

## STOP conditions

- No reusable "reset portal + AT-SPI session" backend method exists and adding
  one exceeds ~40 lines / touches token persistence semantics — checkpoint the
  reset scope with the maintainer.
- `handle_desktop_request`'s future is not cancel-safe (dropping it mid-await
  could leave a half-applied action / corrupt shared state) — this is the key
  risk of Step 1. Audit what `handle_desktop_request` mutates before the first
  await point and across awaits; if a cancel could corrupt state (e.g. a
  partially-registered snapshot, a half-sent input action), report before
  shipping — the deadline may need to guard only the read-only
  observe/doctor/list paths, not mutating `ExecuteAction`.
- The clippy gate surfaces a pre-existing failure unrelated to this change —
  report, don't fix out of scope.

## Maintenance notes

- The primary fix is the deadline; the AT-SPI and capture timeouts are
  defense-in-depth that stop the deadline from being hit on every call against
  a wedged peer.
- Tune `DESKTOP_REQUEST_DEADLINE` below the MCP host's tool-call timeout (the
  observed client gives up at ~15s at the MCP layer; the Rust client read
  timeout is 60s at `service_launcher.rs:35`) so the daemon returns an honest
  structured error before the caller silently times out. 50s server / 60s
  client is a reasonable split; revisit if honest-error latency matters more.
- Separate follow-up (not this plan): capture PNG/JPEG files accumulate under
  the per-user captures dir with no eviction (`SnapshotManager` evicts its
  in-memory records but not the on-disk files) — a slow disk/inode growth,
  worth its own cleanup.
- The reviewer should scrutinize cancel-safety (the STOP condition above) most
  of all: a deadline that can corrupt state on a mutating request is worse
  than the wedge.
