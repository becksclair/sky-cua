# COSMIC cursor-hiding options: patched compositor, transparent Xcursor, or wait

## Context

The agent cursor overlay on COSMIC Wayland needs the user's real cursor
hidden while the agent overlay is visible. COSMIC differs from KDE,
GNOME, and Hyprland in that it offers no public IPC or config for
globally hiding the cursor: cursor visibility lives entirely in
compositor seat state, behind `cursor_image_status` /
`set_cursor_image_status`. A normal Wayland or layer-shell client cannot
toggle that state.

This research records the options surveyed and the two paths shipped.

## Investigation

A fresh current-source pass over `cosmic-comp` and `xdg-desktop-portal-cosmic`
found no public cursor-hide IPC, config key, DBus API, or custom Wayland
protocol. The exposed cursor path remains compositor seat state:
`SeatHandler::cursor_image` forwards to
`seat.set_cursor_image_status(image)`, while public config only exposes
cursor focus behavior such as `focus_follows_cursor` and
`cursor_follows_focus`. Sway / wlroots evidence agrees with the broader
Wayland rule: compositor code hides by unsetting or replacing the
compositor cursor image; a click-through layer-shell client cannot hide
the real cursor.

Three options were considered:

### Option A: Patched `cosmic-comp` with a sky-cua bridge

Add a Unix socket bridge into `cosmic-comp` that toggles
`CursorImageStatus::Hidden` in compositor seat state on demand. The patch
lives at `resources/cosmic/cosmic-comp-sky-cua-cursor-bridge.patch` and
is shipped as a development prototype. The packaged
`sky-cua-cosmic-helper` daemon serves the socket and reports
`supported=false` until `$XDG_RUNTIME_DIR/sky-cua-cosmic-cursor-ready`
exists.

This was proven on the Arch testing-vm by building `cosmic-comp` from the
Arch source commit `b5a1a6d3179810627fa0bffac7bd5d78c7df4fa0` plus the
bridge patch. Artifact:
`artifacts/cosmic-framebuffer-cursor-proof/20260515T142538562074Z/host-summary.json`
reports `ok=true`, `system_cursor_backend=cosmic_comp_bridge`, hidden
true after set, hidden false after hide, host framebuffer agent marker
found while visible, and restored real-cursor marker absent until hide.

The current patch is not the desired upstream contract. It only suppresses
`SeatExt::cursor_image_status()`, not all final cursor render sources, and
it is hardcoded for a single sky-cua client. An upstreamable version
should be generic, token / refcount based, and suppress every final cursor
render source.

### Option B: Transparent Xcursor theme

Generate a valid transparent `sky-cua-blank` Xcursor theme via
`scripts/install_blank_xcursor_theme.py`, start the COSMIC session with
`XCURSOR_THEME=sky-cua-blank`, and let the agent overlay cover what would
otherwise be a transparent native cursor.

The VM session wrapper accepts `cosmic-blank` and `cosmic-transparent` as
session names. The `CosmicTransparentXcursorAdapter` reports
`system_cursor_backend=cosmic_transparent_xcursor` only when COSMIC is
actually launched with that theme. Artifact:
`artifacts/cosmic-transparent-xcursor-cursor-proof/20260516T073232164704Z/host-summary.json`
reports `ok=true`, agent marker visible, agent marker absent after hide,
no native cursor marker in the hidden frame.

This mode preserves the one-visible-cursor invariant while the agent
overlay is visible, but it intentionally does not restore a normal native
cursor when the overlay hides. It is therefore suitable only for
controlled VMs, not for production user sessions where the user expects a
normal cursor between agent actions.

### Option C: Wait for upstream

Wait for `cosmic-comp` to add a public cursor-visibility inhibitor or
accept an upstreamable version of Option A. Until then, unpatched
production COSMIC sessions remain honestly unsupported: no
`$XDG_RUNTIME_DIR/sky-cua-cosmic-cursor-ready` sentinel means
`system_cursor_hide_supported=false`,
`system_cursor_hidden=false`, and `system_cursor_backend=cosmic_comp_bridge`.

## Conclusion

Two ship-ready paths, both honest:

1. **Patched COSMIC** for environments where the user can run a custom
   `cosmic-comp` build. Production-quality cursor hiding, but requires
   the bundled patch; not the desired long-term contract.
2. **Transparent Xcursor** for controlled VM environments where the
   COSMIC session can be started with `XCURSOR_THEME=sky-cua-blank`.
   Preserves the one-visible-cursor invariant only while the overlay is
   visible.

Unpatched production COSMIC remains unsupported for system cursor hiding,
and the runtime reports that truthfully through `AgentCursorCapabilities`.
The visible overlay is unaffected by this gap.

## Implications

- The shipped feature ([`docs/features/compositor-cursor-hiding.md`](../features/compositor-cursor-hiding.md))
  documents both adapters with explicit limitations.
- The bundled patch is a prototype, not the production contract. Future
  work should propose a generic, token / refcount based cursor-visibility
  inhibitor upstream and replace the bundled patch when accepted.
- Detection must be cheap and deterministic: the bridge adapter probes the
  helper socket and the ready sentinel; the transparent-Xcursor adapter
  inspects `XCURSOR_THEME`.
- The unpatched-COSMIC long-term path remains an open backlog item in
  `ROADMAP.md`.
