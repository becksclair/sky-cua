# sky-cua-overlay-host Agent Guide

The desktop agent-cursor overlay host: the `serve` binary the service spawns
(JSON-lines protocol, one request/reply per Unix-socket connection), the
platform-neutral surface host, its Wayland layer-shell and macOS panel
platforms, the wgpu analytic-shader renderer, and the CPU-side cursor motion
driver ported from the Android companion.

## Module layout

- `src/lib.rs` — protocol types (`OverlayHostMessage`/`OverlayHostReply`
  incl. the optional structured `motion` echo), gesture validation, the noop
  backend, backend selection, and `cursor_asset` constants.
- `src/main.rs`, `src/serve.rs` — the binary: command dispatch, and the
  `serve` transports (stdin/stdout, Unix socket, TCP) with the shared polling
  accept loop and `ParentWatch` (every serve loop exits when the process that
  started the host is gone). `serve/linux.rs` is the calloop socket loop,
  `serve/macos.rs` the main-run-loop socket loop, `serve/tests.rs` the
  transport tests (pinned to the noop backend).
- `src/motion.rs` — pure motion math: `Mover2D` vehicle steering, angle and
  arc-length path helpers. Op-for-op port of Android `OverlayMath`; the
  numeric recipe (f32 state, f64 transcendentals truncated back per
  assignment) is a cross-language parity contract — do not "simplify" it.
- `src/cursor_motion.rs` — `CursorMotionDriver`, the stateful ambient loop:
  target precedence, arrival-gated gesture feedback, rotation easing, cloud
  bloom, capture-hide freeze. Pure and deterministically steppable (explicit
  `Instant` + epoch ms); mover dt is monotonic-only, effect timelines are
  epoch-ms-only — never mix.
- `src/surface_host.rs` — `OverlaySurfaceHost<P: SurfacePlatform>`, the
  visible-overlay backend shared by every platform: message handling, the
  arrival wait, tick, render/reply, and the draw loop. The motion driver steps
  exactly once per rendered frame at the top of `draw()`; do not step it
  anywhere else (every render path funnels through `draw`, so a second site
  double-integrates). Nothing here may name a windowing system. Submodules:
  - `surface_host/platform.rs` — the `SurfacePlatform` trait: native
    surfaces as `SurfaceSlot`s, raw handles, the per-frame hand-off
    (`begin_frame`/`prepare_surface`/`finish_frame`), the capture barrier
    mechanism, surface-set changes, and the system-cursor and
    pointer-telemetry adapters. A new desktop is a new implementation of this
    trait, not a copy of the host.
  - `surface_host/geometry.rs` — `OutputGeometry`, the per-output snapshot
    the 60-240 Hz draw path reads. Platforms rebuild it only on output
    events; per-frame code must never query the windowing system for
    geometry (`SurfacePlatform::slot` must not allocate).
  - `surface_host/motion_adapter.rs` — pure mapping from the motion frame to
    per-surface renderer inputs (motion bounds, feedback scenes). Every
    output is a window into the shared desktop-logical scene: the
    glyph/ripple/trail are handed to every output they reach via unclipped
    translation (the WGSL clips per-pixel), so boundary-spanning motion is
    continuous across the arrangement with no edge-pop.
  - `surface_host/renderer_selection.rs` — the wgpu-or-honestly-unsupported
    selection state.
  - `surface_host/capabilities.rs` — structured capability/coverage
    reporting (clients never infer backend state from prose).
  - `surface_host/cursor_state.rs` — pure `AgentCursorState` point helpers
    and pointer-telemetry reconciliation (moves the target, never the drawn
    pose).
- `src/layer_shell.rs` — the Wayland platform (`LayerShellPlatform`): the
  connection and event queue, one layer surface per output, and
  `SKY_CUA_LAYER_SHELL_RENDERER` parsing. Submodules:
  - `layer_shell/wayland.rs` — surface creation, raw handles, and the SCTK
    handler/dispatch impls on `LayerShellState`; output handlers rebuild the
    geometry snapshot.
  - `layer_shell/geometry.rs` — `OutputInfo` to `OutputGeometry`. SCTK's
    `OutputState::info()` clones the full `OutputInfo` (Strings + mode list),
    so it is called from output events only.
  - `layer_shell/capture_barrier.rs` — the frame-callback hide-for-capture
    barrier.
- `src/macos_panel.rs` — the macOS platform (`MacosPanelPlatform`): the
  accessory application, one click-through `NSPanel` per screen, the
  window-list capture barrier, and panel replacement on a screen change.
  Main thread only; public AppKit/CoreGraphics/CoreFoundation interfaces
  only. Submodules:
  - `macos_panel/presence.rs` — the pure show/hide state machine. A Metal
    layer may only be presented to while the window server composites its
    window, and AppKit commits ordering on a later run loop turn, so panels
    are drawn only after the window server lists them on screen.
  - `macos_panel/display.rs` — screen enumeration, `CGDisplayBounds` to
    `OutputGeometry`, and the on-screen window-list query.
  - `macos_panel/run_loop.rs` — `ReadableWake` (a `CFFileDescriptor` source
    that wakes the main run loop for the listener socket) and event
    delivery.
- `src/renderer/` — wgpu renderer: `buffers.rs` (uniform ABI; 288 bytes,
  lane map documented per field), `shaders.rs` (all WGSL; effects are
  composited functions, not passes), `cursor_texture.rs` (SDF glyph bake),
  `scene.rs`, `surface.rs`, `wgpu.rs`, plus test-only `test_support.rs`
  (shared offscreen helpers) and `motion_capture.rs` (dense-frame dump).
- `src/playground.rs` — interactive live testbed; runs its own
  `CursorMotionDriver` so motion reads exactly as production.

## Testing

- `cargo nextest run -p sky-cua-overlay-host` — never `cargo test` (root
  CLAUDE.md explains why). All tests are inline `#[cfg(test)]` modules.
- Cross-language parity is pinned by
  `resources/overlay/agent_overlay_motion_fixtures.json` (generated by
  `scripts/generate_motion_fixtures.py`; Kotlin `OverlayMath` is the
  behavioral reference — on disagreement fix the generator, never Kotlin) and
  `resources/overlay/wgsl_animation_fixtures.json` (GPU compute conformance
  via `cs_main` on a real adapter). Heading assertions must compare wrapped:
  `|wrap_radians(expected − actual)| ≤ tol`; mid-flight mover samples use
  `tolerance.mover`, settled samples the default.
- Env-gated evidence dumps (GPU required, deterministic by construction —
  fabricated `Instant`s and scripted `now_ms`, no wall clock):
  - `SKY_CUA_CAPTURE_GESTURES=1 … -E 'test(capture_gesture_frames_when_requested)'`
  - `SKY_CUA_CAPTURE_MOTION=1 … -E 'test(capture_motion_frames_when_requested)'`
- Visual acceptance is human-judged via the `agent-cursor-debug` skill
  (stills + the motion video harness `scripts/overlay_motion_animations.py`);
  the structured pass/fail glide check is
  `live_agent_cursor_kde_smoke.py --mode layer-shell-motion-glide`.
- The Wayland and macOS platforms have no automated live tests. A change to
  `surface_host` or a platform needs a live run on that desktop; on macOS,
  start `sky-cua-overlay-host serve --socket <relative path>` from a terminal
  and drive it with protocol messages (see the Verification section of
  `docs/features/agent-cursor-overlay.md`).

## macOS pitfalls

- AppKit commits window ordering to the window server when the main run loop
  reaches its waiting stage. Code that waits on the main thread for an
  ordering change must run the run loop (`SurfacePlatform::wait`), not sleep.
- Never present to a panel that is not on screen: the drawable is held, and
  once the layer's drawables are all held the next frame blocks the main
  thread for a second. `PanelPresence` plus the unsynchronized present mode
  (`PRESENT_WITHOUT_DISPLAY_SYNC` in `renderer/surface.rs`) are what prevent
  it; keep both.
- The panels are non-shareable on purpose (`NSWindow.sharingType = none`;
  see "Capture exclusion" in `docs/features/agent-cursor-overlay.md`), so
  on macOS 27 and later `screencapture` and ScreenCaptureKit do not show the
  overlay (the host claims capture exclusion only there). To capture it
  for visual work, start the host (or the daemon that spawns it) with
  `SKY_CUA_OVERLAY_CAPTURE_EXCLUSION=off`. It is a development switch,
  exempted in `scripts/test_env_key_contract.py`, not an operator setting.
- The host exits when its parent process id changes. Attaching a debugger
  re-parents it, so launch the host under the debugger instead of attaching.
- A build into `target/release` replaces the binary a registered release
  daemon spawns. Use the debug profile or a private `CARGO_TARGET_DIR` for
  experiments.

## Invariants

- Clients never infer state from prose: capabilities, lifecycle, and the
  motion pose ride in structured reply fields.
- `state` is the cursor TARGET; the motion driver owns the drawn pose.
  Wholesale `SetCursor` state replacement must stay safe (it only moves the
  target).
- Uniform layout is 288 bytes with every lane assignment documented in
  `buffers.rs`; a new periodic clock input must divide
  `BREATHING_CLOCK_MODULUS_MS` (const-asserted) or use effect-relative time.
- The motion constants are strict shared-spec parity (`[shared.motion]`,
  dp read as logical px). Desktop-specific tuning goes through a
  `[desktop.motion]` spec section, never hardcoded divergence.
