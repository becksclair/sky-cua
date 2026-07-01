# Why the isolated desktop is X11, and how the env recipe forces the X11 lane

## Context

The sky-cua computer-use agent normally drives the user's live login session.
The isolated-desktop feature needed a private graphical desktop the agent could
fully own without disturbing the human. The open question was: which display
protocol can host that private desktop, and how do we make the Linux backend
actually select it when the user's real session is running on the same machine?

## Investigation

The Linux backend is genuinely dual-path. Its X11 lane (`ximagesrc` capture,
XTest pointer/keyboard injection via `xdotool`) is selected purely off `DISPLAY`
and is session-independent. Its Wayland lane (portal capture, kernel/portal
input) is welded to the live login session through per-user, per-compositor
portal restore tokens (`crates/sky-cua-linux/src/portal/token_store.rs`). X11 is
`DISPLAY`-addressable and session-independent; Wayland is addressed by
`WAYLAND_DISPLAY` and, in this codebase, bound to the live session. A nested
Wayland compositor would inherit the portal-binding problem, so the private
desktop must be X11 — a headless `xpra start-desktop` virtual display.

Pointing a daemon at `DISPLAY=:100` is not sufficient to get the X11 lane.
`env_probe::detect_compositor()` reads every `/proc/<pid>/comm` system-wide and
returns `"kde-kwin-wayland"` whenever the user's real KWin is running anywhere on
the machine; `infer_session_kind` then forces `Wayland` because that string
contains `"wayland"`. Two `doctor` runs with `WAYLAND_DISPLAY` unset both reported
`session_kind: wayland`, byte-identical at 14111 bytes. The blocklist var
`SKY_CUA_CLIENT_CLEARED_SESSION_ENV_KEYS=WAYLAND_DISPLAY` did not change this.

The fix is env-only and precise. `infer_session_kind`
(`crates/sky-cua-linux/src/env_probe.rs`) has an early return:

```rust
Some(value) if value == "x11" && has_display && x11_server_available => {
    return SessionKind::X11;
}
```

Setting `XDG_SESSION_TYPE=x11` with `DISPLAY=:N` set and the X server reachable
hits this branch and short-circuits the `/proc` compositor scan before it can
vote. With `DISPLAY=:100 XDG_SESSION_TYPE=x11` (and `WAYLAND_DISPLAY` unset),
`doctor` reported `session_kind: x11`, `capture_backend: x11`,
`input_backend: x_test`, `semantic_backend: atspi`, `display: :100`. The `/proc`
scan still sees `kde-kwin-wayland`, but it no longer wins. No edit to `env_probe`
is required.

Two operational landmines surfaced on xpra 6.4.4. First, an xpra
`start-desktop` display reports a transient screen mode at startup (e.g.
`2048x1536`) before settling on the requested geometry, and that transient mode
can be reported stably across several `xdpyinfo` reads — so a naive "wait for N
stable reads" wrongly accepts it. The robust approach is to wait for the display
to reach the *requested* resolution (parsed from config and applied with
`--resize-display=<WxH>`), falling back to stability detection only when no
resolution is parseable. Second, `xpra list` on 6.4.4 prints a live session as
`\tLIVE session at :100` — the display number is the trailing token, not a
tab-separated column — so the parser tokenizes, requires the exact `:N` token plus
a `live` marker, and double-checks reachability with `xdpyinfo`.

## Conclusion

The isolated desktop must be X11, not a nested Wayland compositor, because the
X11 lane is `DISPLAY`-addressable and session-independent while the Wayland lane
is welded to the live session via portal restore tokens. Selecting that lane on a
machine running a real Wayland compositor requires forcing
`XDG_SESSION_TYPE=x11` (with `DISPLAY=:N` and a reachable X server); this hits the
`infer_session_kind` early return and short-circuits the system-wide `/proc`
compositor scan that would otherwise vote the session back to Wayland.

## Implications

- The env recipe for the isolated daemon includes `DISPLAY=:N` and
  `XDG_SESSION_TYPE=x11`; without the latter the `/proc` scan reselects Wayland.
- No change to `env_probe`'s detection logic was made or is needed; the override
  is purely environmental.
- The regression test `crates/sky-cua-linux/tests/isolated_x11_probe.rs` encodes
  this finding so a future change to `env_probe` cannot silently break the X11
  lane selection under the recipe.
- Virtual-display geometry is read only after the display reaches the requested
  resolution, never in the first instant; `--resize-display=<WxH>` applies the
  requested mode.
