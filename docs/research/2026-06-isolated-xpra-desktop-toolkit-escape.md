# How toolkit apps escape an X11 sandbox, and the daemon-env-boundary fix

## Context

The isolated-desktop feature places the computer-use agent on a private
`xpra start-desktop` X11 display so its actions never touch the user's live
session. The open question was: is setting `DISPLAY=:N` on the daemon enough to
keep everything the agent launches inside the private desktop, or can launched
applications escape back onto the user's real screen?

## Investigation

During the spike, a pure-Xlib app (`xmessage`) correctly rendered on `:100`, but
two Qt apps (`konsole`, `kcalc`) rendered on the host's real desktop even though
`DISPLAY=:100` was set. Two distinct escape mechanisms were responsible:

1. **Toolkit Wayland preference.** `WAYLAND_DISPLAY=wayland-0` was still set, and
   Qt (and GTK) prefer Wayland when it is reachable, ignoring `DISPLAY`. The
   launched window appeared on the host Wayland session.
2. **D-Bus single-instance activation.** KDE/GNOME single-instance apps reach the
   host's already-running instance through `DBUS_SESSION_BUS_ADDRESS` /
   `$XDG_RUNTIME_DIR/bus` and ask it to open a window — escaping even with the
   display vars correct.

A spawn-site survey found roughly forty `Command` sites across
`crates/sky-cua-linux` and `crates/sky-cua-service` that inherit the process
environment wholesale (for example `xdotool` at
`crates/sky-cua-linux/src/x11/input_xtest.rs`). Sanitizing each site individually
would be fragile. The chosen fix sanitizes once, at the daemon's environment
boundary, at spawn. The isolated daemon is spawned with:

- `DISPLAY=:N`
- `XDG_SESSION_TYPE=x11`
- `QT_QPA_PLATFORM=xcb`
- `GDK_BACKEND=x11`
- `DBUS_SESSION_BUS_ADDRESS=<sandbox bus>`
- `SKY_CUA_SERVICE_SOCKET_PATH=<isolated socket>`

and with `WAYLAND_DISPLAY` removed. Because every helper spawn and every
`desktop_launch_app` child inherits this environment verbatim, the whole subtree
is sandboxed by inheritance. `QT_QPA_PLATFORM=xcb` and `GDK_BACKEND=x11` force the
toolkits onto X11 even if a stray Wayland var survives; the sandbox session bus
closes the single-instance escape. The Linux `launch_application` implementation
relies on pure inheritance and deliberately mutates no display/session variable,
so the inheritance contract is the leak-safety guarantee.

Clearing `WAYLAND_DISPLAY` turned out to be a two-part contract, not a single
`env_remove`. The daemon repairs missing graphical-session variables from its own
`/proc`/systemd probing, so it could re-hydrate `WAYLAND_DISPLAY` back from the
live session and reopen the escape. The fix is `LaunchEnvironment::for_isolated_daemon`
(`crates/sky-cua-client/src/launch_environment.rs`), which builds the daemon's
health expectations from the sandbox `spawn_env` — the entries in
`GRAPHICAL_SESSION_ENV_KEYS` that are not in `removed_env` — and sets
`detached_graphical_env = true`. That flag has two effects: the daemon's startup
health is scoped to the sandbox graphical identity (so a correctly-sandboxed
daemon is not rejected as "stale" against the client's host values and re-spawned
in a loop), and the client emits the full `GRAPHICAL_SESSION_ENV_KEYS`
cleared-list to the daemon, whose blocklist then suppresses re-hydrating
`WAYLAND_DISPLAY`. So `WAYLAND_DISPLAY` must be both `env_remove`d on the spawn
command and reported to the daemon as deliberately cleared.

## Conclusion

`DISPLAY=:N` alone does not keep toolkit applications inside an X11 sandbox: Qt/GTK
escape to Wayland when `WAYLAND_DISPLAY` is reachable, and KDE/GNOME
single-instance apps escape through the host session bus. Sanitizing the daemon's
environment once, at spawn — setting the X11 display, forcing the toolkits onto
xcb/x11, pointing at the sandbox session bus, and clearing `WAYLAND_DISPLAY` —
sandboxes the daemon and every program it launches by inheritance. Clearing
`WAYLAND_DISPLAY` is a two-part contract: remove it on the spawn command and tell
the daemon it was deliberately cleared, via the `detached_graphical_env` cleared
list, so the daemon does not repair it back.

## Implications

- Sandboxing lives at one seam (the daemon spawn env), not at the roughly forty
  helper-spawn sites.
- `QT_QPA_PLATFORM=xcb`, `GDK_BACKEND=x11`, and the sandbox
  `DBUS_SESSION_BUS_ADDRESS` are part of the leak-safety recipe, not optional
  niceties.
- The Linux `launch_application` implementation must never set or mutate a
  display/session variable; pure inheritance is the guarantee. The host-leak test
  `crates/sky-cua-linux/tests/isolated_app_launch_leak.rs` guards this by checking
  the launched process's `/proc/<pid>/environ` for the sandbox markers.
- `for_isolated_daemon` plus `detached_graphical_env = true` is required for both
  the health check (no re-spawn loop) and the cleared-list contract (no
  `WAYLAND_DISPLAY` re-hydration).
- Confirmed live (2026-06-30, KDE Wayland): launching `kcalc` through the real
  isolated daemon placed it on `:100` with a clean environ (no `WAYLAND_DISPLAY`),
  while constructing `LinuxDesktopBackend` WITHOUT the cleared-list signal caused
  `hydrate_session_env` to re-add `WAYLAND_DISPLAY=wayland-0` and the app escaped.
  A test (or any caller) that builds the backend directly must replicate the full
  daemon spawn contract — the private session bus AND
  `CLIENT_CLEARED_SESSION_ENV_KEYS` — or it both under-tests the sandbox and risks
  a real leak. The host-leak guard now does exactly this.
