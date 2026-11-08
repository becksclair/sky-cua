# Doctor and discovery

Doctor and discovery tell the agent whether desktop control is available and
what it can target. That covers the advertised tool surface, a structured
readiness report, and the apps, windows, and focused window on the desktop.

## Sub-features

- `disc-tools` advertises tools in `tools/list` according to the surface policy
  (`SKY_CUA_SURFACES=desktop` hides browser and phone tools and
  `capture_screen`, leaving 16 tools). Five advertised tools have no feature
  file or journey in this map: `status`, `setup_desktop`, `session_presence`,
  `activate_window`, and `desktop_scroll`
  (`crates/sky-cua-client/src/mcp_tools/definitions/mod.rs`). Report them as
  not verified.
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
- Terminal: `sky-cua-client <health|doctor|list-apps|list-windows|focused-window>`
  (no arguments), and `sky-cua-client get-app-state` with optional selector
  flags `--app-id`, `--desktop-file-id`, `--window-title`, and `--name`, plus
  `--detail` and `--capture-screen` (each flag takes a value).

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
  window with `backend=x11`, `app_id=zenity.desktop`, and `[focused]` in the
  text content (`focused: true` in the structured result). The driver only
  checks that the marker is listed; read these fields from `steps.jsonl`.
- **Operator CLI.** Run `verify.sh cli list-windows`. Exit code is 0 and the JSON
  reports `environment.display == ":147"`, `session_kind == "x11"`,
  `capture_backend == "x11"`, `input_backend == "x_test"`, and one display
  `x11:DUMMY0` at 1280x800. Run `verify.sh cli doctor`, `list-apps`, and
  `focused-window` the same way. `verify.sh cli` already saves each stdout as
  `cli-<time>-<command>.out` in the run dir.
- **Proof.** `tools-list.json` and the `doctor` entry in `steps.jsonl`.

## Gotchas

- `list_resources` desktop resources reject `title_contains` with
  `InvalidRequest`; it is browser-only. The rejection is a tool result with
  `isError: true` and `structuredContent.error.code`, not a JSON-RPC error. No
  journey exercises it.
- On a remote host `verify.sh cli` forwards its arguments as one unquoted
  string, so a window title containing spaces is split. Journey markers have no
  spaces.
- `compositor` reports the host's `kde-kwin-wayland` even inside the private X11
  desktop. Use `session_kind`, `display`, and `input_backend` to identify the
  lane.
- `verify.sh cli` refuses to run unless the isolated daemon is up, so it cannot
  spawn or reach the user's daemon by accident. Never run the operator CLI
  without the isolated environment.
