# Plan 005: Move CPU-bound and sleep-blocking work off the async executor (capture encode, virtual input, overlay startup)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-linux/src/capture_plan.rs crates/sky-cua-linux/src/actions/mod.rs crates/sky-cua-service/src/browser/model_image.rs crates/sky-cua-service/src/browser/bridge.rs crates/sky-cua-service/src/overlay/host/lifecycle.rs crates/sky-cua-capture/src/lib.rs`
> On any in-scope drift, re-verify the excerpts below before proceeding; on a
> mismatch, STOP.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW-MED (mechanical offloads; ownership moves across closure
  boundaries; no logic changes)
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

The long-lived daemon serves desktop, browser, and phone surfaces from one
Tokio runtime. Three families of blocking work currently run inline on
executor threads:

1. **Image work**: every desktop capture's PNG decode/crop/resize/WebP-encode
   and every browser capture's base64→decode→Lanczos3-resize→re-encode run
   synchronously inside `async fn`s — tens of ms of pure CPU per tool call.
2. **Virtual input pacing**: drags sleep per waypoint, typing sleeps up to
   30ms/char, and ydotool commands busy-wait `thread::sleep(10ms)` loops —
   an input action can pin a worker thread for seconds.
3. **Overlay host startup**: a 2s `std::thread::sleep` poll loop runs on the
   executor — and it's reached while holding the overlay `tokio::Mutex`,
   stalling every concurrent capture/action that needs the overlay.

The Windows backend already does this correctly
(`crates/sky-cua-windows/src/backend.rs:1060` wraps capture image work in
`tokio::task::spawn_blocking`) — this plan brings the Linux/browser/overlay
paths to the same standard.

## Current state

Site 1a — `crates/sky-cua-linux/src/capture_plan.rs:335-358` (inside
`async fn plan_capture`, awaited from `backend.rs` at :416, :1285, :1339):

```rust
let (cropped_path, cropped_image) =
    screenshot::crop_capture(snapshot_id, raw_path, crop.pixel_rect)?;   // sync decode+crop
...
screenshot::prepare_model_capture_from_image(                            // sync resize+encode
    snapshot_id, cropped_image, &cropped_path, Some(cropped_pixel_size),
)?
...
None => screenshot::prepare_model_capture(snapshot_id, raw_path)?,       // sync decode+resize+encode
```

The `screenshot::*` functions live in `crates/sky-cua-capture/src/lib.rs`
(fully synchronous; `grep spawn_blocking crates/sky-cua-capture/src/lib.rs`
→ zero hits at planning time).

Site 1b — `crates/sky-cua-service/src/browser/bridge.rs:323` calls
`prepare_browser_capture` (defined in `browser/model_image.rs:139-171`:
base64 decode → `image::load_from_memory` → `resize_exact` Lanczos3 →
re-encode → base64) inline in the async CDP handler.

Site 2 — `crates/sky-cua-linux/src/actions/mod.rs`: async dispatch calls
sync `self.runtime.virtual_drag(...)` (~:563), `virtual_type_text` (~:630),
and key-sequence equivalents. These bottom out in
`crates/sky-cua-linux/src/virtual_input.rs`: `drag` waypoint loop with
`thread::sleep` (~:314-322), helper typing with 30ms/char pacing +
blocking socket round trips (~:422,440), `run_ydotool_command` spawn +
10ms busy-wait (~:1310-1330). Note the XTest drag path in `actions/mod.rs`
(~:540-545) already uses `tokio::time::sleep(...).await` — that one is fine.

Site 3 — `crates/sky-cua-service/src/overlay/host/lifecycle.rs:150-178`:

```rust
fn wait_for_ready(&mut self) -> Result<(), String> {
    let started = Instant::now();
    ...
    while started.elapsed() < HOST_START_TIMEOUT {          // 2s
        ... child.try_wait() ...
        match self.endpoint.ready_probe() { Ok(()) => return Ok(()), ... }
        std::thread::sleep(HOST_CONNECT_INTERVAL);
    }
    ...
}
```

Reached synchronously via `host.send` → `ensure_running` → `wait_for_ready`,
called from `OverlayController` methods that run under
`self.overlay.lock().await` in `daemon.rs` handlers (:899, :939, :978, :1009
neighborhood).

Exemplar to imitate — `crates/sky-cua-windows/src/backend.rs:1060`:

```rust
let image_result = tokio::task::spawn_blocking(move || { ... })
```

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Build | `cargo build` | exit 0 |
| Tests (targeted) | `cargo nextest run -p sky-cua-linux -p sky-cua-service -p sky-cua-capture` | all pass |
| Whole workspace | `cargo nextest run` | all pass |
| Format | `cargo fmt --check` | exit 0 |

## Scope

**In scope**:
- `crates/sky-cua-linux/src/capture_plan.rs` (wrap the three screenshot calls)
- `crates/sky-cua-service/src/browser/bridge.rs` and/or
  `crates/sky-cua-service/src/browser/model_image.rs` (offload
  `prepare_browser_capture`)
- `crates/sky-cua-linux/src/actions/mod.rs` (route `virtual_*` calls through
  `spawn_blocking`)
- `crates/sky-cua-service/src/overlay/host/lifecycle.rs` +
  `crates/sky-cua-service/src/overlay.rs` (async-safe host startup; see
  Step 3 constraint)

**Out of scope** (do NOT touch):
- `crates/sky-cua-capture/src/lib.rs` internals — keep the sync functions
  sync; offloading happens at call sites.
- `crates/sky-cua-linux/src/virtual_input.rs` internals — the sleeps *inside*
  are correct once the whole call is on a blocking thread. (Exception: none.)
- The phone capture path (`phone/manager/capture.rs`) — that's plan 006.
- KWin discovery (`kwin.rs`) — already wrapped in `run_blocking`; verified.
- Windows backend — already correct.
- Overlay *protocol* or host process behavior.

## Git workflow

- Branch: `bex/advisor-005-executor-offload`
- Commits per site: `perf(linux): offload capture image work to spawn_blocking`,
  `perf(browser): offload model-image encode`, `perf(linux): run virtual input on blocking threads`,
  `perf(overlay): stop blocking the executor during host startup`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Capture image work → `spawn_blocking`

In `capture_plan.rs`, wrap each of the three `screenshot::*` call clusters:

```rust
let (cropped_path, cropped_image) = tokio::task::spawn_blocking({
    let raw_path = raw_path.clone();          // move owned data in
    let pixel_rect = crop.pixel_rect.clone();
    let snapshot_id = snapshot_id.to_owned();
    move || screenshot::crop_capture(&snapshot_id, &raw_path, pixel_rect)
})
.await
.map_err(|join| /* map JoinError into the existing BackendError shape used in this fn */)??;
```

Adapt to the real signatures (read them in `sky-cua-capture/src/lib.rs`
first). Where two sync calls run back-to-back (`crop_capture` then
`prepare_model_capture_from_image`), put them in ONE closure to avoid a
thread-pool bounce and an intermediate clone of `cropped_image`. Find every
error type conversion the surrounding code uses and match it.

Do the same for `prepare_browser_capture` at its `bridge.rs:323` call site.

**Verify**: `cargo nextest run -p sky-cua-linux -p sky-cua-service -p sky-cua-capture` → all pass;
`grep -c spawn_blocking crates/sky-cua-linux/src/capture_plan.rs` → ≥2.

### Step 2: Virtual input → `spawn_blocking`

In `actions/mod.rs`, wrap the sync `self.runtime.virtual_*` calls. The
`runtime` handle is shared — check its type: if it's `Arc`-cloneable, clone
into the closure; if it's `&self` borrowed, restructure to pass owned
parameters into the closure and call a clone of the underlying input handle.
Read how `runtime` is stored (grep `virtual_drag` and the struct definition)
before choosing. Keep the XTest async-sleep path untouched.

**Verify**: `cargo nextest run -p sky-cua-linux` → all pass (the action
executor has fake-backed tests; they prove routing still works).

### Step 3: Overlay host startup off the executor and out from under the lock

Constraint from the advisor's read: `wait_for_ready` itself is sync and
fine on a blocking thread; the problem is (a) it runs on an executor thread,
and (b) the overlay mutex is held across it.

Minimal safe change (do this, not a redesign): find where the async path
enters the sync `host.send` → `ensure_running` chain (grep `fn send` in
`overlay/host/` and its callers in `overlay.rs`). Wrap the *outermost sync
entry point* called from async context in `spawn_blocking`. The overlay
mutex is a `tokio::sync::Mutex` held in `daemon.rs`; do NOT try to release
it around the call in this plan (lock-scope surgery risks reordering
overlay state updates — that's the deferred follow-up). Offloading to
`spawn_blocking` already stops the *executor-thread* stall; the lock still
serializes overlay users, which is its job.

If `host.send`'s receiver types are not `Send`, report via STOP rather than
adding `unsafe` or channel indirection.

**Verify**: `cargo nextest run -p sky-cua-service` → all pass (overlay.rs has
36 tests at planning time — they gate this).

## Test plan

No new tests required — this is a mechanical offload with existing coverage:
`sky-cua-service` overlay tests, `sky-cua-linux` action-executor fake tests,
and browser bridge tests all exercise these paths. Run the full
`cargo nextest run` as the gate. State in your report that latency
improvement was not benchmarked (no benchmark harness in repo) and that live
desktop smokes (`scripts/live_desktop_smoke.py`) were not run unless the
operator asks.

## Done criteria

- [ ] `cargo fmt --check && cargo nextest run` exits 0
- [ ] `grep -c spawn_blocking crates/sky-cua-linux/src/capture_plan.rs` ≥ 2
- [ ] Browser model-image encode site is wrapped (`grep -n spawn_blocking crates/sky-cua-service/src/browser/` → ≥1 hit)
- [ ] `actions/mod.rs` virtual-input dispatch is wrapped (`grep -n spawn_blocking crates/sky-cua-linux/src/actions/mod.rs` → ≥1 hit)
- [ ] Overlay host startup entry from async context is wrapped
- [ ] No files outside the in-scope list modified
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- A type that must cross a `spawn_blocking` boundary is not `Send`
  (raw X11/Wayland handles are the likely culprits in the input runtime) —
  report which type; do not wrap it in unsafe Send.
- Wrapping a call site requires cloning a full-resolution image buffer that
  the current code passes by reference — report; the fix may need a
  signature change beyond mechanical scope.
- Any overlay test failure that looks timing-related (the 2s startup window
  interacts with test fakes) — report rather than raising timeouts.

## Maintenance notes

- Deferred: releasing the overlay mutex during host startup (lock-scope
  surgery), and converting `wait_for_ready` to a genuinely async handshake.
- Reviewer scrutiny: each closure should move *owned* data — flag any new
  `.clone()` of image buffers (there should be none; paths and small structs
  only).
- Future capture-path work (plan 006 phone pipeline) should follow the same
  spawn_blocking pattern established here.
