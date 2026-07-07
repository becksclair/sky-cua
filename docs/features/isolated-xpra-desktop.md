# Isolated xpra desktop

## Status

Shipped (computer-use only). Live-verified on a KDE Wayland host, 2026-06-30:
the gated `isolated_app_launch_leak` host-leak guard passes against real
xpra 6.4.4 + kcalc, and a full MCP end-to-end (`SKY_CUA_ISOLATED_DESKTOP=1` →
`desktop_launch_app` launching kcalc) placed the window inside the `:100`
sandbox — present on `:N`, absent from the user's real `:0`, with a clean
sandbox environ (`DISPLAY=:N`, sandbox `DBUS_SESSION_BUS_ADDRESS`, no
`WAYLAND_DISPLAY`) — while the user's desktop stayed untouched. Also verified:
config/spawn-env unit tests and the gated `isolated_x11_probe` env-recipe
regression test.

The previously-outstanding VM live-proof gate closed 2026-07-07: the
`isolated-xpra` VM smoke profile ran green on the Arch testing-vm (COSMIC
session, `wayland-1`) via `scripts/run_gui_testing_vm_smoke.py --profile
isolated-xpra` (a default build+sync run, then a `--skip-host-build` re-run
after installing xpra in the guest). Evidence: sandbox up on `:131` at
1920x1080, `dep_xpra/openbox/xdotool=true`, xmessage launched into the
sandbox, "app present on :131", "app absent from host displays: :0",
launched-app environ confirmed `DISPLAY=:131` with no `WAYLAND_DISPLAY`, and a
clean stop (`stopped=true`). The same day, both host live-proofs were
re-run and stayed green on the KDE Wayland host: the gated
`isolated_app_launch_leak_guard_keeps_app_off_host` test (`cargo nextest run
-p sky-cua-linux -E 'test(isolated_app_launch_leak_guard_keeps_app_off_host)'`,
31s, real xpra+kcalc, not skipped) and a live MCP end-to-end
(`SKY_CUA_ISOLATED_DESKTOP=1` → `desktop_launch_app` launching kcalc with
`DISPLAY=:100`, `WAYLAND_DISPLAY` unset, private D-Bus
(`unix:path=/tmp/dbus-...`), kcalc visible in the sandbox `observe`, host
session untouched).

Provisioning gap found during the VM run: the testing-vm provisioner does
not yet install `xpra`, `xdotool`, or `xorg-xdpyinfo` into the guest, so the
first `isolated-xpra` profile run needed a manual `pacman -Sy xpra xdotool
xorg-xdpyinfo` before a `--skip-host-build` re-run could pass. Follow-up:
add these packages to `scripts/testing-vm/` provisioning so the profile is
green on a fresh guest without manual intervention. Separately, the guest's
sshd and DHCP lease had died after a host suspend during this session
(recovered via the qemu-guest-agent path: `systemctl restart sshd` plus a
network bounce) — an environment quirk, not a profile defect.

## Summary

When isolated mode is enabled, the computer-use agent runs inside its own private,
headless X11 desktop — an `xpra start-desktop` virtual display hosting a window
manager (Openbox by default) — instead of the user's live login session. Every
computer-use action (screenshots, pointer, keyboard, window/semantic actions,
application launches) happens on that private display, so the human keeps using
their real desktop uninterrupted. The human can optionally open a read-only
viewer to watch the agent without being able to fight it for control. This is a
non-interference feature, not a security sandbox: the private desktop runs as the
same OS user with the same filesystem, network, and system D-Bus access.

Browser-use is intentionally not routed into the private desktop; in isolated
mode the agent launches and drives a browser as an application the same way a
human would. The visible agent-cursor overlay (Wayland layer-shell only) does not
render in the X11 private desktop, which is an accepted non-goal.

## Contract surface

### `[isolated_desktop]` config table

Read from `~/.config/sky-cua/sky-cua.toml` (override `SKY_CUA_CONFIG_PATH`). Every
field is optional; defaults and per-process env overrides are layered on by the
resolver. Resolution precedence is env beats file beats default.

| Field            | Type / values                         | Default       | Meaning |
|------------------|---------------------------------------|---------------|---------|
| `enabled`        | bool                                  | `false`       | Master switch for isolated mode. |
| `display`        | `":N"` or the literal `"auto"`        | `":100"`      | X11 display; `"auto"` picks a free display number and persists the choice. |
| `resolution`     | `"<width>x<height>"` or `"auto"`      | `"auto"`      | Virtual display geometry. `"auto"` (the default) resolves to three-quarters of the largest connected monitor (via `xrandr`, floored to even dimensions) so the read-only viewer is a comfortable window; falls back to `1920x1080` when no monitor can be probed. |
| `window_manager` | string                                | `"openbox"`   | Window manager started inside the private desktop. |
| `viewer`         | `"attach"` \| `"html5"` \| `"none"`   | `"attach"`    | Read-only viewer mode. Unrecognized values fall back to `attach`. |
| `lifecycle`      | `"persistent"` \| `"ephemeral"`       | `"persistent"`| Whether the xpra session survives client exit. Unrecognized values fall back to `persistent`. |

### Environment overrides

The six override variables, each overriding the matching config field. They are
allowlisted in `.mcp.json` `env_vars` so the override survives the MCP launch
wrapper:

- `SKY_CUA_ISOLATED_DESKTOP` — `enabled` (boolean).
- `SKY_CUA_ISOLATED_DESKTOP_DISPLAY` — `display`.
- `SKY_CUA_ISOLATED_DESKTOP_RESOLUTION` — `resolution`.
- `SKY_CUA_ISOLATED_DESKTOP_WINDOW_MANAGER` — `window_manager`.
- `SKY_CUA_ISOLATED_DESKTOP_VIEWER` — `viewer`.
- `SKY_CUA_ISOLATED_DESKTOP_LIFECYCLE` — `lifecycle`.

### `desktop_launch_app` MCP tool

Launches an application into the agent's private desktop.

- Arguments: `command` (non-empty string, required) and `args` (array of strings,
  optional, defaults to empty).
- Returns the launched process `pid`.
- Isolated-only gating: the tool is advertised in every session, but at call time
  it refuses with a structured `IsolatedDesktopRequired` error when the client is
  not isolated. Launching applications onto the user's live session is
  intentionally out of scope. The gate lives at the client only
  (`ServiceClient::is_isolated`); the daemon performs no isolation check and
  launches into whatever environment it was spawned with.

### Isolated daemon socket naming

The isolated daemon listens on a distinct socket so it coexists with the user's
normal daemon through the per-socket singleton lock:

```
$XDG_RUNTIME_DIR/sky-cua/service-isolated-<N>.sock
```

where `<N>` is the resolved display number. The client redirects
`SKY_CUA_SERVICE_SOCKET_PATH` to this path at the isolated daemon for its
lifetime.

### `isolated-desktop` client subcommand

Hidden development/operations subcommand `sky-cua-client isolated-desktop
{ensure|status|stop}`:

- `ensure` brings the private desktop up idempotently and prints its display,
  settled geometry, and viewer mode.
- `status` reports, without starting anything, whether the configured display is
  up, its geometry, the resolved selection (enabled/viewer/lifecycle), and the
  presence of the required dependencies (`xpra`, `openbox`, `xdotool`) — all as
  structured fields.
- `stop` tears the session down and removes a stale `/tmp/.X<N>-lock`.

## Behavior

A second `sky-cua-service` daemon drives the private desktop on its own socket.
The client resolves the isolated selection, ensures the xpra desktop exists,
spawns the daemon with a sandboxed environment and the isolated socket, connects,
and launches the viewer. The daemon stays ignorant of xpra: it simply probes the
sanitized environment and selects the X11 lane.

The X11 lane is forced via `XDG_SESSION_TYPE=x11` with `DISPLAY=:N` set (and
`WAYLAND_DISPLAY` removed). This hits an early return in `env_probe`'s
`infer_session_kind`, short-circuiting the system-wide `/proc` compositor scan
that would otherwise vote the session back to Wayland because the user's real
compositor is running elsewhere on the machine. No change to `env_probe`'s
detection logic is required.

Sandboxing happens once, at the daemon's environment boundary, rather than at each
of the backend's roughly forty helper-spawn sites. The isolated daemon is spawned
with `DISPLAY=:N`, `XDG_SESSION_TYPE=x11`, `QT_QPA_PLATFORM=xcb`, `GDK_BACKEND=x11`,
`DBUS_SESSION_BUS_ADDRESS` set to the sandbox session bus, and
`SKY_CUA_SERVICE_SOCKET_PATH` set to the isolated socket, with `WAYLAND_DISPLAY`
removed. Because every helper the backend spawns and every application
`desktop_launch_app` starts inherits this environment verbatim, they all land
inside the private desktop. This closes both toolkit escapes: Qt/GTK apps no
longer prefer Wayland, and KDE/GNOME single-instance apps no longer reach the
host's running instance over the host session bus.

The isolated daemon's startup health expectations are scoped to the sandbox
graphical identity through `LaunchEnvironment::for_isolated_daemon`, which builds
the expected graphical-session vars from the sandbox `spawn_env` (the entries in
`GRAPHICAL_SESSION_ENV_KEYS` that are not in `removed_env`) and sets
`detached_graphical_env = true`. Without this, the client's normal health check
would demand the daemon echo the host's live-session values (`DISPLAY=:0`, host
session type, host bus, `WAYLAND_DISPLAY`) and reject the correctly-sandboxed
daemon forever, re-spawning it in a loop. The `detached_graphical_env` flag is
also load-bearing for the cleared-list contract: it makes the client emit the
full `GRAPHICAL_SESSION_ENV_KEYS` cleared-list to the daemon, whose blocklist then
suppresses re-hydrating `WAYLAND_DISPLAY` from its own `/proc`/systemd probing.
Clearing `WAYLAND_DISPLAY` is therefore a two-part contract: `env_remove` on the
spawn command and telling the daemon it was deliberately cleared.

When isolated mode is requested but the desktop cannot be established (a required
dependency is missing, or the display cannot be brought up), the client fails
closed with a clear, dependency-naming error rather than silently falling back to
the user's live desktop. Config-resolution errors (problems reading the setting,
not establishing the desktop) degrade with a warning and continue
non-isolated. Dependency presence is probed up front so the error names exactly
which binary (`xpra`, `openbox`, `xdotool`) is missing.

The read-only viewer follows the `viewer` config. `attach` spawns
`xpra attach :N --readonly` using the client's own user-session environment, so
the viewer window renders on the user's real screen rather than inside the
sandbox; `html5` starts the xpra HTML5 listener and logs the URL; `none` launches
nothing. Viewer launch is warn-only and never blocks the session.

The xpra session is named and reused idempotently across agent sessions. With
`lifecycle = persistent` (the default) it survives client exit and is torn down
only by `sky-cua-client isolated-desktop stop`. With `lifecycle = ephemeral` the
client stops the session when it exits (run on both the normal and the error exit
from the MCP loop, since a host closing the pipe returns an error). Teardown stops
the xpra session, reaps the dedicated isolated daemon (it reads the daemon pid
from the socket's singleton lock and verifies the pid is a live `sky-cua-service`
process before signalling, so the user's real daemon on its own socket is never
touched), and removes the daemon socket and a stale `/tmp/.X<N>-lock` — all
filtered strictly by the known display number. A `display = "auto"` choice is
persisted to `$XDG_RUNTIME_DIR/sky-cua/isolated-display` so later sessions reuse
the same number.

## Source paths

- `crates/sky-cua-platform/src/config.rs` — `[isolated_desktop]` table, env
  override constants, `ViewerMode`/`Lifecycle`, `ResolvedIsolatedDesktop`, and
  `resolve_isolated_desktop_selection` / `resolve_isolated_desktop`.
- `crates/sky-cua-client/src/isolated_desktop.rs` — xpra lifecycle module:
  `IsolatedDesktopHandle` (`ensure`/`display`/`geometry`/`socket_path`/
  `spawn_env`/`removed_env`/`launch_viewer`/`stop`), the module `status`/`stop`
  helpers, dependency probing, and the geometry/`xpra list`/`xpra info` parsers.
- `crates/sky-cua-client/src/service_launcher.rs` — the spine: resolves the
  selection, ensures the handle, redirects the socket, applies the sandbox spawn
  env, launches the viewer, and honors the ephemeral lifecycle on shutdown.
- `crates/sky-cua-client/src/launch_environment.rs` —
  `LaunchEnvironment::for_isolated_daemon` and the cleared-list emission.
- `crates/sky-cua-client/src/main.rs` — the hidden `isolated-desktop` subcommand.
- `crates/sky-cua-client/src/mcp_tools.rs`,
  `crates/sky-cua-client/src/mcp_tools/definitions.rs` — the `desktop_launch_app`
  tool, its definition, and the `IsolatedDesktopRequired` gate.
- `crates/sky-cua-platform/src/model/service.rs` — `ServiceRequest::LaunchApplication`
  and `ServiceResponse::LaunchApplication`.
- `crates/sky-cua-platform/src/backend.rs` — the `DesktopBackend::launch_application`
  trait method (default unsupported).
- `crates/sky-cua-linux/src/backend.rs` — the Linux `launch_application`
  implementation (pure env-inherit plus `setsid` detach).
- `crates/sky-cua-service/src/daemon.rs` — daemon dispatch for `LaunchApplication`.
- `crates/sky-cua-linux/src/env_probe.rs` — the `infer_session_kind` X11
  early-return the env recipe relies on.
- `.mcp.json` — the six `SKY_CUA_ISOLATED_DESKTOP*` env-var allowlist entries.

## Verification

- Config unit tests in `crates/sky-cua-platform/src/config.rs` cover the
  `[isolated_desktop]` precedence (env beats file beats default) and the
  viewer/lifecycle parsing and defaults.
- xpra lifecycle unit tests in `crates/sky-cua-client/src/isolated_desktop.rs`
  cover the free-display scan, `xpra list` live-display parsing, `xpra info`
  D-Bus-address parsing, `xdpyinfo` geometry parsing, resolution parsing, the
  spawn-env/removed-env recipe, the isolated socket-path convention, and
  dependency-presence reporting.
- `LaunchEnvironment::for_isolated_daemon` is covered by
  `for_isolated_daemon_scopes_health_to_sandbox_graphical_identity` in
  `crates/sky-cua-client/src/launch_environment.rs`.
- `crates/sky-cua-linux/tests/isolated_x11_probe.rs` — gated env-recipe
  regression test: spins a throwaway xpra (or Xvfb) display, applies the env
  recipe, drives the backend probe, and asserts the X11/XTest lane. Joins the
  `serial-integration` nextest group. Run with
  `cargo nextest run -p sky-cua-linux isolated_x11`; skips cleanly when no
  headless X provider is installed. (Note: the workspace `default-members`
  excludes `sky-cua-linux`, so a bare `cargo nextest run` does not exercise this
  guard; target `-p sky-cua-linux` or `--workspace` explicitly.)
- `crates/sky-cua-linux/tests/isolated_app_launch_leak.rs` — gated host-leak
  guard: launches a Qt app (`kcalc`, `xmessage` fallback) into a throwaway
  sandbox that replicates the *full* daemon spawn contract — a private xpra
  display, a private throwaway `dbus-daemon` session bus, and the
  `CLIENT_CLEARED_SESSION_ENV_KEYS` signal that suppresses session-env
  re-hydration — then asserts the app is present on `:N`, absent from the host,
  and carries the sandbox markers in `/proc/<pid>/environ` (`DISPLAY=:N`, no
  `WAYLAND_DISPLAY`, `QT_QPA_PLATFORM=xcb`, the sandbox `DBUS_SESSION_BUS_ADDRESS`).
  Both the private bus and the hydration-suppression signal are load-bearing:
  omitting either lets a KDE single-instance app re-acquire the host session and
  escape onto the user's desktop. Passes live against real xpra + kcalc (~31s);
  same `default-members` caveat as `isolated_x11`.
- VM smoke: an `isolated-xpra` profile for `scripts/run_gui_testing_vm_smoke.py`
  (modeled on `i3.sh`, wired into the registry, `run-profile.sh`, and the `all`
  set) that brings the private desktop up, launches an app and captures it, and
  asserts no host leak, exiting `67` when xpra is unavailable. Live-run
  2026-07-07 on the Arch testing-vm (COSMIC, `wayland-1`): green after
  installing `xpra`/`xdotool`/`xorg-xdpyinfo` into the guest (the provisioner
  does not yet install them — a follow-up for `scripts/testing-vm/`
  provisioning); confirmed sandbox-up, app-present-on-sandbox,
  app-absent-from-host, and clean-stop assertions all passed.

## Known limitations

- Computer-use only. Browser-use is not routed into the private desktop; the
  agent launches a browser as an application instead.
- The visible agent-cursor overlay does not render in the X11 private desktop
  (the renderer is Wayland layer-shell only). The human watches the real X cursor
  move through the read-only viewer.
- Not a security sandbox: same OS user, filesystem, network, and system D-Bus
  access. The boundary is non-interference, not containment.
- Requires `xpra` (6.x), `openbox`, and `xdotool` on the host, plus GStreamer with
  the `ximagesrc` and `pngenc` elements for capture. Missing dependencies fail
  closed with a structured, naming error.
- Host live-proof is complete (leak guard + headline `desktop_launch_app`
  end-to-end, KDE Wayland, 2026-06-30, re-proven 2026-07-07). The
  `isolated-xpra` VM smoke profile is also live-proven (2026-07-07, Arch
  testing-vm, COSMIC guest). The remaining gap is operational, not a proof
  gap: the testing-vm provisioner does not yet install `xpra`/`xdotool`/
  `xorg-xdpyinfo`, so a fresh guest needs a manual package install before the
  profile passes — tracked as a `scripts/testing-vm/` provisioning follow-up.
- Ephemeral teardown reaps the xpra session and the isolated daemon on the
  normal and pipe-error MCP exits, but not on `SIGKILL`/panic; a hard-killed
  ephemeral session leaves the xpra server, recoverable via
  `sky-cua-client isolated-desktop stop`.

## Related

- [`docs/research/2026-06-isolated-xpra-desktop-x11-lane.md`](../research/2026-06-isolated-xpra-desktop-x11-lane.md)
- [`docs/research/2026-06-isolated-xpra-desktop-toolkit-escape.md`](../research/2026-06-isolated-xpra-desktop-toolkit-escape.md)
- [`docs/features/browser-mcp-tools.md`](browser-mcp-tools.md)
- [`docs/features/agent-cursor-overlay.md`](agent-cursor-overlay.md)
- [`ROADMAP.md`](../../ROADMAP.md)
