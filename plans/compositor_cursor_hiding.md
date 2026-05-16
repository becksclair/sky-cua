# Compositor Cursor Hiding

This ExecPlan is a living document. Maintain it according to `/home/bex/.agents/PLANS.md`.

## Purpose

The Linux agent cursor must always remain visible to the user when `AgentCursorState.visible` is true, and the user's real compositor cursor must be hidden at the same time. The desired user-visible invariant is one cursor, not two: the sky-cua agent cursor overlay is visible, click-through, and positioned from the same state that drives screenshots; the real system cursor is hidden or truthfully reported as not hidden.

This plan completes the compositor-specific cursor hiding work for the Linux environments sky-cua targets:

- X11 via XFixes, already implemented.
- KDE Plasma/KWin via the bundled KWin effect, already implemented.
- GNOME Shell by extending the existing bundled GNOME Shell extension.
- Hyprland by toggling Hyprland's compositor-owned `cursor:invisible` setting and restoring the previous value.
- COSMIC by adding a compositor-side bridge, because ordinary Wayland and layer-shell clients cannot globally hide the cursor.

Generic Wayland clients cannot hide the compositor cursor globally. Any design that disables the layer-shell overlay because cursor hiding is unavailable is rejected. Overlay visibility and system cursor hiding are separate capabilities: overlay state must continue to work, and cursor hiding failures must be surfaced through diagnostics and `AgentCursorCapabilities`.

## Progress

- [x] 2026-05-15T13:05Z - Reconfirmed the existing sky-cua overlay architecture, GNOME extension, KWin effect, X11 XFixes adapter, and current capability model.
- [x] 2026-05-15T13:05Z - Reverted the rejected conservative layer-shell auto-selection change; the worktree was clean before this plan was created.
- [x] 2026-05-15T13:05Z - Added shared capability variants for GNOME Shell, Hyprland, and COSMIC cursor hiding.
- [x] 2026-05-15T13:05Z - Added a GNOME Shell overlay backend and extended the existing Shell extension DBus service to draw the agent cursor and hide/restore the real pointer.
- [x] 2026-05-15T13:05Z - Added a Hyprland layer-shell system cursor adapter that toggles `cursor:invisible` and restores the previous value.
- [x] 2026-05-15T13:05Z - Added the overlay-host COSMIC bridge client contract for `CosmicCompBridge`.
- [x] 2026-05-15T14:27Z - Verified Hyprland on the Arch testing VM with current code using `scripts/run_gui_testing_vm_smoke.py --profile wayland-layer-shell-overlay --desktop-env Hyprland --wayland-display wayland-1`; artifact `/workspace/artifacts/codex-e2e/agent-cursor-wayland-layer-shell/20260515T142710878162Z/` reports `ok=true`, `system_cursor_backend=hyprland_config`, hidden true after set, hidden false after hide, and visible marker found. This rerun also proved stale COSMIC bridge state no longer steals Hyprland detection.
- [x] 2026-05-15T13:27Z - Re-verified KWin on the Arch testing VM with `scripts/run_gui_testing_vm_smoke.py --profile kde-kwin-effect-system-install --desktop-env KDE --wayland-display wayland-0 --vm-name testing-vm --libvirt-uri qemu:///session --skip-host-build --skip-sync`; artifact `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T132649888064Z/host-summary.json` reports `ok=true`, host framebuffer marker found, `backend=kwin_effect`, hidden true after set, hidden false after hide, and cleanup removed the system effect files.
- [x] 2026-05-15T14:27Z - Re-verified X11/i3 on the Arch testing VM with current code using `scripts/run_gui_testing_vm_smoke.py --profile i3 --skip-host-build --skip-sync`; artifact `/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260515T142731049499Z/` reports `ok=true`, `backend=x11_shaped_window`, `system_cursor_backend=x11_xfixes`, hidden true after set/show, hidden false after hide, visible marker found, and click-through proved.
- [x] 2026-05-15T14:05Z - Verified GNOME Shell on the Arch testing VM through the existing extension UUID in a normal GDM GNOME Wayland user session. Artifact `artifacts/gnome-framebuffer-cursor-proof/20260515T140437893805720Z/host-summary.json` reports `ok=true`, `backend=gnome_shell_extension`, `system_cursor_backend=gnome_shell_extension`, hidden true after set, hidden false after hide, and the host framebuffer marker present only while the agent cursor is visible.
- [x] 2026-05-15T13:52Z - Added the packaged COSMIC cursor bridge daemon mode to `sky-cua-cosmic-helper`, the overlay-host autostart/client path, and the `resources/cosmic/cosmic-comp-sky-cua-cursor-bridge.patch` compositor integration artifact.
- [x] 2026-05-15T13:52Z - Rechecked unpatched COSMIC on the Arch testing VM. The helper socket reports `supported=false` without `$XDG_RUNTIME_DIR/sky-cua-cosmic-cursor-ready`, and overlay capabilities remain honest with `system_cursor_hide_supported=false`, `system_cursor_hidden=false`, and `system_cursor_backend=cosmic_comp_bridge`.
- [x] 2026-05-15T14:25Z - Verified COSMIC full cursor hiding on the Arch testing VM with current code and a patched `cosmic-comp` built from Arch's installed source commit `b5a1a6d3179810627fa0bffac7bd5d78c7df4fa0` plus `resources/cosmic/cosmic-comp-sky-cua-cursor-bridge.patch`. The new runner profile `scripts/run_gui_testing_vm_smoke.py --profile cosmic-patched-cursor-host-proof --desktop-env COSMIC --wayland-display wayland-1 --vm-name testing-vm --libvirt-uri qemu:///session` writes `artifacts/cosmic-framebuffer-cursor-proof/20260515T142538562074Z/host-summary.json`, reporting `ok=true`, `system_cursor_backend=cosmic_comp_bridge`, hidden true after set, hidden false after hide, host framebuffer agent marker found while visible, and restored real-cursor marker absent until hide.
- [x] 2026-05-16T07:32Z - Added and verified the no-patch COSMIC VM mode. `scripts/install_blank_xcursor_theme.py` generates a valid transparent `sky-cua-blank` Xcursor theme, the VM session wrapper accepts `cosmic-blank`/`cosmic-transparent`, and `CosmicTransparentXcursorAdapter` reports `system_cursor_backend=cosmic_transparent_xcursor` only when COSMIC is actually launched with that theme. `scripts/run_gui_testing_vm_smoke.py --profile cosmic-transparent-xcursor-host-proof --desktop-env COSMIC --wayland-display wayland-1 --vm-name testing-vm --libvirt-uri qemu:///session` writes `artifacts/cosmic-transparent-xcursor-cursor-proof/20260516T073232164704Z/host-summary.json`, reporting `ok=true`, agent marker visible, agent marker absent after hide, and no native cursor marker in the hidden frame. This mode preserves the one-visible-cursor invariant while the agent overlay is visible, but it intentionally does not restore a normal native cursor when the overlay hides.
- [x] 2026-05-16T07:32Z - Hardened the current COSMIC bridge prototype: `show` now clears the hidden sentinel even when compositor integration is absent, bridge startup clears stale hidden state, repeated `set_hidden(state)` calls are no-ops, and stale bridge sockets/sentinels are cleaned during VM session switches.
- [x] Preserve and revalidate existing KWin and X11 behavior.
- [x] 2026-05-15T13:55Z - Re-ran local validation: `cargo fmt --check`, focused cursor model tests, full `sky-cua-overlay-host`, full `sky-cua-cosmic-helper`, GNOME setup asset test, service overlay tests, Ruff format/check, basedpyright, `uv run pytest scripts/test_python_harness_helpers.py -q`, and `python3 scripts/build_plugin.py`. The built plugin now includes `resources/cosmic/cosmic-comp-sky-cua-cursor-bridge.patch`.
- [x] 2026-05-15T14:22Z - Added an automated live smoke profile for patched COSMIC host-framebuffer proof.

## Discoveries

- `resources/gnome-shell-extension/codex-window-control@openai.com/extension.js` already exports `com.openai.Codex.WindowControl` at `/com/openai/Codex/WindowControl` for `ListWindows` and `ActivateWindow`. The GNOME work must extend this extension instead of creating a second one.
- `crates/sky-cua-linux/src/setup.rs` already writes and enables that GNOME extension from embedded `metadata.json` and `extension.js` strings. New extension assets or files must be added to this setup path.
- GNOME Shell 49 moved away from `Meta.CursorTracker.set_pointer_visible()` toward cursor visibility inhibition. The extension should prefer `global.backend.get_cursor_tracker().inhibit_cursor_visibility()` and `uninhibit_cursor_visibility()` when available, with a guarded fallback to `set_pointer_visible(false)` and `set_pointer_visible(true)` for older Shell versions.
- Hyprland owns cursor rendering. Current Hyprland source exposes the runtime setting `cursor:invisible`, and its renderer hides the cursor by calling the compositor cursor manager path that resets the cursor image. sky-cua should use Hyprland's supported runtime config channel first, restore the previous value, and treat an eventual Hyprland plugin as a later hardening option.
- COSMIC's compositor path uses Smithay cursor image state. Its seat extension has `cursor_image_status` and `set_cursor_image_status`, and rendering returns no cursor when the status is `CursorImageStatus::Hidden`. No public external COSMIC IPC/config hook was found for globally hiding the cursor. Proper support therefore needs a COSMIC compositor-side bridge or accepted upstream integration, not a normal layer-shell client trick.
- A fresh current-source pass found no public `cosmic-comp` cursor-hide IPC, config key, DBus API, or custom Wayland protocol. The exposed cursor path remains compositor seat state: `SeatHandler::cursor_image` forwards to `seat.set_cursor_image_status(image)`, while public config only exposes cursor focus behavior such as `focus_follows_cursor` and `cursor_follows_focus`.
- Sway/wlroots evidence agrees with the broader Wayland rule: compositor code hides by unsetting or replacing the compositor cursor image; a click-through layer-shell client cannot globally hide the real cursor.

## Decision Log

- Keep the agent cursor overlay visible whenever possible. Do not make overlay startup conditional on system cursor hide support.
- Keep `AgentCursorCapabilities.visible_overlay`, `system_cursor_hide_supported`, `system_cursor_hidden`, and `system_cursor_backend` independently truthful. A compositor may support visible overlay before it supports real cursor hiding.
- Extend the existing GNOME Shell extension and DBus service. Do not introduce a second GNOME extension UUID or a competing Shell service.
- Implement Hyprland first through `hyprctl` runtime configuration because it is available to normal clients and maps to compositor-owned cursor hiding. Snapshot and restore the previous value.
- Treat normal unpatched COSMIC as unsupported for dynamic hide/show until COSMIC has an upstream compositor cursor-visibility inhibitor/API. For controlled VMs, support `cosmic_transparent_xcursor` as a dedicated session mode with a transparent native cursor theme plus the sky-cua layer-shell overlay.
- Keep the current COSMIC compositor patch as a development prototype and proof harness, not the desired long-term production contract. An upstreamable version should be generic, token/refcount based, and suppress all final cursor render sources rather than only `SeatExt::cursor_image_status()`.
- Preserve KWin effect behavior as the best native Wayland model: the compositor effect draws the cursor and hides/restores the real system cursor inside the compositor.

## Existing Architecture

The shared wire model lives in `crates/sky-cua-platform/src/model.rs`. `AgentCursorCapabilities` already reports:

- `backend`
- `visible_overlay`
- `screenshot_synthetic_cursor`
- `click_through`
- `capture_exclusion`
- `system_cursor_hide_supported`
- `system_cursor_hidden`
- `system_cursor_backend`
- `needs_user_install`
- `reason`

`AgentCursorSystemCursorBackendKind` currently has `None`, `Unsupported`, `WaylandClientUnsupported`, `X11Xfixes`, `KwinEffect`, `WindowsWin32`, and `MacosNative`.

The overlay host lives in `crates/sky-cua-overlay-host`. It selects concrete backends in `src/lib.rs`, draws layer-shell cursors in `src/layer_shell.rs`, draws X11 shaped-window cursors in `src/x11.rs`, talks to the KWin effect in `src/kwin_effect.rs`, and hides the real cursor through `src/system_cursor.rs`.

`src/system_cursor.rs` already contains the right seam:

- `SystemCursorAdapter::wayland_client_unsupported(...)`
- `SystemCursorAdapter::x11(...)`
- `backend()`
- `supported()`
- `hidden()`
- `reason()`
- `set_hidden(bool)`
- `restore()`

KWin is separate because the compositor effect owns both planes. The effect resources live under `resources/kwin/effects/sky-cua-agent-cursor/`; `systemcursoradapter.cpp` calls compositor cursor hide/show APIs, and `crates/sky-cua-overlay-host/src/kwin_effect.rs` reports `KwinEffect` with `system_cursor_hide_supported: true`.

GNOME window targeting already exists:

- `resources/gnome-shell-extension/codex-window-control@openai.com/extension.js`
- `resources/gnome-shell-extension/codex-window-control@openai.com/metadata.json`
- `crates/sky-cua-linux/src/windowing/gnome_extension.rs`
- `crates/sky-cua-linux/src/setup.rs`

Hyprland and COSMIC detection/windowing code already exists:

- `crates/sky-cua-linux/src/windowing/hyprland.rs`
- `crates/sky-cua-linux/src/windowing/cosmic.rs`
- `crates/sky-cua-linux/src/cosmic_helper.rs`
- `scripts/testing-vm/profiles/hyprland.sh`
- `scripts/testing-vm/profiles/cosmic.sh`
- `scripts/testing-vm/profiles/cosmic-helper.sh`

## Implementation Plan

### 1. Shared capability contract

Update `crates/sky-cua-platform/src/model.rs` with explicit compositor cursor-hiding backends:

- `GnomeShellExtension`
- `HyprlandConfig`
- `CosmicCompBridge`

Keep the old variants stable. Add serialization tests near `agent_cursor_capabilities_report_backend_as_snake_case()` so the new variants serialize as `gnome_shell_extension`, `hyprland_config`, and `cosmic_comp_bridge`.

Expected validation:

- `cargo test -p sky-cua-platform agent_cursor`

### 2. Overlay host adapter selection

Update `crates/sky-cua-overlay-host/src/system_cursor.rs` so `SystemCursorAdapter` can hold compositor-specific Wayland adapters in addition to `Unsupported` and `X11`.

Add detection helpers that are cheap and deterministic:

- GNOME Shell: `XDG_CURRENT_DESKTOP`, `DESKTOP_SESSION`, or a successful DBus probe for `com.openai.Codex.WindowControl`.
- Hyprland: `HYPRLAND_INSTANCE_SIGNATURE` plus `hyprctl` availability.
- COSMIC: `XDG_CURRENT_DESKTOP` or `DESKTOP_SESSION` containing `COSMIC`, plus the bridge availability check.

Update `crates/sky-cua-overlay-host/src/layer_shell.rs` so layer-shell no longer hardcodes `WaylandClientUnsupported`. Instead it asks for the best compositor adapter and falls back to `WaylandClientUnsupported` with the current reason when no compositor adapter is available.

The ordering should be explicit:

1. GNOME extension adapter when the Shell DBus service is reachable.
2. Hyprland adapter when running under Hyprland.
3. COSMIC bridge adapter when the bridge is reachable.
4. `WaylandClientUnsupported` for generic Wayland.

Expected validation:

- `cargo test -p sky-cua-overlay-host system_cursor`
- `cargo test -p sky-cua-overlay-host layer_shell`

### 3. GNOME Shell extension cursor service

Extend `resources/gnome-shell-extension/codex-window-control@openai.com/extension.js`.

The existing DBus interface should remain backward compatible. Add methods on the same `com.openai.Codex.WindowControl` service and object path:

- `SetAgentCursorState(s json) -> (b ok, s message, s status_json)`
- `HideAgentCursor(s reason) -> (b ok, s message, s status_json)`
- `ShowAgentCursor() -> (b ok, s message, s status_json)`
- `AgentCursorStatus() -> (s status_json)`

`SetAgentCursorState` accepts the serialized `AgentCursorState` shape used by the Rust overlay host. The extension only needs native coordinates for user-visible rendering. If `native_point` is absent, it may use `model_point` only when the coordinate space is already desktop-native; otherwise it returns `ok=false` with a precise message and leaves existing state unchanged.

The extension must create a Shell actor for the agent cursor, place it above normal windows, and keep it click-through. Use the existing `cursor-chat.png` visual asset from `crates/sky-cua-overlay-host/assets/cursor-chat.png`, copied into the extension resources. Preserve the Rust constants from `crates/sky-cua-overlay-host/src/lib.rs`: rendered size 23x24 and hotspot 10,11.

When agent cursor visibility turns on:

- Show and position the Shell actor.
- Hide the real pointer by acquiring a GNOME cursor visibility inhibitor when available.
- Fall back to `Meta.CursorTracker.set_pointer_visible(false)` only when the inhibitor API is unavailable.
- Track whether sky-cua currently owns the hide operation.

When agent cursor visibility turns off or the extension disables:

- Hide the Shell actor.
- Release the inhibitor or call `set_pointer_visible(true)` only if sky-cua owns the hide operation.
- Clear state so GNOME Shell reloads do not leave stale cursor state.

Update `crates/sky-cua-linux/src/setup.rs` so it writes any new extension asset files. Keep `GNOME_EXTENSION_UUID` unchanged.

Add Rust adapter support in `crates/sky-cua-overlay-host`:

- New module such as `src/gnome_shell.rs` or a contained adapter in `src/system_cursor.rs`.
- Use the session bus and existing session environment hydration conventions where applicable.
- Probe with a low-cost `AgentCursorStatus` call.
- Send `SetAgentCursorState` from layer-shell updates when GNOME is selected.
- Call `ShowAgentCursor` or `HideAgentCursor` on restore/drop.

Expected validation:

- Unit tests for GNOME DBus command construction or DBus client serialization.
- A setup test proving `metadata.json`, `extension.js`, and the cursor asset are written.
- Live GNOME smoke: enable/reload Shell extension, start overlay, move agent cursor, verify exactly one visible cursor by screenshot/pixel comparison and `AgentCursorStatus().system_cursor_hidden == true`.

### 4. Hyprland cursor adapter

Add `HyprlandSystemCursorAdapter`.

Probe:

- Require `HYPRLAND_INSTANCE_SIGNATURE`.
- Require `hyprctl` on PATH.
- Read current `cursor:invisible` using `hyprctl getoption cursor:invisible -j` when JSON is available, with a text parser fallback for older output.

Hide:

- Snapshot the previous `cursor:invisible` value before the first hide.
- Set `cursor:invisible true` through Hyprland's runtime config channel.
- Prefer `hyprctl keyword cursor:invisible true` for broad compatibility.
- Optionally probe `hyprctl eval` support and use it only if it is clearly available and tested.

Restore:

- Restore the exact previous boolean value when sky-cua hides the cursor.
- Restore on `set_hidden(false)`, `restore()`, and adapter drop paths already exercised by overlay shutdown.
- If restore fails, report a diagnostic with the failed command and keep `system_cursor_hidden` truthful.

Layer-shell remains the user-visible agent cursor renderer under Hyprland. Hyprland owns only the real cursor hide/show side.

Expected validation:

- Unit tests for `hyprctl getoption` JSON and text parsing.
- Unit tests that hide/restore command ordering snapshots the previous value once and restores it once.
- Live Hyprland smoke in `scripts/testing-vm/profiles/hyprland.sh`: start overlay, assert `hyprctl getoption cursor:invisible` changes while visible and returns to the original value after hide/shutdown, and verify the layer-shell cursor remains visible.

### 5. COSMIC compositor bridge

Do not implement COSMIC as generic Wayland. Proper COSMIC support needs a bridge that runs with compositor authority or an accepted compositor IPC hook.

First implementation target:

- Add a COSMIC bridge resource under `resources/cosmic/` or a dedicated crate if the repo's existing `crates/sky-cua-linux/src/cosmic_helper.rs` path proves to be only windowing support.
- The bridge must expose a small local DBus or Unix-socket API that can set the active seat cursor image status to hidden and restore the previous status.
- The compositor-side code must call COSMIC/Smithay's equivalent of `seat.set_cursor_image_status(CursorImageStatus::Hidden)` and preserve the previous cursor image/status for restore.
- The normal sky-cua overlay host talks to this bridge through `CosmicCompBridge`.

Required behavior:

- Probe reports unsupported when the bridge is absent.
- Setup/doctor reports a concrete COSMIC bridge missing/installed/running state.
- The overlay still renders through layer-shell even before the bridge is installed.
- With the bridge installed, `system_cursor_hide_supported` is true, `system_cursor_backend` is `cosmic_comp_bridge`, and `system_cursor_hidden` is true while the agent cursor is visible.

Expected validation:

- Unit tests for bridge client request/response parsing.
- Doctor/setup tests for absent and installed bridge states.
- Live COSMIC VM smoke using `scripts/testing-vm/profiles/cosmic.sh` or `scripts/testing-vm/profiles/cosmic-helper.sh`: prove the real cursor disappears while the layer-shell agent cursor remains visible, then prove the real cursor returns after shutdown.

### 6. KWin and X11 regression pass

Keep current KWin and X11 implementations intact.

Regression checks:

- KWin effect path still reports `backend: kwin_effect`, `visible_overlay: true`, `system_cursor_hide_supported: true`, and `system_cursor_hidden: true` while visible.
- X11 shaped-window path still reports `backend: x11_shaped_window`, `system_cursor_backend: x11_xfixes`, and hides/restores through XFixes.
- Generic Wayland sessions without a supported compositor adapter still report `wayland_layer_shell` for the visible overlay and `wayland_client_unsupported` only for the real cursor hide backend.

Expected validation:

- `cargo test -p sky-cua-overlay-host`
- Existing KWin live smoke from `plans/native_agent_cursor_overlay.md`.
- Existing X11 smoke or a new focused X11 overlay smoke if the old proof is stale.

### 7. Diagnostics and service integration

Update service-facing diagnostics so unsupported and failed hide paths are understandable:

- Unsupported generic Wayland: "Wayland clients cannot hide the compositor cursor globally; install or enable a compositor adapter."
- GNOME extension unavailable: "GNOME Shell extension is not installed, enabled, or serving DBus."
- GNOME API unavailable: "GNOME Shell cursor visibility API unavailable."
- Hyprland command failure: include the failing `hyprctl` subcommand and exit detail.
- COSMIC bridge unavailable: "COSMIC cursor bridge is not installed or not reachable."

The action path must not fail merely because system cursor hiding is unsupported. It may fail when the selected visible overlay backend itself cannot render.

Add tests for capability truthfulness:

- Visible overlay true and system cursor unsupported.
- Visible overlay true and system cursor supported but command failed.
- Visible overlay true and system cursor hidden.
- Overlay hidden restores system cursor and reports hidden false.

### 8. Packaging and setup

Update packaging only for artifacts required by the implementation:

- GNOME extension asset copy in `crates/sky-cua-linux/src/setup.rs`.
- Plugin bundle inclusion in `scripts/build_plugin.py` if the new GNOME asset or COSMIC bridge resource is not already captured.
- COSMIC bridge install/setup command if the bridge is packaged with sky-cua.

No Hyprland install step is expected for the config adapter.

Expected validation:

- `python3 scripts/build_plugin.py`
- Inspect the built plugin bundle for the GNOME extension files and any COSMIC bridge resources.

## Acceptance Matrix

KWin:

- Agent cursor visible through the KWin effect.
- Real cursor hidden through the KWin effect.
- Cursor returns after hide/shutdown.
- Capabilities report `backend: kwin_effect`, `system_cursor_backend: kwin_effect`, `system_cursor_hidden: true` while visible.

X11:

- Agent cursor visible through the shaped-window overlay.
- Real cursor hidden through XFixes.
- Cursor returns after hide/shutdown.
- Capabilities report `backend: x11_shaped_window`, `system_cursor_backend: x11_xfixes`, `system_cursor_hidden: true` while visible.

GNOME:

- Existing extension UUID and DBus service remain valid for window targeting.
- Extension draws the agent cursor actor.
- Extension hides the real pointer using inhibitor or guarded legacy API.
- Cursor returns after hide, extension disable, and overlay shutdown.
- Capabilities report `backend: wayland_layer_shell` or a GNOME-specific visible backend if one is added, `system_cursor_backend: gnome_shell_extension`, `system_cursor_hidden: true` while visible.

Hyprland:

- Layer-shell agent cursor remains visible.
- `cursor:invisible` becomes true only while sky-cua owns the hide state.
- Previous `cursor:invisible` value is restored exactly.
- Capabilities report `backend: wayland_layer_shell`, `system_cursor_backend: hyprland_config`, `system_cursor_hidden: true` while visible.

COSMIC:

- Layer-shell agent cursor remains visible.
- COSMIC compositor bridge hides the real cursor through compositor seat state.
- Previous cursor state is restored on hide/shutdown.
- Capabilities report `backend: wayland_layer_shell`, `system_cursor_backend: cosmic_comp_bridge`, `system_cursor_hidden: true` while visible.

Generic Wayland:

- Layer-shell agent cursor remains visible.
- Real cursor hide is reported unsupported, not silently claimed.
- Actions continue unless visible overlay rendering fails.
- Capabilities report `backend: wayland_layer_shell`, `system_cursor_backend: wayland_client_unsupported`, `system_cursor_hidden: false`.

## Validation Commands

Run the narrowest relevant checks as each milestone lands:

- `cargo fmt --check`
- `cargo test -p sky-cua-platform agent_cursor`
- `cargo test -p sky-cua-overlay-host system_cursor`
- `cargo test -p sky-cua-overlay-host layer_shell`
- `cargo test -p sky-cua-overlay-host`
- `cargo test -p sky-cua-service overlay`
- `uv run ruff format --check scripts`
- `uv run ruff check scripts`
- `uv run basedpyright`
- `uv run pytest`
- `python3 scripts/build_plugin.py`

Accepted live compositor proofs now cover:

- KDE/KWin profile with the bundled KWin effect installed and enabled.
- X11/i3 profile with XFixes-backed cursor hiding.
- GNOME Shell profile with the bundled extension enabled.
- Hyprland profile using compositor-owned `cursor:invisible` restore.
- COSMIC patched-compositor proof via `--profile cosmic-patched-cursor-host-proof`.
- COSMIC no-patch transparent-session proof via `--profile cosmic-transparent-xcursor-host-proof`.

These proofs are the current Linux acceptance baseline. Re-run the affected profile and refresh the cited artifact whenever the overlay host, helper bridge, VM session wrappers, or compositor integration changes.

Each live proof must capture:

- The command used to start the session and overlay.
- The capability JSON while cursor is visible.
- A screenshot or pixel-level proof showing the agent cursor visible and the real cursor absent.
- The restore proof after hide/shutdown.

## Risks

GNOME Shell API drift is likely. The extension must guard every cursor API call and report precise status JSON instead of crashing the Shell extension.

Hyprland runtime config restore is user-visible state. The adapter must snapshot once, restore exactly, and avoid overwriting user changes made after sky-cua starts unless sky-cua still owns the hide state.

COSMIC still has two distinct lanes. The patched compositor bridge is live-proved and gives proper dynamic hide/show, but it remains a downstream prototype until COSMIC grows an upstream cursor-visibility inhibitor/API. Unpatched COSMIC still cannot do dynamic hide/show correctly; sky-cua must either report system cursor hiding unsupported or run the dedicated transparent-session mode that keeps the native cursor transparent for the whole session.

## Recovery and Idempotence

Every adapter must restore on normal hide, overlay host shutdown, and backend drop.

GNOME extension disable must release cursor inhibition even if sky-cua crashes.

Hyprland adapter must restore the saved `cursor:invisible` value and should log command failures with enough detail to repair the user session manually.

COSMIC bridge must preserve and restore the previous seat cursor state. If the bridge process or compositor component restarts, the client must reprobe and avoid claiming the cursor is hidden until a fresh hide succeeds.

## Outcome

This plan is complete when all compositor rows in the acceptance matrix have source implementation, automated checks where practical, and live proof artifacts. The final state must satisfy the invariant that sky-cua always attempts to show the agent overlay when requested, and uses compositor-specific mechanisms to hide the real cursor wherever that is actually possible.
