# X11 shaped-window backend on XWayland vs real X11

## Context

The agent cursor overlay needs a visible-overlay path on X11/i3 sessions
that does not rely on Wayland layer-shell. An X11 shaped-window backend
based on `x11rb` was added that uses X Shape bounding regions for
transparency and an empty input shape for click-through. This research
records why the accepted X11 visual proof must come from a real Xorg
session rather than host XWayland, and what the embedded acceptance path
looks like.

## Investigation

The X11 shaped-window backend lives in `crates/sky-cua-overlay-host/src/x11.rs`
and uses `x11rb = "0.13.2"`. It creates a top-level X11 window, applies a
bounding region matching the cursor asset alpha, sets an empty input
region, and renders the bundled `cursor-chat.png` from
`crates/sky-cua-overlay-host/assets/`.

On Asgard (KDE Plasma 6 Wayland with XWayland), the X11 backend
instantiates correctly when forced via `SKY_CUA_OVERLAY_BACKEND=x11`. It
opens the X display, creates the window, and reports `x11_shaped_window`
through the overlay-host capability protocol. However, the rendered
overlay does not appear in Wayland portal capture. The forced-XWayland
visible smoke produced a frame that did not contain the cursor marker even
though the X11 client itself was visible to a separate `xwininfo` probe.

This is consistent with how KDE composites XWayland surfaces: portal
ScreenCast captures the Wayland output, and XWayland surfaces are composed
into that output through the regular surface tree, but a click-through
shaped X11 window without its own pointer focus and without a normal
window-manager decoration path does not reliably become a visible Wayland
surface. In short, host XWayland is not a portable substitute for a real
X11 session for this backend.

A real-X11 acceptance command was added:
`scripts/live_agent_cursor_x11_overlay_smoke.py --current-display`. On the
Asgard host (Wayland session) it refuses cleanly with
`XDG_SESSION_TYPE=wayland` and instructs the operator to switch to a real
X11 session. This is the right behavior; it prevents accidentally
treating XWayland as proof.

For agent-driven proof the smoke supports an embedded acceptance mode:
`scripts/live_agent_cursor_x11_overlay_smoke.py --embedded-session`. It
launches `Xvfb` plus `Openbox` as a minimal X11 session, runs the visible
overlay, captures with `import` or equivalent, hides, re-shows, captures
again, and proves click-through with a Tk target window underneath.
Artifact:
`artifacts/codex-e2e/agent-cursor-x11-overlay/20260514T221333143236Z/summary.json`
reports `system_cursor_hide_supported=true`,
`system_cursor_hidden_after_set=true`,
`system_cursor_hidden_after_hide=false`, and
`system_cursor_hidden_after_show=true`.

The accepted production proof for X11/i3 is the `i3` VM runner profile.
That profile boots an Arch testing-vm into a real Xorg/i3 guest session,
reconstructs `DISPLAY` and `XAUTHORITY` from the active Xorg command when
the user environment has stale Wayland values, and runs the X11 overlay
smoke against the VM's real X server. Artifact:
`/workspace/artifacts/codex-e2e/agent-cursor-x11-overlay/20260515T142731049499Z/`.

## Conclusion

The X11 shaped-window backend is shipped and proven on real Xorg sessions
but is not accepted on host XWayland for visible-overlay proof. The
two acceptance paths are:

1. **Embedded `Xvfb` plus `Openbox` smoke** for code-level proof of the
   backend's render, hide, re-show, click-through, and XFixes system cursor
   hide/show behavior.
2. **Real Xorg VM session** via the `i3` profile for production-equivalent
   proof on a desktop session.

XWayland is suitable for X11-only AT-SPI selector and metadata work
(matching, `xprop`, `xwininfo`) but not for proving that a click-through
shaped X11 window renders visibly through Wayland portal capture.

## Implications

- The X11 overlay backend is documented as shipped in
  `docs/features/agent-cursor-overlay.md`, with the XWayland limitation
  recorded under "Known limitations".
- Smokes that need visible-overlay proof on X11 must use either the
  embedded `Xvfb`+`Openbox` mode or the `i3` VM profile. They must not
  treat host XWayland as acceptance.
- The i3 VM profile must reconstruct `DISPLAY` and `XAUTHORITY` from the
  active Xorg command when the user environment has stale Wayland values,
  to avoid the smoke accidentally targeting an old Plasma session's
  XWayland.
