---
name: computer-use
description: "Use when operating desktop apps through sky-cua computer-use tools: discovery, windows, accessibility trees, screenshots, semantic actions, physical input, session presence, and verification on supported Linux and Windows backends."
---

# Computer Use

For web-page content, use `browser-use` when `browser_*` tools are available.
Browser CSS pixels and desktop screenshot pixels are unrelated; never reuse
coordinates across them.

## Coordinates

- `get_app_state` and `screenshot` return `snapshot_id`. With a captured
  snapshot that includes coordinate-mapping metadata, x/y plus `snapshot_id`
  are pixels in that snapshot. Structure-only snapshot ids still scope
  `element_index` lookups but cannot translate screenshot pixels. Without
  `snapshot_id`, x/y are live screen coordinates.
- `screenshot` defaults to the primary display. Window targets (`window_id`,
  `pid`, `app_id`, `wm_class`, `title`, terminal selectors) activate,
  focus-verify, and crop. `display_*` captures one monitor;
  `capture_all_displays` captures the virtual desktop.
- Always pass the source `snapshot_id` for screenshot-based actions, especially
  with cropped windows, non-primary displays, scaling, or negative origins.
- `environment.displays` lists display ids, primary status, logical rects,
  scale, and backend; windows/focused apps include `display` when known.

## State

- Use `doctor` before the first action when desktop access, capture, input, or
  session presence may be unavailable. For remote lockable sessions, check
  `doctor.session_presence`: use `unlock_session` when unlock is supported,
  otherwise `hold_session` when inhibition is supported. Presence is opt-in via
  `SKY_CUA_PRESENCE_ENABLED`; unsupported errors mean "not armed".
- Use `list_windows` for exact `window_id`, focus, bounds, display, and
  terminal metadata. Use `focused_window` only when current focus is the target.
- `get_app_state` is structured state: diagnostics, element anchors, text/value
  readback, optional focused-app screenshot. Default compact is usually enough;
  use full only when you need verbose element details or full capability data.
  Use `element_query`/`element_offset`/`element_limit` on dense trees. Use
  `capture_screen: "never"` for structure-only passes and `"always"` for a
  fresh focused-app image.
- `screenshot` is visual state. Use it instead of `get_app_state` for a
  specific window/display image or pixel target. Use
  `screenshot_delivery: "inline"` only when local file paths are unreadable.
- The accessibility tree is structure, not truth. Fallback trees have real
  window bounds but blunt roles; treat them as visual anchors. When tree and
  screenshot disagree, the screenshot wins.
- Tool success means input was injected, not that UI changed. Reacquire state
  and screenshot before chaining through menus, popovers, renames, or fallback
  surfaces.
- `SessionEnvRepaired` is context, not error. On Linux, check
  `doctor.session_env` before judging a thin app list or missing capture/input
  as desktop unavailable.

## Actions

- Prefer semantic primitives when `semantic_actions` support them:
  `focus_element`, `activate_element`, `select_element`, `expand_element`,
  `collapse_element`, `toggle_element`, or named/indexed `perform_action`.
- Use physical click/drag/scroll for sliders, canvases, splitters,
  drag-and-drop, custom-painted widgets, and anything visible but unclear in
  the tree.
- Use `set_value` only with a proven semantic write path and readback.
  Otherwise click to focus, select all (`Cmd + A / Ctrl + A`) when replacing,
  type, then verify with a fresh snapshot.
- `activate_window` targets by `window_id`, `pid`, `app_id`, `wm_class`,
  `title`, or terminal selectors (`tty`, `terminal_pid`, ...). `workspace`
  metadata is backend-native, not portable.
- Do not pre-call `activate_window` before targeted `screenshot`; screenshot
  already activates and focus-verifies. Use `activate_window` only when you
  need focus without a fresh image.

## Linux notes

- `activate_window` success is focus-verified on Linux, including KDE/KWin;
  `focused_window` works on KWin too. Errors name the missing backend seam.
- XWayland editors may need keyboard input via the X11 lane rather than the
  portal keyboard lane.
- Native Wayland apps can expose good structure yet report wrong actionable
  bounds; fallback-only Wayland windows need fresh screenshots after
  context-menu, submenu, or inline-rename steps.
- If semantic click wedges and the visible target is clear, click coordinates.

## App guidance

For app-specific behavior, check the snapshot's `app_guidance` field or
`references/apps/*.md` (index in `references/apps/index.json`).
