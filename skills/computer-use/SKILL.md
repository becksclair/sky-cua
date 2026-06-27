---
name: computer-use
description: "Use when operating desktop apps through sky-cua computer-use tools: discovery, windows, accessibility trees, screenshots, semantic actions, physical input, session presence, and verification on supported Linux and Windows backends."
---

# Computer Use

For web-page content, use `browser-use` when `browser_*` tools are available.
Browser CSS pixels and desktop screenshot pixels are unrelated; never reuse
coordinates across them.

## Coordinates

- `observe(surface="desktop")` and `capture_desktop` return `snapshot_id`.
  With a captured snapshot that includes coordinate-mapping metadata, x/y plus
  `snapshot_id` are pixels in that snapshot. Structure-only snapshot ids still
  scope `element_index` lookups but cannot translate screenshot pixels. Without
  `snapshot_id`, x/y are live screen coordinates.
- `capture_desktop` captures exactly one screen, never the whole multi-monitor
  desktop, and defaults to the main display. Call it with no selector for the
  normal case. A window selector (`window_id`, `pid`, `app_id`, `wm_class`,
  `title`, terminal selectors) activates, focus-verifies, and crops to that
  window. A display selector (`display_id`, `display_name`, `display_index`)
  captures one specific monitor and is only needed when the target is on a
  non-main display. Every call resolves to one screen; there is no all-display
  or virtual-desktop capture.
- Always pass the `snapshot_id` returned by the specific observation or capture
  call that produced the image for screenshot-based actions,
  especially with cropped windows, non-primary displays, scaling, or negative
  origins. If a targeted `capture_desktop` fails with
  `CaptureSourceGeometryMissing` or `targeted screenshot requires capture
  source geometry`, the issue is missing portal stream geometry, not the
  capture scope: refresh the relevant window/display state (re-observe or
  `doctor`) and retry the same targeted capture once. There is no broader
  whole-desktop capture to fall back to; widening the scope does not escape
  this error.
- `environment.displays` lists display ids, primary status, logical rects,
  scale, and backend; windows/focused apps include `display` when known.

## State

- Use only tool names advertised by `tools/list`. Grouped response payloads
  are under `structuredContent.result`; validation failures are under
  `structuredContent.error`. Delegated branch failures keep the branch payload
  under `structuredContent.result` and set `isError=true`.
- Use `status(component="session_presence")` for session-presence status,
  `list_resources(surface="desktop", resource=...)` for apps/windows/focused
  window, `observe(surface="desktop")` for desktop state, and
  `capture_desktop` for desktop screenshots.
- Use `doctor` before the first action when desktop access, capture, input, or
  session presence may be unavailable. For remote lockable sessions, check
  `structuredContent.result.session_presence`. Use
  `session_presence(operation="unlock"|"hold")` when supported.
  Presence is opt-in via `SKY_CUA_PRESENCE_ENABLED`; unsupported errors mean
  "not armed".
- Use `list_resources(surface="desktop", resource="windows")` for exact
  `window_id`, focus, bounds, display, and terminal metadata. Use the
  focused-window resource only when current focus is the target.
- `observe(surface="desktop")` is structured state: diagnostics, element
  anchors, text/value readback, optional focused-app screenshot. Default
  compact detail is usually enough; use full only when you need verbose element
  details or full capability data. Use `element_query`/`element_offset`/
  `element_limit` on dense trees. Use `capture_screen: "never"` for
  structure-only passes and `"always"` for a fresh focused-app image.
- When an observation includes capture metadata, inspect
  `capture.inspection_image_path` first. Other capture paths are source/debug
  artifacts, not the recommended visual inspection image.
- `capture_desktop` is visual state. Use it instead of structured observation
  for a single-screen image or pixel target — main display by default, or one
  window/display when selected. Use `screenshot_delivery: "inline"` only when
  local file paths are unreadable.
- The accessibility tree is structure, not truth. Fallback trees have real
  window bounds but blunt roles; treat them as visual anchors. When tree and
  screenshot disagree, the screenshot wins.
- Pixel actions are scoped to the `snapshot_id` whose image supplied the
  target. Reacquire after visible transitions such as menus, popovers, renames,
  or submenus instead of chaining from stale captures.
- `SessionEnvRepaired` is context, not error. On Linux, check
  `doctor.session_env` before judging a thin app list or missing capture/input
  as desktop unavailable.
- Check `doctor.display_topology` before judging display-targeted screenshots:
  `display_count=0` or `DisplayTopologyUnavailable` means targeted display
  geometry is not authoritative yet; `selected_provider="xrandr"` or
  `DisplayTopologyInferred` means geometry came from fallback and window
  targets plus returned `snapshot_id` are safer for pixel actions.
- For a window-targeted screenshot, pass the exact `window_id` directly; the
  tool handles activation/focus verification where the backend can prove it.

## Actions

- Desktop actions are grouped as `setup_desktop`, `session_presence`,
  `activate_window`, `desktop_semantic`, `desktop_toggle`, `desktop_scroll`,
  `desktop_pointer`, `desktop_keyboard`, `desktop_action`, and
  `desktop_set_value`.
- For desktop actions, observe first, then pass a concrete target from that
  observation. `desktop_semantic` uses `operation` plus a semantic target.
  `desktop_action` uses `operation` plus a semantic target; `perform_action`
  also requires `action_name` or `action_index`. `desktop_toggle` takes a
  semantic target only. `desktop_scroll` takes `direction` plus a
  snapshot-bound target from the same observation. `desktop_set_value` takes
  `value` plus a semantic target. `desktop_pointer` takes `operation` plus
  coordinates or an allowed snapshot target. `desktop_keyboard` takes
  `operation` plus `text` or `key`; optional snapshot/window scope can activate
  the target window but does not select an editable element.
- Prefer semantic primitives when `semantic_actions` support them:
  `desktop_semantic` for focus/select/expand/collapse, `desktop_toggle` for
  toggles, and `desktop_action` for activate plus named/indexed custom actions.
- Use physical click/drag/scroll for sliders, canvases, splitters,
  drag-and-drop, custom-painted widgets, and anything visible but unclear in
  the tree.
- Use `desktop_set_value` only with a proven semantic write path and readback.
  Otherwise click to focus, select all with the literal key payload `Ctrl+A` on
  Linux/Windows or `Meta+A` on macOS when replacing, type, then verify with a
  fresh snapshot.
- `activate_window` targets by `window_id`, `pid`, `app_id`, `wm_class`,
  `title`, or terminal selectors (`tty`, `terminal_pid`, ...).
  `workspace` metadata is backend-native, not portable.
- Do not pre-call activate-window before targeted desktop capture; targeted
  capture already activates and focus-verifies. Use activate-window only when
  you need focus without a fresh image.

### Desktop argument shape

Every desktop action is one flat JSON object. `snapshot_id` is only the opaque
id string from the matching desktop observe/capture result; never pack
`operation`, coordinates, or selector fields inside it.

Valid state and capture calls:

```json
{
  "surface": "desktop",
  "capture_screen": "always"
}
```

```json
{
  "window_id": "win-..."
}
```

```json
{
  "display_id": "display-..."
}
```

Valid coordinate click:

```json
{
  "operation": "click",
  "snapshot_id": "desktop-...",
  "x": 640,
  "y": 420
}
```

Valid snapshot element click:

```json
{
  "operation": "click",
  "snapshot_id": "desktop-...",
  "element_index": 12
}
```

Valid semantic action:

```json
{
  "operation": "perform_action",
  "snapshot_id": "desktop-...",
  "element_index": 12,
  "action_name": "press"
}
```

Valid desktop scroll:

```json
{
  "direction": "down",
  "snapshot_id": "desktop-...",
  "element_index": 12,
  "pages": 1
}
```

Valid keyboard calls:

```json
{
  "operation": "type_text",
  "text": "hello"
}
```

```json
{
  "operation": "press_key",
  "key": "Enter"
}
```

`capture_desktop` captures one screen and chooses exactly one source: no
selector for the main display, one window selector, or one display selector.
Do not mix window selectors with display selectors.

## Linux notes

- `activate_window` success is focus-verified on Linux, including KDE/KWin;
  `focused_window` works on KWin too. Errors name the missing backend seam.
- On KDE/KWin Wayland, prefer `window_id` over `pid` when both are available.
  `window_id` identifies the exact window, while `pid` can be ambiguous for
  multi-window apps and compositor-managed surfaces.
- XWayland editors may need keyboard input via the X11 lane rather than the
  portal keyboard lane.
- Native Wayland apps can expose good structure yet report wrong actionable
  bounds; fallback-only Wayland windows need fresh screenshots after
  context-menu, submenu, or inline-rename steps.
- If semantic click wedges and the visible target is clear, click coordinates.

## App guidance

For app-specific behavior, check the snapshot's `app_guidance` field or
`references/apps/*.md` (index in `references/apps/index.json`).
