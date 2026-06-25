# Unified WGPU desktop agent overlay

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` current as work proceeds.

This plan follows the repository ExecPlan rules in `plans/AGENTS.md`. It is written so a stateless implementer can start from this file and deliver the feature without conversation history.

## Purpose / Big Picture

After this work, the Linux desktop agent cursor overlay has one production visual implementation: the shared WGPU renderer. WGPU is the Rust GPU rendering API already used by this repository for the current Wayland layer-shell overlay through Vulkan or GLES. A renderer draws pixels. A host creates compositor or window-system surfaces for those pixels. The target architecture is one renderer plus small platform hosts.

The user-visible result is desktop computer-use matching the Android companion overlay's visual language: pink edge glow, inward wave, halo, tap ripple, trail, cursor glide, heading rotation, no-no shake as a renderable effect, and consistent timing and color constants. The old X11 shaped-window rectangle renderer, the Wayland SHM renderer, and the GNOME Shell JavaScript cursor actor stop being production visual renderers. If a desktop session cannot provide a WGPU-capable overlay surface, sky-cua reports that truth through structured overlay capabilities instead of falling back to a different drawing path.

This core plan is scoped to Linux desktop visible overlay unification on Wayland, shared desktop/Android constants and reference fixtures, WGPU renderer extraction, GPU-driven desktop effects, Wayland hardening, legacy-renderer retirement, VM verification, and one final operator-desktop acceptance pass. Screenshot-synthetic cursor support remains separate and must keep working even when visible overlay support is unavailable. Windows and macOS enum variants are out of scope.

X11 WGPU support and desktop no-no input interception/sound are follow-on plans, not blockers for retiring this core plan. This core plan still removes old X11/GNOME/SHM production drawing and documents X11/GNOME visible overlay as unsupported unless a later WGPU host plan lands.

Two constraints are non-negotiable:

1. All visible desktop effects and animations are GPU-rendered. The CPU may validate input, choose host surfaces, update bounded uniform/instance/storage buffers, coordinate host state, and run reference tests. It must not rasterize glow, waves, halo, ripple, trail, rotation, scale, no-no frames, or cursor frames into CPU pixel buffers for normal runtime rendering. WGPU should be the renderer, not a GPU-flavored final blit for CPU animation.
2. All implementation and verification testing runs in the repository testing VM. The operator's actual desktop is reserved for one controlled final acceptance pass after every required VM gate passes.

Success is observable from source and runtime. Rust tests prove contracts, protocol, generated spec, host lifecycle, renderer selection, WGPU buffer ABI, WGSL conformance, capture barriers, GPU-rendering boundaries, and capability reporting. Android JVM tests prove the companion uses the same generated constants and reference fixtures. VM live smokes prove visible cursor rendering, transparent composition, click-through behavior, hide-for-capture barriers, system-cursor restore, frame scheduling, restart behavior, and WGPU capabilities on real compositors. Android visual harness proof demonstrates phone parity. A final desktop pass confirms the already-proven build on the operator's actual desktop without iterative debugging there.

## Progress

- [x] 2026-06-25: Loaded repo ExecPlan rules, `plans/AGENTS.md`, the overlay pointer animation skill, current overlay host backend selection, current layer-shell WGPU renderer, X11 shaped-window renderer, GNOME Shell cursor bridge, platform cursor model, service overlay controller, overlay-host lifecycle, and phone companion overlay seams.
- [x] 2026-06-25: Chose the core architecture: one WGPU renderer, compositor-specific surface hosts, and a shared overlay spec as the source of truth for visual constants.
- [x] 2026-06-25: Reviewed the initial plan against current source and current library docs. Tightened hidden-risk coverage around raw-window-handle lifetime, WGPU surface capability checks, alpha compositing, protocol compatibility, action timing, unit conversion, X11 full-screen overlay requirements, GNOME retirement boundaries, no-no input handling, generated spec integration, and GPU-rendered effects.
- [x] 2026-06-25: Added a coordinator/parallel-worker execution model with ownership boundaries, merge gates, integration order, worker handoff contracts, and cross-worker verification requirements.
- [x] 2026-06-25: Reworked the plan to resolve the open implementation risks directly: visual/action timing is now a contract, one-shot animations use `AnimateGesture` with a protocol version bump, host behavior is governed by a state machine, capture requires an applied-frame barrier, shader behavior must pass GPU conformance tests, renderer surfaces use host-owned RAII guards, multi-output coverage fails closed, legacy renderer retirement happens after WGPU proof, and X11/no-no input/sound are scoped as follow-on plans.
- [x] Phase 0: Baseline, VM setup, and contract freeze.
- [x] Phase 1: Shared spec and generator.
- [x] Phase 2: Platform protocol, lifecycle, action timing, and capture barriers.
- [x] Phase 3: Renderer extraction with static Wayland WGPU parity.
- [ ] Phase 4: GPU effects, WGSL conformance, and deterministic rendering tests.
- [ ] Phase 5: Wayland hardening, multi-output correctness, frame pacing, and failure recovery.
- [ ] Phase 6: Android consumer migration and parity fixtures.
- [ ] Phase 7: Legacy renderer retirement and unsupported backend reporting.
- [ ] Phase 8: Documentation, packaging, and full testing-VM closeout.
- [ ] Phase 9: Final operator-desktop acceptance and plan retirement.

- [x] 2026-06-25: Phase 0 VM baseline accepted on host commit `7585f2d5afbb402facb408614a1149209a7a6d7a`, Arch `testing-vm`, KVM/QEMU, Virtio GPU, KDE/KWin Wayland on `wayland-0`.
- [x] 2026-06-25: VM `all` smoke passed with artifacts under `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260625T024935Z`, `/workspace/artifacts/gui-desktop-smoke/targeted-screenshot/20260625T025012Z`, `/workspace/artifacts/gui-desktop-smoke/display-screenshot/20260625T025015Z`, `/workspace/artifacts/session-env-smoke/20260625T025018Z`, `/workspace/artifacts/text-readback-smoke/20260625T025020Z`, `/workspace/artifacts/gui-desktop-smoke/codex-desktop/20260625T025022Z`, `/workspace/artifacts/opencode-zenity-smoke/20260625T025026Z`, `/workspace/artifacts/opencode-kdialog-smoke/20260625T025102Z`, `/workspace/artifacts/pi-zenity-smoke/20260625T025132Z`, `/workspace/artifacts/pi-kdialog-smoke/20260625T025210Z`, and KWin effect artifacts `/workspace/artifacts/codex-e2e/agent-cursor-kde/0625025251566881-kwin-nested` plus `/workspace/artifacts/codex-e2e/agent-cursor-kde/0625025257267646-kwin-user`.
- [x] 2026-06-25: Phase 0 unit baseline passed in the VM: `cargo test -p sky-cua-platform`, `cargo test -p sky-cua-service overlay`, `cargo test -p sky-cua-overlay-host`, `uv run pytest scripts/test_agent_cursor_smokes.py scripts/test_overlay_pointer_animations.py`, `cd android/phone-companion && JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT="$HOME/Android/Sdk" ./gradlew :app:testDebugUnitTest --offline`, `cargo test -p sky-cua-overlay-host protocol_messages_use_snake_case_kind_values`, and `cargo test -p sky-cua-service derives_cursor_state_from_explicit_click_coordinates`.
- [x] 2026-06-25: Closed Phase 0 VM readiness gaps by adding `uv`, `python-pytest`, `jdk21-openjdk`, Android command-line tools, API 36, platform-tools, and build-tools 36.0.0 to the VM provisioning path; live VM was updated and the exact offline Android gate passed after one online cache warm-up.
- [x] 2026-06-25: Frozen shared-contract decisions for Wave 1: spec keys/units, generated Rust/Kotlin module names, `AgentOverlayGestureEvent`, `AnimateGesture`, nested capabilities, action timing, host state machine, capture barrier, renderer/surface boundary, WGPU buffer ABI, fixture shapes, multi-output fail-closed policy, and VM artifact naming.
- [x] 2026-06-25: Phase 1 shared spec/codegen accepted with `resources/overlay/agent_overlay_spec.toml`, strict `scripts/generate_overlay_spec.py`, generated `sky_cua_platform::overlay_spec`, generated Kotlin `OverlaySpec`, and VM-verified idempotent `--check`.
- [x] 2026-06-25: Phase 2 C1 platform/protocol slice accepted: added `Point2`, `AgentOverlayGestureKind`, `AgentOverlayGestureEvent`, nested overlay capability fields, `OverlayHostMessageKind::AnimateGesture`, and overlay-host protocol version 2. Remaining Phase 2 timing, dedupe/stale handling, lifecycle states, and capture barriers stay in C2.
- [x] 2026-06-25: Phase 3 static renderer extraction accepted: WGPU instance/device/queue/pipeline/cursor texture moved under `crates/sky-cua-overlay-host/src/renderer/`, Wayland host now owns raw-handle `SurfaceGuard`s, all active surfaces are validated before WGPU support is claimed, and static layer-shell WGPU parity passed in the testing VM.
- [x] 2026-06-25: Phase 2 C2 accepted after coordinator takeover from a partial worker handoff: service pre-dispatch visual feedback now starts before backend input dispatch without delaying it, successful pointer actions send one-shot `AnimateGesture` events, failed pointer actions cancel pending visual feedback, hide-for-capture sends a sequence and requires a matching `applied_sequence`, and overlay-host backends validate gesture shape, dedupe `event_id`, reject stale sequences, clamp duration, and report lifecycle/barrier fields.
- [x] 2026-06-25: Phase 6 Android constants/JVM slice accepted: Android overlay math/view/controller constants now forward to generated `OverlaySpec`, `NO_NO_WIGGLE_DEG` was added to `[shared.effects]`, and shared motion fixtures are consumed by Android JVM tests. Phase 6 visual artifacts remain open because `adb devices` in the VM listed no attached device/emulator.

## Surprises & Discoveries

- Observation: The current desktop WGPU renderer is not generic. It is `WgpuLayerRenderer` inside `crates/sky-cua-overlay-host/src/layer_shell.rs`, and it creates WGPU surfaces directly from Wayland `wl_surface` raw handles.
  Evidence: `crates/sky-cua-overlay-host/src/layer_shell.rs` defines `WgpuLayerRenderer`, `WgpuSurfaceEntry`, `create_wgpu_surfaces`, and `LayerShellApp::select_renderer`.

- Observation: Wayland layer-shell still has a normal SHM fallback. In auto mode, if WGPU initialization fails, it reports `renderer_backend: wayland_shm` and draws CPU pixels. That fallback conflicts with the WGPU-only production invariant.
  Evidence: `RequestedLayerShellRenderer::Shm`, `LayerShellRenderer::Shm`, `draw_shm`, and the `wgpu unavailable, using shm fallback` branch in `crates/sky-cua-overlay-host/src/layer_shell.rs`.

- Observation: X11 currently draws the cursor by converting cursor pixels into X11 rectangles and filling them into a shaped cursor-sized window. It reports `backend: x11_shaped_window`, `renderer_backend: none`, and `visible_overlay: true`. This is a separate renderer and must be removed from production selection.
  Evidence: `crates/sky-cua-overlay-host/src/x11.rs` owns `CursorImage`, `pixel_rectangles`, `draw_cursor`, `poly_fill_rectangle`, and `capabilities` reporting.

- Observation: GNOME Shell currently draws with a JavaScript `St.Widget` actor inside the bundled extension. That cannot reuse the Rust WGPU renderer. The extension also owns window-control DBus methods, so only cursor drawing APIs and actor state should be retired, not the extension wholesale.
  Evidence: `resources/gnome-shell-extension/codex-window-control@openai.com/extension.js` defines `SetAgentCursorState`, `HideAgentCursor`, `ShowAgentCursor`, `AgentCursorStatus`, `_cursorActor`, `_createCursorActor`, and `_showAgentCursor`; `crates/sky-cua-overlay-host/src/gnome_shell.rs` wraps those calls.

- Observation: The phone companion has the richer animation state machine and desktop does not. Android has `OverlayMath`, `AgentOverlayController`, and `AgentOverlayView`; desktop overlay-host IPC only has `hello`, `capabilities`, `set_cursor`, `hide`, `show`, `ping`, and `shutdown`.
  Evidence: `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlayMath.kt`, `AgentOverlayController.kt`, and `AgentOverlayView.kt`; `crates/sky-cua-overlay-host/src/lib.rs` defines `OverlayHostMessageKind`.

- Observation: The desktop service currently attaches or updates cursor state after an action completes. `ServiceRequest::ExecuteAction` calls `route_action(...)` first, then `overlay.update_from_action(...)`. Pre-dispatch visual movement would require action-pipeline changes, not just host protocol changes.
  Evidence: `crates/sky-cua-service/src/daemon.rs` calls `route_action(self.backend.as_ref(), request.clone()).await` before `self.overlay.lock().await.update_from_action(&request, &mut outcome)`.

- Observation: WGPU raw-handle surface creation is possible but comes with strict lifetime constraints. `SurfaceTargetUnsafe::RawHandle` accepts raw display/window handles, and the raw handles must remain valid until after the returned `wgpu::Surface` is dropped. This affects the renderer/host boundary and drop order.
  Evidence: current WGPU docs for `SurfaceTargetUnsafe::RawHandle` and current code in `create_wgpu_surfaces` using `instance.create_surface_unsafe(...)`.

- Observation: WGPU surface support is adapter and surface specific. `SurfaceCapabilities` can return empty `formats` or `present_modes` for an incompatible surface. This means selecting an adapter for the first surface is not enough; every output surface must be checked before claiming full WGPU support.
  Evidence: current WGPU docs for `SurfaceCapabilities`; current `WgpuLayerRenderer::new` chooses an adapter compatible with the first Wayland surface only.

- Observation: Transparent composition is not automatic. WGPU exposes `CompositeAlphaMode::Opaque`, `PreMultiplied`, `PostMultiplied`, `Inherit`, and `Auto`. `Opaque` ignores alpha. `PreMultiplied` expects RGB already multiplied by alpha, matching the current shader and blend state. Live smoke must prove transparency instead of trusting enum availability.
  Evidence: current WGPU docs for `CompositeAlphaMode`; current shader outputs `vec4(color.rgb * color.a, color.a)` and the pipeline uses `PREMULTIPLIED_ALPHA_BLENDING`.

- Observation: Present mode must stay conservative. `PresentMode::Fifo` is documented as supported on all platforms and is the default. `Mailbox` may not be supported everywhere. The renderer can prefer `Mailbox` only when `SurfaceCapabilities::present_modes` includes it, and must otherwise choose `Fifo` or a documented `Auto*` fallback.
  Evidence: current WGPU docs for `PresentMode`; current code already prefers `Mailbox` else `Fifo`.

- Observation: The repo currently pins `wgpu = 29.0.3` with `default-features = false` and features `std`, `wgsl`, `vulkan`, and `gles`. It also pins `x11rb = 0.13.2` with only `shape` and `xfixes`. A later X11 WGPU host probably needs either raw-window-handle construction by hand or x11rb features `allow-unsafe-code` plus `raw-window-handle`.
  Evidence: root `Cargo.toml`; local cargo registry docs for `x11rb-0.13.2` say `allow-unsafe-code` enables `xcb_ffi::XCBConnection`, and `xcb_ffi` has raw-window-handle impls behind the `raw-window-handle` feature.

- Observation: Android uses dp-based tuning and device-pixel gesture coordinates, while desktop layer-shell currently renders in compositor logical/output-local coordinates. The shared spec cannot blindly say “pixels” without a conversion rule.
  Evidence: `OverlayMath.kt` names motion constants in dp/s and dp/s^2; `AgentOverlayView.kt` applies density via `dp(...)`; `layer_shell.rs` maps `CoordinateSpace::DesktopLogical` through output-local layer coordinates.

- Observation: Shared Rust/Kotlin fixtures alone do not prove the GPU path. A WGSL shader can drift while Rust and Kotlin tests still pass. The plan therefore requires WGSL compute conformance or offscreen render invariants that exercise the actual shader code.
  Evidence: the renderer target is WGPU/WGSL; CPU reference math is only an oracle.

- Observation: The live testing VM initially lacked `uv`, `pytest`, JDK 21, and `$HOME/Android/Sdk`, so the Phase 0 Python and Android commands could not run as written even though the smoke runner itself was healthy.
  Evidence: `uv run pytest ...` failed with `uv: command not found`; `./gradlew :app:testDebugUnitTest --offline` first failed because `/usr/lib/jvm/java-21-openjdk` was absent, then because the Android SDK and Gradle dependency cache were absent.

- Observation: The VM pacman database can drift behind mirrors during long-lived sessions.
  Evidence: installing `uv-0.11.21-1` first failed with mirror 404s; `sudo pacman -Syy --noconfirm` refreshed the DB and installed `uv-0.11.24-1`, `python-pytest`, and `jdk21-openjdk`.

- Observation: Android's offline unit-test gate is only meaningful after the VM has the SDK and a warmed Gradle dependency cache.
  Evidence: after installing command-line tools 21.0, `platforms;android-36`, `build-tools;36.0.0`, and running one online `./gradlew :app:testDebugUnitTest`, the exact offline command passed.

- Observation: Phase 1 generator strictness needed to cover missing nested tables as well as missing leaf keys.
  Evidence: coordinator review found `_validate_node` rejected missing leaf keys but would allow omitted subsections such as `[shared.motion]`; the generator and `scripts/test_overlay_spec_codegen.py` now reject missing nested sections.

- Observation: Keeping a WGPU surface alive without the instance registry triggered a live VM panic during Phase 3 smoke.
  Evidence: the first `wayland-layer-shell-overlay` rerun panicked with `Surface[Id(0,1)] does not exist`; the coordinator changed `WgpuOverlayRenderer::new` to borrow `WgpuOverlayInstance` and kept `LayerShellApp.surface_guards` declared before `LayerShellApp.instance` so guards drop before the instance.

- Observation: Phase 6 exposed one Android constant that was still duplicated outside the generated spec.
  Evidence: `OverlayMath.NO_NO_WIGGLE_DEG` was a hard-coded `20f`; `[shared.effects].no_no_wiggle_deg = 20.0` was added to `resources/overlay/agent_overlay_spec.toml` and regenerated into Rust/Kotlin constants.

- Observation: The testing VM can run Android JVM parity tests but cannot currently produce Android visual harness artifacts.
  Evidence: the Phase 6 worker ran `adb devices` inside the VM and received an empty device list; `./gradlew :app:testDebugUnitTest --offline` passed, including the shared fixture test.

## External Research Snapshot

Sources checked for this plan:

- `https://docs.rs/wgpu/latest/wgpu/enum.SurfaceTargetUnsafe.html` for WGPU raw-handle surface creation. Key constraint: raw display/window handles must remain valid until the returned `Surface` is dropped.
- `https://docs.rs/wgpu/latest/wgpu/struct.SurfaceCapabilities.html` for per-surface/per-adapter capability checks. Key constraint: `formats` and `present_modes` can be empty for incompatible surfaces; `RENDER_ATTACHMENT` usage is guaranteed when a surface is compatible.
- `https://docs.rs/wgpu/latest/wgpu/enum.CompositeAlphaMode.html` for alpha handling. Key constraint: `Opaque` ignores alpha; `PreMultiplied` respects alpha when color channels are already premultiplied; `Inherit` depends on native WSI state.
- `https://docs.rs/wgpu/latest/wgpu/enum.PresentMode.html` for presentation modes. Key constraint: `Fifo` is supported on all platforms; `Mailbox` and `Immediate` are not universal.
- `https://docs.rs/raw-window-handle/latest/raw_window_handle/enum.RawWindowHandle.html` and `https://docs.rs/raw-window-handle/latest/raw_window_handle/enum.RawDisplayHandle.html` for supported handle variants. Key fact: raw-window-handle 0.6.2 has Wayland, XCB, and Xlib window/display variants.
- Local cargo registry source for `x11rb-0.13.2` because the online search did not return a stable docs result during review. Key fact: `allow-unsafe-code` enables `xcb_ffi::XCBConnection`; raw-window-handle integration is feature-gated.

If any dependency version changes before implementation, repeat this research before changing the plan. Do not copy examples from older WGPU or raw-window-handle versions without checking the currently pinned APIs.

## Decision Log

- Decision: Treat “unify on WGPU” as one renderer core plus per-session surface hosts.
  Rationale: Wayland layer-shell and any future host need different windowing setup, but cursor pixels, animation math, shader code, colors, geometry, and effects should be shared.
  Date/Author: 2026-06-25 / Codex

- Decision: Make `resources/overlay/agent_overlay_spec.toml` the editable source of truth and generate Rust and Kotlin constants from it.
  Rationale: Runtime TOML parsing inside animation hot paths is unnecessary, while manual duplication is exactly the drift this work removes.
  Date/Author: 2026-06-25 / Codex

- Decision: Add `schema_version = 1` to the shared spec and fail generation on unknown keys or invalid ranges.
  Rationale: A cross-language spec must be a contract, not a loose bag of constants. Unknown keys and invalid values should fail early.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Separate source asset metrics from presentation metrics.
  Rationale: The cursor PNG source, browser/synthetic cursor size, Android dp presentation size, desktop logical presentation size, and hotspots are related but not identical.
  Date/Author: 2026-06-25 / Codex

- Decision: Use an explicit `AnimateGesture` overlay-host message and bump `OVERLAY_HOST_PROTOCOL_VERSION`.
  Rationale: `SetCursor` is persistent state, while tap/ripple/trail/no-no events are one-shot animation events. Retrying state after a host restart must not replay a gesture or sound. Since the service and host are bundled together, a protocol bump is cleaner than stuffing event semantics into optional state fields.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Desktop visible action animation is visual feedback, not a dispatch synchronization contract.
  Rationale: Waiting for a 950 ms glide before every click would silently change automation latency. The service may start a glide when it accepts a coordinate action and trigger ripple/trail after success, but input dispatch remains governed by the existing backend action contract. Overlay render failure does not block input dispatch unless an existing backend-specific input precondition already requires real pointer movement.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Add structured nested capabilities for overlay effects and coverage.
  Rationale: Three booleans are too coarse. Clients and tests need to know which effects, coordinate spaces, coverage, hit testing, and sound are actually supported without parsing `reason` prose.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Use a host state machine with capture precedence.
  Rationale: Pointer tracking, action animation, capture hiding, no-no feedback, shutdown, and system-cursor hiding can race unless the host has explicit states and transition precedence.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Hide-for-capture requires an applied-frame barrier.
  Rationale: A successful `Hide` request is not enough if the transparent frame has not reached the compositor yet. Capture must wait until every active surface has presented or acknowledged the hidden sequence.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Fail closed on incomplete output coverage in the core Wayland implementation.
  Rationale: A global cursor overlay that silently disappears on one monitor is worse than an unsupported state. Partial coverage may be implemented later only with structured coverage reporting and explicit tests.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Remove SHM from production visible rendering rather than preserving it as debug fallback.
  Rationale: Keeping a second production-like visual path weakens the WGPU-only invariant. Debug backdrops should live in the WGPU playground; `WaylandShm` can remain only as a legacy deserialization enum value.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: GNOME Shell extension cursor drawing is not a target renderer.
  Rationale: A GNOME Shell JavaScript actor cannot share the Rust WGPU renderer. If GNOME lacks a usable standalone WGPU overlay surface, visible overlay support should report unsupported there until a real host exists. The extension's non-overlay window-control API must not be deleted as collateral damage.
  Date/Author: 2026-06-25 / Codex

- Decision: X11 WGPU host work is a follow-on plan, not a core-plan blocker.
  Rationale: X11 needs separate proof of ARGB visuals, compositor behavior, transparent root-spanning windows, input regions, RandR, Vulkan/GLES surface creation, and crash recovery. The core migration should remove the old X11 renderer and report unsupported X11 visible overlay until a dedicated X11 WGPU plan lands.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Desktop no-no input interception and sound are follow-on plans, not core-plan blockers.
  Rationale: Rendering no-no as an effect belongs in the core GPU renderer, but click interception and sound add input UX and process-management complexity. They should not hold renderer unification hostage.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: All visible desktop effects and animations must be GPU-rendered.
  Rationale: The goal is not merely to host a CPU-rendered animation in a WGPU window. Glow, inward waves, halo, ripple, trail, cursor transform/rotation, and no-no shake should be expressed as WGPU draw passes, shader math, uniforms, instance buffers, or storage buffers. CPU work is limited to input validation, host coordination, bounded state uploads, and test/reference math.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Use the testing VM for all implementation and verification testing; reserve the actual desktop for final acceptance only.
  Rationale: Overlay rendering, system-cursor hiding, input regions, deployment, compositor integration, capture synchronization, sound, and crash recovery can disturb the operator's desktop. Workers and the coordinator must prove changes in the isolated testing VM first. The final desktop run is a narrow acceptance gate, not a place for iterative debugging.
  Date/Author: 2026-06-25 / ChatGPT

- Decision: Freeze the Phase 1 shared spec as `resources/overlay/agent_overlay_spec.toml`, `schema_version = 1`, with sections `[shared.colors]`, `[shared.timing]`, `[shared.motion]`, `[shared.effects]`, `[desktop.geometry]`, `[desktop.rendering]`, `[android.geometry]`, `[android.rendering]`, and `[sound]`; field names carry units such as `_ms`, `_dp`, `_logical_px`, `_dp_per_s`, `_dp_per_s2`, `_deg`, `_alpha_0_1`, `_alpha_0_255`, and `_fraction`.
  Rationale: Workers need one canonical schema and unit policy before generated Rust/Kotlin constants, fixtures, shaders, and Android consumers can safely converge.
  Date/Author: 2026-06-25 / Codex

- Decision: Freeze generated names as `sky_cua_platform::overlay_spec` backed by `crates/sky-cua-platform/src/overlay_spec_generated.rs` and Kotlin `object OverlaySpec` in `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlaySpec.kt`.
  Rationale: Cross-worker work must not create competing constant modules or generated-file formats.
  Date/Author: 2026-06-25 / Codex

- Decision: Freeze the desktop one-shot event contract around `AgentOverlayGestureEvent { event_id, sequence, kind, coordinate_space, mapping_id, points, duration_ms, source_action }`, `AgentOverlayGestureKind::{Tap, Drag, Swipe, NoNo}`, `Point2`, and `OverlayHostMessageKind::AnimateGesture` with a protocol version bump.
  Rationale: `SetCursor` stays persistent state, while gestures, ripples, trails, and no-no feedback are one-shot events that must dedupe and must not replay after host restart.
  Date/Author: 2026-06-25 / Codex

- Decision: Freeze nested capabilities as explicit effect support, coverage, coordinate spaces, renderer backend, adapter/backend names, protocol version, effect schema version, active/rendered output counts, maximum gesture points, and structured reasons; the core Wayland policy remains fail-closed unless all active outputs have WGPU coverage.
  Rationale: Unsupported or partial sessions need machine-readable truth instead of prose fallback inference.
  Date/Author: 2026-06-25 / Codex

- Decision: Freeze runtime timing, lifecycle, and capture semantics to the tables already in this plan: visual feedback does not delay backend input dispatch, host states are `Hidden`, `VisibleIdle`, `AgentAnimating`, `CaptureHidden`, `NoNoFeedbackRenderOnly`, and `FailedOrUnsupported`, and hide-for-capture waits for an applied-frame sequence before service capture.
  Rationale: These contracts let service, host, renderer, and tests move in parallel without changing automation latency or reintroducing capture races.
  Date/Author: 2026-06-25 / Codex

- Decision: Freeze the renderer boundary as one WGPU renderer plus host-owned RAII surface guards; renderer modules must not import Wayland, X11, GNOME, DBus, or service modules, and WGPU buffer ABI structs must be explicit `repr(C)` or otherwise documented with size/alignment tests against WGSL layout.
  Rationale: This keeps unsafe raw-handle lifetime and platform surfaces outside the renderer while preventing duplicate shader/buffer ABI paths.
  Date/Author: 2026-06-25 / Codex

- Decision: Freeze fixture names and artifact policy as `resources/overlay/agent_overlay_motion_fixtures.json`, `resources/overlay/wgsl_animation_fixtures.json`, VM artifacts under `/workspace/artifacts/...`, and final operator acceptance under `artifacts/final-desktop-overlay-acceptance/`.
  Rationale: Rust, Kotlin, WGSL, and live-smoke proof need shared sample names and a stable evidence trail.
  Date/Author: 2026-06-25 / Codex

- Decision: Keep the service-side action timing split explicit with `prepare_action_visual` before backend dispatch and `update_from_action` after backend dispatch.
  Rationale: The visible cursor can start moving when the service accepts a coordinate action, but success effects remain post-dispatch and failed actions cancel pending visual feedback. This preserves existing automation latency while giving the renderer enough intent for visual feedback.
  Date/Author: 2026-06-25 / Codex

- Decision: Add `shared.effects.no_no_wiggle_deg` instead of leaving the Android no-no amplitude as a local constant.
  Rationale: Phase 6 parity needs the no-no render effect to share the same generated source of truth that Phase 4 WGSL effects will consume.
  Date/Author: 2026-06-25 / Codex

## Outcomes & Retrospective

Phase 0 is complete. The testing VM baseline passed after VM readiness fixes for Python and Android tooling. The VM provisioning path now installs Python/Android prerequisites required by later gates.

Phase 1 is complete. The shared TOML spec is the source of truth, generated Rust and Kotlin constants are byte-identical on repeat runs, stale generated files are caught by `--check`, invalid specs are rejected, and the stricter nested-section validation is covered by tests. Coordinator VM verification passed:

    python3 scripts/generate_overlay_spec.py --check
    uv run pytest scripts/test_overlay_spec_codegen.py -q
    uv run ruff format --check scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    uv run ruff check scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    uv run basedpyright scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    cargo test -p sky-cua-platform overlay_spec_tests

Phase 2 is complete. The C1 slice added the platform-neutral event model, `AnimateGesture`, protocol version 2, and nested capability fields. The C2 slice added service timing and capture semantics plus host-side lifecycle/barrier fields and gesture validation. C2 was completed by coordinator cleanup after the worker handed off a partial diff with local overlay-host tests passing and service tests still incomplete. Coordinator VM verification passed:

    cargo test -p sky-cua-platform
    cargo test -p sky-cua-overlay-host
    cargo test -p sky-cua-service overlay
    cargo test -p sky-cua-service derives_cursor_state_from_explicit_click_coordinates

Phase 3 is complete. The Wayland host still owns compositor objects, output/layer lifecycle, input regions, pointer tracking, frame callbacks, and the legacy SHM fallback; the new renderer modules own WGPU instance interaction, adapter/device/queue setup, surface policy, cursor texture upload, shader/pipeline setup, and per-surface draw submission. The coordinator corrected the raw-handle lifetime after a VM smoke panic and reran the decisive gates. Coordinator VM verification passed:

    cargo test -p sky-cua-overlay-host
    cargo test -p sky-cua-service overlay
    python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=/home/bex/projects/sky-cua/artifacts/testing-vm/known_hosts --profile wayland-layer-shell-overlay

The Phase 3 live artifact is `/workspace/artifacts/codex-e2e/agent-cursor-kde/0625035012797790-vis`, with `before.jpg` and `visible.jpg`. The smoke reported `renderer_backend: "wgpu"`, llvmpipe via Vulkan, `visible_overlay_captured: true`, and hotspot pixel deltas near the cursor.

Wave 1 integration verification also passed in the VM:

    python3 scripts/generate_overlay_spec.py --check
    uv run pytest scripts/test_overlay_spec_codegen.py -q
    cargo test -p sky-cua-platform
    cargo test -p sky-cua-overlay-host protocol_messages_use_snake_case_kind_values
    cargo test -p sky-cua-overlay-host
    cargo test -p sky-cua-service overlay
    cd android/phone-companion && JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT="$HOME/Android/Sdk" ./gradlew :app:testDebugUnitTest --offline

Phase 6 is complete for generated constants and JVM fixture parity, but remains open for Android visual artifacts. Android remains Canvas-native. The VM accepted:

    python3 scripts/generate_overlay_spec.py --check
    uv run pytest scripts/test_overlay_spec_codegen.py -q
    uv run ruff format --check scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    uv run ruff check scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    uv run basedpyright scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    cargo test -p sky-cua-platform
    cd android/phone-companion && JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT="$HOME/Android/Sdk" ./gradlew :app:testDebugUnitTest --offline

The Android visual harness artifact gate did not run because the VM has no attached Android device or emulator; the closest passing gate is `OverlaySpecFixtureTest` consuming the shared motion fixtures from JVM resources.

Wave 2 integration verification passed in the VM:

    cargo fmt --check
    python3 scripts/generate_overlay_spec.py --check
    uv run pytest scripts/test_overlay_spec_codegen.py -q
    uv run ruff format --check scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    uv run ruff check scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    uv run basedpyright scripts/generate_overlay_spec.py scripts/test_overlay_spec_codegen.py
    cargo test -p sky-cua-platform
    cargo test -p sky-cua-overlay-host
    cargo test -p sky-cua-service overlay
    cd android/phone-companion && JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT="$HOME/Android/Sdk" ./gradlew :app:testDebugUnitTest --offline
    python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=/home/bex/projects/sky-cua/artifacts/testing-vm/known_hosts --profile wayland-layer-shell-overlay

The Wave 2 live artifact is `/workspace/artifacts/codex-e2e/agent-cursor-kde/0625043426158731-vis`, with `before.jpg` and `visible.jpg`. The smoke reported `renderer_backend: "wgpu"`, llvmpipe via Vulkan, `visible_overlay_captured: true`, and no synthetic cursor fallback.

## Context and Orientation

The sky-cua desktop overlay has three layers. The platform model in `crates/sky-cua-platform/src/model.rs` defines serializable state and capabilities shared by service, client, tests, and overlay host. The service controller in `crates/sky-cua-service/src/overlay.rs` owns current cursor state, creates screenshot-synthetic cursor markers, hides visible overlays around captures, and communicates with the overlay-host process. The overlay host in `crates/sky-cua-overlay-host/` owns the native visible overlay process and currently chooses a backend from environment/session variables.

The overlay-host backend selection lives in `crates/sky-cua-overlay-host/src/lib.rs`. `OverlayHostBackend::from_env` reads `SKY_CUA_OVERLAY_BACKEND`. In `auto`, it currently tries Wayland layer-shell when `WAYLAND_DISPLAY` exists, tries the GNOME Shell extension in a GNOME session, and tries X11 when the environment is an X11 session or has `DISPLAY` without Wayland. The backend enum currently includes `Noop`, `GnomeShell`, `LayerShell`, and `X11`.

Wayland layer-shell is the core target host. The current implementation in `crates/sky-cua-overlay-host/src/layer_shell.rs` creates one layer surface per output, sets each layer to cover the output, sets an empty input region to make the overlay click-through, hides/restores the system cursor through `src/system_cursor.rs`, and uses pointer tracking from `src/pointer_tracking.rs`. Its default renderer is WGPU, but it still has a Wayland SHM fallback that draws pixels on the CPU. After this plan, production visible overlay means WGPU or unsupported.

The WGPU code in `layer_shell.rs` currently owns both host concerns and renderer concerns. Host concerns include Wayland connection, output/layer creation, configure events, raw Wayland display/window handles, and layer commit/frame callbacks. Renderer concerns include instance/device/queue creation, cursor texture upload, shader/pipeline setup, surface configuration, vertex generation, and frame presentation. Phase 3 splits those without changing current static Wayland WGPU behavior.

KWin support is not a visual renderer and should stay that way. The KWin effect under `resources/kwin/effects/sky-cua-agent-cursor/` is a compositor shim for hiding/restoring the real cursor and reporting pointer movement. It remains a system-cursor and pointer-tracking adapter used by the WGPU overlay, not a cursor drawing path.

The Android phone companion overlay is richer than desktop today. `android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlayMath.kt` contains pure math and constants for moving cursor, breathing glow, wave phase, ripple, trail, and no-no head shake. `AgentOverlayController.kt` owns animation state and drives frames. `AgentOverlayView.kt` draws the full-screen pass-through accessibility overlay. The host calls phone `overlay_active` to show the ambient overlay and `overlay_gesture` to animate taps and swipes. Desktop should share the visual grammar and generated constants without making Android depend on desktop code.

## Plan of Work

### Test environment policy

The testing VM is the mandatory execution environment for all tests associated with this plan. Before running any test or smoke, read `.agents/skills/vm-tests/SKILL.md` and use the repository's VM harnesses. This applies to baseline tests, narrow worker tests, integration tests, packaging/deployment verification, compositor/live UI tests, GPU backend tests, Android harnesses, crash recovery, cursor hiding, click-through, capture hiding, no-no rendering, and backend unsupported probes.

The operator's actual desktop must not be used for iterative implementation testing. Workers must not directly launch or deploy `sky-cua-overlay-host`, hide the system cursor, change input regions, play sounds, run Wayland/X11 live smokes, or exercise the installed plugin on the actual desktop. Source editing, git inspection, and non-executing document work may occur outside the VM, but authoritative test results must come from the testing VM.

The coordinator must run the complete VM smoke profile before final acceptance:

    cd /home/bex/projects/sky-cua
    python3 scripts/run_gui_testing_vm_smoke.py --profile all

Narrow VM commands during development supplement rather than replace the final `all` profile. Every recorded test result and artifact must identify the VM image/profile, desktop environment, GPU/backend, commit SHA, and command.

Only after implementation is complete, all worker packages are integrated, the full automated suite passes in the VM, the `all` VM smoke profile passes, and VM visual artifacts have been reviewed may the coordinator perform Phase 9 on the operator's actual desktop. If the final desktop test fails, stop testing there, restore the desktop to its prior state, return the defect to the testing VM, fix it, and repeat the affected VM verification before another desktop acceptance attempt.

### Mandatory risk-closure gates

The following gates are not commentary. They are requirements that must be satisfied by the relevant phases before the coordinator may close the core plan:

| Risk to close | Required plan fix | Phase/gate that proves it |
| --- | --- | --- |
| State and one-shot gesture events becoming muddled | `SetCursor` remains persistent state; `AnimateGesture` is a one-shot event with `event_id`, `sequence`, dedupe, stale rejection, and protocol version bump | Phase 2 protocol tests |
| Overlay animation accidentally changing input dispatch latency | Action timing table defines visual feedback separately from backend input dispatch; overlay render failure does not block input except existing backend preconditions | Phase 2 service tests |
| Capture racing with the compositor | Hide-for-capture waits for an applied-frame barrier across all active surfaces before service capture | Phase 2 and Phase 5 capture smokes |
| WGPU adapter chosen before surfaces are known | Renderer initialization is two-stage and receives host-created surface guards before adapter/device selection | Phase 3 extraction tests |
| Native window/display lifetime escaping into renderer code | Unsafe raw-handle lifetime is encapsulated in host-owned RAII surface guards; renderer never exposes naked `wgpu::Surface<'static>` as public state | Phase 3 code review and tests |
| CPU animation masquerading as WGPU rendering | Runtime effects are driven by WGSL using uniforms/instance/storage buffers; CPU reference math is test-only | Phase 4 GPU-boundary review |
| Rust/Kotlin fixtures proving only CPU math | WGSL compute conformance and/or offscreen render invariants test the actual shader path | Phase 4 conformance tests |
| Multi-monitor partial rendering lying to clients | Core Wayland fails closed unless every active output has compatible WGPU coverage | Phase 5 VM multi-output tests |
| Ambient animation burning power indefinitely | Rendering is frame-callback/bounded-scheduler driven, pauses when hidden/unsupported/capture-hidden, and has per-frame allocation/draw-call budgets | Phase 5 pacing/perf checks |
| Legacy renderers removed before replacement proof | SHM/GNOME/X11 retirement waits until static extraction, GPU effects, and Wayland hardening are proven | Phase 7 after Phases 3-5 |
| X11 or no-no sound blocking the core migration | X11 WGPU host and no-no input/sound are follow-on plan seeds; core only keeps no-no as a GPU-rendered effect | Follow-on Plan Seeds |
| VM safety being bypassed | All implementation/verification tests run in testing VM; actual desktop is final acceptance only | Phase 8 and Phase 9 |

### Coordination and parallel work model

This plan is intentionally splittable, but it needs a coordinator. The coordinator owns source-of-truth decisions, phase boundaries, merge order, and final proof. Workers may implement bounded packages in parallel, but they must not independently redefine shared contracts, generated schemas, protocol names, capability fields, spec keys, renderer interfaces, shader buffer ABI, or test artifact formats after the coordinator freezes them.

The coordinator loop for every worker package is:

1. Assign one package with exact files or seams, expected tests, and non-goals.
2. Require the worker to start from current `git status --short` and note any pre-existing user changes in its paths.
3. Require the worker to read the nearest `AGENTS.md` for every path it touches.
4. Require a short handoff containing touched paths, changed public contracts, generated files, GPU-boundary changes, commands run, VM identity/profile, skipped live gates, artifacts, and follow-ups.
5. Review the diff against this ExecPlan before merging.
6. Run the package's narrow checks in the testing VM.
7. Re-run the coordinator integration gate for any shared contract or generated-file change.
8. Update this plan's `Progress`, `Surprises & Discoveries`, and `Decision Log` before assigning dependent work.

Shared files that should generally be coordinator-owned are:

    crates/sky-cua-platform/src/model.rs
    crates/sky-cua-platform/src/lib.rs
    crates/sky-cua-overlay-host/src/lib.rs
    crates/sky-cua-overlay-host/src/main.rs
    crates/sky-cua-service/src/overlay.rs
    crates/sky-cua-service/src/daemon.rs
    Cargo.toml
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlaySpec.kt
    resources/overlay/agent_overlay_spec.toml
    resources/overlay/agent_overlay_motion_fixtures.json
    resources/overlay/wgsl_animation_fixtures.json

Worker packages:

| Package | Primary responsibility | Primary paths | Must avoid | Narrow checks |
| --- | --- | --- | --- | --- |
| A | Baseline, VM readiness, contract freeze | VM smoke scripts, current test harnesses, this plan notes | Runtime contract changes | Phase 0 commands in VM |
| B | Shared spec and generator only | `resources/overlay/agent_overlay_spec.toml`, generator, generated files, codegen tests | Service protocol and renderer behavior | codegen check, Python tests, platform tests |
| C1 | Platform model and serialized protocol | `sky-cua-platform`, overlay-host message structs | Service timing and rendering | serialization/protocol tests |
| C2 | Service action timing, host lifecycle, capture barrier | `sky-cua-service/src/overlay*`, `daemon.rs`, host lifecycle | Shader/effect rendering | service overlay and lifecycle tests |
| D | WGPU renderer extraction, static cursor parity | overlay-host renderer modules and `layer_shell.rs` extraction | Visual effects beyond static cursor | overlay-host tests, Wayland static smoke in VM |
| F | GPU effects and WGSL conformance | WGPU shaders, scene/effect code, WGSL compute tests, Rust fixtures | Android UI migration | overlay-host tests, shader conformance tests, deterministic render tests |
| W | Wayland hardening | layer-shell host, multi-output, frame pacing, restart/device-loss handling | New visual effects | Wayland VM live smokes and failure matrix |
| G | Android consumer migration and parity | Android overlay math/view/controller tests | Desktop renderer internals | Android JVM tests and overlay pointer harness from VM |
| E | Legacy renderer retirement | backend selection, SHM removal, GNOME cursor retirement, X11 unsupported path | New renderer features | backend tests, unsupported-mode probes, packaging if resources change |
| J | Docs, packaging, VM closeout | docs, ROADMAP, packaging/check scripts if needed | Runtime behavior changes except doc-discovered fixes | full automated suite and VM `all` profile |
| K | Final desktop acceptance | controlled operator-desktop acceptance notes/artifacts | Iterative debugging | final scripted acceptance only |

Follow-on packages, not part of core retirement:

| Follow-on | Scope | Entry condition |
| --- | --- | --- |
| X | X11 WGPU host proof/implementation | Core WGPU renderer extracted and legacy X11 drawing retired |
| N | Desktop no-no input catcher and sound | GPU no-no render effect implemented and core Wayland behavior stable |

Hard dependency order:

    A before all runtime edits.
    B before Android consumer migration and before effects consume constants.
    C1 before C2, D consumers, and capability-dependent work.
    C2 before F depends on event semantics or capture barriers.
    D before F, W, and any future X11 WGPU host.
    F after B + C1 + C2 + D.
    W after D and mostly after F for effect-aware hide/capture/frame-pacing proof.
    G after B and fixture-format freeze; final parity after F.
    E after F + W have proven the WGPU replacement.
    J after E and all VM runtime gates.
    K only after J and all VM gates are green.

Integration branches should be small and phase-shaped. A worker that discovers a missing shared contract should stop and hand that discovery to the coordinator instead of patching five seams at once. The coordinator may then create a small contract patch and rebase or retarget dependent workers.

Workers should not hand-edit generated files unless their package owns the generator. If generated files conflict, regenerate from canonical TOML on the coordinator integration branch. Workers should not run broad formatters that touch unrelated files with user changes. Format touched scope first; the coordinator can run root formatting after merging a batch.

Coordinator integration gates:

- All gates below run in the testing VM.
- After B: codegen `--check`, codegen tests, Rust platform tests.
- After C1: platform serialization tests, overlay-host protocol tests, old-message/protocol-mismatch compatibility tests.
- After C2: service action timing tests, host lifecycle tests, capture-barrier tests.
- After D: overlay-host tests and Wayland static-cursor smoke.
- After F: Rust fixture tests, WGSL compute conformance tests, deterministic offscreen render tests, GPU-boundary source check.
- After W: Wayland live smokes, multi-output/hotplug/failure matrix, frame pacing/performance checks.
- After G: Android JVM tests and Android visual artifact harness from the VM environment.
- After E: backend-selection tests, unsupported-mode probes, packaging if resources changed.
- Before J closes VM verification: full acceptance suite, `python3 scripts/run_gui_testing_vm_smoke.py --profile all`, and collected VM artifacts.
- After J: coordinator-only Phase 9 desktop acceptance. A desktop failure returns the work to VM-only development.

No worker may mark a phase complete based only on unit tests if the phase has live-smoke acceptance. A VM live gate may be skipped only with the exact unavailable VM prerequisite. Skips are not success. Unavailability on the VM must not be worked around by moving the test to the operator desktop.

### Phase 0: Baseline, VM readiness, and contract freeze

Run the current tests in the testing VM before runtime edits and record failures in this plan. From `/home/bex/projects/sky-cua` inside the VM, run:

    cargo test -p sky-cua-platform
    cargo test -p sky-cua-service overlay
    cargo test -p sky-cua-overlay-host
    uv run pytest scripts/test_agent_cursor_smokes.py scripts/test_overlay_pointer_animations.py

Run Android pure overlay tests if the VM can reach the Android SDK/device harness:

    cd android/phone-companion
    JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT="$HOME/Android/Sdk" ./gradlew :app:testDebugUnitTest --offline

Capture current overlay selection behavior without a live compositor where possible:

    cargo test -p sky-cua-overlay-host protocol_messages_use_snake_case_kind_values
    cargo test -p sky-cua-service derives_cursor_state_from_explicit_click_coordinates

Run the relevant VM smoke profile available at this point and record the exact VM image/profile:

    python3 scripts/run_gui_testing_vm_smoke.py --profile all

If `all` is not yet usable, record the failing prerequisite and run the narrowest current VM profile that exercises desktop overlay setup. Do not continue to runtime refactors until the coordinator has accepted the baseline.

Before parallel implementation starts, freeze these contracts in this plan or a coordinator-owned patch:

- shared spec section/key naming and units,
- generated Rust/Kotlin module names,
- `AgentOverlayGestureEvent` and capability field names,
- `AnimateGesture` message shape and protocol version bump,
- action timing contract,
- host state machine,
- capture barrier semantics,
- renderer instance/surface ownership boundary,
- WGPU buffer ABI convention,
- fixture JSON shape and tolerance policy,
- multi-output failure policy,
- VM artifact naming.

### Phase 1: Shared spec and generator

Create `resources/overlay/agent_overlay_spec.toml` with `schema_version = 1` and explicit sections. Suggested structure:

    schema_version = 1

    [shared.colors]
    [shared.timing]
    [shared.motion]
    [shared.effects]
    [desktop.geometry]
    [desktop.rendering]
    [android.geometry]
    [android.rendering]
    [sound]

The TOML must include units in field names, for example `_ms`, `_dp`, `_logical_px`, `_dp_per_s`, `_dp_per_s2`, `_deg`, `_alpha_0_1`, `_alpha_0_255`, and `_fraction`. It must include cursor source metrics, base CSS/synthetic metrics, desktop presentation metrics, Android presentation metrics, hotspot fractions or coordinates, source viewbox dimensions, colors, glow, wave, halo, ripple, trail, motion, no-no render effect, and optional sound metadata.

Add generator validation. The generator must reject:

- unknown keys,
- missing required keys,
- invalid schema version,
- negative or zero invalid durations,
- alpha outside declared range,
- nonfinite floats,
- invalid hotspot or cursor dimensions,
- inconsistent source/presentation geometry,
- excessive gesture point limits,
- unknown enum/string values.

Generated files:

    crates/sky-cua-platform/src/overlay_spec_generated.rs
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlaySpec.kt

Generated files must include a “do not edit” header, source path, schema version, and generator identifier/hash. The Rust module should expose `sky_cua_platform::overlay_spec` with typed constants or typed `const` structs using platform-neutral types only. The Kotlin file should expose `object OverlaySpec` with `const val` where possible.

Add `scripts/generate_overlay_spec.py` with `--check` and idempotent output. Add `scripts/test_overlay_spec_codegen.py` for normal generation, stale-file detection, validation failures, and byte-identical regeneration.

Do not migrate Android consumers in this phase beyond compiling generated constants if necessary. Android consumer migration belongs to Phase 6.

### Phase 2: Platform protocol, service timing, host lifecycle, and capture barriers

Add a platform-neutral event model. Prefer this shape over `Vec<AgentCursorPoint>` so coordinate metadata is not repeated per point:

    pub struct AgentOverlayGestureEvent {
        pub event_id: String,
        pub sequence: u64,
        pub kind: AgentOverlayGestureKind,
        pub coordinate_space: CoordinateSpace,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub mapping_id: Option<String>,
        pub points: Vec<Point2>,
        pub duration_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub source_action: Option<ActionName>,
    }

    pub struct Point2 {
        pub x: f64,
        pub y: f64,
    }

    pub enum AgentOverlayGestureKind {
        Tap,
        Drag,
        Swipe,
        NoNo,
    }

Use an explicit overlay-host message kind and bump the protocol:

    OverlayHostMessageKind::AnimateGesture

Rules:

- `SetCursor` and `Show` carry persistent state only.
- `AnimateGesture` carries one-shot event intent only.
- Host deduplicates `event_id` within a bounded recent-event cache.
- Host rejects stale `sequence` values with diagnostics.
- Host clamps duration through generated spec constants.
- Tap and NoNo require one point; Drag and Swipe require at least two points.
- Point count is bounded by generated spec.
- Coordinates must be finite.
- The visible host should receive `DesktopLogical` points when rendering desktop overlay.
- Host restart restores persistent cursor state but does not replay old gesture events.
- Rapid redirects replace the current glide target instead of queueing unbounded animations.
- Hide-for-capture cancels or suspends visible effects according to the host state machine.

Action timing contract:

| Action | Visual begins | Input dispatch | Visual completion |
| --- | --- | --- | --- |
| Click | The service may send glide intent when it accepts a coordinate action. | Dispatch follows the existing backend action contract and is not delayed waiting for visible glide. | Tap ripple is sent only after successful dispatch. Failed dispatch cancels/clears pending visual feedback. |
| PerformSecondaryAction | Same as Click, with action-specific source metadata. | Same as Click. | Same as Click. |
| Drag | The service may send drag/swipe visual intent when it accepts the action. | Dispatch follows backend gesture contract. | Trail completion/fade follows successful outcome; failed dispatch cancels or marks the animation failed. |
| Swipe | Same as Drag. | Same as Drag. | Same as Drag. |
| Non-pointer action | No new gesture event. | Existing action flow. | Existing overlay state unchanged unless the action explicitly hides/clears it. |
| Failed action | Cancel pending visual event or show no success ripple. | Never dispatch if the backend input precondition failed. | State remains prior or becomes hidden with diagnostics. |

The overlay is explanatory visual feedback, not a promise that input waited for the visible cursor. Docs must not claim “cursor moved before click” unless a backend-specific actual pointer movement contract proves that behavior.

Host state machine:

    Hidden
    VisibleIdle
    AgentAnimating
    CaptureHidden
    NoNoFeedbackRenderOnly
    FailedOrUnsupported

Transition rules:

- CaptureHidden wins over all other states.
- Shutdown from any state restores the system cursor and releases surfaces.
- Host restart reconstructs only persistent state, never one-shot events.
- Agent action interrupts NoNoFeedbackRenderOnly.
- Rapid action updates redirect AgentAnimating instead of queueing unbounded events.
- Operator pointer tracking updates idle cursor state only when not in CaptureHidden.
- Device/surface loss moves to FailedOrUnsupported with structured diagnostics.

Capture barrier:

- Service sends hide-for-capture with a new sequence.
- Host cancels/suspends visible effects, submits transparent frames to every active surface, and waits for compositor frame acknowledgement or a documented timeout.
- Host reply includes `applied_sequence` or equivalent.
- Service captures only after the applied barrier reply.
- Restore uses a new sequence and does not replay canceled gestures.

Capabilities should use structured nested data, for example:

    pub struct AgentOverlayEffectsCapabilities {
        pub glide: bool,
        pub rotation: bool,
        pub halo: bool,
        pub ripple: bool,
        pub trail: bool,
        pub edge_glow: bool,
        pub inward_wave: bool,
        pub no_no_render: bool,
        pub hit_test: bool,
        pub sound: bool,
    }

    pub enum AgentOverlayCoverageKind {
        None,
        Full,
        Partial,
    }

Capabilities must also report supported coordinate spaces, maximum gesture points, renderer backend, adapter/backend name when available, protocol version, effect schema version, active output count, rendered output count, and reason diagnostics. Partial output coverage is unsupported for the core implementation unless the coordinator explicitly changes the failure policy.

Host lifecycle must distinguish:

    process unavailable
    socket ready
    backend initializing
    backend ready
    backend unsupported

WGPU initialization must not be mistaken for a startup crash merely because adapter/device/pipeline setup takes longer than the current short host readiness timeout.

### Phase 3: Renderer extraction with static Wayland WGPU parity

Create renderer modules under `crates/sky-cua-overlay-host/src/renderer/`:

    src/renderer/mod.rs
    src/renderer/wgpu.rs
    src/renderer/scene.rs
    src/renderer/animation.rs
    src/renderer/buffers.rs
    src/renderer/shaders.rs

Use a two-stage renderer/surface setup. The renderer cannot choose an adapter before seeing compatible surfaces. The shape should be closer to:

    let instance = WgpuOverlayInstance::new(label)?;
    let surfaces = host.create_surface_guards(&instance)?;
    let renderer = WgpuOverlayRenderer::new(instance, &surfaces, cursor, spec)?;

Do not expose naked `wgpu::Surface<'static>` as a public field. Host modules own RAII surface guards that guarantee native display/window lifetime and drop order. Renderer code borrows opaque surface entries and may configure/present them, but unsafe raw-handle lifetime is encapsulated by the host/surface guard.

Move WGPU-specific pieces from `layer_shell.rs` into renderer modules: instance creation, adapter/device/queue setup, surface capability validation, cursor texture creation, shader loading, pipeline creation, buffer creation, surface configuration, present-mode selection, alpha-mode selection, frame acquisition, and presentation.

Leave layer-shell with only host responsibilities: Wayland connection, output/layer creation, configure/close events, input region changes, pointer tracking, system cursor adapter, raw Wayland handles, frame callbacks, and conversion of native surfaces into surface guards.

WGPU buffer ABI rules:

- Host-visible structs used for uniforms/storage/instances must be `#[repr(C)]` or otherwise explicitly packed.
- Alignment and padding must be documented next to the WGSL struct.
- Add compile-time size/alignment assertions where practical.
- Use the repository's accepted byte-conversion pattern, or add/research a dependency only if necessary and document it in the Decision Log.
- Tests must compare representative Rust buffer bytes to expected WGSL layout assumptions.

Surface policy:

- Validate every active output surface against the selected adapter.
- For core Wayland, fail closed if all active outputs cannot be rendered with compatible WGPU surfaces.
- Do not silently draw on only the first monitor.
- Prefer `CompositeAlphaMode::PreMultiplied`; use `Auto` or `Inherit` only with VM transparency proof. Never choose `Opaque` while claiming transparent visible overlay support.
- Prefer `Mailbox` only when supported; otherwise use `Fifo`.

At the end of Phase 3, `SKY_CUA_LAYER_SHELL_RENDERER=wgpu` and default auto mode still draw the existing static cursor on Wayland in the VM. This phase must not include Android-style visual effects yet.

### Phase 4: GPU effects, WGSL conformance, and deterministic render tests

Implement visible effects as GPU-driven WGPU rendering. CPU normal-runtime work may update bounded uniforms/storage/instances, but it must not rasterize or precompose animated effects.

Target pass topology:

- Full-screen triangle or equivalent analytic shader for edge glow and inward waves using rounded-rectangle distance fields.
- Instanced quads for cursor, halo, and ripple.
- Instanced segment/capsule primitives for trails.
- One cursor texture.
- Shared frame uniform with time/spec/output state.
- Bounded gesture/control-point storage buffer.
- Premultiplied alpha throughout.

WGSL should compute per-frame position, easing, rotation, ripple radius, wave phase, halo pulse, trail fade, no-no render offset, and alpha from event seeds, target/control points, durations, and host-owned animation clock. Rust and Kotlin reference math are oracles, not the runtime animation engine.

Color semantics are part of the rendering contract. TOML colors are authored as sRGB channel values unless explicitly named otherwise. The renderer must document whether shader math operates in sRGB or linear space, configure sRGB/non-sRGB surface formats deliberately, and keep premultiplied-alpha behavior correct for that choice. VM visual tests must include light, dark, transparent, and high-contrast backdrops to catch alpha/color-space mistakes.

All tunable constants reach WGSL through generated uniforms/storage values. Do not manually duplicate TOML constants inside shader source except for unavoidable compile-time structural constants that are documented and tested.

Add GPU conformance tests:

- Preferred: a WGPU compute test shader uses the same WGSL animation functions as rendering and writes samples to a storage buffer that is compared against fixtures.
- Also add offscreen render invariants for deterministic frames.

Deterministic render/playground requirements:

- Renderer and playground accept an injected deterministic clock.
- Test exact frames at 0 ms, 50 ms, 120 ms, 250 ms, 500 ms, and completion for representative gestures.
- Verify hotspot stability under rotation, monotonic ripple radius, monotonic ripple alpha fade, brightest trail head, glow transparency outside expected region, no-no begins and ends at zero rotation, and hide produces a fully transparent surface.

Create shared fixtures:

    resources/overlay/agent_overlay_motion_fixtures.json
    resources/overlay/wgsl_animation_fixtures.json

Rust reference tests, Kotlin tests, and WGSL conformance tests must consume the same canonical samples or generated copies. Fixture values include input points, elapsed milliseconds, expected cursor position, expected heading degrees, expected glow alpha band, expected ripple progress, expected trail alpha samples, expected no-no rotation offset, and explicit tolerances.

Update `crates/sky-cua-overlay-host/src/playground.rs` so the desktop playground uses the shared WGPU renderer, deterministic clock, and GPU effect paths. It should exercise tap, redirect, swipe, fan, no-no render, grid, dark, light, and transparent backdrops. If the playground remains Wayland-only, state that in its help text and docs.

### Phase 5: Wayland hardening, multi-output correctness, frame pacing, and recovery

Harden the Wayland host after static extraction and GPU effects are proven.

Multi-output cases to cover in VM where possible:

- negative monitor origins,
- mixed logical output positions,
- fractional scaling,
- mixed scale factors,
- portrait or transformed outputs,
- mirrored outputs if the VM supports them,
- hotplug/output removal during idle and animation,
- layout changes during animation,
- zero-sized configure or temporary unconfigured surface,
- different refresh rates where testable,
- adapter/surface mismatch.

Core failure policy is fail closed for incomplete coverage. Capabilities must report coverage `none` or `full`; do not report full visible overlay if only a subset of outputs is rendered.

Frame scheduling and power behavior:

- Render from compositor frame callbacks or a bounded scheduler tied to requested animation frames, not an unconditional fixed polling loop.
- Stop requesting frames while hidden, capture-hidden, unsupported, or without configured surfaces.
- Coalesce rapid IPC updates.
- Avoid per-frame heap allocation after warm-up.
- Bound buffer writes and draw calls.
- Record frame CPU submission time and, where available, GPU timing for common VM resolutions.
- Test Vulkan and GLES paths when the VM can expose both.

Failure matrix to test or explicitly mark VM-unavailable:

- invalid and oversized gesture payloads,
- empty paths,
- NaN and infinity,
- rapid redirect storms,
- duplicate event IDs,
- stale sequence numbers,
- old protocol host or service mismatch,
- host killed mid-animation,
- host restart while visible,
- host restart during capture hiding,
- device loss or simulated device unavailable,
- surface lost/outdated/timeout/occluded/validation errors,
- out-of-memory where testable,
- output hotplug,
- capture hide timeout,
- unsupported alpha mode,
- system-cursor restore after crash.

### Phase 6: Android consumer migration and parity fixtures

After Phase 1 generator and fixture shape are frozen, update Android consumers:

    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlayMath.kt
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/AgentOverlayController.kt
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/AgentOverlayView.kt
    android/phone-companion/app/src/test/java/com/skycua/phonecompanion/overlay/AgentOverlayTest.kt

Keep public Kotlin names stable where tests call them; forwarding constants are acceptable during migration, but the source of truth must be `OverlaySpec`.

Run Android unit checks from the testing VM using the Android rules in `android/phone-companion/AGENTS.md`: JDK 21, AGP 9.2.1, Gradle 9.5.1, `compileSdk = 36`, `targetSdk = 36`, `minSdk = 30`, and no standalone Kotlin Gradle plugin.

Android's Canvas-based runtime is allowed to remain Android-native for this plan. The GPU-only constraint applies to desktop visible runtime rendering. Android parity here means shared constants, shared reference fixtures, and consistent visual behavior, not replacing Android Canvas with WGPU.

### Phase 7: Legacy renderer retirement and unsupported backend reporting

Retire legacy renderers only after Phases 3 through 5 prove the WGPU replacement in the VM.

Wayland:

- Remove SHM as a production visible renderer.
- Keep CPU drawing helpers only for screenshot synthesis or isolated tests if still needed.
- Move debug backdrops/fills into the WGPU playground.
- Keep `AgentCursorRendererBackendKind::WaylandShm` only for backward-compatible deserialization if necessary.

X11:

- Remove the rectangle drawing path from production backend selection.
- Explicit `SKY_CUA_OVERLAY_BACKEND=x11` returns Noop with a reason like `X11 visible overlay requires a WGPU X11 host, which is tracked as a follow-on plan`.
- Do not report `visible_overlay: true` with `renderer_backend: none` for X11.
- Do not keep a cursor-sized X11 window and claim it is enough for the WGPU visual language.

GNOME:

- Retire cursor actor drawing and cursor DBus methods from the visible overlay path.
- Preserve non-overlay window-control APIs such as `ListWindows` and `ActivateWindow` unless a separate review says otherwise.
- Explicit `SKY_CUA_OVERLAY_BACKEND=gnome` returns Noop with a reason that GNOME Shell visual rendering was retired and no WGPU GNOME host is available.

Auto-selection:

- Try Wayland layer-shell WGPU when `WAYLAND_DISPLAY` exists.
- Do not fall through to GNOME actor drawing or X11 rectangle drawing.
- Unsupported sessions report structured Noop capabilities with precise reasons.

### Phase 8: Documentation, packaging, and testing-VM closeout

Update `docs/features/agent-cursor-overlay.md` so it describes:

- the WGPU-only desktop visual contract,
- explicit visual/action timing semantics,
- shared generated spec and validation,
- protocol version and `AnimateGesture`,
- host state machine and capture barrier,
- GPU conformance tests,
- Wayland coverage/failure policy,
- retired SHM/GNOME/X11 visual fallbacks,
- X11 WGPU and no-no input/sound as follow-on plans,
- VM-only verification and final desktop acceptance policy,
- exact verification commands and artifact paths.

Update `docs/runtime/phone-companion-protocol.md` only if phone protocol fields change. Update `ROADMAP.md` with the completed core item and follow-up sub-items for X11 WGPU host and no-no input/sound if still desired.

Run the full automated acceptance suite in the VM:

    cargo fmt --check
    cargo test
    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest scripts/test_agent_cursor_smokes.py scripts/test_overlay_pointer_animations.py scripts/test_overlay_spec_codegen.py
    cd android/phone-companion && JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT="$HOME/Android/Sdk" ./gradlew :app:testDebugUnitTest --offline
    python3 scripts/build_plugin.py
    python3 scripts/run_gui_testing_vm_smoke.py --profile all

Do not delete this ExecPlan yet. Retirement waits for Phase 9.

### Phase 9: Final operator-desktop acceptance

This is the only phase permitted to exercise the completed feature on the operator's actual desktop. It begins only after all implementation is complete, every required automated and live test has passed in the testing VM, the full VM `all` profile has passed, and VM artifacts have been reviewed.

The coordinator performs a narrow scripted acceptance pass using the packaged/deployed build already proven in the VM. Verify at minimum:

- deployment freshness and expected binary/plugin versions,
- WGPU backend and structured capability readback,
- static cursor plus every required GPU effect and animation,
- click-through outside normal overlay surfaces,
- hide-for-capture with no residual overlay frame,
- system-cursor hide and restore,
- clean shutdown,
- uninstall/rollback or restoration to the operator's prior state,
- no persistent compositor, input, sound, or cursor changes after the test.

Record the exact commit, package artifact, desktop environment, GPU/backend, commands, and artifacts under:

    artifacts/final-desktop-overlay-acceptance/

The desktop pass is acceptance only. If anything fails, stop, restore the desktop, reopen the relevant phase, reproduce and fix the issue in the testing VM, rerun all affected VM gates plus the full VM profile, and only then schedule another final desktop pass.

After Phase 9 passes, follow `plans/AGENTS.md`: extract durable research to `docs/research/YYYY-MM-<slug>.md` if useful, update feature docs and ROADMAP, and delete this ExecPlan.

## Follow-on Plan Seeds

### X11 WGPU host follow-on

Create a separate ExecPlan if X11 visible overlay support is still desired. The proof must be timeboxed and answer:

- Is a 32-bit ARGB visual and colormap available?
- Is an X11 compositing manager present, and what happens without one?
- Can transparent override-redirect root-spanning or per-monitor WGPU windows composite correctly?
- Does fullscreen-window unredirect break visibility?
- Can Shape/XFixes set empty input regions independently of visual shape?
- Can RandR layout/hotplug be handled?
- Can Vulkan and GLES surfaces be created through the chosen raw-handle path?
- Can system cursor restoration survive forced host termination?

If those cannot be proven in the VM, keep X11 visible overlay unsupported.

### No-no input catcher and sound follow-on

The core renderer implements the no-no render effect, but click interception and sound require a separate plan. That plan must decide whether clicking the cursor consumes the click, forwards/reinjects it, exists only in idle mode, times out, and follows glyph alpha or a rectangular bound.

Sound must be optional and nonblocking. If implemented through external commands, use fixed arguments, no shell invocation, child reaping, rate limiting, cached command selection, clear missing-player behavior, and no impact on IPC replies.

## Concrete Steps

Start every implementation session with:

    cd /home/bex/projects/sky-cua
    git status --short

Every worker handoff must answer:

    package: <A/B/C1/C2/D/F/W/G/E/J/K or follow-on>
    touched_paths: <paths>
    public_contracts_changed: <yes/no; list structs/enums/schema keys/protocol names>
    generated_files_changed: <yes/no; list generator command>
    gpu_runtime_boundary_changed: <yes/no; explain any CPU animation/raster path>
    test_environment: <testing VM image/profile, desktop environment, GPU/backend, commit SHA>
    commands_run: <exact commands and result>
    live_gates_run_or_skipped: <exact gate and reason>
    artifacts: <paths or none>
    follow_up_required: <blocking/nonblocking>

The coordinator rejects handoffs that say only “tests pass” without naming commands. The coordinator also rejects authoritative tests run on the operator desktop instead of the testing VM.

For unsupported-session proof, add or update non-live tests for explicit backend modes and env auto-selection:

    SKY_CUA_OVERLAY_BACKEND=gnome sky-cua-overlay-host probe
    SKY_CUA_OVERLAY_BACKEND=x11 sky-cua-overlay-host probe
    SKY_CUA_OVERLAY_BACKEND=none sky-cua-overlay-host probe

In CI/unit tests, use mocked environment where possible. Unsupported GNOME/X11 visible renderers must return Noop capabilities with precise structured reasons.

## Validation and Acceptance

Phase 0 is accepted when the testing VM baseline is recorded, missing VM prerequisites are explicit, and the coordinator has frozen the first version of every shared contract listed in Phase 0.

Phase 1 is accepted when changing one value in `agent_overlay_spec.toml` causes generator `--check` to fail until regenerated, generated Rust/Kotlin constants reflect the new value, invalid specs are rejected, and codegen output is byte-identical on repeat runs.

Phase 2 is accepted when `AnimateGesture` round-trips with the bumped protocol version, old/new version mismatch errors are explicit, event dedup/stale sequence/cancellation rules are tested, action timing tests match the table, host lifecycle distinguishes initializing from unavailable, and hide-for-capture waits for an applied-frame barrier.

Phase 3 is accepted when Wayland WGPU static cursor parity works in the VM, renderer extraction removes WGPU device/shader/pipeline/surface policy from `layer_shell.rs`, unsafe raw-handle lifetime is encapsulated in host-owned surface guards, every active output surface is validated, and overlay-host tests pass.

Phase 4 is accepted when desktop WGPU shows required effects, WGSL compute conformance or offscreen render tests compare the actual shader path against fixtures, deterministic frame tests pass, sRGB/linear color handling and premultiplied alpha are documented and visually tested, and source review confirms no normal-runtime CPU rasterization of visible effects/animations.

Phase 5 is accepted when Wayland VM smokes cover transparent composition, click-through, hide-for-capture barrier, system-cursor restore, output coverage, frame pacing, host restart, surface/device loss handling where testable, and the failure matrix is either passed or marked VM-unavailable with precise blockers.

Phase 6 is accepted when Android overlay consumers use generated constants, Kotlin tests consume shared fixtures or generated copies, Android JVM tests pass in the VM environment, and Android visual artifacts show parity for corners, redirect, swipes, and fan scenarios.

Phase 7 is accepted when production selection has no active SHM, GNOME actor, or X11 rectangle visual renderer; unsupported GNOME/X11 modes report honest Noop capabilities; packaging still includes any non-overlay GNOME resources required for window control; and backend-selection tests pass.

Phase 8 is accepted when docs/ROADMAP are updated, packaging succeeds, the full automated acceptance suite passes in the VM, `python3 scripts/run_gui_testing_vm_smoke.py --profile all` passes, and VM artifacts are recorded.

Phase 9 is accepted when the controlled operator-desktop acceptance pass succeeds on the exact commit/package already proven in the VM and the desktop is restored cleanly afterward.

The full plan is accepted only after Phases 0 through 9 pass. If a broad command such as `cargo test` fails for an unrelated pre-existing reason, record the exact failure and the narrower passing commands that cover this work. Do not substitute a desktop run for a missing VM gate.

## Idempotence and Recovery

The code generator must be idempotent. Running `uv run python scripts/generate_overlay_spec.py` repeatedly should produce byte-identical generated Rust and Kotlin files unless the TOML changes.

Backend selection changes must fail closed. If WGPU initialization fails, the overlay host should return Noop capabilities with a reason and keep screenshot-synthetic cursor support unaffected. It must not panic, leave the system cursor hidden, or claim `visible_overlay: true`.

System cursor hiding must restore on shutdown and drop. Preserve current drop/restore patterns and add tests where practical for state transitions. If a VM live smoke leaves the system cursor hidden, use existing KWin/system cursor recovery first, then document the failure before retrying.

WGPU surface loss, resize, occlusion, timeout, and validation errors need explicit handling. Lost/outdated surfaces should reconfigure and retry on the next frame. Timeout/occluded frames should not kill the host. Validation errors should produce diagnostics and fail closed. No path may keep a destroyed native window's `wgpu::Surface` alive.

Live smoke scripts should skip honestly when a required compositor, display, Android device passthrough/forwarding, or portal is unavailable in the testing VM. Do not make tests pass by weakening assertions around visible overlay capabilities. Unsupported sessions should assert the unsupported reason. Do not move a skipped VM test onto the operator desktop; treat the missing VM prerequisite as a blocker until the VM can exercise it.

Parallel-work recovery rule: if a worker branch conflicts in a coordinator-owned contract file, do not resolve by preserving both semantics. Re-open the contract decision, choose one source of truth, update this plan, then rebase or discard the losing branch. Duplicate protocol fields, duplicate spec keys, duplicate renderer entry points, duplicate buffer ABIs, or parallel generated-file formats are merge blockers.

If final operator-desktop acceptance fails or leaves any cursor/compositor/input state behind, restore the desktop immediately and return to VM-only development. A desktop failure invalidates final acceptance but does not authorize iterative debugging on the desktop.

## Artifacts and Notes

Current source anchors:

    crates/sky-cua-platform/src/model.rs
    crates/sky-cua-service/src/overlay.rs
    crates/sky-cua-service/src/daemon.rs
    crates/sky-cua-service/src/overlay/host/mod.rs
    crates/sky-cua-service/src/overlay/host/lifecycle.rs
    crates/sky-cua-overlay-host/src/lib.rs
    crates/sky-cua-overlay-host/src/main.rs
    crates/sky-cua-overlay-host/src/layer_shell.rs
    crates/sky-cua-overlay-host/src/x11.rs
    crates/sky-cua-overlay-host/src/gnome_shell.rs
    crates/sky-cua-overlay-host/src/playground.rs
    resources/gnome-shell-extension/codex-window-control@openai.com/extension.js
    resources/kwin/effects/sky-cua-agent-cursor/
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlayMath.kt
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/AgentOverlayController.kt
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/AgentOverlayView.kt
    android/phone-companion/app/src/test/java/com/skycua/phonecompanion/overlay/AgentOverlayTest.kt
    .agents/skills/overlay-pointer-animations/SKILL.md
    .agents/skills/vm-tests/SKILL.md

Expected new source anchors:

    resources/overlay/agent_overlay_spec.toml
    resources/overlay/agent_overlay_motion_fixtures.json
    resources/overlay/wgsl_animation_fixtures.json
    scripts/generate_overlay_spec.py
    scripts/test_overlay_spec_codegen.py
    crates/sky-cua-platform/src/overlay_spec_generated.rs
    crates/sky-cua-platform/src/overlay_animation.rs
    crates/sky-cua-overlay-host/src/renderer/mod.rs
    crates/sky-cua-overlay-host/src/renderer/wgpu.rs
    crates/sky-cua-overlay-host/src/renderer/scene.rs
    crates/sky-cua-overlay-host/src/renderer/animation.rs
    crates/sky-cua-overlay-host/src/renderer/buffers.rs
    crates/sky-cua-overlay-host/src/renderer/shaders.rs
    android/phone-companion/app/src/main/java/com/skycua/phonecompanion/overlay/OverlaySpec.kt

Expected artifact directories:

    artifacts/overlay-pointer-animations/
    artifacts/desktop-overlay-pointer-animations/
    artifacts/gui-testing-vm/
    artifacts/final-desktop-overlay-acceptance/

All implementation and verification artifacts except the final acceptance directory must originate from the testing VM. The final desktop directory must contain only the narrow Phase 9 acceptance evidence.

Worker handoffs should stay concise in this plan or in commit/PR text. Do not create per-worker shadow plans. If a worker produces durable research, extract it later into `docs/research/YYYY-MM-<slug>.md`; if it produces live proof, point to the artifact directory and summarize one line here.

## Interfaces and Dependencies

Rust workspace dependencies are root-managed. Current relevant root dependencies are:

    wgpu = { version = "29.0.3", default-features = false, features = ["std", "wgsl", "vulkan", "gles"] }
    x11rb = { version = "0.13.2", features = ["shape", "xfixes"] }
    wayland-client = { version = "0.31.11", features = ["system"] }
    wayland-protocols = { version = "0.32.9", features = ["client", "staging"] }
    smithay-client-toolkit = { version = "0.20.0", default-features = false }
    calloop = "0.14.4"
    calloop-wayland-source = "0.4.1"
    pollster = "0.4.0"
    image = { version = "0.25.8", default-features = false, features = ["jpeg", "png", "webp"] }
    serde = { version = "1.0.228", features = ["derive"] }
    serde_json = "1.0.145"
    toml = "1.1.2"

Do not add a new graphics dependency for the desktop renderer unless WGPU cannot draw one of the required primitives. Do not add CPU raster/vector animation dependencies for normal visible effects; animation should live in the WGPU renderer. If a dependency is added, document its version, reason, and why existing dependencies could not solve the problem.

The shared spec should be exposed from Rust as `sky_cua_platform::overlay_spec` or a similarly explicit module. Generated constants should be plain values so renderer hot paths do not parse config files during frames.

The desktop overlay host protocol remains over the existing socket/TCP transport. The new one-shot animation event uses the existing channel with `OverlayHostMessageKind::AnimateGesture`; do not create a second IPC channel unless profiling shows a measured reason.

The WGPU renderer must not import Wayland, X11, GNOME, DBus, or service modules. Host modules may import WGPU only for instance/surface creation/configuration and renderer integration. Unsafe raw-handle lifetime must be isolated in host-owned RAII surface guards.

The Android companion should not import Rust-generated files directly. It receives generated Kotlin constants and keeps its existing Android build system. Any Gradle integration for generation must respect `android/phone-companion/AGENTS.md`.

The KWin effect remains a pointer-tracking and system-cursor helper. It must not become a cursor renderer. If its API needs to report animation-related pointer positions, keep those reports as input to the WGPU renderer.

## Change Notes

- 2026-06-25 / Codex: Created the initial multi-phase ExecPlan from current source state and requested architecture direction: remove desktop-specific cursor drawing from X11/GNOME, unify desktop visuals under WGPU, and share Android-style animation constants and behavior across phone and desktop.
- 2026-06-25 / ChatGPT: Reviewed and enhanced the plan against current source and current WGPU/raw-window-handle/x11rb facts. Added missing constraints for raw handle lifetime, surface capability validation, alpha/present-mode behavior, protocol compatibility, action timing, unit normalization, X11 full-screen WGPU proof, GNOME non-overlay preservation, no-no input regions, structured capability fields, and full GPU rendering of visible effects/animations.
- 2026-06-25 / ChatGPT: Added a coordinator/parallel-worker execution model: worker packages A-J, hard dependency order, coordinator-owned shared files, contract-freeze rules, handoff format, integration gates, and merge-conflict recovery rules.
- 2026-06-25 / ChatGPT: Reworked the plan into a tighter core/follow-on implementation plan. The open risks are now handled by concrete phase requirements: explicit action timing semantics, `AnimateGesture` event protocol and version bump, host state machine, capture applied-frame barrier, two-stage WGPU renderer initialization, RAII surface ownership, WGSL compute/offscreen GPU conformance tests, deterministic frame tests, color-space/alpha requirements, Wayland multi-output fail-closed policy, frame scheduling/power budgets, stronger failure matrix, nested effect/coverage capabilities, stricter shared-spec schema validation, legacy-renderer retirement after GPU proof, X11/no-no input/sound as follow-ons, and mandatory testing-VM verification before final desktop acceptance.
