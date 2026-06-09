# TODO: Improve Codebase Architecture

This file tracks architecture improvement candidates found during codebase review. Treat it as a triage backlog: validate evidence first, then design and implement incrementally.

<!-- improve-codebase-architecture:start -->

Generated: 2026-06-09
Scope: repository root on current checkout, reconciled against `ROADMAP.md`
Analysis notes: This backlog is architecture support for roadmap work, not a parallel product roadmap. Direct roadmap blockers are prioritized over general cleanup. The worktree was already dirty; this file is the only intended local artifact from this reconciliation pass. Prior implemented items `ICA-001` through `ICA-007` are preserved as completed history.

## Roadmap Alignment

Architecture work should serve the roadmap phases in `ROADMAP.md`:

- Direct roadmap blockers:
  - `ICA-008` and `ICA-012` support Host portability -> Browser MCP managed lifecycle.
  - `ICA-014` supports Windows parity -> Windows agent cursor overlay and host IPC.
  - `ICA-010` supports Linux desktop parity -> Detached session-env repair and Host portability -> Detached launch breadth.
  - `ICA-011` supports Diagnostics and operator UX -> Curated VM runner profile set.
- Supporting infrastructure:
  - `ICA-009` supports Codex Desktop compatibility, release/deploy safety, and installed-plugin proof quality, but should not outrank direct browser/Windows/runtime blockers.
  - `ICA-013` is cleanup that makes `ICA-008`, `ICA-009`, and `ICA-011` cheaper; do it opportunistically or inside those slices, not as standalone roadmap work.
- Explicitly deferred:
  - A broad service daemon dispatcher split, generic Windows backend split, and `paths.rs` split are parked until an active roadmap slice needs them. The roadmap already has more concrete seams for browser lifecycle, Windows overlay IPC, VM profiles, and launch breadth.

## Triage Summary

| ID | Priority | Effort | Risk | Confidence | Status | Roadmap alignment | Cluster |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ICA-008 | P1 | M | Medium | High | ready-for-design | Host portability: Browser MCP managed lifecycle | `crates/sky-cua-service/src/browser/{bridge,session,cdp}.rs` |
| ICA-014 | P1 | M | Medium | High | ready-for-design | Windows parity: Windows agent cursor overlay and host IPC | overlay host platform modules and service overlay transport |
| ICA-010 | P2 | M | Medium | High | candidate | Linux desktop parity / Host portability: detached launch breadth | `crates/sky-cua-client/src/service_launcher.rs`, service health env contracts |
| ICA-011 | P2 | M | Medium | High | candidate | Diagnostics and operator UX: curated VM runner profile set | `scripts/run_gui_testing_vm_smoke.py` |
| ICA-012 | P2 | M | Medium | Medium | needs-validation | Host portability: Browser MCP managed lifecycle | browser snapshot/diagnostic wire contract |
| ICA-009 | P2 | M | Medium | High | candidate | Host portability: Codex Desktop compatibility and deploy proof | `scripts/_app_server_harness.py`, `scripts/deploy_release_plugin.py` |
| ICA-013 | P3 | S | Low | High | candidate | Enabler for roadmap-aligned work; not standalone roadmap scope | Python/Rust browser test fixture layout |

## Completed Prior Items

- [x] `ICA-001` Linux input action execution boundary.
- [x] `ICA-002` MCP tool specification/execution split.
- [x] `ICA-003` overlay state, host IPC, and synthetic cursor composition split.
- [x] `ICA-004` testing-VM smoke profile object extraction.
- [x] `ICA-005` Windows backend split validation note.
- [x] `ICA-006` Linux app-state capture planning boundary.
- [x] `ICA-007` service desktop request lane.

## Tasks

- [ ] **ICA-008: Introduce a browser bridge operation boundary**
  Priority: P1. Effort: M. Risk: Medium. Confidence: High. Status: ready-for-design
  Roadmap alignment: Host portability -> Browser MCP managed lifecycle.
  Cluster: `crates/sky-cua-service/src/browser/bridge.rs`, `crates/sky-cua-service/src/browser/session.rs`, `crates/sky-cua-service/src/browser/cdp.rs`, `crates/sky-cua-service/src/browser/tests.rs`
  Dependency category: Remote but owned, using ports and adapters; Evented or asynchronous boundary
  Problem: Browser operations repeatedly expose socket selection, bridge readiness diagnostics, deadlines, attach/enable, stale-session recovery, and action retry sequencing to individual operation families.
  Evidence:
  - `ROADMAP.md`: Browser MCP managed lifecycle needs isolated browser/profile launch, shipped snapshot/screenshot/action proof, deterministic cleanup, and later Codex Desktop delegation through the shared runtime.
  - `crates/sky-cua-service/src/browser/bridge.rs`: `list_tabs_from_bridge`, `open_tab_from_bridge`, `claim_tab_from_bridge`, `move_mouse_from_bridge`, and `run_cdp_action_from_bridge` each rebuild env selection, socket discovery, empty-socket diagnostics, and deadline setup.
  - `crates/sky-cua-service/src/browser/session.rs`: `attach_and_enable_tab_until`, stale-session reclaim, recovery diagnostics, and `move_mouse_from_socket` encode browser-session lifecycle policy.
  - `crates/sky-cua-service/src/browser/cdp.rs`: `cdp_action_from_socket` has its own recover-and-retry wrapper for CDP actions.
  - `crates/sky-cua-service/src/browser/tests.rs`: one large module covers socket inventory, protocol framing, tab operations, session recovery, coordinate conversion, action retries, snapshot behavior, and fake-server helpers.
  Why coupled: The current `user_chrome` bridge and the planned managed-browser bridge will need the same operation contract: resolve a target, pick or create a controllable endpoint, bind a tab/session, execute CDP or extension actions, retry recoverable stale state, and report stable diagnostics.
  Suggested first move: Add characterization tests around a small `BrowserBridgeExecutor` that resolves selection/sockets/deadline once, then proves open, claim, CDP action, and extension move-mouse paths preserve current diagnostics and retry behavior.
  Testing impact: New boundary tests should assert empty socket diagnostics, first-responsive-socket behavior, stale owner reclaim, debugger detach/reattach, CDP retry, and extension move-mouse retry. Existing operation-specific tests can be moved or reduced only after the executor tests cover the same behavior.
  Needs human decision: None for a service-internal boundary; keep public browser tool names and service wire shapes unchanged.
  Acceptance criteria:
  - [ ] A crate-local browser operation boundary owns socket selection, deadline construction, and disconnected/unsupported diagnostics.
  - [ ] Tab attach/enable/reclaim/retry policy is reusable by CDP actions and extension-only move-mouse.
  - [ ] The boundary can accept the future managed-browser endpoint without duplicating operation policy.
  - [ ] Existing `BrowserRequest`/`BrowserResponse` serde shapes remain unchanged.
  - [ ] Focused service tests cover the boundary before redundant lower-level tests are removed.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add characterization tests for current bridge selection and recovery behavior.
  - [ ] Sketch the target public interface.
  - [ ] Migrate one representative flow, preferably `browser_snapshot` or `browser_move_mouse`.
  - [ ] Expand migration to open/claim/navigate/click/type/key/scroll/screenshot.
  - [ ] Split shared fake-server fixtures from `browser/tests.rs` after replacement coverage passes.

- [ ] **ICA-014: Split overlay host platform transport for Windows overlay work**
  Priority: P1. Effort: M. Risk: Medium. Confidence: High. Status: ready-for-design
  Roadmap alignment: Windows parity -> Windows agent cursor overlay and host IPC.
  Cluster: `crates/sky-cua-overlay-host/src/lib.rs`, `crates/sky-cua-overlay-host/src/main.rs`, `crates/sky-cua-service/src/overlay/host.rs`, `crates/sky-cua-service/src/overlay.rs`
  Dependency category: Remote but owned, using ports and adapters; Global, nondeterministic, or platform dependency
  Problem: The roadmap's Windows overlay work needs platform-specific visible-overlay backends and IPC transports, but current overlay host process serving and service-side host client are Unix-socket centered, with non-Unix disabled rather than represented as a transport option.
  Evidence:
  - `ROADMAP.md`: Windows agent cursor overlay explicitly calls for cfg-scoping Linux-only cursor/compositor code, splitting service overlay transport by platform, adding a Windows visible overlay host, and proving Windows overlay behavior in a VM.
  - `crates/sky-cua-overlay-host/src/lib.rs`: Linux backend modules are cfg-scoped, while the top-level `OverlayHostBackend` selection still owns cross-platform backend selection in one enum.
  - `crates/sky-cua-overlay-host/src/main.rs`: `serve --socket <path>` uses `UnixListener` on Unix and reports socket mode as not implemented on non-Unix.
  - `crates/sky-cua-service/src/overlay/host.rs`: `OverlayHostConnection::from_service_socket` creates a Unix socket process client on Unix and disables host IPC on non-Unix.
  - `crates/sky-cua-service/src/overlay.rs`: `OverlayController` already coordinates host IPC separately from synthetic screenshot cursor composition, so the next boundary can stay transport-focused.
  Why coupled: Visible overlay behavior, host process lifecycle, and transport mechanics need to evolve together for Windows, but Linux compositor backends and Unix socket assumptions should not leak into the Windows host or service client.
  Suggested first move: Define a platform-neutral `OverlayHostTransport` or `OverlayHostClient` boundary with Unix-socket and Windows named-pipe/localhost implementations, then keep Linux compositor backends and a future Windows layered-window backend behind platform-specific modules.
  Testing impact: Add transport contract tests for request/reply, protocol mismatch, host unavailable, failed request resets, and cleanup. Keep existing Unix socket round-trip tests as the Unix adapter proof; add Windows adapter tests when the transport lands.
  Needs human decision: Choose named pipe versus localhost TCP for Windows service-to-overlay-host IPC before implementation.
  Acceptance criteria:
  - [ ] Service overlay host IPC is selected through an explicit platform transport boundary.
  - [ ] Linux Unix-socket behavior, diagnostics, and cleanup remain unchanged.
  - [ ] Non-Unix no longer means "overlay host process IPC is not implemented" once a Windows transport is selected.
  - [ ] Linux-only compositor and cursor-hiding code stays cfg-scoped outside Windows host modules.
  - [ ] The boundary can host a Windows transparent layered-window backend without changing service-level cursor state APIs.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Decide Windows transport: named pipe or localhost TCP.
  - [ ] Add contract tests around the current Unix transport behavior.
  - [ ] Extract the service-side transport/client interface.
  - [ ] Extract host-side serving transport from overlay backend selection.
  - [ ] Add the Windows transport adapter before implementing the Windows visible overlay backend.

- [ ] **ICA-010: Extract service launch environment repair and health matching policy**
  Priority: P2. Effort: M. Risk: Medium. Confidence: High. Status: candidate
  Roadmap alignment: Linux desktop parity -> Detached session-env repair; Host portability -> Detached launch breadth.
  Cluster: `crates/sky-cua-client/src/service_launcher.rs`, `crates/sky-cua-service/src/daemon.rs`, `crates/sky-cua-service/src/browser/sockets.rs`, `crates/sky-cua-platform/src/paths.rs`
  Dependency category: Global, nondeterministic, or platform dependency
  Problem: `service_launcher.rs` owns IPC client caching, child process lifecycle, endpoint resolution, Linux desktop env reconstruction, browser env freshness checks, stale-daemon rejection, and health response comparison in one large file.
  Evidence:
  - `ROADMAP.md`: Detached session-env repair still needs a stripped-env VM runner profile, and Host portability still calls for detached launch breadth across more desktop/session launchers.
  - `crates/sky-cua-client/src/service_launcher.rs`: desktop and browser health env keys are local constants beside socket I/O and child process state.
  - `crates/sky-cua-client/src/service_launcher.rs`: `probe_desktop_env_vars` reconstructs XDG runtime, DBus, Wayland, X11, logind desktop metadata, systemd user env, and PATH.
  - `crates/sky-cua-service/src/daemon.rs`: `desktop_env_values_present` independently enumerates desktop health env keys.
  - `crates/sky-cua-service/src/browser/sockets.rs`: browser socket env keys and browser selection are daemon-side constants also mirrored by client health matching.
  Why coupled: Launch decisions and stale-daemon decisions depend on the same env contract that the daemon reports through health. Today that contract is split across client and service files, while the client file also owns unrelated stream and process mechanics.
  Suggested first move: Move desktop/browser health key definitions and launch-env repair into a focused client module, or a small platform contract module if both client and service must share the key list. Keep `ServiceClient` focused on connect/spawn/call orchestration.
  Testing impact: Preserve current launcher tests for repaired env propagation and stale daemon rejection, then add direct tests for the launch environment policy with env guards. Pair this with the roadmap's stripped-env VM profile when the behavior changes.
  Needs human decision: Decide whether health key lists are platform contracts in `sky-cua-platform` or crate-local duplication with explicit tests is acceptable.
  Acceptance criteria:
  - [ ] Env repair policy is testable without constructing a `ServiceClient`.
  - [ ] Client and daemon health key lists cannot drift silently.
  - [ ] `ServiceClient` no longer mixes stream caching and Linux session reconstruction.
  - [ ] Existing service socket/TCP override behavior remains unchanged.
  - [ ] The roadmap stripped-env profile has a clear hook for validating launch repair behavior.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add tests that pin health env key lists and stale-daemon comparison behavior.
  - [ ] Extract `LaunchEnvironment` or equivalent.
  - [ ] Rewire service spawning and startup health checks through the new policy.
  - [ ] Keep endpoint/path ownership in `sky-cua-platform::paths`.

- [ ] **ICA-011: Model GUI VM smoke profiles as first-class profile descriptors**
  Priority: P2. Effort: M. Risk: Medium. Confidence: High. Status: candidate
  Roadmap alignment: Diagnostics and operator UX -> Curated VM runner profile set.
  Cluster: `scripts/run_gui_testing_vm_smoke.py`, `scripts/testing-vm/profiles/**`, `scripts/test_python_harness_helpers.py`
  Dependency category: Local-substitutable; Global, nondeterministic, or platform dependency
  Problem: The VM runner still centralizes profile registry, host build/sync, remote shell construction, portal reset/preauthorization, special profile dispatch, libvirt framebuffer capture, marker polling, remote JSON reads, and host summary writing.
  Evidence:
  - `ROADMAP.md`: Diagnostics and operator UX calls for a curated VM runner profile set covering text-readback smokes, detached session-env, and the current cursor matrix.
  - `scripts/run_gui_testing_vm_smoke.py`: `PROFILES` is a flat tuple, while `main` branches for preauthorization and special profiles before falling back to `run_remote_profile`.
  - `scripts/run_gui_testing_vm_smoke.py`: remote shell strings for process reset, display wake, portal refresh, preauthorization, and generic profile execution repeat runtime-dir and desktop-env setup.
  - `scripts/run_gui_testing_vm_smoke.py`: COSMIC and KWin host-framebuffer proof functions embed remote scripts, marker waits, VM screenshots, remote JSON reads, marker probes, and summary writing.
  - `scripts/test_python_harness_helpers.py`: VM runner tests monkeypatch many central `main` dependencies, which is a sign the profile boundary is not explicit enough.
  Why coupled: A profile has preconditions, desktop env, remote command, artifact protocol, readiness marker, host proof, and acceptance criteria. Those concepts are currently encoded through central branching and long inline scripts.
  Suggested first move: Introduce a `VmProfile` descriptor and `RemoteRunner` helper for desktop env exports, SSH invocation, runtime-dir setup, remote JSON reads, and marker waits; migrate one special host-framebuffer profile as proof.
  Testing impact: Add descriptor/command-construction tests before changing dispatch. Existing profile behavior should remain covered by pure tests and the relevant live VM smoke when implementation happens.
  Needs human decision: Decide the trimmed pre-merge profile set before treating the descriptor registry as final.
  Acceptance criteria:
  - [ ] Profile registry entries describe preauthorization, remote command, host proof needs, and curated-set membership without central `if profile == ...` growth.
  - [ ] Remote runtime-dir and desktop-env setup is shared.
  - [ ] KWin and COSMIC host-framebuffer proof summaries keep the same JSON fields.
  - [ ] Existing `--profile all` semantics are unchanged.
  - [ ] The roadmap's curated profile set can be read from the profile descriptors.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add characterization tests for profile descriptors and summary JSON.
  - [ ] Extract `RemoteRunner`.
  - [ ] Migrate one special proof profile.
  - [ ] Migrate remaining special proof profiles.
  - [ ] Mark the curated profile membership once the roadmap decision is made.

- [ ] **ICA-012: Type browser snapshot and diagnostic severity contracts**
  Priority: P2. Effort: M. Risk: Medium. Confidence: Medium. Status: needs-validation
  Roadmap alignment: Host portability -> Browser MCP managed lifecycle.
  Cluster: `crates/sky-cua-platform/src/model/browser.rs`, `crates/sky-cua-service/src/browser/snapshot.rs`, `crates/sky-cua-client/src/mcp_tools/browser/response.rs`, browser tests
  Dependency category: Remote but owned, using ports and adapters
  Problem: Browser snapshot summaries and MCP error decisions depend on raw JSON keys and string diagnostic codes rather than typed snapshot fields or shared diagnostic severity.
  Evidence:
  - `ROADMAP.md`: Browser MCP managed lifecycle requires the shipped snapshot/screenshot/action tool sequence to run in a managed context; snapshot and diagnostic contracts need to survive both user-profile and managed-profile targets.
  - `crates/sky-cua-platform/src/model/browser.rs`: `BrowserSnapshotResponse.snapshot` is `Option<serde_json::Value>`.
  - `crates/sky-cua-client/src/mcp_tools/browser/response.rs`: snapshot summary code manually probes `viewport`, `text`, `elements`, `index`, `role`, `name`, `href`, and `bounds` JSON keys.
  - `crates/sky-cua-client/src/mcp_tools/browser/response.rs`: fatal/error policy is split across `is_fatal_browser_diagnostic`, per-response `*_is_error` functions, and `browser_diagnostics_are_error`.
  - `crates/sky-cua-service/src/browser/diagnostics.rs` and `crates/sky-cua-service/src/browser/bridge.rs`: service-side validation emits string-coded diagnostics that the client reinterprets for MCP `isError`.
  Why coupled: The service produces the browser snapshot and diagnostics, but the client owns text rendering and error classification by assuming JSON field names and string code meanings.
  Suggested first move: Validate the current snapshot shape against real browser smoke artifacts, then introduce typed `BrowserPageSnapshot`, `BrowserViewport`, and `BrowserElementSummary` structs or a shared browser diagnostic severity helper.
  Testing impact: Add serde compatibility tests and golden summary tests before changing the public structured content. Keep `Value` compatibility only if external hosts rely on arbitrary extra fields.
  Needs human decision: Decide whether `browser_snapshot` structured content is public enough to require a compatibility transition with both typed fields and legacy `snapshot` JSON.
  Acceptance criteria:
  - [ ] Snapshot producer and client renderer share a typed contract or a documented compatibility shim.
  - [ ] Browser diagnostic severity/error policy is defined once and reused by all browser response shapers.
  - [ ] Existing browser MCP structured output remains compatible or migration is documented.
  - [ ] Privacy-sensitive snapshot extraction remains covered by tests.
  - [ ] The contract supports both existing user-profile browser targets and future managed-browser targets.
  Work checklist:
  - [ ] Validate external compatibility expectations for `snapshot`.
  - [ ] Add characterization tests for current snapshot JSON and summary text.
  - [ ] Sketch typed snapshot and diagnostic severity options.
  - [ ] Migrate one response shaper behind the shared policy.
  - [ ] Remove stringly duplicated error lists only after all browser tools use the shared policy.

- [ ] **ICA-009: Extract a shared Codex app-server JSON-RPC client**
  Priority: P2. Effort: M. Risk: Medium. Confidence: High. Status: candidate
  Roadmap alignment: Host portability -> Codex Desktop compatibility; Diagnostics and operator UX proof quality.
  Cluster: `scripts/_app_server_harness.py`, `scripts/deploy_release_plugin.py`, `scripts/test_python_harness_helpers.py`
  Dependency category: Evented or asynchronous boundary; Local-substitutable
  Problem: Rich smokes and release deploy both drive `codex app-server` over stdio JSON-RPC, but they implement separate process readers, request id handling, initialization sequencing, timeout behavior, stderr capture, and shutdown policy.
  Evidence:
  - `ROADMAP.md`: Codex Desktop compatibility is shipped, and remaining diagnostics/operator UX work depends on honest proof harnesses rather than process-only checks.
  - `scripts/_app_server_harness.py`: `JsonRpcProcess` owns line-oriented app-server stdio, while `run_rich_app_server_turn` builds env, starts the server, records transcripts/timing, handles server requests, and performs initialize/status/thread/turn sequencing.
  - `scripts/deploy_release_plugin.py`: `AppServerClient` independently starts `codex app-server --listen stdio://`, spawns stdout/stderr reader threads, tracks notifications, sends initialize/initialized, and performs plugin install/reload requests.
  - `scripts/AGENTS.md`: rich `codex app-server` is the current installed-plugin acceptance lane, so protocol drift here affects both deploy and proof harnesses.
  Why coupled: Both flows share the same transport lifecycle and failure modes, but only the smoke harness has transcript/timing artifacts and only release deploy has queue-backed stderr handling.
  Suggested first move: Create `_codex_app_server.py` with request/notify, initialize/initialized, timeout/error handling, stderr capture, transcript hooks, and clean shutdown; keep turn policy in `_app_server_harness.py` and release operations in `deploy_release_plugin.py`.
  Testing impact: Add pure tests for request id matching, notification buffering, server-request handling hooks, timeout stderr reporting, and close/kill behavior with fake subprocess streams. Existing deploy and rich-smoke helper tests should migrate to the shared client.
  Needs human decision: None unless release deploy intentionally needs different `--listen` arguments than the rich harness.
  Acceptance criteria:
  - [ ] Rich smokes and release deploy use one app-server transport/client implementation.
  - [ ] Release deploy keeps plugin install and MCP reload behavior unchanged.
  - [ ] Rich smokes keep transcript, request log, timing, and elicitation/approval handling unchanged.
  - [ ] The client never blocks indefinitely on stderr or stdout during process exit.
  - [ ] This refactor does not delay direct Browser MCP managed lifecycle or Windows overlay work.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add fake-process tests for current app-server request/notification behavior.
  - [ ] Introduce the shared transport with compatibility wrappers.
  - [ ] Migrate release deploy first because its operation set is smaller.
  - [ ] Migrate rich smokes and preserve artifact file outputs.
  - [ ] Remove duplicated client classes after both paths pass Python checks.

- [ ] **ICA-013: Split broad helper tests into subsystem fixtures**
  Priority: P3. Effort: S. Risk: Low. Confidence: High. Status: candidate
  Roadmap alignment: Enabler for `ICA-008`, `ICA-009`, and `ICA-011`; not standalone roadmap scope.
  Cluster: `scripts/test_python_harness_helpers.py`, `crates/sky-cua-service/src/browser/tests.rs`
  Dependency category: In-process; Local-substitutable
  Problem: The largest test modules import and exercise many unrelated harness/runtime helpers, so refactors have to navigate broad import-time dependencies and mixed monkeypatch fixtures.
  Evidence:
  - `scripts/test_python_harness_helpers.py`: imports plugin bundle helpers, deploy scripts, install scripts, live desktop smokes, Chrome host smokes, VM runner, marketplace tools, and MCP stdio helpers in one module.
  - `scripts/test_python_harness_helpers.py`: one file covers VM provisioning, remote profile command construction, MCP install behavior, build/release deploy behavior, cursor geometry, Chrome native-host helpers, and smoke config.
  - `crates/sky-cua-service/src/browser/tests.rs`: one large module mixes sockets, protocol frame I/O, bridge operations, session recovery, snapshot expression privacy, coordinate conversion, and fake server helpers.
  Why coupled: The tests have become fixture libraries plus many subsystem suites, which makes boundary refactors noisier and hides which behavior belongs to which module.
  Suggested first move: Split tests only when doing an adjacent roadmap-aligned slice, or when a focused refactor first needs shared fixtures. Do not lead with this as independent cleanup.
  Testing impact: This is mostly test architecture. The acceptance check is that the same focused test commands pass and test names remain discoverable by subsystem.
  Needs human decision: None.
  Acceptance criteria:
  - [ ] Python helper tests are split into focused modules as part of the relevant implementation slices.
  - [ ] Browser service Rust tests have shared fake-server fixtures plus focused modules for sockets, session, actions, snapshot, and coordinates.
  - [ ] No production behavior changes are mixed into a pure test split.
  - [ ] Existing narrow test commands still pass.
  Work checklist:
  - [ ] Validate import dependencies and fixture sharing.
  - [ ] Add shared test support modules when a roadmap-aligned slice needs them.
  - [ ] Move one coherent test group first.
  - [ ] Continue moving groups in small reviewable patches.
  - [ ] Remove stale imports and duplicate monkeypatch setup.

## Recommended Design Direction For ICA-008

Recommended option: a service-internal `BrowserBridgeExecutor` plus a `BrowserSessionBinding` helper.

Minimal interface:

```rust
let executor = BrowserBridgeExecutor::from_env(timeout)?;
executor.run_cdp(target, tab_id, BrowserCdpAction::Snapshot).await
```

This hides socket selection and deadline setup, but session recovery may still leak into CDP and extension action modules.

Flexible interface:

```rust
let executor = BrowserBridgeExecutor::from_env(timeout)?;
executor.with_bound_tab(target, tab_id, |tab| async move {
    tab.run_cdp(BrowserCdpAction::Snapshot).await
}).await
```

This best matches the current coupling: the executor owns bridge selection, while the bound tab owns claim/attach/enable/recover/retry. It supports CDP actions, extension-only operations like `moveMouse`, and the planned managed-browser endpoint without making every caller know stale-session recovery.

Default-case interface:

```rust
browser_ops::snapshot(target, tab_id).await
```

This is simplest for callers, but risks keeping the implementation as many shallow wrapper functions unless the operation module still uses a deeper executor internally.

Ports/adapters interface:

```rust
trait BrowserBridgeTransport {
    async fn request(&mut self, method: &str, params: Value, deadline: Instant) -> Result<Value, DiagnosticEntry>;
}
```

This is useful for tests and future managed-browser transports, but it should stay below the executor. Starting with a transport trait alone would not remove the exposed session policy.

Chosen shape: use the flexible interface internally, with fakeable transport/socket adapters beneath it. It keeps the public service wire unchanged, gives stale-session behavior one owner, and matches the roadmap's managed lifecycle by allowing `user_chrome` and managed browser endpoints to share operation policy.

## RFC Draft For ICA-008

Title: Refactor browser bridge operations behind a `BrowserBridgeExecutor`

Problem: Browser MCP service operations have a clean module split on disk, but important runtime policy is still repeated across operations: socket family selection, bridge readiness diagnostics, deadlines, tab attach/enable, stale-session reclaim, and recoverable CDP retry. This makes the roadmap's managed browser lifecycle more error-prone because a new managed endpoint could duplicate the current `user_chrome` policy.

Evidence:
- `ROADMAP.md` calls for browser managed lifecycle: launch/own an isolated browser/profile, run the shipped snapshot/screenshot/action sequence, clean up deterministically, and eventually delegate Codex Desktop's companion Browser Use adapter through the shared runtime.
- `crates/sky-cua-service/src/browser/bridge.rs` repeats socket selection and deadline setup across bridge entrypoints.
- `crates/sky-cua-service/src/browser/session.rs` owns attach/enable/reclaim policy but `move_mouse_from_socket` still wraps recovery separately.
- `crates/sky-cua-service/src/browser/cdp.rs` wraps CDP action recovery separately.
- `crates/sky-cua-service/src/browser/tests.rs` mixes fixture support and tests for many behavior layers.

Proposed interface: introduce a crate-local `BrowserBridgeExecutor` that resolves selection, sockets, diagnostics, and deadlines once. Add a `BrowserSessionBinding` or equivalent that owns claim/attach/enable/recover/retry and exposes `run_cdp` plus `run_extension_action` for move-mouse. Later managed-browser work should provide another endpoint/transport adapter to the same operation boundary.

Dependency strategy: Remote but owned, using ports and adapters. Production uses the current Unix socket/native-host bridge transport for `user_chrome`. Managed-browser work can add a process/profile owner and transport adapter. Tests use fake socket/transport fixtures and keep contract-style tests for frame protocol and request IDs.

Testing strategy: Add boundary tests for empty socket diagnostics, first responsive socket selection, stale-owner reclaim, debugger detach/reattach, CDP retry, and extension move-mouse retry. Add managed-endpoint characterization when `plans/browser_use_mcp.md` is implemented. Only then split or delete redundant operation-level tests.

Migration plan: Introduce the executor behind existing functions, migrate `snapshot` or `move_mouse` first, migrate remaining browser operations, then split shared test fixtures from the monolithic browser test module.

Risks and non-goals: Do not change MCP tool names, service wire shapes, diagnostic codes, or browser target semantics. Do not implement the managed-browser lifecycle in this refactor; the boundary should make that roadmap work smaller.

Acceptance criteria:
- Existing browser MCP tests pass.
- Existing browser live smoke commands remain the proof gate for behavior changes.
- Browser operation functions no longer duplicate socket/deadline/recovery setup.
- Public structured content and diagnostics remain compatible.
- `plans/browser_use_mcp.md` can plug its managed endpoint into the operation boundary without reimplementing current session recovery.

## Parking Lot

- [ ] Revisit `crates/sky-cua-service/src/daemon.rs` after browser lifecycle settles; a request dispatcher split may be useful, but `ICA-007` already removed the daemon-wide lock and the browser route is changing quickly.
- [ ] Revisit a broad Windows backend split only after active Windows capture-ladder or Windows overlay work needs it; avoid imposing a Linux-derived boundary too early.
- [ ] Revisit `crates/sky-cua-platform/src/paths.rs` if more transport/state policy lands there; today it is high impact but still small enough to review.
- [ ] Keep Wayland fallback vision anchors and CDUL Linux enhancements in their ExecPlans unless implementation exposes a deeper architectural seam; they are product/runtime feature work first, not standalone architecture backlog items.

<!-- improve-codebase-architecture:end -->
