# Isolated desktop and app launch

The isolated desktop gives the agent a private, headless X11 desktop (xpra plus
Openbox) with its own sky-cua daemon, D-Bus session bus, and AT-SPI registry.
The agent launches applications into it with `desktop_launch_app` and drives
them without touching the user's real screen, session bus, or running daemon.

## Sub-features

- `iso-ensure` brings the private desktop up idempotently and reports display,
  geometry, socket, and viewer.
- `iso-status` reports up/down, geometry, selection, and dependency presence
  without starting anything.
- `iso-daemon` runs a second daemon on `service-isolated-<N>.sock` beside the
  user's daemon.
- `iso-launch` starts an application inside the private desktop and returns its
  pid.
- `iso-contain` keeps launched apps on the private display and bus, with nothing
  on host displays.
- `iso-gate` refuses `desktop_launch_app` with `IsolatedDesktopRequired` when the
  client is not isolated.
- `iso-stop` tears down xpra, the buses, the AT-SPI owners, and the isolated
  daemon, leaving no residue.

## How to get to it (user POV)

- Set `SKY_CUA_ISOLATED_DESKTOP=1` (or `[isolated_desktop] enabled = true`) and
  start `sky-cua-client mcp`. The client brings the desktop up on connect. The
  default lifecycle is persistent: the desktop outlives the client until
  `isolated-desktop stop`. Only `SKY_CUA_ISOLATED_DESKTOP_LIFECYCLE=ephemeral`
  tears it down when the client exits. `verify.sh` pins `persistent`.
- Run `sky-cua-client isolated-desktop ensure|status|stop` (hidden development
  subcommand, dispatched before the operator CLI; `verify.sh cli` reaches it).
  `ensure` never starts a viewer.
- Call the MCP tool `desktop_launch_app {command, args}`.
- The viewer defaults to `attach`: once the isolated daemon is healthy,
  `sky-cua-client mcp` opens `xpra attach :N --readonly` as a window on the
  user's real screen (`html5` and `none` are the alternatives). `verify.sh`
  pins `SKY_CUA_ISOLATED_DESKTOP_VIEWER=none` so no window appears there;
  `status.viewer` prints it as `None`.

## Driving it with verify.sh

Preconditions:

- `verify.sh launch` printed `ready: display=:147 ...` and `verify.sh doctor`
  printed `verdict=drivable`.

- **Ensure and status.** Bring the desktop up and inspect it. `verify.sh launch`
  writes `launch-ensure.txt` with `display=:147`, `geometry=1280x800`, and
  `socket=.../service-isolated-147.sock`. `verify.sh doctor` shows
  `status.up=true`, `status.geometry=1280x800`, and all `status.dep_*=true`.
- **Second daemon.** Confirm the isolated daemon is ours and the user's is
  untouched. `verify.sh doctor` shows `isolated_daemon_pid=<pid>` with `exe`
  under `~/.cache/sky-cua-verify/target*/release/`, plus
  `user_daemons_unchanged=true`.
- **Launch an app.** Run `verify.sh drive --journey semantic`. Its first call is
  `desktop_launch_app {"command": "sh", "args": ["-c", "zenity --entry --title='<marker>' ... > <run>/semantic-zenity-output.txt; echo $? > <run>/semantic-zenity-exit.txt"]}`.
  `summary.json` shows `desktop_launch_app returned a pid` and
  `dialog window listed on the private desktop` (from
  `list_resources {surface: desktop, resource: windows}`).
- **Containment.** The same run checks zenity's `/proc/<pid>/environ`. The
  checks `app DISPLAY is the private display` (`:147`),
  `app has no WAYLAND_DISPLAY`, and `app session bus is private, not the user's`
  must pass, along with `window absent from every queryable host X display`.
- **Stop.** Run `verify.sh teardown`. It prints `display=:147`, `stopped=true`,
  and `teardown: residue=none`. `teardown.txt` lists the user's daemon with the
  same pid as `foreign-daemons-before.txt`.
- **Proof.** `images/*-observe-*.png` show the dialog on a black 1280x800
  private screen with the marker in its title.
- **Gate (not scripted).** `iso-gate` needs a non-isolated client, which would
  be the user's live lane. It is covered by unit tests only. Do not drive it
  here.

## Gotchas

- `desktop_launch_app` runs any command with the daemon's sandbox environment.
  `sh -c` is the supported way to capture app output for side-effect proof.
- `desktop_launch_app` succeeds for shell launches (pid included) but carries a
  `DestinationAppShotUnavailable` diagnostic. Poll the windows list for the
  title marker before observing.
- KDE single-instance apps (kcalc, kwrite) need the full sandbox. The lane
  provides it, but a stray host instance of the same app can still be reused
  through D-Bus activation. Prefer zenity for journeys.
- `xpra` writes its cookie into `~/.Xauthority`. That is xpra behavior, not
  residue to clean.
- A hard-killed `drive` or `launch` can leave xpra up. `verify.sh teardown` is
  idempotent and recovers it.
