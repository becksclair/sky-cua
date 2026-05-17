# TODO: Improve Codebase Architecture

This file tracks architecture improvement candidates found during codebase review. Treat it as a triage backlog: validate evidence first, then design and implement incrementally.

<!-- improve-codebase-architecture:start -->

Generated: 2026-05-17  
Scope: repository root on `main`, with emphasis on Rust runtime boundaries and Python smoke harnesses  
Analysis notes: Read-only architecture pass plus this triage artifact. No source refactors were made; live desktop or VM smokes were not run.

## Triage Summary

| ID | Priority | Effort | Risk | Confidence | Status | Cluster |
| --- | --- | --- | --- | --- | --- | --- |
| ICA-001 | P1 | M | Medium | High | ready-for-design | `crates/sky-cua-linux/src/backend.rs` action and coordinate routing |
| ICA-002 | P2 | M | Medium | High | candidate | `crates/sky-cua-client/src/mcp_server.rs` MCP tool definitions and handlers |
| ICA-003 | P2 | M | Medium | Medium | needs-validation | `crates/sky-cua-service/src/overlay.rs` agent cursor overlay controller |
| ICA-004 | P2 | M | Medium | High | candidate | `scripts/run_gui_testing_vm_smoke.py` GUI VM profile runner |
| ICA-005 | P3 | S | Low | High | needs-validation | `crates/sky-cua-windows/src/backend.rs` Windows backend monolith |
| ICA-006 | P2 | L | Medium | High | candidate | `crates/sky-cua-linux/src/backend.rs` app-state capture/snapshot pipeline |

## Tasks

- [ ] **ICA-001: Introduce a Linux input action execution boundary**  
  Priority: P1; Effort: M; Risk: Medium; Confidence: High; Status: ready-for-design  
  Cluster: `crates/sky-cua-linux/src/backend.rs` (`execute_action`, `click`, `scroll`, `drag`, `type_text`, `press_key`, `set_value`, point mapping helpers)  
  Dependency category: Global, nondeterministic, or platform dependency  
  Problem: Linux action execution and coordinate planning are concentrated in one backend orchestration file, so maintainers and unit tests must reason about semantic AT-SPI attempts, physical backend selection, X11 activation, portal lifecycle diagnostics, KDE clipboard fallback, backend-specific coordinate conversion, and user-facing outcome text as one large surface.  
  Evidence:
  - `crates/sky-cua-linux/src/backend.rs`: `execute_action` dispatches every `ActionName` directly to methods on `LinuxDesktopBackend`.
  - `crates/sky-cua-linux/src/backend.rs`: `click`, `secondary_click`, `scroll`, `drag`, `type_text`, `press_key`, and `set_value` each select backends, map points, call platform adapters, and construct user-facing outcomes.
  - `crates/sky-cua-linux/src/backend.rs`: backend and coordinate helpers such as `effective_keyboard_input_backend_for_target`, `point_for_element_for_backend`, `point_from_screenshot_pixels`, and `linux_virtual_point_from_screenshot_pixels` keep important keyboard-routing and coordinate-conversion rules outside the action methods that depend on them.
  - `crates/sky-cua-linux/src/backend.rs`: the test module imports many private helpers from action routing, capture diagnostics, coordinate mapping, and window matching, indicating unit tests are coupled to internals instead of a smaller action boundary.
  Why coupled: Physical action behavior depends on shared state and sequencing: resolved snapshot elements, capture metadata, input backend selection, X11 window focus, portal lifecycle diagnostics, KDE clipboard fallback, and backend-specific coordinate spaces.  
  Recommended design: Introduce a crate-local, Linux-only, plan-first `LinuxActionExecutor` under `crates/sky-cua-linux/src/actions/`. Keep route planning and coordinate mapping in the executor, and keep portal, XTest, Linux virtual input, and window activation as thin fakeable ports/adapters. Do not create a shared Windows/Linux action abstraction in the first pass.  
  Suggested first move: Add characterization tests around the proposed executor using fake input adapters for portal, XTest, and Linux virtual input; move only point planning plus `click` first.  
  Testing impact: New boundary tests should assert action plans/outcomes for snapshot coordinates, explicit screen coordinates, portal diagnostics, and backend-unavailable errors. Existing private-helper tests should move only after replacement coverage proves the same observable behavior.  
  Needs human decision: Confirm that action execution should stay entirely inside `sky-cua-linux` and not become a platform-neutral trait shared with Windows yet.  
  Acceptance criteria:
  - [ ] A small action boundary exists with one crate-local `pub(crate)` entry point for Linux `ActionRequest` execution.
  - [ ] Portal, XTest, and Linux virtual input decisions are tested through the boundary with fake adapters or explicit local substitutes.
  - [ ] `LinuxDesktopBackend::execute_action` becomes mostly environment preparation plus a boundary call.
  - [ ] No user-facing `ActionOutcome` messages, diagnostic codes, or coordinate semantics change without explicit tests.
  - [ ] Semantic-first behavior is preserved and tested: AT-SPI default action for click, semantic action tools, and semantic `set_value` still run before physical fallback where current behavior does so.
  - [ ] KDE clipboard text fallback, `perform_secondary_action`, heuristics-backed `set_value` fallback, and portal lifecycle diagnostics are covered by boundary tests or explicit local substitutes.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add characterization tests for current click, drag, scroll, type, key, and set-value routing.
  - [ ] Sketch the target public interface and dependency injection shape.
  - [ ] Migrate point planning plus `click` as the first proof flow.
  - [ ] Migrate remaining physical input actions.
  - [ ] Move backend-specific helpers into the new module once tests pass.
  - [ ] Remove redundant private-helper tests only after replacement boundary tests pass.
  - [ ] Update `crates/sky-cua-linux/AGENTS.md` if the orchestration contract changes.

- [ ] **ICA-002: Split MCP tool specification from MCP tool execution**  
  Priority: P2; Effort: M; Risk: Medium; Confidence: High; Status: candidate  
  Cluster: `crates/sky-cua-client/src/mcp_server.rs` (`handle_tool_call`, `tool_definitions`, schema helpers, parsers, summaries)  
  Dependency category: Remote but owned, using ports and adapters  
  Problem: MCP schema construction, argument parsing, service request mapping, service response rendering, and JSON-RPC framing live in one large protocol module, making tool surface changes harder to review for wire compatibility.  
  Evidence:
  - `crates/sky-cua-client/src/mcp_server.rs`: `handle_tool_call` maps tool names to service calls and renders `structuredContent`/`content`.
  - `crates/sky-cua-client/src/mcp_server.rs`: `tool_definitions` builds the full tool list and embeds model-capability-dependent schema text.
  - `crates/sky-cua-client/src/mcp_server.rs`: schema helpers and argument parsers live beside framing helpers such as `read_message` and `write_message`.
  - `crates/sky-cua-client/src/mcp_server.rs`: tests assert both schema shape and protocol framing from the same module.
  Why coupled: Tool names, schemas, argument parsing, service requests, and response rendering must evolve together, while JSON-RPC framing is a separate protocol concern.  
  Suggested first move: Create an internal `tool_registry` module that owns tool specs and maps parsed calls to `ServiceRequest`; keep framing in `mcp_server.rs`.  
  Testing impact: Add registry tests that cover each tool's schema, parsed request, and response rendering. Keep framing tests in `mcp_server.rs`.  
  Needs human decision: Confirm that the existing convention "MCP JSON-RPC behavior and tool text/structured output live in `mcp_server.rs`" permits a sibling internal `tool_registry` or `mcp_tools` module, with `mcp_server.rs` remaining the protocol entrypoint.  
  Acceptance criteria:
  - [ ] Tool definitions and handler mapping are generated from one registry source of truth.
  - [ ] `mcp_server.rs` no longer needs to know each tool's schema details.
  - [ ] Existing tool names and schema compatibility tests remain green.
  - [ ] Registry or golden tests cover `tools/list` for both image-capable and text-only model sessions, including omission of `capture_screen` when images are disabled.
  - [ ] The registry does not depend on `MessageFraming`, `read_message`, or `write_message`; framing and session initialization tests remain in the protocol module.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add a registry-level characterization test for current `tools/list`.
  - [ ] Move one non-action tool, such as `doctor`, behind the registry.
  - [ ] Move action tools as a grouped family.
  - [ ] Preserve model image capability gating for `get_app_state`.
  - [ ] Keep JSON-RPC framing tests in the protocol module.

- [ ] **ICA-003: Separate overlay state, host IPC, and synthetic cursor composition**  
  Priority: P2; Effort: M; Risk: Medium; Confidence: Medium; Status: needs-validation  
  Cluster: `crates/sky-cua-service/src/overlay.rs` visible-overlay host IPC and synthetic screenshot cursor composition  
  Dependency category: Remote but owned, using ports and adapters; Local-substitutable  
  Problem: `OverlayController` owns service-visible cursor state, overlay host process lifecycle, host protocol replies, capture hide/restore policy, action-to-cursor mapping, and screenshot cursor composition in one large module.  
  Evidence:
  - `crates/sky-cua-service/src/overlay.rs`: `OverlayController` mutates state and sends host messages from methods such as `set_state`, `hide`, `show`, `update_from_action`, `prepare_for_capture`, and `restore_after_capture`.
  - `crates/sky-cua-service/src/overlay.rs`: `OverlayHostConnection` and `ProcessOverlayHostClient` implement host process IPC and diagnostics in the same file as service state.
  - `crates/sky-cua-service/src/overlay.rs`: `compose_synthetic_cursor` opens and rewrites screenshot images in the same module as host IPC.
  - `crates/sky-cua-service/src/overlay.rs`: tests cover host process round trips, state updates, action-derived cursor points, and image composition from the same private module.
  Why coupled: Cursor state, host capabilities, capture exclusion, and synthetic fallback share one user-facing status, but their dependencies differ: process IPC, action metadata, image I/O, and time.  
  Suggested first move: Extract screenshot cursor composition first because `compose_synthetic_cursor` is already focused and independently image-testable, then split host IPC behind an `OverlayHost` port once controller-level behavior is preserved.  
  Testing impact: Boundary tests should assert host-unavailable diagnostics, capture hide/restore behavior, and synthetic cursor composition separately.  
  Needs human decision: Confirm whether visible overlay host behavior and screenshot synthetic cursor should remain one service-level feature contract.  
  Acceptance criteria:
  - [ ] Host IPC can be tested with a fake host without constructing the full controller.
  - [ ] Synthetic cursor composition has focused image tests independent of host process state.
  - [ ] `OverlayController` reads like service policy rather than transport and image plumbing.
  - [ ] Existing controller-level integration tests remain for visible-overlay host round trip, hide/show, capture guard restore, and host-unavailable-is-diagnostic-not-action-failure behavior.
  - [ ] The extracted synthetic cursor module has no dependency on host process state and preserves `AgentCursorSyntheticFailed` and `AgentCursorSyntheticOutOfBounds` diagnostics.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add or preserve characterization tests for host unavailable, protocol mismatch, capture hide/restore, and synthetic out-of-bounds diagnostics.
  - [ ] Extract host IPC behind a narrow port.
  - [ ] Extract screenshot cursor composition into a pure/local-I/O module.
  - [ ] Rewire `OverlayController` to coordinate the two smaller boundaries.

- [ ] **ICA-004: Model testing-VM smoke profiles as profile objects instead of ad hoc runner branches**  
  Priority: P2; Effort: M; Risk: Medium; Confidence: High; Status: candidate  
  Cluster: `scripts/run_gui_testing_vm_smoke.py`, `scripts/test_python_harness_helpers.py`, `scripts/testing-vm/profiles/**`  
  Dependency category: Local-substitutable; Global, nondeterministic, or platform dependency  
  Problem: The VM smoke runner mixes CLI parsing, host build/sync, SSH command construction, portal reset/preauthorization, profile dispatch, framebuffer capture, remote marker polling, and profile-specific result validation in one large operator script.  
  Evidence:
  - `scripts/run_gui_testing_vm_smoke.py`: `main` parses profile options and directly handles build, sync, portal refresh, preauthorization, and profile dispatch.
  - `scripts/run_gui_testing_vm_smoke.py`: special profiles are selected through branch checks before falling back to `run_remote_profile`.
  - `scripts/run_gui_testing_vm_smoke.py`: the COSMIC patched and transparent-xcursor host proof functions duplicate remote overlay-host script setup, ready-file waiting, framebuffer capture, JSON summary writing, and marker probing.
  - `scripts/test_python_harness_helpers.py`: helper tests exercise many scattered runner and smoke details in one large test file.
  Why coupled: Host-framebuffer proof profiles share a lifecycle: prepare host/guest environment, launch a remote proof script, wait for proof markers, collect local and remote artifacts, capture VM framebuffers, run marker probes, and emit stable summary JSON. Generic shell-backed profiles can remain a simpler path until the special proof profiles are factored.  
  Suggested first move: Define a small `SmokeProfile`/`ProfileResult` protocol and migrate one special host-framebuffer proof profile into it without changing CLI flags.  
  Testing impact: Unit tests should validate profile command generation, remote artifact paths, and result interpretation through profile objects. Existing subprocess monkeypatch tests can move after equivalent profile tests exist.  
  Needs human decision: None, unless CLI output or artifact directory names are considered externally stable.  
  Acceptance criteria:
  - [ ] For host-framebuffer proof profiles, common SSH environment construction, artifact directory setup, ready-marker polling, framebuffer capture, remote JSON loading, marker-probe serialization, and host-summary writing are shared.
  - [ ] Profile-specific code supplies preconditions, mode-specific validation, and summary fields.
  - [ ] Existing CLI flags and artifact JSON fields remain compatible.
  - [ ] Summary JSON fields, artifact directory names, exit-code semantics, and CLI flags are covered by characterization tests for both COSMIC host-proof modes and the KWin system-install proof before behavior is moved.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add characterization tests for one profile's generated commands and summary fields.
  - [ ] Introduce profile protocol/data classes.
  - [ ] Migrate one special profile.
  - [ ] Migrate the second duplicated profile.
  - [ ] Consider splitting the helper test file after behavior coverage is stable.

- [ ] **ICA-005: Validate a Windows-local action/capture/windowing split**  
  Priority: P3; Effort: S; Risk: Low; Confidence: High; Status: needs-validation  
  Cluster: `crates/sky-cua-windows/src/backend.rs`, `crates/sky-cua-windows/src/uia.rs`  
  Dependency category: Global, nondeterministic, or platform dependency  
  Problem: The Windows backend is a large v1 module combining environment probing, window enumeration, UIA fallback, GDI capture, SendInput, RDP-safe window-message input, coordinate mapping, and Win32 error handling.  
  Evidence:
  - `crates/sky-cua-windows/src/backend.rs`: `get_app_state` probes input/semantic availability, selects windows, captures screenshots, builds fallback trees, and reports diagnostics.
  - `crates/sky-cua-windows/src/backend.rs`: `execute_action` chooses UIA, SendInput, or window-message fallbacks.
  - `crates/sky-cua-windows/src/backend.rs`: `execute_send_input_action` and `execute_window_message_action` duplicate the action surface for two input transports.
  - `crates/sky-cua-windows/src/backend.rs`: capture helpers and raw input helpers live below the same file-level boundary.
  Why coupled: Windows app-state and action execution share selected window identity, capture coordinate mapping, and transport availability, but the code is newer and may still be changing quickly.  
  Suggested first move: Write a short validation note that maps current Windows responsibilities and decides whether to refactor now, defer, or only split mechanical modules such as `capture`, `input`, and `windowing`.  
  Testing impact: Characterize SendInput vs window-message routing and capture coordinate mapping before any later code movement.  
  Needs human decision: Decide whether Windows v1 is stable enough to refactor now or should remain easy to edit while behavior is still being proved.  
  Acceptance criteria:
  - [ ] A short validation note decides whether to refactor now, defer, or only split mechanical modules.
  - [ ] No Linux-derived abstraction is imposed unless it reduces Windows-specific complexity.
  - [ ] Any split preserves current Windows wire shapes and fallback diagnostics.
  - [ ] The validation note explicitly maps current Windows responsibilities: UIA-first semantic path, SendInput transport, WindowsMessages/RDP transport, GDI capture, window enumeration/selection, and stream-pixel-to-desktop coordinate mapping.
  - [ ] Any later refactor preserves the current UIA-first fallback diagnostic, SendInput outcome messages, WindowsMessages outcome messages, and stream-pixel coordinate tests.

- [ ] **ICA-006: Separate Linux app-state capture planning from snapshot/app selection**  
  Priority: P2; Effort: L; Risk: Medium; Confidence: High; Status: candidate  
  Cluster: `crates/sky-cua-linux/src/backend.rs` (`get_app_state`, portal/PipeWire/X11/Screenshot capture fallback, `apply_model_capture`, capture diagnostics)  
  Dependency category: Global, nondeterministic, or platform dependency; Local-substitutable  
  Problem: Linux `get_app_state` interleaves environment readiness, portal lifecycle, PipeWire/X11/Screenshot capture fallback, model image capture metadata, AT-SPI app discovery, native window fallback, and final snapshot assembly, making capture behavior and semantic/window fallback behavior hard to characterize independently.  
  Evidence:
  - `crates/sky-cua-linux/src/backend.rs`: `get_app_state` builds doctor/session diagnostics, starts portal state, attempts PipeWire capture, falls back through X11 or Screenshot portal capture, discovers AT-SPI apps, merges native window fallback data, and assembles the final snapshot.
  - `crates/sky-cua-linux/src/backend.rs`: `apply_model_capture` mutates `CaptureInfo` fields for screenshot paths, pixel sizes, model image format/quality/bytes, encode timing, original pixel size, and logical-to-pixel scale.
  - `crates/sky-cua-linux/src/backend.rs`: `push_capture_diagnostics` encodes important `PortalApprovalPending`, `PipeWireStreamFailed`, and `CaptureBackendDowngraded` behavior.
  - `crates/AGENTS.md` and `crates/sky-cua-linux/AGENTS.md`: project guidance treats `capture.backend` versus `capture.image_backend`, `CaptureBackendDowngraded`, and `PortalApprovalPending` as explicit runtime contracts.
  Why coupled: Capture planning, model-image preparation, diagnostics, app/window fallback selection, and snapshot assembly currently share one long method even though capture transports and semantic/window tree construction have different dependencies and test fixtures.  
  Suggested first move: Add characterization tests around current capture planning outcomes before introducing a `LinuxCapturePlanner` or equivalent crate-local module.  
  Testing impact: Boundary tests should cover `CaptureScreenMode::Never`, Portal PipeWire success, PipeWire failure with Screenshot fallback, X11 capture, no-capture cases, and the distinction between `capture.backend` and `capture.image_backend`.  
  Needs human decision: Confirm whether this should follow ICA-001 or can proceed in parallel; both touch `backend.rs` and its test module.  
  Acceptance criteria:
  - [ ] Capture planning returns `CaptureInfo` plus diagnostics for `Never`, Portal PipeWire success, PipeWire failure with Screenshot fallback, X11 capture, and no-capture cases.
  - [ ] `capture.backend` versus `capture.image_backend` semantics remain unchanged.
  - [ ] `PortalApprovalPending`, `PipeWireStreamFailed`, and `CaptureBackendDowngraded` diagnostics are preserved.
  - [ ] App/window selection and AT-SPI tree construction can be tested without invoking capture transports.
  - [ ] `LinuxDesktopBackend::get_app_state` reads as orchestration over capture planning plus semantic/window snapshot construction.
  Work checklist:
  - [ ] Validate the evidence and mark false assumptions.
  - [ ] Add characterization tests for current capture fallback and diagnostic behavior.
  - [ ] Sketch the target capture planning boundary.
  - [ ] Migrate `apply_model_capture` and capture diagnostic construction behind the new boundary.
  - [ ] Keep app/window selection behavior unchanged while moving capture planning.
  - [ ] Remove redundant old helper tests only after replacement boundary tests pass.

## Parking Lot

- [ ] Check whether app/window matching helpers in `crates/sky-cua-linux/src/backend.rs` should become a dedicated matcher module after ICA-001, or whether they are better left near snapshot construction.
- [ ] Check whether `crates/sky-cua-platform/src/model.rs` needs smaller model submodules only after a wire-contract-heavy change; current evidence was size, not enough coupling proof.
- [ ] After ICA-001, ICA-003, and ICA-005, audit coordinate conversion behavior across Linux action targeting, overlay cursor state derivation, and Windows stream-pixel-to-desktop mapping. Add shared golden cases before considering any shared coordinate abstraction.

<!-- improve-codebase-architecture:end -->
