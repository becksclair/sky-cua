# Compositor cursor hiding

## Status

Shipped on Linux for KWin, X11/i3, GNOME Shell, Hyprland, and patched
COSMIC, with a separate VM-only transparent-Xcursor mode for unpatched
COSMIC. Last verified: 2026-05-16 across the Arch `testing-vm` desktop
matrix. Long-term unpatched COSMIC support remains a backlog item.

## Summary

When the agent cursor overlay is visible, the user's real compositor cursor
is hidden so the user sees one cursor, not two. Cursor hiding is
backend-specific because generic Wayland clients cannot globally hide the
compositor pointer. The visible-overlay state and system-cursor-hidden
state are independent capabilities; failure of one does not disable the
other.

This feature owns the system cursor hide path. The visible overlay itself
is owned by [`docs/features/agent-cursor-overlay.md`](agent-cursor-overlay.md).

## Contract surface

Public model in `crates/sky-cua-platform/src/model.rs`:

- `AgentCursorCapabilities.system_cursor_hide_supported: bool`
- `AgentCursorCapabilities.system_cursor_hidden: bool`
- `AgentCursorCapabilities.system_cursor_backend: AgentCursorSystemCursorBackend`
- `AgentCursorCapabilities.pointer_tracking_backend: AgentCursorPointerTrackingBackend`
- `AgentCursorCapabilities.pointer_tracking_exact: bool`
- `AgentCursorCapabilities.reason: Option<String>` — explanation when
  `system_cursor_hide_supported=false`.

`AgentCursorSystemCursorBackend` variants currently shipped:

- `none`, `unsupported`, `wayland_client_unsupported`
- `x11_xfixes`
- `kwin_effect`
- `gnome_shell_extension`
- `hyprland_config`
- `cosmic_comp_bridge`
- `cosmic_transparent_xcursor`
- `windows_win32`, `macos_native` — placeholders for future native adapters

## Behavior

Per-backend hide path:

| Compositor          | Backend                       | Hide mechanism                                                                  | Restore on overlay hide |
| ------------------- | ----------------------------- | ------------------------------------------------------------------------------- | ----------------------- |
| X11 (Xorg, i3)      | `x11_xfixes`                  | `XFixesHideCursor` / `XFixesShowCursor` from the overlay host                   | Yes                     |
| KDE Plasma / KWin   | `kwin_effect`                 | C++ shim calls compositor `hideCursor()` / `showCursor()` and emits `PointerMoved(x, y, sequence)` while visible; layer-shell draws the agent cursor | Yes |
| GNOME Shell         | `gnome_shell_extension`       | Extension uses `inhibit_cursor_visibility()` (Shell 49+) or `set_pointer_visible(false)` (older) with a guarded fallback | Yes |
| Hyprland            | `hyprland_config`             | Snapshots the previous `cursor:invisible` value, sets it through `hyprctl`, restores afterward | Yes (previous value) |
| COSMIC (patched)    | `cosmic_comp_bridge`          | Patched `cosmic-comp` exposes a Unix socket sentinel; bridge daemon toggles `CursorImageStatus::Hidden` in compositor seat state | Yes |
| COSMIC (no-patch)   | `cosmic_transparent_xcursor`  | COSMIC session is launched with `XCURSOR_THEME=sky-cua-blank`; the agent overlay covers what would be a transparent native cursor | No (no normal cursor when overlay hides) |

Adapter selection ordering inside the overlay host's
`SystemCursorAdapter`:

1. GNOME extension adapter when the Shell DBus service is reachable.
2. Hyprland adapter when running under Hyprland.
3. COSMIC bridge adapter when the bridge is reachable.
4. `WaylandClientUnsupported` for generic Wayland (with a Wayland-focus
   reason).

KWin's effect is separate because a generic click-through layer-shell client
cannot hide the compositor cursor or observe global pointer motion. The effect
does not draw the agent cursor; when it is loaded beside the layer-shell backend,
`backend=wayland_layer_shell`, `system_cursor_backend=kwin_effect`, and
`system_cursor_hide_supported=true`. On updated shims, KDE also reports
`pointer_tracking_backend=kwin_effect_signal` and
`pointer_tracking_exact=true`.

X11 is selected when the overlay host's X11 backend is active.

Capability honesty rules:

- Repeated `set_hidden(state)` calls are no-ops.
- Bridge / extension startup clears stale hidden state.
- `show` clears the hidden sentinel even when compositor integration is
  absent.
- Stale bridge sockets and sentinels are cleaned during VM session
  switches.

## Source paths

- `crates/sky-cua-platform/src/model.rs` — backend variants and capability
  fields
- `crates/sky-cua-overlay-host/src/system_cursor.rs` — adapter trait and
  selection plus the Hyprland-config, COSMIC bridge, and transparent-
  Xcursor adapter implementations
- `crates/sky-cua-overlay-host/src/x11.rs` — XFixes adapter wiring
- `crates/sky-cua-overlay-host/src/layer_shell.rs` — Wayland visual overlay
  and system-cursor adapter selection
- `crates/sky-cua-overlay-host/src/gnome_shell.rs` — GNOME extension
  adapter client
- `resources/kwin/effects/sky-cua-agent-cursor/` — KWin cursor-hide and
  pointer-position shim
- `resources/gnome-shell-extension/codex-window-control@openai.com/extension.js`
  — GNOME extension cursor service (extends existing window-control
  service; same UUID and DBus path)
- `resources/cosmic/cosmic-comp-sky-cua-cursor-bridge.patch` — COSMIC
  compositor patch (development prototype)
- `crates/sky-cua-cosmic-helper/` — packaged COSMIC bridge daemon
- `scripts/install_blank_xcursor_theme.py` — generates the
  `sky-cua-blank` transparent Xcursor theme for the no-patch COSMIC mode

## Verification

Unit and crate-level:

```bash
cargo test -p sky-cua-platform agent_cursor
cargo test -p sky-cua-overlay-host system_cursor
cargo test -p sky-cua-overlay-host layer_shell
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

Latest accepted artifacts:

- KDE/KWin: `artifacts/kde-framebuffer-cursor-proof/kwin-system-install/20260515T132649888064Z/host-summary.json`
- Hyprland: `/workspace/artifacts/codex-e2e/agent-cursor-wayland-layer-shell/20260515T142710878162Z/`
- X11/i3: `/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260515T142731049499Z/`
- GNOME (real GDM Wayland session): `artifacts/gnome-framebuffer-cursor-proof/20260515T140437893805720Z/host-summary.json`
- COSMIC patched compositor (built from Arch's `cosmic-comp` source commit `b5a1a6d3179810627fa0bffac7bd5d78c7df4fa0` plus the bridge patch): `artifacts/cosmic-framebuffer-cursor-proof/20260515T142538562074Z/host-summary.json`
- COSMIC transparent Xcursor (no-patch fallback): `artifacts/cosmic-transparent-xcursor-cursor-proof/20260516T073232164704Z/host-summary.json`

Each accepted artifact reports `ok=true`, host framebuffer agent marker
present while overlay is visible, no native cursor marker until hide, and
clean cleanup.

## Local install and update on a real Plasma host

`scripts/install_kwin_effect.py` builds the cursor shim from the current sources,
installs it system-wide (`sudo cmake --install`, the only step that escalates;
the exact command is printed first), enables the next generated effect id
persistently in `kwinrc`, and drives the running KWin to that new build:

```bash
python3 scripts/install_kwin_effect.py --status   # session + effect state as JSON
python3 scripts/install_kwin_effect.py            # build, install, reload
```

Update detection uses a build stamp: the deploy hashes the shim sources into
`SKY_CUA_EFFECT_BUILD_ID`, the compiled effect reports it
through the `com.skycua.AgentCursor.BuildId` DBus slot, and the script
considers the deploy converged only when the running build id matches the
installed one. KWin does not dlclose replaced effect libraries, so the deploy
does not replace the active `.so` in place. It installs the next generated id
(`sky-cua-agent-cursor-000001`, `...-000002`, ...), unloads the previously
loaded sky-cua id to release `/com/skycua/AgentCursor`, reconfigures KWin,
loads the new id, and then removes every older exact sky-cua id from KWin
plugin paths and `kwinrc`. Cleanup keeps only the active generated id after a
successful deploy. If KWin DBus is unreachable, the new id is installed and
enabled for the next Plasma session start without attempting live load. The
deploy never restarts KWin itself: restarting `plasma-kwin_wayland.service`
took a whole session down during live verification (it can also bring KWin
back without re-claiming the `org.kde.KWin` DBus name). `--no-notify` is kept
as a legacy no-op for automation compatibility. The shim itself starts hidden — autoloading with the session
must not hide the user's cursor until the layer-shell overlay host activates
it — and carries an 8s idle auto-hide failsafe that restores the system cursor
when the overlay host stops refreshing the cursor state (see the watchdog chain in
[`agent-cursor-overlay.md`](agent-cursor-overlay.md)).

The same deploy runs from the plugin lanes:

```bash
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime --kwin-effect
python3 scripts/deploy_plugin.py --kwin-effect
```

Legacy installs without the `BuildId` slot report `unknown` and are treated as
stale. Rerun the installer after KWin package updates: the effect links
against the installed KWin headers and a stale binary can fail to load.

## Known limitations

- **Generic Wayland clients cannot hide the compositor cursor globally.**
  Click-through layer-shell surfaces have no pointer focus, so a generic
  Wayland layer-shell backend always reports
  `system_cursor_hide_supported=false` with a Wayland-focus reason.
- **The COSMIC compositor patch is a development prototype, not the
  desired long-term contract.** An upstreamable version should be generic,
  token / refcount based, and suppress all final cursor render sources
  rather than only `SeatExt::cursor_image_status()`. See
  `docs/research/2026-05-cosmic-cursor-hiding-options.md`.
- **The `cosmic_transparent_xcursor` mode does not restore a normal
  cursor when the overlay hides.** It preserves the one-visible-cursor
  invariant only while the agent overlay is visible. It is intended for
  controlled VMs.
- **GNOME Shell 49+ uses cursor visibility inhibition rather than
  `set_pointer_visible()`.** The extension prefers the new API and falls
  back guarded for older Shell versions; very old Shell versions may
  exhibit visible-cursor flicker during inhibit/uninhibit cycles.

## Related

- Companion feature: [`docs/features/agent-cursor-overlay.md`](agent-cursor-overlay.md)
- Research: [`docs/research/2026-05-cosmic-cursor-hiding-options.md`](../research/2026-05-cosmic-cursor-hiding-options.md)
- ROADMAP entries: [`ROADMAP.md`](../../ROADMAP.md) under "Linux desktop
  parity" — both the shipped item and the long-term unpatched-COSMIC
  follow-up.
- Originating ExecPlan (retired into this feature doc; see git history for `plans/compositor_cursor_hiding.md`).
