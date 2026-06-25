# Agent cursor overlay

## Status

Shipped on Linux with WGPU-only production visible rendering on Wayland.
Windows native overlay deferred until a Windows machine is available for live
proof. Last verified: 2026-06-25 in the Arch `testing-vm` for the WGPU
layer-shell path, retired X11/GNOME/SHM contracts, and package build.

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
Coordinate pointer actions may start a glide when accepted, but backend input
dispatch follows the existing action contract and is not delayed waiting for
animation completion. Success effects such as tap ripples are emitted only
after successful dispatch; failed dispatch cancels pending visual feedback.

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
does not rasterize or precompose edge glow, inward waves, halo, ripple, trail,
cursor glide/rotation, no-no frames, or cursor pixels for the normal WGPU
runtime path.

The WGPU pass draws a full-screen analytic shader into a premultiplied-alpha
surface. TOML color channels are authored as sRGB 0..255 values and normalized
before upload. The surface policy prefers sRGB swapchain formats
(`Bgra8UnormSrgb`, then `Rgba8UnormSrgb`, then any sRGB format), and the shader
returns premultiplied color so transparent composition remains correct with
`CompositeAlphaMode::PreMultiplied` when available.

System cursor hiding (hiding the compositor's real pointer while the agent
overlay is visible) is a separate capability described in
`docs/features/compositor-cursor-hiding.md`. Visible-overlay state and
system-cursor state are independent; failure of one does not disable the
other.

## Source paths

- `crates/sky-cua-platform/src/model.rs` — public types and serialization
- `crates/sky-cua-service/src/overlay.rs` — `OverlayController`, state
  ownership, and synthetic screenshot compositing
- `crates/sky-cua-overlay-host/` — overlay host crate
  - `src/layer_shell.rs` — generic Wayland layer-shell backend (KWin, Hyprland)
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
cargo test
uv run ruff format --check scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest scripts/test_agent_cursor_smokes.py scripts/test_overlay_pointer_animations.py scripts/test_overlay_spec_codegen.py
cd android/phone-companion && JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANDROID_SDK_ROOT="$HOME/Android/Sdk" ./gradlew :app:testDebugUnitTest --offline
python3 scripts/build_plugin.py
```

The overlay-host crate includes WGPU shader validation, compute conformance
against `resources/overlay/wgsl_animation_fixtures.json`, and offscreen render
invariants for hidden transparency and deterministic visible frames.

VM acceptance via `scripts/run_gui_testing_vm_smoke.py`:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile all --desktop-env KDE --wayland-display wayland-0
python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile kde-kwin-effect-system-install --vm-name testing-vm --libvirt-uri qemu:///session --desktop-env KDE --wayland-display wayland-0
python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile wayland-layer-shell-overlay --desktop-env Hyprland --wayland-display wayland-1
python3 scripts/run_gui_testing_vm_smoke.py --host 127.0.0.1 --port 22222 --user skycua --ssh-option StrictHostKeyChecking=no --ssh-option UserKnownHostsFile=artifacts/testing-vm/known_hosts --profile i3 --desktop-env i3
```

Latest accepted artifacts:

- Phase 8 no-git package staging proof:
  `/home/skycua/workspace-coord/dist/plugin/sky-cua`
- Latest VM all-profile overlay/desktop lanes:
  `/workspace/artifacts/gui-desktop-smoke/wayland-pointer/20260625T064230Z`,
  `/workspace/artifacts/gui-desktop-smoke/targeted-screenshot/20260625T064308Z`,
  `/workspace/artifacts/gui-desktop-smoke/display-screenshot/20260625T064311Z`,
  `/workspace/artifacts/session-env-smoke/20260625T064314Z`,
  `/workspace/artifacts/text-readback-smoke/20260625T064317Z`, and
  `/workspace/artifacts/gui-desktop-smoke/codex-desktop/20260625T064319Z`;
  the remaining all-profile blocker is external OpenCode/Pi agent auth/billing,
  with OpenCode artifact `/workspace/artifacts/opencode-zenity-smoke/20260625T064323Z`.
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

- Wayland-only, via the layer-shell renderer (`src/playground.rs`), which reuses
  the production `CursorImage` and `draw_cursor_asset`, so the previewed glyph is
  the real desktop cursor at the 2x desktop render size. It is a distinct
  production path from the KWin compositor effect, so on KDE it previews the
  layer-shell rendering of the same asset rather than the effect's.
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
- **Final operator-desktop acceptance remains separate.** The VM closeout
  proves source, package, and VM behavior. The operator desktop is reserved for
  one controlled Phase 9 acceptance pass using the already-proven package.

## Related

- Research: [`docs/research/2026-05-kwin-effect-discovery.md`](../research/2026-05-kwin-effect-discovery.md)
- Research: [`docs/research/2026-05-x11-shaped-window-vs-layer-shell.md`](../research/2026-05-x11-shaped-window-vs-layer-shell.md)
- Companion feature: [`docs/features/compositor-cursor-hiding.md`](compositor-cursor-hiding.md)
- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Active closeout ExecPlan:
  [`plans/wgpu_agent_overlay_unification.md`](../../plans/wgpu_agent_overlay_unification.md)
