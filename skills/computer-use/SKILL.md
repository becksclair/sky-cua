---
name: computer-use
description: "Use when operating desktop applications through sky-cua computer-use tools: app/window discovery, accessibility trees, screenshots, semantic element actions, clicks, typing, dragging, scrolling, and verification across Linux, macOS, and Windows."
---

# Computer Use

If the task is inside a web page and `browser_*` tools are available, use the
`browser-use` skill instead. Desktop and browser coordinate spaces are
unrelated; never carry coordinates across them.

## Coordinates

- `click`/`drag`/`perform_secondary_action` x/y are pixels of the screenshot
  from the `get_app_state` or `screenshot` snapshot you pass as `snapshot_id`;
  the runtime maps them back to the desktop. Without `snapshot_id`, x/y are
  live screen coordinates for the active input backend.
- Desktops may have multiple displays. `environment.displays` lists display
  ids, primary status, logical rects, scale, and backend; windows and focused
  apps include their chosen `display` when the backend can assign one.
- For visual work in a known window, prefer `screenshot` with `window_id` or
  another window target field. It activates and focus-verifies the window, then
  returns a cropped, unoccluded screenshot. Coordinates for follow-up actions
  are pixels in that cropped screenshot.
- With no selector, `screenshot` captures the primary display only. For a
  monitor-specific view, pass `display_id`/`display_name`/`display_index` from
  `environment.displays`. Use `capture_all_displays: true` only when the whole
  virtual desktop is necessary.
- For actions based on any screenshot, always pass that snapshot's
  `snapshot_id`; this lets the backend translate screenshot pixels across
  cropped windows, non-primary displays, and negative-origin monitor layouts.

## State

- For remotely launched desktop threads, check `doctor.session_presence` before
  the first UI action when the session may be locked or lockable. If doctor
  reports unlock capability, call `unlock_session` first; if it reports
  inhibition only, call `hold_session` first. The whole feature is opt-in
  through `SKY_CUA_PRESENCE_ENABLED` on the daemon: when it is off, explicit
  `hold_session`/`unlock_session`/`release_session` calls are rejected with
  `ActionUnsupportedForEnvironment`, so treat that error as "not armed", not
  as a failure to retry.
- Use `list_windows` to get exact `window_id` values, focus state, and native
  window bounds plus each window's display assignment. Use `focused_window`
  only when you already intend to work in the current focused window.
- `get_app_state` is the structured state source: diagnostics, element
  anchors, text readback, and optional `screenshot_path`. Use full `detail`
  once for orientation, then `detail: "compact"` for action loops.
  `capture_screen: "never"` for structure-only passes; `"always"` when you
  need a full focused-app screenshot alongside the tree.
- `screenshot` is the visual state source. Use it instead of `get_app_state`
  when the primary need is to inspect or click within a specific window. Pass
  the same target fields accepted by `activate_window` (`window_id`, `pid`,
  `app_id`, `wm_class`, `title`, or terminal selectors). If you cannot read
  local files by path, pass `screenshot_delivery: "inline"`.
- The accessibility tree is structure, not truth. Fallback-only trees have
  real window bounds but blunt roles — treat their regions as visual anchors,
  not widgets. When tree and screenshot disagree, the screenshot wins.
- Action-tool success means the input was injected, not that the UI changed.
  State can lag the tree on fallback surfaces and transient UI (menus,
  popovers, renames): reacquire state — including a fresh screenshot — before
  chaining actions through them.
- `SessionEnvRepaired` in diagnostics means the runtime recovered missing
  desktop env; it is context, not an error. On Linux, before concluding the
  desktop is unavailable (thin app list, missing capture/input), check
  `doctor.session_env`.

## Actions

- If the element advertises a matching `semantic_actions` entry, use the
  narrow primitive (`activate_element`, `select_element`, `expand_element`,
  `collapse_element`, `toggle_element`, `focus_element`) or `perform_action`
  for a named/indexed AT-SPI action. Otherwise use coordinate `click`.
- Sliders, canvases, splitters, drag-and-drop, custom-painted widgets, and
  anything "obvious on screen, murky in the tree" want physical actions, not
  semantic guesses.
- `set_value` only where the element exposes a proven semantic write path
  with `value`/`text.content` readback. Otherwise: click to focus, select all
  (`Cmd + A / Ctrl + A`) when replacing, type, then verify via readback in a
  fresh snapshot — fields may hold stale text, and writes can silently miss.
- `activate_window` targets by `window_id`, `pid`, `app_id`, `wm_class`,
  `title`, or terminal selectors (`tty`, `terminal_pid`, ...). `workspace`
  metadata is backend-native, not portable.
- Do not call `activate_window` before a targeted `screenshot`; the screenshot
  tool does the activation and focus verification itself. Use `activate_window`
  directly only when you need focus without a fresh image.

## Linux notes

- `activate_window` success is focus-verified on all Linux backends,
  including KDE/KWin; trust it without a confirming screenshot.
  `focused_window` works on KWin too. If either fails, read the error —
  it names the missing backend seam.
- XWayland editors may need keyboard input via the X11 lane rather than the
  portal keyboard lane.
- Native Wayland apps can expose good structure yet report wrong actionable
  bounds; fallback-only Wayland windows need a fresh screenshot after every
  context-menu, submenu, or inline-rename step.
- If the visible button is clear and the semantic click wedges, click the
  button by coordinates.

## App guidance

For app-specific behavior, check the snapshot's `app_guidance` field or
`references/apps/*.md` (index in `references/apps/index.json`).
