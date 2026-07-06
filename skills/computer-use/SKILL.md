---
name: computer-use
description: "Use when operating desktop apps through sky-cua computer-use tools: discovery, windows, accessibility trees, screenshots, semantic actions, physical input, session presence, and verification on supported Linux and Windows backends."
---

# Computer Use

For web-page content, use `browser-use` when browser tools are available.
Browser CSS pixels and desktop screenshot pixels are unrelated; never reuse
coordinates between them.

## Tool Surface

- Use only sky-cua desktop tools advertised by `tools/list`: discovery,
  observation, doctor/status, desktop capture, setup/session presence, window
  activation, semantic actions, pointer, keyboard, scroll, toggle, and value
  setting.
- `desktop_pointer` is the click/move/drag tool, and `desktop_keyboard` is the
  type/key tool. There is no separate input grant to unlock. Do not call
  `request_access`, and do not substitute another built-in computer-use server.
- If desktop action tools are missing from `tools/list`, the sky-cua MCP
  connection is stale. Reconnect or restart it instead of falling back to a
  different server.
- Grouped response payloads live under `structuredContent.result`; validation
  failures live under `structuredContent.error`. Delegated branch failures keep
  their branch payload under `structuredContent.result` and set `isError=true`.

## Coordinates and Capture

- `observe(surface="desktop")` and `capture_desktop` return `snapshot_id`.
  With capture metadata, x/y plus that `snapshot_id` are pixels in that
  snapshot. Structure-only snapshot ids still scope element lookups but cannot
  translate screenshot pixels. Without `snapshot_id`, x/y are live screen
  coordinates.
- `capture_desktop` captures exactly one screen or one window crop. With no
  selector it captures the main display only. There is no whole-desktop,
  all-display, or virtual-desktop fallback.
- For a target app, identify the window first and capture it by window selector,
  or capture its specific display. Do not assume the app is on the primary
  monitor; windows expose display metadata when known.
- Always pass the `snapshot_id` from the exact observation or capture that
  produced the image used for pixel action, especially with cropped windows,
  non-primary displays, scaling, or negative origins.
- Targeted crops self-heal missing KDE/KWin portal stream position from display
  topology when possible. Persistent `CaptureSourceGeometryMissing` or
  "targeted screenshot requires capture source geometry" means topology could
  not place the frame: refresh window/display state with observe or doctor and
  retry that same targeted capture once. Widening scope cannot escape the error.
- Pixel coordinates expire after visible transitions such as menus, popovers,
  renames, submenus, resizes, or display changes. Re-observe or re-capture
  before continuing.

## State

- Use session-presence status for lock/hold/unlock state; use desktop resources
  for apps/windows/focus; use desktop observe for structure; use
  `capture_desktop` for visual state and pixel targets.
- Run `doctor` before the first action when desktop access, capture, input, or
  session presence may be unavailable. Presence is opt-in via
  `SKY_CUA_PRESENCE_ENABLED`; unsupported errors mean it is not armed.
- Use window resources for exact `window_id`, focus, bounds, display, and
  terminal metadata. Use focused-window state only when current focus is the
  intended target.
- `observe(surface="desktop")` is structured state: diagnostics, element
  anchors, text/value readback, and optional focused-app screenshot. Compact
  detail is usually enough; use element query/offset/limit on dense trees.
- When an observation includes capture metadata, inspect
  `capture.inspection_image_path` first. Other paths are source/debug artifacts.
- The accessibility tree is structure, not truth. Fallback trees have real
  window bounds but blunt roles; treat them as visual anchors. When tree and
  screenshot disagree, the screenshot wins.
- `SessionEnvRepaired` is context, not error. On Linux, check
  `doctor.session_env` before judging a thin app list or missing capture/input
  as desktop unavailable.
- Check `doctor.display_topology` before judging display-targeted screenshots:
  `display_count=0` or `DisplayTopologyUnavailable` means targeted display
  geometry is not authoritative yet; fallback or inferred topology makes window
  targets plus returned snapshot ids safer than raw display clicks.

## Actions

- Keep operation names, selectors, coordinates, text, keys, and `snapshot_id` as
  top-level tool fields. `snapshot_id` is only the opaque id string; never pack
  JSON or action fields into it.
- Observe first, then pass a concrete target from that observation. Semantic
  actions, toggles, scrolls, and value setting should reference the current
  snapshot/element they came from.
- Prefer semantic primitives when `semantic_actions` advertise them:
  focus/select/expand/collapse, toggles, activate/custom actions, and numeric
  value setting. These affordances track current element state. If a semantic op
  is not advertised and returns `ActionRequiresPhysicalInput`, use pointer input.
- Use physical click/drag/scroll for sliders without a value interface,
  canvases, splitters, drag-and-drop, custom-painted widgets, and anything
  visible but unclear in the tree.
- A drag only grabs when its start coordinate lands on the draggable handle
  itself: slider thumb, scrollbar grip, or drag source, not the surrounding
  track. Use `duration_ms` around 400-800 for sliders and drag-and-drop so
  paced motion tracks reliably, then read back the result.
- For text fields, use semantic value setting only with a proven semantic write
  path and readback. Otherwise click to focus, select all with `Cmd+A` or
  `Ctrl+A` as appropriate, type, then verify with a fresh snapshot.
- Do not pre-call activate-window before targeted desktop capture. Targeted
  capture already activates and focus-verifies where the backend can prove it.
  Use activate-window only when you need focus without a fresh image.

## Linux Notes

- `activate_window` success is focus-verified on Linux, including KDE/KWin;
  focused-window discovery works on KWin too. Errors name the missing backend
  seam.
- On KDE/KWin Wayland, prefer `window_id` over `pid` when both are available.
  `window_id` identifies the exact window; `pid` can be ambiguous for
  multi-window apps and compositor-managed surfaces.
- XWayland editors may need keyboard input via the X11 lane rather than the
  portal keyboard lane.
- Native Wayland apps can expose good structure yet report wrong actionable
  bounds. Fallback-only Wayland windows need fresh screenshots after context
  menus, submenus, or inline rename steps.
- If semantic click wedges and the visible target is clear, click coordinates.

## App Guidance

For app-specific behavior, check the snapshot's `app_guidance` field or
`references/apps/*.md` (index in `references/apps/index.json`).
