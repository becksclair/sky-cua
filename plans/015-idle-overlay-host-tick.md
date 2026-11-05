# Plan 015: Stop the overlay host from ticking at the display refresh rate while the cursor is hidden

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-overlay-host/src/layer_shell.rs crates/sky-cua-overlay-host/src/lib.rs crates/sky-cua-overlay-host/src/main.rs`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts are verbatim unless marked abridged.

## Status

- **Priority**: P2
- **Effort**: S-M
- **Risk**: LOW-MED
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

`sky-cua-overlay-host` is a long-lived child of the daemon. All three of its
serve loops (Unix socket via calloop, TCP accept loop, JSON-lines over
stdin) wake on a timer whose period is `pointer_tick_interval()`: the
fastest connected display's refresh period, clamped to 60–240 Hz. That is
right while the agent cursor is visible or animating, because `tick()`
follows the real pointer and advances motion. But `tick()` does nothing
useful while the overlay is hidden: `follow_tracked_pointer` returns
immediately when the state is not visible, and `render_current` runs only
when `should_animate()`. The result is 60 to 240 wakeups per second, for
the life of the host, on a desktop where the agent cursor is hidden most of
the time. The `Noop` backend (used when no layer-shell compositor is
available) ticks every 16 ms and never renders anything. The fix has three
parts: a pure decision that returns a slow idle period when there is nothing
to follow or animate; re-arming the Unix-socket loop's calloop timer when a
request makes the overlay visible (that timer re-arms only from its own
callback, so without this a `Show` after a capture would leave the cursor
frozen for up to the idle period: `Mover2D::step` clamps dt to
`CURSOR_MAX_STEP_S`, so missed time is lost, and
`restore_after_capture` sends `Show` after every capture `Hide`); and
capping the TCP accept loop's poll sleep, because that loop handles one
request per connection and its sleep bounds per-request latency on Windows.
Message handling on the JSON-lines loop is not delayed (stdin wakes it), and
`SetCursor`/`Show` render their first frame synchronously in the handler.

## Current state

`crates/sky-cua-overlay-host/src/layer_shell.rs`:

- Lines 373-378 (verbatim):

  ```rust
  pub fn tick(&mut self) {
      let _ = self.follow_tracked_pointer();
      if self.app.should_animate(current_epoch_ms()) {
          let _ = self.render_current();
      }
  }
  ```

- Lines 380-398 (verbatim):

  ```rust
  /// Tick cadence matched to the fastest connected display's current mode, so
  /// the agent-cursor follow updates at the panel's refresh rate (e.g. 4.2 ms
  /// on a 240 Hz screen) instead of a fixed 60 Hz. Clamped to [60, 240] Hz and
  /// defaulting to 60 Hz when no refresh rate is advertised.
  pub fn pointer_tick_interval(&self) -> std::time::Duration {
      // Reads the geometry snapshot, not OutputState: this re-arms on every
      // timer fire, and `OutputState::info` clones the full mode list.
      let max_mhz = self
          .app
          .output_geometry
          .iter()
          .flatten()
          .filter_map(|geometry| geometry.refresh_mhz)
          .max();
      let hz = max_mhz
          .map(|mhz| (f64::from(mhz) / 1000.0).clamp(60.0, 240.0))
          .unwrap_or(60.0);
      std::time::Duration::from_secs_f64(1.0 / hz)
  }
  ```

- `follow_tracked_pointer` (lines 469-480, abridged): first line is
  `if !self.app.state.as_ref().is_some_and(|state| state.visible) { return Ok(()); }`.
- `LayerShellApp::should_animate` (lines 677-686, verbatim):

  ```rust
  fn should_animate(&self, _now_ms: u64) -> bool {
      // The driver keeps `animating` true while the mover is unsettled, a
      // gesture is pending arrival or playing feedback, or the cloud is
      // mid-bloom; pending gestures have no epoch expiry, so no duration
      // reaper may run here.
      self.last_motion
          .as_ref()
          .is_some_and(|frame| frame.animating)
          || self.glow_active()
  }
  ```

  and `glow_active` (693-697) is `state.visible && self.visible_overlay_supported()`.
  So `should_animate()` is false whenever the state is hidden and the motion
  driver has settled; `app.motion.hide(..)` on a `Hide` message (line ~224)
  freezes or drops the gesture pipeline.
- `handle_message` (lines 163-260, abridged): `SetCursor` sets
  `self.app.state = message.state` and calls `self.render_reply()`
  synchronously (lines 212-215); `Hide` (216-250) sets `state.visible = false`,
  calls `self.app.motion.hide(..)` (223) and then `render_reply`; `Shutdown`
  hides. The `Hide` reply's `render_reply` → `render_current` → `draw` steps
  the motion driver with `visible = false` and stores the result in
  `self.last_motion` (`layer_shell.rs:592`); `frame(false)` reports
  `animating == false` (`cursor_motion.rs:485`). So `should_animate()` is
  false synchronously inside the `Hide` handler, and the idle gate engages
  on the very next timer fire. The one exception: if `render_current` bails
  before `draw` because no layer is open (`layer_shell.rs:449-451`),
  `last_motion` goes stale and the host keeps refresh-rate ticking, which is
  today's behaviour (no regression).
- `wait_for_arrival` (starts line 258; loop 285-303) loops `self.tick()`
  and sleeps `self.pointer_tick_interval().min(remaining)` while a gesture is
  pending; it returns `Unavailable` at once when hidden, and otherwise the
  overlay is visible/animating, so it is unaffected by an idle gate. The
  pointer tracker is an evented `mpsc` channel that is not drained while
  hidden regardless of interval; the capture barrier waits synchronously in
  the `Hide` handler; the KWin system-cursor heartbeat runs only while the
  agent cursor is visible. Nothing depends on the refresh period while hidden.
- `LayerShellOverlayBackend { event_queue, app: LayerShellApp, system_cursor, pointer_tracker, conn }`
  (lines 69-75); constructing one needs a live Wayland connection, so unit
  tests cannot build it. The file's `#[cfg(test)]` module (line 748) tests
  pure helpers only. `should_animate` and `current_epoch_ms`
  (`layer_shell.rs:740`) are private items in the same module, callable from
  `pointer_tick_interval` with no visibility change.

`crates/sky-cua-overlay-host/src/lib.rs`:

- Lines 505-509: `pub enum OverlayHostBackend { Noop(NoopOverlayBackend), #[cfg(target_os = "linux")] LayerShell(Box<layer_shell::LayerShellOverlayBackend>) }`.
- Lines 605-624 (verbatim):

  ```rust
  pub fn tick(&mut self) {
      match self {
          Self::Noop(_backend) => {}
          #[cfg(target_os = "linux")]
          Self::LayerShell(backend) => backend.tick(),
      }
  }

  /// Cadence the host event loop should tick at, matched to the fastest
  /// connected display so the agent-cursor follow renders at the panel's
  /// refresh rate instead of a fixed 60 Hz. Falls back to 60 Hz when no
  /// display refresh rate is known.
  #[must_use]
  pub fn pointer_tick_interval(&self) -> std::time::Duration {
      match self {
          Self::Noop(_backend) => std::time::Duration::from_millis(16),
          #[cfg(target_os = "linux")]
          Self::LayerShell(backend) => backend.pointer_tick_interval(),
      }
  }
  ```

- `#[cfg(test)] mod tests` at lines 689-690: it has NO `use super::*`; it
  imports a fixed list including `NoopOverlayBackend` and
  `OVERLAY_HOST_PROTOCOL_VERSION`, so new tests must name
  `super::OverlayHostBackend` and `super::IDLE_TICK_INTERVAL` explicitly.
  `NoopOverlayBackend::with_reason(..)` (line 190) is the constructor; the
  `Noop` variant is constructible from the module.

`crates/sky-cua-overlay-host/src/main.rs`, the three consumers:

- Line 29: `const SOCKET_LOOP_TICK_INTERVAL: Duration = Duration::from_millis(16);`
  (initial timer only).
- Lines 243-266 (`run_unix_socket_event_loop`): the Unix listener is a
  separate `Generic` source whose callback calls `state.accept_ready()`
  (lines 243-255), so accepts do not wait for the timer; the calloop timer
  (257-266) re-arms ONLY from its own callback with
  `TimeoutAction::ToDuration(state.backend.pointer_tick_interval())` after
  `state.backend.tick()`. `UnixSocketLoopState { listener, backend, signal, label }`
  is built at lines 268-273 and the `calloop::{...}` import is near the top
  of the file. There is no path that re-arms the timer when a request
  arrives.
- Line 165: the JSON-lines loop uses `line_rx.recv_timeout(backend.pointer_tick_interval())`;
  a line arriving wakes it immediately regardless of the timeout.
- Lines 336-346 (`run_accept_loop`, used by TCP): on `WouldBlock` it calls
  `backend.tick()` then `std::thread::sleep(backend.pointer_tick_interval())`.
  It handles one request per connection and the service opens a new
  connection per message (`crates/sky-cua-service/src/overlay/host/mod.rs:22,59-66`
  always uses `TcpEndpoint` on non-Unix), so this sleep bounds EVERY request's
  latency on Windows, where the backend is always `Noop` (layer-shell is
  Linux-only). Today that bound is 16 ms.

Conventions: `cargo nextest run`; conventional commits, e.g.
`fix(service): hard-cap daemon log generations and back off accept errors`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-overlay-host --all-targets -- -D warnings` | exit 0 |
| Crate tests | `cargo nextest run -p sky-cua-overlay-host` | all pass (105 at `e928a40`, plus yours) |
| Idle-tick tests | `cargo nextest run -p sky-cua-overlay-host tick_interval` | 2 new tests pass |

## Scope

**In scope** (the only files you should modify):
- `crates/sky-cua-overlay-host/src/layer_shell.rs`
- `crates/sky-cua-overlay-host/src/lib.rs`
- `crates/sky-cua-overlay-host/src/main.rs` (the Unix-socket loop's timer re-arm and the TCP accept loop's sleep cap)
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- The JSON-lines loop in `main.rs` (`recv_timeout` already wakes on input).
- Replacing the TCP accept loop's poll with a blocking accept plus timer.
- The pointer tracker, renderer, motion driver, or `wait_for_arrival`.
- The service side (`crates/sky-cua-service/src/overlay/**`).
- The 60–240 Hz clamp for the visible case.

## Git workflow

- Branch: `bex/overlay-host-idle-tick`
- Commit only if the operator authorised it; style:
  `perf(overlay-host): tick at a slow idle rate while the cursor is hidden`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add a pure interval policy with tests

In `lib.rs`, near `OVERLAY_HOST_PROTOCOL_VERSION`, add one definition (it
must live in `lib.rs`, not `layer_shell.rs`, because `layer_shell` is
`cfg(target_os = "linux")` and the Noop backend needs it on every target):

```rust
/// Idle cadence while nothing is visible or animating: cuts 60–240 Hz idle
/// wakeups by 6–24x. Message handling on the Unix-socket and JSON-lines loops
/// is not gated on the tick timer (the socket loop re-arms it on each
/// request), and `SetCursor`/`Show` render their first frame synchronously,
/// so a show is never delayed by it.
pub const IDLE_TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
```

In `layer_shell.rs`, add (outside `impl` blocks, near the top-level helpers):

```rust
/// The event-loop tick period: the display refresh period while the overlay
/// is visible or the motion driver is animating, otherwise the idle period.
pub(crate) fn tick_interval_for(
    visible: bool,
    animating: bool,
    refresh_interval: std::time::Duration,
) -> std::time::Duration {
    if visible || animating {
        refresh_interval
    } else {
        crate::IDLE_TICK_INTERVAL
    }
}
```

Then change `pointer_tick_interval` so its existing body computes
`refresh_interval` and the function returns

```rust
let visible = self.app.state.as_ref().is_some_and(|state| state.visible);
let animating = self.app.should_animate(current_epoch_ms());
tick_interval_for(visible, animating, refresh_interval)
```

Update the doc comment to say "while visible or animating; otherwise
`IDLE_TICK_INTERVAL`".

Add to the `#[cfg(test)]` module at the bottom of `layer_shell.rs`:

```rust
#[test]
fn tick_interval_is_refresh_rate_while_visible_or_animating_and_idle_otherwise() {
    let refresh = std::time::Duration::from_micros(4_167); // 240 Hz
    assert_eq!(super::tick_interval_for(true, false, refresh), refresh);
    assert_eq!(super::tick_interval_for(false, true, refresh), refresh);
    assert_eq!(super::tick_interval_for(true, true, refresh), refresh);
    assert_eq!(super::tick_interval_for(false, false, refresh), crate::IDLE_TICK_INTERVAL);
    assert!(crate::IDLE_TICK_INTERVAL >= std::time::Duration::from_millis(50));
}
```

**Verify**: `cargo nextest run -p sky-cua-overlay-host tick_interval` → 1 pass;
`cargo clippy -p sky-cua-overlay-host --all-targets -- -D warnings` → exit 0.

### Step 2: Make the Noop backend idle too

In `lib.rs` `pointer_tick_interval`, change the Noop arm from
`std::time::Duration::from_millis(16)` to `IDLE_TICK_INTERVAL`, and update
the doc comment to mention the idle period and that the Noop backend never
renders, so it always idles.

Add to `mod tests` in `lib.rs` (line 690; it has no `use super::*`, so name
the items explicitly):

```rust
#[test]
fn noop_backend_tick_interval_is_idle() {
    let backend = super::OverlayHostBackend::Noop(NoopOverlayBackend::with_reason("test"));
    assert_eq!(backend.pointer_tick_interval(), super::IDLE_TICK_INTERVAL);
}
```

**Verify**: `cargo nextest run -p sky-cua-overlay-host tick_interval` → 2 pass;
`cargo clippy -p sky-cua-overlay-host --all-targets -- -D warnings` → exit 0.

### Step 3: Re-arm the Unix-socket loop's timer when a request changes the cadence

In `main.rs`: add `LoopHandle` and `RegistrationToken` to the existing
`calloop::{...}` import. Extend `UnixSocketLoopState` with
`handle: LoopHandle<'static, UnixSocketLoopState>`,
`tick_timer: Option<RegistrationToken>` and `armed_interval: Duration`. Add
to its `impl`:

```rust
/// (Re)arms the frame-pacing timer when the backend now wants a faster
/// cadence than the armed one, so a request that makes the overlay visible
/// does not wait out an idle period armed while it was hidden.
fn rearm_tick_timer_if_sooner(&mut self) {
    let interval = self.backend.pointer_tick_interval();
    if self.tick_timer.is_some() && interval >= self.armed_interval {
        return;
    }
    if let Some(token) = self.tick_timer.take() {
        self.handle.remove(token);
    }
    self.armed_interval = interval;
    self.tick_timer = self
        .handle
        .insert_source(Timer::from_duration(interval), |_deadline, _timer, state| {
            state.backend.tick();
            state.armed_interval = state.backend.pointer_tick_interval();
            TimeoutAction::ToDuration(state.armed_interval)
        })
        .ok();
}
```

Then in `run_unix_socket_event_loop`: delete the standalone timer
`insert_source` (lines 257-266); build `state` with `handle: handle.clone()`,
`tick_timer: None`, `armed_interval: SOCKET_LOOP_TICK_INTERVAL`; call
`state.rearm_tick_timer_if_sooner()` once and bail with
"failed to register overlay host socket timer" if `state.tick_timer.is_none()`;
and in the listener callback call `state.rearm_tick_timer_if_sooner();`
immediately after `state.accept_ready();`. Keep `SOCKET_LOOP_TICK_INTERVAL`
(it is now only the initial `armed_interval`; if clippy flags it unused,
delete the constant and use `IDLE_TICK_INTERVAL` as the initial value).

**Verify**: `cargo clippy -p sky-cua-overlay-host --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-overlay-host` → all pass.

### Step 4: Cap the TCP accept loop's poll sleep

In `main.rs` add `const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(16);`
next to `SOCKET_LOOP_TICK_INTERVAL`, and in `run_accept_loop` replace
`std::thread::sleep(backend.pointer_tick_interval());` with
`std::thread::sleep(backend.pointer_tick_interval().min(ACCEPT_POLL_INTERVAL));`
with a one-line comment: the TCP loop polls accept, so its sleep bounds
per-request latency; the idle period applies only to the evented loops. This
plan saves no wakeups on the TCP lane.

**Verify**: `cargo clippy -p sky-cua-overlay-host --all-targets -- -D warnings` → exit 0;
`grep -n 'ACCEPT_POLL_INTERVAL' crates/sky-cua-overlay-host/src/main.rs` → 2 matches.

### Step 5: Whole-crate run and a live spot check

`cargo fmt --check` → exit 0; `cargo nextest run -p sky-cua-overlay-host` →
all pass (105 + 2).

Optional live check on a KDE/Wayland session. It needs the new build
installed through the `cua-deploy` flow, which requires the operator's
authorisation; skip if not authorised. With the agent cursor hidden, measure
`perf stat -e task-clock -p "$(pgrep -d, -f 'sky-cua-overlay-host serve')" sleep 10`
before and after (the `-d,` form handles the isolated second-daemon lane's
extra host process). Then exercise the two cases that matter: a show from
hidden (`desktop_pointer` click) and a `capture_desktop` while the cursor is
visible (which sends `Hide` then `Show`); confirm the glide and follow
resume with no visible stall. Record what you observed; do not block on this
step.

## Test plan

- `tick_interval_is_refresh_rate_while_visible_or_animating_and_idle_otherwise`
  (layer_shell.rs) and `noop_backend_tick_interval_is_idle` (lib.rs).
- Existing 105 overlay-host tests unchanged; no overlay-host or service test
  asserts on host response latency, and the TCP idle-client test still fits
  its 700 ms read timeout.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-overlay-host --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-overlay-host` exits 0 with 2 new tests
- [ ] `grep -n 'fn tick_interval_for' crates/sky-cua-overlay-host/src/layer_shell.rs` matches and `grep -n 'pub const IDLE_TICK_INTERVAL' crates/sky-cua-overlay-host/src/lib.rs` matches
- [ ] `grep -n 'Duration::from_millis(16)' crates/sky-cua-overlay-host/src/lib.rs` prints nothing
- [ ] `grep -n 'fn rearm_tick_timer_if_sooner\|ACCEPT_POLL_INTERVAL' crates/sky-cua-overlay-host/src/main.rs` shows the method and 2 constant uses
- [ ] `git diff --name-only e928a40 -- crates/` prints exactly `crates/sky-cua-overlay-host/src/layer_shell.rs`, `crates/sky-cua-overlay-host/src/lib.rs`, `crates/sky-cua-overlay-host/src/main.rs`
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- calloop 0.14.5 rejects a `LoopHandle<'static, UnixSocketLoopState>` stored
  inside the loop's own data (Step 3). Report the compiler error; the
  fallback design is a `calloop::ping` source the listener callback triggers
  to re-arm, but that is for the maintainer to choose.
- The live spot check shows the cursor failing to follow the real pointer
  after a show, or a glide stuttering after a capture. Report; do not tune
  the constant blindly.
- Any existing overlay-host test fails.

## Maintenance notes

- The TCP accept loop (Windows, always Noop) caps its sleep at
  `ACCEPT_POLL_INTERVAL`, so this plan saves no wakeups there; it only stops
  making that lane slower. Replacing the poll with a blocking accept plus a
  timer is a separate change.
- `pointer_tick_interval` now reads `should_animate`, which inspects
  `last_motion` and `glow_active`; both are cheap field reads. If a future
  gesture state is "pending but not animating" and still needs ticks, add it
  to `tick_interval_for`'s inputs rather than widening `should_animate`.
- Why the idle gate engages after `Hide`: `motion.hide()` (`cursor_motion.rs:242`)
  only clears `pending`/`active`; the `animating` flag clears because the
  `Hide` handler renders, and `draw` stores `frame(false)` into
  `last_motion` (`layer_shell.rs:592`, `cursor_motion.rs:485`). If
  `render_current` bails before `draw` (no open layer), ticking stays at the
  refresh rate, as today.
- The re-arm in Step 3 only ever shortens the armed interval; lengthening
  happens naturally on the next timer fire via `TimeoutAction::ToDuration`.
