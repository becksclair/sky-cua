# Agent cursor overlay

## Status

Shipped on Linux with WGPU-only production visible rendering on Wayland.
Windows native overlay deferred until a Windows machine is available for live
proof. Last verified: 2026-07-02 on the operator KDE Wayland desktop (2560x1440
logical) — phone-parity cursor motion: the structured
`layer-shell-motion-glide` smoke passed (snap at A, unsettled strictly-between
mid-flight via the `motion` echo, exact settle at B), the offline dense-frame
dump shows the momentum bow on redirect and the ripple-free glide until
arrival, and the live stills harness captured the glow + glyph + ripple over
the real desktop (`artifacts/overlay-motion-animations/`). Portal MP4
recording and VM overlay profiles not yet run for this change. The prior
glyph/aura/shadow stills acceptance is 2026-06-27; the prior full VM/commit
acceptance is 2026-06-25 at commit
`00a4eb657237924c0bb3b15ae1ce72ae4e593e2b`.

## Summary

A visible overlay marker showing where the agent is about to click or has
clicked, plus a synthetic cursor composited into the model-facing screenshot
so Codex sees the same placement. The overlay state lives in the service and
is published over a JSON-lines IPC to a separate `sky-cua-overlay-host`
process that owns compositor-specific drawing.

## Contract surface

Public model in `crates/sky-cua-platform/src/model.rs`:

- `AgentCursorState { visible, sequence, model_point, native_point, snapshot_id, source_action, updated_at_ms }`
- `AgentCursorPoint { x, y, coordinate_space, mapping_id }`
- `AgentCursorCapabilities { backend, renderer_backend, visible_overlay, screenshot_synthetic_cursor, click_through, capture_exclusion, pointer_tracking_backend, pointer_tracking_exact, system_cursor_hide_supported, system_cursor_backend, system_cursor_hidden, needs_user_install, reason }`
- `AgentCursorBackendKind`: production visible rendering currently uses
  `wayland_layer_shell` or `none`. `x11_shaped_window`,
  `gnome_shell_extension`, `cosmic_comp_bridge`, and
  `cosmic_transparent_xcursor` remain serialized compatibility values or
  historical capability values, not selectable production renderers for the
  WGPU desktop overlay.
- `AgentCursorRendererBackendKind`: production visible rendering emits `wgpu`
  or `none`. `wayland_shm` remains only for backward-compatible
  deserialization of old status.
- `AgentCursorPointerTrackingBackendKind`: `kwin_effect_signal`,
  `privileged_input_helper`, `x11_query`, `none`.
- `AgentCursorPlane`: `UserVisible`, `ScreenshotSynthetic`
- `AgentCursorSystemCursorBackend`: see `docs/features/compositor-cursor-hiding.md`
- `AppStateSnapshot.agent_cursor`, `ActionOutcome.agent_cursor` (preserved in compact snapshots)

Service IPC variants: `ServiceRequest::AgentCursorStatus`, `SetAgentCursor`,
`HideAgentCursor`, `ShowAgentCursor`. These are internal and not exposed as
MCP tools.

Overlay-host JSON-lines protocol on
`$XDG_RUNTIME_DIR/sky-cua/agent-cursor.sock` with versioned messages
`hello`, `capabilities`, `set_cursor`, `animate_gesture`, `hide`, `show`,
`ping`, `shutdown`. `animate_gesture` carries `AgentOverlayGestureEvent`
for one-shot tap, drag, swipe, and no-no render effects.

`set_cursor` is persistent state. `animate_gesture` is one-shot event intent
with `event_id`, `sequence`, gesture kind, coordinate space, optional mapping
id, bounded points, duration, and source action metadata. The host deduplicates
recent event ids, rejects stale sequences, clamps durations from the generated
spec, and never replays one-shot events after restart.

Every layer-shell reply (including `ping`) carries an optional structured
`motion` field — `{ x, y, heading_deg, speed, settled,
pending_gesture_feedback }` — echoing where the vehicle-steered glyph
actually is, in the mover's coordinate space. `state` remains the target;
`motion` is the drawn pose. The field is serde-optional (absent from older
hosts and the noop backend), so no protocol version bump.

Environment variables (allowlisted in `resources/chrome_preflight.py`):

- `SKY_CUA_AGENT_CURSOR` — `auto` (default), `on`, `off`
- `SKY_CUA_OVERLAY_BACKEND` — `auto`, `wayland_layer_shell`, `x11`,
  `gnome_shell_extension`, `none`. `x11` and `gnome_shell_extension` return
  Noop capabilities with explicit unsupported reasons until WGPU hosts exist.
- `SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE` — `auto`, `on`, `off`
- `SKY_CUA_OVERLAY_HOST_PATH` — explicit path; defaults to bundled binary
- `SKY_CUA_SCREENSHOT_CURSOR` — `auto`, `on`, `off`

Shared visual constants and deterministic fixtures:

- `resources/overlay/agent_overlay_spec.toml` — source of truth for colors,
  timing, geometry, motion, and desktop/Android rendering constants.
- `resources/overlay/agent_overlay_motion_fixtures.json` — cross-language
  motion/effect math samples.
- `resources/overlay/wgsl_animation_fixtures.json` — WGPU shader conformance
  samples for deterministic desktop frames.

## Behavior

### Idle cleanup policy

The service owns the visual overlay lifecycle. Agents should end their
computer-use session explicitly, and sky-cua hides the overlay as part of that
cleanup path. If an agent turn is interrupted or abandoned, the service idle
watchdog hides the whole agent-cursor overlay after 15 seconds. The same
service cleanup runs lazily before snapshot synthesis, so stale visible cursor
state is not composited into a later capture.

The overlay host does not run an independent idle-hide policy. Its socket loop
uses `calloop` timers only to advance frame-paced work such as pointer tracking;
it keeps the overlay visible until the service sends `hide`, `show`, or
`shutdown`.

On KDE, the KWin shim keeps a separate 8 second fail-safe solely to restore the
system compositor cursor if the service or overlay host dies after hiding it.
The shim does not draw the agent cursor; layer-shell owns all visuals. It emits
`PointerMoved(double x, double y, qulonglong sequence)` while the agent cursor
is visible so the click-through layer-shell overlay can follow the compositor
pointer without shell polling. `PointerStateJson` remains a status/fallback
method. Hide/Show and the failsafe keep the `StateJson` introspection's
`visible` field in sync.


The overlay has two planes that operate independently:

1. **`UserVisible`** — a native overlay surface the user can see on the
   desktop. Production drawing is WGPU-only through Wayland layer-shell.
2. **`ScreenshotSynthetic`** — a marker composited into the screenshot Codex
   receives. Always available when capture is available, regardless of
   visible-overlay support.

Backend selection at `auto`:

- Wayland sessions with `WAYLAND_DISPLAY`: `wayland_layer_shell` attempts WGPU
  on every active output. If WGPU initialization, surface coverage, or frame
  submission fails, the host fails closed with `visible_overlay=false`,
  `renderer_backend=none`, `coverage=none`, structured counts, and a reason.
- KDE/KWin: the layer-shell WGPU renderer owns all visuals. The compiled KWin
  effect, when installed and loaded, is used only as
  `system_cursor_backend=kwin_effect` and
  `pointer_tracking_backend=kwin_effect_signal`.
- Hyprland and other layer-shell compositors: the same WGPU renderer is used.
  When no exact compositor tracker is available, the privileged input helper
  can stream relative evdev motion as
  `pointer_tracking_backend=privileged_input_helper` with
  `pointer_tracking_exact=false`.
- GNOME Shell: the bundled `codex-window-control@openai.com` extension keeps
  its non-overlay window-control DBus APIs. Its cursor actor renderer is
  retired. Explicit `SKY_CUA_OVERLAY_BACKEND=gnome_shell_extension` returns
  Noop capabilities explaining that no WGPU GNOME overlay host is available.
- X11/i3: the shaped-window rectangle renderer is retired. Explicit
  `SKY_CUA_OVERLAY_BACKEND=x11` returns Noop capabilities explaining that X11
  visible overlay requires a follow-on WGPU X11 host.
- `SKY_CUA_LAYER_SHELL_RENDERER=shm` is rejected as a retired legacy renderer;
  production Wayland visuals require WGPU.

Action-driven cursor placement:

- Explicit `x`/`y` action arguments are screenshot pixels and become the
  model-facing cursor point.
- Element-targeted actions use the centre of `resolved_element.bounds` when
  the bounds are in `StreamPixels`.
- Drags update the cursor to the final target point.
- Snapshotless explicit-coordinate actions use native-only cursor state
  rather than stale model pixels.
- Successful pointer actions whose coordinates cannot be mapped clear stale
  cursor state instead of leaking last-cursor pixels.

Capture guard: when `SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE=on`, the overlay
controller hides the visible overlay just before capture and restores it
afterward. The service sends a hide sequence, the host presents transparent
frames on every active surface, and capture waits for the matching applied
barrier before reading the desktop. The synthetic screenshot cursor is
composited regardless.

The visible overlay is explanatory feedback, not an input-dispatch clock.
Coordinate pointer actions start a real glide when accepted: the service sends
the target before input dispatch (`prepare_action_visual`), and the host's
CPU motion driver sails the glyph toward it with vehicle-steering physics.
Backend input dispatch follows the existing action contract and is not delayed
waiting for animation completion — the click lands immediately while the glyph
may still be in flight. Gesture visual feedback (ripple, press squash, trail)
is arrival-gated to match the phone: it starts when the glyph settles at the
gesture point, not when the event arrives. Failed dispatch cancels pending
visual feedback.

The host lifecycle states are `Hidden`, `VisibleIdle`, `AgentAnimating`,
`CaptureHidden`, `NoNoFeedbackRenderOnly`, and `FailedOrUnsupported`.
`CaptureHidden` takes precedence over animation, shutdown restores compositor
cursor state and releases surfaces, and unsupported or failed WGPU coverage is
reported through structured capabilities instead of prose fallback inference.

### WGPU desktop effects

On Wayland layer-shell with `renderer_backend=wgpu`, visible desktop effects
are rendered by `crates/sky-cua-overlay-host/src/renderer/shaders.rs` in WGSL.
The CPU host validates events, maps coordinates into output-local scene data,
updates bounded uniform/storage buffers, and advances a monotonic clock. It
does not rasterize or precompose edge glow, halo, ripple, trail, cursor
glide/rotation, no-no frames, or cursor pixels for the normal WGPU runtime
path.

The WGPU pass draws a full-screen analytic shader into a premultiplied-alpha
surface. TOML color channels are authored as sRGB 0..255 values and normalized
before upload. The surface policy requires a non-sRGB 8-bit swapchain format
(`Bgra8Unorm`, then `Rgba8Unorm`) and fails closed when neither is advertised;
the shader
returns premultiplied color for `CompositeAlphaMode::PreMultiplied`. Avoiding an
sRGB swapchain here is deliberate: transfer-encoding RGB after premultiplication
can make stored color channels exceed alpha, causing the Wayland compositor to
produce pale or white fringes around translucent edge and cursor smoke.

The analytic effect math is kept in parity with the Android companion's
`OverlayMath`: the shared spec drives identical timing, easing, and amplitude
constants, and the WGSL curves — eased-triangle breathing, the no-no head-shake
envelope, the damped-cosine click bounce, and the eased tap-ripple radius —
match the Kotlin reference. `resources/overlay/wgsl_animation_fixtures.json`
pins the desktop output against the canonical samples, including off-quarter
phases that distinguish the curves, and the GPU conformance test asserts them
on a real adapter.

#### Cursor motion (vehicle steering)

The drawn cursor position is owned by a CPU-side motion driver
(`src/cursor_motion.rs` around the pure math in `src/motion.rs`), an op-for-op
port of the Android companion's `OverlayMath.Mover2D` and ambient controller
loop. Every upstream input — `set_cursor` repositioning, gesture events,
compositor pointer telemetry — only moves a *target*; the vehicle model does
the sailing with a bounded turn rate (homing-boosted near the target so it
spirals in instead of orbiting), accel-limited speed with arrive-radius
deceleration, a settle snap that resets the resting nose heading, and a
`CURSOR_MAX_STEP_S` dt clamp. A redirect mid-flight keeps its momentum and
bows the path — the signature curve. Constants come from the shared spec's
`[shared.motion]` unchanged (dp read as logical px, strict phone parity);
tuning for large desktops, if ever needed, is a `[desktop.motion]` follow-up.

The driver steps exactly once per rendered frame at the top of
`LayerShellApp::draw` (both the tick timer and message replies route through
it), with dt from a monotonic clock and effect timelines on epoch
milliseconds. It also eases the glyph rotation toward the travel heading
(`CURSOR_ROTATE_RATE_DEG_PER_S`, active above
`CURSOR_ROTATE_MIN_SPEED_DP_PER_S`, back to rest when parked; the no-no wiggle
owns rotation while it plays), ramps the smoke-aura alpha in over ~0.8 s on a
cold show, arrival-gates gesture feedback
(`speed == 0 && dist <= GESTURE_ARRIVE_LOGICAL_PX`, feedback clock starts at
arrival), chases the arc-length head of drag/swipe polylines with full
steering physics while the trail traces the ideal path (the phone's signature
pursuit asymmetry), and resamples the trail into `TRAIL_SAMPLES` arc-length
points per frame. Targets are clamped into output bounds before steering so
an off-screen gesture target can never wedge the arrival gate open. The
in-shader drag lerp (`animated_cursor_position`) is retired; the shader draws
at `frame.cursor.xy` with rotation from `surface_size_px.w` and cloud alpha
scaling `halo.w`. Capture hides freeze the driver so restore resumes from the
same pose; plain hides drop the gesture pipeline and re-bloom the aura on the
next show.

Multi-output rendering treats every output as a window into the shared
desktop-logical scene (`layer_shell/motion_adapter.rs`): the mover integrates
in global logical space, and the glyph, ripple, and trail are handed to every
output they reach — each translated (unclipped) into that output's local
coordinates while the WGSL clips per-pixel. A glide or a boundary-spanning
gesture therefore renders continuously across a monitor seam, following the
compositor's arrangement (logical positions), instead of clipping at an edge
and popping onto the neighbour. Because each surface applies its own integer
`render_scale`, outputs at different scales stay visually continuous across the
seam; the glyph-visible flag (`flags.x`) tracks cursor presence only, so an
output that receives a gesture scene but not the cursor draws the ripple/trail
without a stray glyph. All logical-px geometry lanes — cursor footprint,
trail stroke, ripple radii/stroke, and the no-no halo radius — scale with the
buffer, so effects keep their authored size on HiDPI / fractionally-scaled
outputs. Cheap per-output culls (footprint reach for the glyph, ripple-radius
reach for the scene) keep the continuous per-frame shader work on the one or
two outputs actually touched.

The ambient edge glow is gated on the agent-in-control lease (`flags.w`), the
desktop analogue of the Android companion's `glowActive`: it lights while the
overlay holds a visible state at full coverage and stays dark otherwise,
deliberately decoupled from per-surface cursor presence.

On the desktop the edge glow is a drifting, domain-warped value-noise (fbm)
field rather than the stroked/blurred band the Android companion draws: a bright
rim hugs the very edge (~0.8mm), a fuller smoke layer banks against the border
and breaks into wisps further in, and the whole effect crawls over time. It is
saturated toward the light-pink palette even in low-density haze, while rim and
body alpha combine as overlapping translucent layers instead of an additive
sum. This keeps the lavender-pink appearance without opaque hot spots. It is
sized in physical units — the host packs each output's logical-pixels-per-
millimetre (derived from the `wl_output` physical size and logical size) into
`surface_size_px.z`, so the rim width and the ~2.5cm containment depth read the
same across monitors of differing DPI. The band is additionally capped to a
fraction of the smaller screen dimension so it stays proportional on small
panels instead of swallowing them. The separate concentric inward waves are
retired on the desktop path — their motion is folded into the glow's drift — so
`inward_waves` is a no-op there while the Android companion still renders
discrete waves from `OverlaySpec.Android`. The desktop edge glow therefore no
longer reads the `glow_*`/`wave_*` stroke/blur spec constants, which remain
authored for the Android renderer.

#### Cursor glyph, smoke aura, and shadow

The agent pointer is a vector glyph (the path is parity with the Android
companion's `agent_cursor.xml`), but it is **not** pre-rasterized into the
texture. `render_vector_cursor` rounds the glyph path (Chaikin corner-cutting,
`CURSOR_CORNER_ROUNDING` iterations) and bakes a **signed distance field** of
that path into the cursor texture's R channel; the shader reconstructs the
black-fill / white-outline glyph from it with `fwidth`-based anti-aliasing at
the final framebuffer resolution. This is the key correctness property: a thin
high-contrast outline pre-rasterized into pixels cannot survive the GPU
minifying the oversized texture (`CURSOR_TEXTURE_SCALE`× the footprint) without
stair-stepping, whereas a smooth SDF samples and mipmaps cleanly and yields a
crisp edge at any per-output scale. The texture therefore carries a full
box-filtered mip chain and is sampled trilinear, and it is a **linear**
(`Rgba8Unorm`) format — an sRGB format would gamma-warp the distance values.

The texture packs four independent fields, not an RGBA image: R = the glyph
SDF (shader arrow), G = the smoke anchor (below), B = a stepped luminance and
A = coverage (both only for the CPU blit fallback / tests). The outline ring
half-width is `CURSOR_STROKE_EDGE` in normalized SDF units; the Rust constant
and the WGSL constant must match (guarded by `stroke_edge_matches_shader_constant`).
The glyph is tinted toward the agent palette — a deep-plum fill rising to a
soft pink-white outline — rather than pure black/white.

The cursor's aura reuses the **edge-glow recipe with the glyph silhouette as
the anchor** instead of the screen border: a centred radial field always reads
as a disc, so `render_vector_cursor` runs a two-pass chamfer distance transform
seeded on the glyph coverage and stores it in the G channel (1 on the outline →
0 at the smoke margin). `cursor_smoke` billows the same domain-warped fbm off
that anchor — fuller near the outline, wisping outward, with the outer boundary
feathered to nothing so the cloud dissolves with no visible edge. The aura's
size is the `AGENT_CURSOR_SMOKE_MARGIN` band, its density is the noise
threshold, and it is sampled at a small up-left uv offset so the cloud centres
on the hotspot rather than the glyph centroid (the arrow body sits down-right
of the tip). It uses the same bounded rim/body alpha union and lifted haze tint
as the edge smoke, so the two effects retain one visual identity.

The grounding shadow is its **own pass rendered under the smoke** (`cursor_shadow`
before `cursor_smoke` in `render_pixel`), sampled from the same SDF at a blurred
mip with a soft falloff. Drawing it under the smoke is deliberate: bundled with
the arrow on top of the smoke it darkens its own pink aura into a smudge;
underneath, it darkens the background and is covered where the smoke is dense.
The smoke's keep-off-the-arrow mask reads the **unshifted** glyph coverage so
shifting the cloud does not punch a gap that exposes the shadow.

Sizing is a startup-budget constraint, not just an aesthetic one:
`CursorImage::load` runs at process start and must finish well under the host's
2-second startup timeout (`HOST_START_TIMEOUT`), or the service kills the
overlay host and only the standalone KWin effect's edge glow survives. The
per-pixel analytic SDF (one polygon/path eval per texel, not per supersample),
the O(n) chamfer transform, and the glyph-bbox-bounded design keep a load that
once ran ~2.1 s down to a few hundred ms.

System cursor hiding (hiding the compositor's real pointer while the agent
overlay is visible) is a separate capability described in
`docs/features/compositor-cursor-hiding.md`. Visible-overlay state and
system-cursor state are independent; failure of one does not disable the
other.

## Source paths

- `crates/sky-cua-platform/src/model.rs` — public types and serialization
- `crates/sky-cua-service/src/overlay.rs` — `OverlayController`, state
  ownership, and synthetic screenshot compositing; pure point/state derivation
  lives in `overlay/cursor_geometry.rs` and gesture construction in
  `overlay/gesture.rs`
- `crates/sky-cua-overlay-host/` — overlay host crate
  - `src/layer_shell.rs` — generic Wayland layer-shell backend (KWin, Hyprland);
    submodules `layer_shell/geometry.rs` (event-invalidated per-output geometry
    snapshots so the frame loop never clones `OutputInfo`),
    `layer_shell/motion_adapter.rs` (motion-frame → per-layer scene mapping),
    and `layer_shell/wayland.rs` (surface creation + SCTK handlers)
  - `src/motion.rs` — pure vehicle-steering math (`Mover2D`, angle/path
    helpers), op-for-op port of Android `OverlayMath`
  - `src/cursor_motion.rs` — the stateful `CursorMotionDriver` (target
    precedence, arrival gate, rotation easing, cloud bloom, capture freeze)
  - `src/renderer/` — WGPU renderer, effect scene, uniform/storage buffer ABI,
    WGSL shader source, and offscreen/conformance tests
  - `src/system_cursor.rs` — system cursor adapter trait
  - `src/playground.rs` — interactive desktop pointer playground (preview tool)
  - `assets/cursor-chat.png` — cursor asset, byte-identical to the Chrome
    extension's
- `resources/kwin/effects/sky-cua-agent-cursor/` — C++ KWin cursor shim
- `resources/gnome-shell-extension/codex-window-control@openai.com/` —
  GNOME Shell window-control extension; cursor methods now report retired/
  unsupported instead of drawing a Shell actor
- `scripts/live_agent_cursor_kde_smoke.py`,
  `scripts/live_agent_cursor_x11_overlay_smoke.py`,
  `scripts/live_wayland_layer_shell_overlay_smoke.py` — live smokes

## Verification

Unit and crate-level tests:

```bash
cargo fmt --check
cargo nextest run
uv run python scripts/generate_overlay_spec.py --check
uv run python scripts/generate_motion_fixtures.py --check
uv run ruff format --check scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest scripts/test_agent_cursor_smokes.py scripts/test_overlay_pointer_animations.py scripts/test_overlay_motion_animations.py scripts/test_generate_motion_fixtures.py scripts/test_overlay_spec_codegen.py
cd android/phone-companion && JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT="$HOME/Android/Sdk" ./gradlew :app:testDebugUnitTest --offline
python3 scripts/build_plugin.py
```

Cursor-motion parity and evidence:

- Cross-language mover fixtures: `resources/overlay/agent_overlay_motion_fixtures.json`
  gains `mover_trajectory`, `approach_angle`, `wrap_radians`, and
  `trail_resample` families generated by `scripts/generate_motion_fixtures.py`
  (Kotlin `OverlayMath.Mover2D` is the behavioral reference; the generator is
  fixed, never Kotlin). Consumed by Kotlin `OverlaySpecFixtureTest` and the
  Rust `motion::tests` fixture consumer. Heading comparisons are wrapped
  (`|wrap_radians(expected − actual)| ≤ tol`), mid-flight samples use
  `tolerance.mover`, settled samples the default.
- Deterministic offline motion frames (GPU adapter required):
  `SKY_CUA_CAPTURE_MOTION=1 cargo nextest run --release -p sky-cua-overlay-host -E 'test(capture_motion_frames_when_requested)'`
  steps the real motion driver + renderer at fixed dt through corner-glide,
  mid-flight redirect, swipe-chase, and arrival-gated-tap scenarios (raw
  frames + manifest under `/tmp/overlay-demo/motion`, for manual channel/
  frame inspection). `scripts/overlay_motion_animations.py --offline` is the
  self-contained contact-sheet path: it re-runs the same deterministic
  capture into its own temp dir and montages those frames — it does not
  consume a pre-existing manual dump.
- Live motion harness (operator KDE Wayland, human-judged):
  `uv run python scripts/overlay_motion_animations.py` — drives the overlay
  host directly on a private socket (no service, no real input), records via
  the ScreenCast portal (spectacle-stills fallback), and writes contact
  sheets to `artifacts/overlay-motion-animations/`.
- Structured pass/fail glide smoke:
  `python3 scripts/live_agent_cursor_kde_smoke.py --mode layer-shell-motion-glide`
  asserts snap-on-first-show, an unsettled mid-flight pose strictly between
  the endpoints via the `motion` reply echo, and exact settle at the target.

The overlay-host crate includes WGPU shader validation, compute conformance
against `resources/overlay/wgsl_animation_fixtures.json`, and offscreen render
invariants for hidden transparency and deterministic visible frames. The cursor
glyph path has its own unit coverage: the texture is the footprint-plus-smoke-
margin size, the smoke anchor saturates inside the glyph and falls to zero in
the far margin, the corners are transparent, and `stroke_edge_matches_shader_constant`
guards the Rust↔WGSL stroke-width constant against drift. On-screen cursor
quality is judged by eye via the `agent-cursor-debug` skill, not a pass/fail
assertion.

VM acceptance via `scripts/run_gui_testing_vm_smoke.py`:

```bash
SKY_CUA_SMOKE_OPENCODE_MODEL=opencode/nemotron-3-ultra-free SKY_CUA_SMOKE_PI_MODEL=opencode/nemotron-3-ultra-free python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile all --sync-opencode-settings --sync-pi-settings
python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile kde-kwin-effect-system-install --vm-name testing-vm --libvirt-uri qemu:///session --desktop-env KDE --wayland-display wayland-0
python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile wayland-layer-shell-overlay --desktop-env Hyprland --wayland-display wayland-1
python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile i3 --desktop-env i3
```

Latest accepted artifacts:

- Package staging proof:
  `/home/bex/projects/sky-cua/dist/plugin/sky-cua`
- Latest VM all-profile overlay/desktop lanes:
  `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260625T083156Z`,
  `/workspace/artifacts/gui-desktop-smoke/targeted-screenshot/20260625T083319Z`,
  `/workspace/artifacts/gui-desktop-smoke/display-screenshot/20260625T083321Z`,
  `/workspace/artifacts/session-env-smoke/20260625T083324Z`,
  `/workspace/artifacts/text-readback-smoke/20260625T083327Z`,
  `/workspace/artifacts/gui-desktop-smoke/codex-desktop/20260625T083328Z`,
  `/workspace/artifacts/opencode-zenity-smoke/20260625T083332Z`,
  `/workspace/artifacts/opencode-kdialog-smoke/20260625T083402Z`,
  `/workspace/artifacts/pi-zenity-smoke/20260625T083546Z`,
  `/workspace/artifacts/pi-kdialog-smoke/20260625T083750Z`,
  `/workspace/artifacts/codex-e2e/agent-cursor-kde/0625083829345047-kwin-nested`, and
  `/workspace/artifacts/codex-e2e/agent-cursor-kde/0625083835737905-kwin-user`.
- Final operator desktop acceptance:
  `artifacts/final-desktop-overlay-acceptance/0625082705147305-vis`,
  `artifacts/final-desktop-overlay-acceptance/0625083856230087-hide`, and
  `artifacts/final-desktop-overlay-acceptance/0625083911586546-click`.
- KDE/KWin WGPU layer-shell Package E proof:
  `/workspace/artifacts/codex-e2e/agent-cursor-kde/0625053947174748-vis`
- Screenshot-synthetic preservation:
  `/workspace/artifacts/gui-desktop-smoke/targeted-screenshot/20260625T053958Z`
- KDE/KWin layer-shell visuals plus compiled cursor shim:
  `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T132649888064Z/host-summary.json`
- KDE/KWin layer-shell sequence: `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100302670580-syn`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100303845615-vis`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100305142807-hide`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100306568235-click`
- Hyprland layer-shell: `/workspace/artifacts/codex-e2e/agent-cursor-wayland-layer-shell/20260515T142710878162Z/`
- X11/i3 unsupported contract:
  `/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260625T054024051391Z`
- GNOME Shell actor renderer: retired; window-control API remains packaged
- COSMIC patched bridge: `artifacts/cosmic-framebuffer-cursor-proof/20260515T142538562074Z/host-summary.json`
- COSMIC transparent Xcursor: `artifacts/cosmic-transparent-xcursor-cursor-proof/20260516T073232164704Z/host-summary.json`

## Pointer playground

`sky-cua-overlay-host playground` is an interactive desktop preview of the agent
cursor — the desktop analogue of the Android `PointerPlaygroundActivity`. It opens
a maximized, input-capturing Wayland layer-shell surface, hides the system cursor,
and draws the real agent cursor glyph wherever the pointer moves, so the
computer-use pointer can be eyeballed over live content or a controlled backdrop.

```bash
# transparent: the agent cursor over your live desktop content
./target/release/sky-cua-overlay-host playground

# controlled backdrops for contrast checks (grid mirrors the Android playground)
./target/release/sky-cua-overlay-host playground --backdrop grid
./target/release/sky-cua-overlay-host playground --backdrop dark
./target/release/sky-cua-overlay-host playground --backdrop light
```

- Wayland-only, via the layer-shell renderer (`src/playground.rs`), which drives
  the production `WgpuOverlayRenderer` with the production `CursorImage`, so the
  previewed glyph is rendered by the same SDF / smoke / shadow shader path as the
  real desktop cursor. It is a distinct production path from the KWin compositor
  effect, so on KDE it previews the layer-shell rendering rather than the
  effect's.
- It owns pointer input while open (you cannot click the apps behind it). Move
  the mouse to preview; quit with Ctrl-C — the compositor restores the real
  cursor on exit.
- For a non-disruptive bounded check, wrap it: `timeout -k 2 4
  ./target/release/sky-cua-overlay-host playground --backdrop grid` and capture
  with `spectacle -b -n -f -o <path>` (grim needs wlr-screencopy, which KWin does
  not expose).

## Known limitations

- On KWin with fractional scaling and panels, KWin renders the visible
  hardware cursor offset by roughly the work-area origin during
  RemoteDesktop-driven input, on both the EIS and legacy portal lanes.
  Input dispatch is unaffected: clicks land at the requested coordinates,
  matching the agent cursor, and the offset ghost cursor is KWin's visual
  artifact. Verified live on Plasma (scale 1.5, left panel) on 2026-06-10
  with synthetic-cursor captures and a small-button click probe.
  Installing the sky-cua KWin cursor shim hides the system cursor while
  the layer-shell agent cursor is shown, which removes the visual confusion;
  install or update it with `python3 scripts/install_kwin_effect.py` (see
  [`compositor-cursor-hiding.md`](compositor-cursor-hiding.md)).
  `SKY_CUA_PORTAL_EIS=never` forces the legacy pointer lane for input-lane
  debugging.

- **Windows overlay is deferred.** The shared model and IPC contract were
  designed not to make Windows harder, but no Windows live proof exists yet.
  The Windows backend reports `visible_overlay=false` with a clear reason.
- **X11 and GNOME visible overlays are unsupported until WGPU hosts exist.**
  Their old production-visible renderers were retired so callers do not see a
  second visual implementation with weaker effects and capability reporting.
- **Wayland SHM visible rendering is retired.** The `wayland_shm` renderer
  enum value remains for old JSON only; `SKY_CUA_LAYER_SHELL_RENDERER=shm`
  reports unsupported instead of drawing CPU pixels.
- **KWin cursor shim needs system install for production.** User-level
  install under `~/.local/lib/qt6/plugins/kwin/effects/plugins` does not get
  discovered by a running KWin process even with explicit `loadEffect` and
  reconfigure. The accepted production lane installs under `/usr` via
  `kde-kwin-effect-system-install`. See
  `docs/research/2026-05-kwin-effect-discovery.md` for the investigation.
- **Generic Wayland layer-shell cannot hide the system cursor.** Click-
  through layer-shell surfaces have an empty input region and do not own
  pointer focus, so they report `system_cursor_hide_supported=false`.
- **Unpatched COSMIC has no global cursor-hide path.** The
  `cosmic_transparent_xcursor` mode is a VM-only fallback that requires the
  COSMIC session to start with `XCURSOR_THEME=sky-cua-blank`. It does not
  restore a normal cursor when the overlay hides. See
  `docs/research/2026-05-cosmic-cursor-hiding-options.md`.
- **No-no input interception and sound are follow-on work.** The WGPU renderer
  can draw the no-no render effect, but click interception and audio feedback
  are not part of this shipped contract.
- **Motion constants are strict phone parity.** Glide speed is 950 logical
  px/s, so a corner-to-corner traverse on a large desktop takes seconds and
  the arrival-gated ripple fires visibly after the (already-dispatched)
  click. If that reads badly in practice, the tuning valve is a
  `[desktop.motion]` spec override, not divergent hardcoded constants.
- **Pointer telemetry now glides.** Physical/compositor pointer movement
  retargets the mover instead of teleporting the glyph, so the agent glyph
  trails a fast human-driven flick with momentum curvature. Deliberate
  (everything sails); judge with the harness's `fast_flick` scenario.
- **No aura fade-out on hide.** The desktop stops rendering entirely when
  hidden, so only the bloom-in half of the phone's cloud fade exists.
- **L-shaped multi-monitor dead zones.** A glide passing through a point
  inside the union bounding box but outside every output has no surface to
  draw on for those frames — physically correct (there is no screen there),
  and it follows the arrangement: the glyph reappears where the next output
  begins rather than popping at an edge.
- **Live motion recording is KDE/portal-dependent.** The harness's primary
  recorder needs `org.freedesktop.portal.ScreenCast` + PipeWire + GStreamer
  and shows one share dialog on first run (restore token persisted under the
  gitignored artifacts dir); the fallback is a ~2 fps spectacle still loop.
  Recordings capture the operator's live desktop — never commit them.

## Related

- Research: [`docs/research/2026-05-kwin-effect-discovery.md`](../research/2026-05-kwin-effect-discovery.md)
- Research: [`docs/research/2026-05-x11-shaped-window-vs-layer-shell.md`](../research/2026-05-x11-shaped-window-vs-layer-shell.md)
- Companion feature: [`docs/features/compositor-cursor-hiding.md`](compositor-cursor-hiding.md)
- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
