# KWin window targeting (focused_window + verified activation)

## Status

Shipped. Last verified: 2026-06-12, live round-trip on KDE Plasma 6 Wayland
(`cargo run -p sky-cua-linux --example kwin_focus_probe`).

## Summary

KWin Wayland now supports `focused_window`, exact `activate_window` with
focus verification, and verified targeted keyboard input — the same
capability tier as the GNOME extension, COSMIC, Hyprland, i3, and X11
backends. Active-window readback and activation run through the KWin
scripting API with results returned over a session-bus callback.

## Contract surface

- `focused_window` works on KWin; the discovery window list marks the
  active window with `focused: true`.
- `activate_window` on KWin verifies focus through the standard registry
  verification poll instead of returning a `WindowActivationSent`
  best-effort diagnostic.
- Doctor: the `kwin` window probe reports `can_focus_windows` equal to
  scripting availability; `can_target_windows` is true on KDE sessions
  with `org.kde.KWin /Scripting` reachable. Scripting reachability is
  probed with a side-effect-free `isScriptLoaded` call (TTL-cached ~30s),
  so doctor reports stay honest when the scripting seam is down. The
  qdbus6/qdbus PATH requirement is gone — activation no longer shells out.
- DBus callback object (internal, not a stable API): the daemon serves
  `com.skycua.KWinScript` at `/com/skycua/KWinScript` on its session-bus
  connection; KWin scripts call `Result(token, payload)` and
  `ActiveWindowChanged(payload)` on the daemon's unique bus name.
- Loaded KWin script plugin names (per-process, cleaned on reuse):
  `sky-cua-kwin-query-<pid>-<token>` (transient) and
  `sky-cua-focus-watch-<pid>` (persistent watcher).

## Behavior

`org.kde.KWin.queryWindowInfo` is an interactive window picker — it blocks
on a human click and returns `UserCancel` otherwise, which is why earlier
builds stubbed active-window readback entirely. The replacement is the
kdotool pattern, daemon-shaped:

1. **Callback channel** (`kwin_script.rs`): a shared zbus session
   connection serves the callback object. Transient scripts are written to
   a temp file, loaded via `org.kde.kwin.Scripting.loadScript`, run at
   `/Scripting/Script{id}`, and report through `callDBus` to the daemon's
   unique name; the daemon awaits the result with a 3s timeout, then
   stops, unloads, and removes the script.
2. **focused_window**: a persistent watcher script subscribes
   `workspace.windowActivated` and pushes active-window JSON
   (`internalId`, caption, resourceClass, pid) into a local cache, seeded
   with the active window at load. Watcher health is checked per query
   with `isScriptLoaded`; if KWin restarted (script gone), the cache is
   cleared and the watcher reloaded. Until the cache holds an event, a
   transient one-shot script reads `workspace.activeWindow` directly.
3. **Verified activation**: the activation script finds the window by
   `internalId`, activates and raises it, then reads back
   `workspace.activeWindow.internalId` in the same script run. Verdicts:
   `verified`, `dispatched` (focus had not landed yet — the registry
   verification poll settles it), `no-match` (error). Both Plasma 6
   (`windowList`/`activeWindow`/`windowActivated`) and Plasma 5 spellings
   are tolerated in the scripts.
4. **Resilience**: window listing treats active-window readback as
   best-effort — a wedged scripting seam degrades to an unmarked window
   list instead of failing discovery. All scripting DBus calls are
   individually timeboxed.

## Source paths

- `crates/sky-cua-linux/src/kwin_script.rs` — callback channel, scripts,
  watcher, payload parsing.
- `crates/sky-cua-linux/src/kwin.rs` — `query_active_window`,
  `activate_window`, discovery focus marking.
- `crates/sky-cua-linux/src/windowing/registry.rs` — `probe_kwin`
  capability flags; KWin focus-verification special-casing removed.
- `crates/sky-cua-linux/src/backend.rs` — KWin carve-outs removed from
  `focused_window`, `activate_window`, and targeted-input gating.
- `crates/sky-cua-linux/src/doctor.rs` — KDE readiness messaging.
- `crates/sky-cua-linux/examples/kwin_focus_probe.rs` — live smoke probe.

## Verification

- Unit tests in `kwin_script.rs` (payload parsing, script generation) and
  updated registry/doctor tests; full workspace `cargo test` green.
- Live: `cargo run -p sky-cua-linux --example kwin_focus_probe` on a KDE
  Plasma 6 Wayland session — prints the active window, the discovered
  window list with focus marking, and performs an activate/verify/restore
  round-trip. Verified 2026-06-12 on the primary desktop (Ghostty ↔ Brave).

## Known limitations

- KWin scripting is fire-by-script: a malicious or broken co-resident
  KWin script could interfere; verdicts rely on KWin's own
  `workspace.activeWindow` truth.
- The watcher cache can briefly lag if KWin emits no `windowActivated`
  after the last active window closes; the one-shot fallback covers
  fresh-cache cases but not every stale-cache window. The registry
  verification poll re-reads discovery, which bounds the impact.
- Watcher lifecycle: the service unloads its watcher on graceful shutdown
  (`kwin_script::shutdown` from the IPC server exit path); crashed
  processes leave a dead watcher until the next sky-cua start sweeps it
  (orphan temp-file + `/proc` liveness) or KWin restarts.
- Rejected alternatives: `ext-foreign-toplevel`/`zwlr-foreign-toplevel`
  (KWin upstream declined, bug 502647), `xdg-activation-v1` (cannot
  activate another client's surface), `org_kde_plasma_window_management`
  (requires an `X-KDE-Wayland-Interfaces` desktop-file grant; viable
  future upgrade for event-driven listing without script machinery).

## Related

- `docs/features/kwin-x11-workspace-metadata.md` — window metadata fields.
- `docs/features/linux-targeting-and-diagnostics.md` — targeting registry.
- ROADMAP: Linux desktop parity → KWin window targeting.
