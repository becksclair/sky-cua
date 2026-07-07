# Service desktop request lane

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `~/.agents/PLANS.md` and `plans/AGENTS.md`.

## Purpose / Big Picture

After this change, the `sky-cua-service` daemon can answer safe service requests while a slow desktop operation is still running. A daemon is the long-lived background process that receives JSON service requests from the MCP client and calls the platform desktop backend. Today the Unix/TCP IPC server wraps the entire `ServiceDaemon` in one async mutex, so one request that waits for portal approval, capture, or physical input blocks everything behind it. Recent MCP work made `tools/call` concurrent on the client side, so the service must become the explicit concurrency boundary instead of accidentally serializing all calls.

The observable outcome is a service-level test where a fake blocked desktop action does not stop `Health` from completing, while a second desktop-sensitive request still waits its turn. The user should also be able to run the normal service, client, and VM smoke commands without changes to serialized request or response shapes.

## Progress

- [x] (2026-05-19 16:22Z) Read the service, client, plan, and VM-smoke guidance relevant to `ICA-007`.
- [x] (2026-05-19 16:23Z) Established a green baseline with `rtk cargo test -p sky-cua-service`: 33 tests passed.
- [x] (2026-05-19 16:27Z) Added characterization tests proving safe request bypass and desktop-lane serialization. The first red run failed because `ServiceDaemon::handle` required `&mut self`, so an `Arc<ServiceDaemon>` could not serve concurrent calls.
- [x] (2026-05-19 16:32Z) Refactored `ServiceDaemon` so request handling takes `&self`, `Health` bypasses the desktop lane, and snapshots/overlay are protected by narrow internal mutexes.
- [x] (2026-05-19 16:33Z) Updated `ipc_server.rs` to hold `Arc<ServiceDaemon>` instead of `Arc<Mutex<ServiceDaemon>>` and initially preserved last-client cursor cleanup with a post-overlay-lock active-connection recheck.
- [x] (2026-05-19 16:35Z) Re-ran `rtk cargo test -p sky-cua-service`: 35 tests passed.
- [x] (2026-05-19 16:43Z) Ran the requested cleanup/review pass for this slice. Cleanup fixed a lost-wakeup risk in the new `Notify`-based tests. Review found that the post-overlay-lock active-connection recheck still left a post-check/pre-hide reconnect race, so `ipc_server.rs` now uses `ConnectionTracker` to serialize active-count changes with final cursor cleanup.
- [x] (2026-05-19 16:56Z) Passed the requested Plasma/KWin `wayland-pointer` VM smoke proof with current checkout synced and rebuilt: `pty_0d281557`, artifacts `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260519T165455Z`.
- [x] (2026-05-19 17:08Z) Re-ran the Plasma/KWin `wayland-pointer` VM smoke after review-work fixes: `pty_e3075332`, artifacts `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260519T170803Z`.

## Surprises & Discoveries

- Observation: `crates/sky-cua-client/src/mcp_server.rs` already spawns concurrent `tools/call` work and serializes stdout separately.
  Evidence: `serve` uses `tokio::spawn`, `tokio::task::spawn_blocking`, and a dedicated response writer task.

- Observation: the service still serializes every parsed request under one daemon-wide lock.
  Evidence: `crates/sky-cua-service/src/ipc_server.rs` calls `daemon.lock().await.handle(request).await` inside `handle_stream`.

- Observation: workspace formatting currently has pre-existing drift outside this service slice.
  Evidence: `rtk cargo fmt --check` now reports diffs in `crates/sky-cua-linux/src/x11/input_xtest.rs` and `crates/sky-cua-linux/src/x11/windowing.rs`, which were not edited for `ICA-007`. The touched service files were formatted directly with `rustfmt --edition 2024`.

- Observation: `tokio::sync::Notify::notify_waiters()` was the wrong signal primitive for the new concurrency tests.
  Evidence: the tests wait after spawning the blocking task; `notify_waiters()` does not store a permit for future waiters, so a fast task could lose the signal. The tests now use `notify_one()` for stored start/release permits.

- Observation: rechecking `active_connections` inside daemon cursor cleanup was not enough to close the reconnect race.
  Evidence: review-work identified a remaining window after the recheck but before `overlay.hide()`. `ConnectionTracker` now holds the active-count mutex while hiding the cursor on final disconnect, so a new accept cannot increment the active count halfway through that decision.

- Observation: the Plasma VM smoke exposed unrelated-but-blocking KWin and KDE text-input defects while proving the service slice.
  Evidence: KWin activation initially left a stale `sky-cua-activate-window` script loaded because `unloadScript` expects a plugin name, not the script path. Later `type_text` failed until the GTK fixture kept deterministic desktop coordinates, the smoke recorded entry focus before typing, empty `wl-paste --list-types` output was treated as an empty clipboard, and KDE clipboard paste used EIS-backed `Ctrl+v` instead of legacy `NotifyKeyboardKeycode`.

## Decision Log

- Decision: implement a service-owned desktop lane rather than reverting MCP concurrency or serializing in the client.
  Rationale: desktop-mutating operations need ordering, but safe service metadata should not wait behind unrelated capture/input work. The service has the state needed to classify and order requests correctly.
  Date/Author: 2026-05-19 / Sky

- Decision: begin with service-level characterization tests using a fake backend.
  Rationale: concurrency bugs are easy to create and hard to see in live desktop smokes. A fake backend can prove the intended scheduling behavior before source movement.
  Date/Author: 2026-05-19 / Sky

- Decision: keep only `Health` outside the desktop lane in the first implementation slice.
  Rationale: `Health` does not touch the desktop backend, snapshots, or overlay host. Other candidates such as `Doctor`, `ListWindows`, `FocusedWindow`, and `AgentCursorStatus` may be safe later, but proving them requires narrower evidence and is not necessary to remove the daemon-wide mutex bottleneck.
  Date/Author: 2026-05-19 / Sky

- Decision: use an async `ConnectionTracker` in `ipc_server.rs` instead of a daemon callback that checks an atomic active count.
  Rationale: the cleanup decision and cursor hide must be atomic with respect to new connection registration. Holding the tracker mutex during final cleanup excludes new registrations until stale cursor cleanup completes.
  Date/Author: 2026-05-19 / Sky

- Decision: keep the first service concurrency slice conservative by only bypassing the desktop lane for `Health`.
  Rationale: the tests prove the intended bottleneck removal without pretending other backend-reading requests are safe. `Doctor`, `ListWindows`, `FocusedWindow`, and `AgentCursorStatus` can be reconsidered with narrower evidence later.
  Date/Author: 2026-05-19 / Sky

## Outcomes & Retrospective

Implemented. `ServiceDaemon::handle` now takes `&self`, `Health` can complete while a fake desktop operation is blocked, and non-`Health` desktop-sensitive requests serialize through an explicit `desktop_lane`. `ipc_server.rs` now shares `Arc<ServiceDaemon>` instead of `Arc<Mutex<ServiceDaemon>>`, and `ConnectionTracker` preserves final cursor cleanup without reopening the reconnect race.

The slice also hardened the Plasma VM proof path. KWin activation cleanup now unloads by plugin name and refreshes stale scripts before loading. The GTK pointer fixture keeps stable desktop-coordinate target points after activation, the smoke records entry focus before typing, and KDE text fallback handles empty clipboards and uses EIS-backed paste chords.

Validation completed:

    rtk cargo test -p sky-cua-service
    rtk cargo test -p sky-cua-client
    rtk cargo test -p sky-cua-linux actions::tests
    rtk cargo test -p sky-cua-linux portal::remote_desktop::tests
    rtk cargo test -p sky-cua-linux windowing::registry::tests
    rtk cargo test -p sky-cua-linux kwin::tests
    rtk uv run ruff format --check scripts/gtk_pointer_smoke_fixture.py scripts/live_wayland_pointer_smoke.py
    rtk uv run ruff check scripts/gtk_pointer_smoke_fixture.py scripts/live_wayland_pointer_smoke.py
    rtk uv run basedpyright scripts/gtk_pointer_smoke_fixture.py scripts/live_wayland_pointer_smoke.py
    rtk git diff --check
    rtk uv run python scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile wayland-pointer --desktop-env KDE --wayland-display wayland-0
    (PTY: pty_0d281557)

The accepted VM proofs are `pty_0d281557`, which completed successfully with artifacts at `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260519T165455Z`, and the post-review rerun `pty_e3075332`, which completed successfully with artifacts at `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260519T170803Z`.

## Context and Orientation

The main service request router lives in `crates/sky-cua-service/src/daemon.rs`. `ServiceDaemon::handle` currently takes `&mut self`, touches session idle state, matches every `ServiceRequest`, calls the `DesktopBackend` trait, stores snapshots through `SnapshotManager`, and mutates `OverlayController` cursor state.

The IPC server lives in `crates/sky-cua-service/src/ipc_server.rs`. On Unix it accepts `UnixStream`s; on Windows it accepts `TcpStream`s. Both paths call `handle_stream`, parse newline-delimited JSON into `ServiceRequest`, then call `daemon.lock().await.handle(request).await`. Because that mutex is held across the whole request await, every request is serialized even when it only needs a health response.

The backend trait lives in `crates/sky-cua-platform/src/backend.rs`. A `DesktopBackend` is a platform-specific implementation for Linux, Windows, or fallback environments. Calls like `get_app_state`, `execute_action`, `activate_window`, setup, and portal-token reset can touch the real desktop and must be ordered. Calls like `Health` can be answered from service state without touching the backend.

The client-side pressure comes from `crates/sky-cua-client/src/mcp_server.rs`, which now lets multiple `tools/call` requests run concurrently while keeping stdout writes ordered. `crates/sky-cua-client/src/service_launcher.rs` lets cloned clients share or open service connections per call, so concurrent MCP work can reach the daemon through multiple IPC streams.

## Plan of Work

First, add tests in `crates/sky-cua-service/src/daemon.rs` or a new runtime test module with a fake backend that can block inside `execute_action`. The first test should start one desktop-lane request, wait until the fake backend proves it is blocked, then assert that `Health` completes quickly. The second test should start two desktop-lane requests and assert the second does not enter the fake backend until the first is released.

Second, introduce a shared runtime boundary. The intended shape is `ServiceRuntime` with narrow state holders: a backend handle, a `SessionStore`, a `SnapshotManager`, an `OverlayController`, the socket path, and a `desktop_lane: tokio::sync::Mutex<()>`. Request handling should take `&self`. State that must be mutated across a desktop operation should be protected by small locks, but those locks must not become a replacement daemon-wide lock held across every request. The desktop lane is the long-running ordering primitive for capture, action, activation, setup, and portal-token reset.

Third, classify requests conservatively. The initial safe bypass set is `Health`; additional bypasses such as `Doctor`, `ListWindows`, `FocusedWindow`, and `AgentCursorStatus` should only move outside the desktop lane after source evidence shows their backend calls and overlay status behavior cannot race the desktop session. For this slice, proving `Health` bypass is sufficient and lower risk.

Fourth, update `crates/sky-cua-service/src/ipc_server.rs` to store `Arc<ServiceRuntime>` or an equivalent shared runtime instead of `Arc<tokio::sync::Mutex<ServiceDaemon>>`. Preserve active connection accounting, the reconnect race fix in last-client cursor cleanup, and idle timeout behavior.

Finally, record progress on `ICA-007` (completed; backlog file retired 2026-07-07, see git history) after the slice is verified. Do not retire this plan until source tests, clean-and-review, and the requested VM smoke gate have passed.

## Concrete Steps

Run all commands from `/home/bex/projects/sky-cua`.

1. Add the failing concurrency characterization tests:

    rtk cargo test -p sky-cua-service service_runtime_health_bypasses_blocked_desktop_request
    rtk cargo test -p sky-cua-service service_runtime_serializes_desktop_lane_requests

2. Implement the smallest runtime split that makes those tests pass while preserving existing daemon behavior.

3. Re-run narrow and then crate-level validation:

    rtk cargo test -p sky-cua-service service_runtime_health_bypasses_blocked_desktop_request
    rtk cargo test -p sky-cua-service service_runtime_serializes_desktop_lane_requests
    rtk cargo test -p sky-cua-service

4. Run the requested cleanup/review pass for the current working tree, then rerun the relevant service checks.

5. Run the VM smoke profile for this slice. The default proof should be Plasma/KWin `wayland-pointer` through the port-forward form if `testing-vm` does not resolve:

    uv run python scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile wayland-pointer --desktop-env KDE --wayland-display wayland-0

## Validation and Acceptance

The service slice is accepted when `cargo test -p sky-cua-service` passes and at least one new test proves `Health` completes while a fake desktop-lane request is blocked. A second test must prove two desktop-lane requests preserve ordering. The serialized `ServiceRequest` and `ServiceResponse` shapes must not change.

The live proof is accepted when the VM runner completes a real-session profile after rebuilding and syncing the current checkout. If the VM is unavailable or on the wrong desktop, record the exact command, failure, and next environment step rather than claiming runtime proof.

## Idempotence and Recovery

The Rust tests are safe to run repeatedly. The VM runner builds host artifacts and syncs the checkout by default; rerun without `--skip-host-build` or `--skip-sync` unless the VM checkout and runtime artifacts have been explicitly confirmed current. If a smoke leaves stale `sky-cua-service`, `sky-cua-overlay-host`, `service.sock`, or `agent-cursor.sock` state, rerun the normal VM runner because it resets guest sky-cua processes before profiles.

## Artifacts and Notes

Baseline before implementation:

    rtk cargo test -p sky-cua-service
    cargo test: 33 passed (1 suite, 0.05s)

Relevant backlog item: `ICA-007` (completed; backlog file retired 2026-07-07, see git history).

## Interfaces and Dependencies

The new service runtime must keep using `sky_cua_platform::model::ServiceRequest` and `ServiceResponse`; no wire format change is planned. It must keep using `DesktopBackend` from `sky_cua_platform::backend` for desktop calls. It should keep `SessionStore`, `SnapshotManager`, and `OverlayController` inside the service crate, behind narrow ownership boundaries.

The expected internal interface is a shared runtime value with an async `handle(&self, request: ServiceRequest) -> ServiceResponse` method and small synchronous helpers for last-client cursor cleanup and idle-time queries. Names can differ if the resulting interface is equally explicit and tests prove the same behavior.
