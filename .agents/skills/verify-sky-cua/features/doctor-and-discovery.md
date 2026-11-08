# Doctor and discovery

Doctor and discovery tell the agent whether desktop control is available and
what it can target. That covers the advertised tool surface, a structured
readiness report, and the apps, windows, and focused window on the desktop.

## Sub-features

- `disc-tools` advertises tools in `tools/list` according to the surface policy
  (`SKY_CUA_SURFACES=desktop` hides browser and phone tools).
- `disc-doctor` returns a structured readiness report (session kind, display,
  topology, capture, input, and semantic backends).
- `disc-windows` lists desktop windows with `window_id`, backend, `app_id`, pid,
  and focus (`list_resources {surface: desktop, resource: windows}`).
- `disc-apps` lists accessible apps (`resource: apps`). Not yet scripted.
- `disc-focused` returns the focused window (`resource: focused_window`). Not yet
  scripted.
- `disc-operator` provides operator CLI equivalents: `health`, `doctor`,
  `list-apps`, `list-windows`, `focused-window`, and `get-app-state`.

## How to get to it (user POV)

- MCP `tools/list`, `doctor {}`, and
  `list_resources {surface: "desktop", resource: "apps"|"windows"|"focused_window"}`.
- Terminal: `sky-cua-client <health|doctor|list-apps|list-windows|focused-window>`.

## Driving it with verify.sh

Preconditions:

- Baseline from `README.md`. `verify.sh doctor` prints `verdict=drivable`.

- **Tool surface.** Run `verify.sh drive --journey smoke`. `tools-list.json` is
  written, and the check `tools/list advertises the desktop tools` lists
  `desktop_launch_app`, `observe`, `desktop_keyboard`, `desktop_set_value`,
  `desktop_action`, and `doctor`. The check
  `browser and phone tools are absent (SKY_CUA_SURFACES=desktop)` passes.
- **Doctor.** Same run. `doctor {}` returns no `isError`, and
  `session_kind == "x11"` (checks `doctor returned without isError` and
  `doctor reports session_kind x11`).
- **Windows.** During the semantic or keyboard journey,
  `list_resources {surface: "desktop", resource: "windows"}` lists the marker
  window with `backend=x11`, `app_id=zenity.desktop`, and `[focused]`.
- **Operator CLI.** Run `verify.sh cli list-windows`. Exit code is 0 and the JSON
  reports `environment.display == ":147"`, `session_kind == "x11"`,
  `capture_backend == "x11"`, `input_backend == "x_test"`, and one display
  `DUMMY0` at 1280x800. Run `verify.sh cli doctor` and `verify.sh cli list-apps`
  the same way and save stdout into the run dir when citing them.
- **Proof.** `tools-list.json` and the `doctor` entry in `steps.jsonl`.

## Gotchas

- `list_resources` desktop resources reject `title_contains` with
  `InvalidRequest`; it is browser-only.
- `compositor` reports the host's `kde-kwin-wayland` even inside the private X11
  desktop. Use `session_kind`, `display`, and `input_backend` to identify the
  lane.
- `verify.sh cli` refuses to run unless the isolated daemon is up, so it cannot
  spawn or reach the user's daemon by accident. Never run the operator CLI
  without the isolated environment.
