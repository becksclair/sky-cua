# Agent cursor overlay

## Status

Shipped on Linux. Windows native overlay deferred until a Windows machine is
available for live proof. Last verified: 2026-05-15 across the Arch
`testing-vm` desktop matrix.

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
- `AgentCursorCapabilities { backend, visible_overlay, screenshot_synthetic_cursor, click_through, capture_exclusion, system_cursor_hide_supported, system_cursor_backend, system_cursor_hidden, needs_user_install, reason }`
- `AgentCursorBackendKind`: `wayland_layer_shell`, `kwin_effect`, `x11_shaped_window`, `gnome_shell_extension`, `cosmic_comp_bridge`, `cosmic_transparent_xcursor`, `none`
- `AgentCursorPlane`: `UserVisible`, `ScreenshotSynthetic`
- `AgentCursorSystemCursorBackend`: see `docs/features/compositor-cursor-hiding.md`
- `AppStateSnapshot.agent_cursor`, `ActionOutcome.agent_cursor` (preserved in compact snapshots)

Service IPC variants: `ServiceRequest::AgentCursorStatus`, `SetAgentCursor`,
`HideAgentCursor`, `ShowAgentCursor`. These are internal and not exposed as
MCP tools.

Overlay-host JSON-lines protocol on
`$XDG_RUNTIME_DIR/sky-cua/agent-cursor.sock` with versioned messages
`hello`, `capabilities`, `set_cursor`, `hide`, `show`, `ping`, `shutdown`.

Environment variables (allowlisted in `resources/chrome_preflight.py`):

- `SKY_CUA_AGENT_CURSOR` — `auto` (default), `on`, `off`
- `SKY_CUA_OVERLAY_BACKEND` — `auto`, `wayland_layer_shell`, `kwin_effect`,
  `x11`, `gnome_shell_extension`, `cosmic_comp_bridge`, `none`
- `SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE` — `auto`, `on`, `off`
- `SKY_CUA_OVERLAY_HOST_PATH` — explicit path; defaults to bundled binary
- `SKY_CUA_SCREENSHOT_CURSOR` — `auto`, `on`, `off`

## Behavior

The overlay has two planes that operate independently:

1. **`UserVisible`** — a native overlay window, layer, or compositor effect
   the user can see on the desktop. Backend selection is automatic by default
   and depends on the active compositor.
2. **`ScreenshotSynthetic`** — a marker composited into the screenshot Codex
   receives. Always available when capture is available, regardless of
   visible-overlay support.

Backend selection at `auto`:

- KDE/KWin: prefers the compiled `kwin_effect` when it is discoverable
  (requires system install under `/usr`; user-level install is not picked up
  by a running KWin), falls back to `wayland_layer_shell`.
- Hyprland: `wayland_layer_shell`. Layer-shell surfaces must wait for
  configure events before drawing.
- GNOME Shell: extends the bundled `codex-window-control@openai.com`
  extension to draw the cursor and report state over DBus.
- X11/i3: `x11_shaped_window` using `x11rb` with X Shape bounding regions
  for transparency and an empty input shape for click-through.
- COSMIC: `cosmic_comp_bridge` when a patched `cosmic-comp` is running with
  the bundled patch applied; `cosmic_transparent_xcursor` as a no-patch
  fallback that requires the COSMIC session to start with
  `XCURSOR_THEME=sky-cua-blank`.

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
afterward. The synthetic screenshot cursor is composited regardless.

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
  - `src/x11.rs` — X11 shaped-window backend
  - `src/kwin_effect.rs` — KWin effect host bridge
  - `src/gnome_shell.rs` — GNOME Shell extension client
  - `src/cosmic_bridge.rs` — COSMIC compositor bridge client
  - `src/system_cursor.rs` — system cursor adapter trait
  - `assets/cursor-chat.png` — cursor asset, byte-identical to the Chrome
    extension's
- `resources/kwin/effects/sky-cua-agent-cursor/` — C++ KWin effect plugin
- `resources/gnome-shell-extension/codex-window-control@openai.com/` —
  GNOME Shell extension (extended for cursor)
- `resources/cosmic/cosmic-comp-sky-cua-cursor-bridge.patch` — COSMIC
  compositor patch
- `crates/sky-cua-cosmic-helper/` — COSMIC bridge daemon (autostart, client)
- `scripts/live_agent_cursor_kde_smoke.py`,
  `scripts/live_agent_cursor_x11_overlay_smoke.py`,
  `scripts/live_wayland_layer_shell_overlay_smoke.py` — live smokes

## Verification

Unit and crate-level tests:

```bash
cargo test -p sky-cua-platform
cargo test -p sky-cua-service overlay
cargo test -p sky-cua-overlay-host
cargo test -p sky-cua-cosmic-helper
```

VM acceptance via `scripts/run_gui_testing_vm_smoke.py`:

```bash
python3 scripts/run_gui_testing_vm_smoke.py --profile kde-kwin-effect-system-install --vm-name testing-vm --libvirt-uri qemu:///session
python3 scripts/run_gui_testing_vm_smoke.py --profile wayland-layer-shell-overlay --desktop-env Hyprland --wayland-display wayland-1
python3 scripts/run_gui_testing_vm_smoke.py --profile i3
python3 scripts/run_gui_testing_vm_smoke.py --profile cosmic-patched-cursor-host-proof --desktop-env COSMIC --wayland-display wayland-1
python3 scripts/run_gui_testing_vm_smoke.py --profile cosmic-transparent-xcursor-host-proof --desktop-env COSMIC --wayland-display wayland-1
```

Latest accepted artifacts (per `CONTINUITY.md` 2026-05-15):

- KDE/KWin compiled effect: `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T132649888064Z/host-summary.json`
- KDE/KWin layer-shell sequence: `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100302670580-syn`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100303845615-vis`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100305142807-hide`, `/workspace/artifacts/codex-e2e/agent-cursor-kde/0515100306568235-click`
- Hyprland layer-shell: `/workspace/artifacts/codex-e2e/agent-cursor-wayland-layer-shell/20260515T142710878162Z/`
- X11/i3 shaped window: `/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260515T142731049499Z/`
- GNOME Shell extension: `artifacts/gnome-framebuffer-cursor-proof/20260515T140437893805720Z/host-summary.json`
- COSMIC patched bridge: `artifacts/cosmic-framebuffer-cursor-proof/20260515T142538562074Z/host-summary.json`
- COSMIC transparent Xcursor: `artifacts/cosmic-transparent-xcursor-cursor-proof/20260516T073232164704Z/host-summary.json`

## Known limitations

- **Windows overlay is deferred.** The shared model and IPC contract were
  designed not to make Windows harder, but no Windows live proof exists yet.
  The Windows backend reports `visible_overlay=false` with a clear reason.
- **KWin compiled effect needs system install for production.** User-level
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
- **XWayland visible overlay does not appear in portal capture.** The X11
  shaped-window backend instantiates on XWayland but its overlay is not
  picked up by Wayland portal capture. The accepted X11 visual proof comes
  from a real Xorg session, not host XWayland. See
  `docs/research/2026-05-x11-shaped-window-vs-layer-shell.md`.

## Related

- Research: [`docs/research/2026-05-kwin-effect-discovery.md`](../research/2026-05-kwin-effect-discovery.md)
- Research: [`docs/research/2026-05-x11-shaped-window-vs-layer-shell.md`](../research/2026-05-x11-shaped-window-vs-layer-shell.md)
- Companion feature: [`docs/features/compositor-cursor-hiding.md`](compositor-cursor-hiding.md)
- ROADMAP entry: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop parity"
- Originating ExecPlan (retired into this feature doc; see git history for `plans/native_agent_cursor_overlay.md`).
