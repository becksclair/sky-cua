---
name: computer-use
description: "Use sky-cua desktop tools for native apps, windows, OS dialogs, and physical browser-chrome UI on Linux or Windows. Do not use for browser page content or tasks explicitly using browser_input, observe(surface=browser), browser screenshots/CSS coordinates, or phone/Android control."
---

# Computer Use

Use `browser-use` for browser page content and requests that explicitly target
sky-cua browser-surface tools, even when a browser shortcut affects chrome UI.
Browser CSS pixels and desktop screenshot pixels are unrelated; never reuse coordinates between them.

Load `references/platform-linux.md` only for Linux, KDE/KWin, XWayland, or native Wayland behavior.

## Mandatory plan fields

- **The only correct cropped-pixel action is** `desktop_pointer(operation="click", x=<crop-local x>, y=<crop-local y>, snapshot_id=<exact targeted-capture snapshot_id>)`. Copy that field contract into the plan. Never calculate, map, scale, or translate crop-local coordinates into desktop coordinates; authoritative geometry is enforced by the producing `snapshot_id`.
- For a requested state change, record the initial structured value, perform the action once, obtain a fresh structured readback, and require both the requested final value and `final_value != initial_value`. For Save, record the initial unsaved/saved indicator and require a fresh changed saved indicator. For Toggle, require the fresh structured final switch state to differ from the recorded initial state. Missing, ambiguous, or unchanged readback is failed/unverified; do not repeat the action speculatively.

## Tool Surface

- Use only sky-cua desktop tools advertised by `tools/list`: discovery, observation, doctor/status, desktop capture, setup/session presence, window activation, semantic actions, pointer, keyboard, scroll, toggle, and value setting.
- `desktop_pointer` is the click/secondary-click/drag tool, and `desktop_keyboard` is the type/key tool. The desktop surface has no move-only operation.
- There is no separate input grant to unlock.
- Never call `request_access` or substitute another built-in computer-use server.
- If desktop action tools are missing from `tools/list`, the sky-cua MCP connection is stale.
- Reconnect or restart a stale connection instead of falling back to a different server.
- Read successful grouped payloads under `structuredContent.result`.
- Read validation failures under `structuredContent.error`.
- Delegated branch failures keep their branch payload under `structuredContent.result` and set `isError=true`.

## Coordinates and Capture

- `observe(surface="desktop")` and `capture_desktop` return `snapshot_id`.
- With capture metadata, x/y plus that `snapshot_id` are pixels in that snapshot.
- Structure-only snapshot ids scope element lookups but cannot translate screenshot pixels.
- Without `snapshot_id`, x/y are live screen coordinates.
- `capture_desktop` captures exactly one screen or one window crop.
- With no selector, `capture_desktop` captures the main display only.
- There is no whole-desktop, all-display, or virtual-desktop fallback.
- Identify the target window first and capture it by window selector or its specific display.
- Do not assume the app is on the primary monitor; use returned window display metadata when known.
- Pass the `snapshot_id` from the exact observation or capture that produced the image used for pixel action, especially for cropped windows, non-primary displays, scaling, or negative origins.
- For a cropped-window pixel action, pass the crop-local x/y with that exact `snapshot_id`; never translate the crop coordinate to a guessed absolute desktop point.
- Targeted crops can self-heal missing portal stream position from display topology.
- If `CaptureSourceGeometryMissing` or "targeted screenshot requires capture source geometry" persists, refresh window/display state with observe or doctor and retry that same targeted capture once.
- Widening capture scope cannot escape a targeted-capture geometry error.
- Pixel coordinates expire after visible transitions such as menus, popovers, renames, submenus, resizes, or display changes.
- Re-observe or re-capture before continuing after a visible transition.

## State

- Use session-presence status for lock/hold/unlock state.
- Use desktop resources for apps, windows, and focus.
- Use desktop observe for structure.
- Use `capture_desktop` for visual state and pixel targets.
- Run `doctor` before the first action when desktop access, capture, input, or session presence may be unavailable.
- Presence is opt-in via `SKY_CUA_PRESENCE_ENABLED`; unsupported errors mean it is not armed.
- Use window resources for exact `window_id`, focus, bounds, display, and terminal metadata.
- Use focused-window state only when current focus is the intended target.
- `observe(surface="desktop")` returns diagnostics, element anchors, text/value readback, and an optional focused-app screenshot.
- Use element query/offset/limit on dense trees when compact detail is insufficient.
- When an observation includes capture metadata, inspect `capture.inspection_image_path` first.
- Other capture paths are source/debug artifacts.
- The accessibility tree is structure, not truth.
- Fallback trees have real window bounds but blunt roles; treat them as visual anchors.
- When tree and screenshot disagree, the screenshot wins.
- Check `doctor.display_topology` before judging display-targeted screenshots.
- `display_count=0` or `DisplayTopologyUnavailable` means targeted display geometry is not authoritative yet.
- Prefer window targets plus returned snapshot ids over raw display clicks when topology is fallback or inferred.

## Actions

- Keep operation names, selectors, coordinates, text, keys, and `snapshot_id` as top-level tool fields.
- Treat `snapshot_id` as only the opaque id string; never pack JSON or action fields into it.
- Observe first and pass a concrete target from that observation.
- Reference the current snapshot/element for semantic actions, toggles, scrolls, and value setting.
- Prefer primitives advertised by `semantic_actions`: focus/select/expand/collapse, toggles, activate/custom actions, and numeric value setting.
- When the required semantic control is advertised, use it before keyboard or pointer input; keyboard convenience is not a reason to bypass a proven semantic control.
- Semantic affordances track current element state.
- If a semantic op is not advertised and returns `ActionRequiresPhysicalInput`, use pointer input.
- Use physical click/drag/scroll for sliders without a value interface, canvases, splitters, drag-and-drop, custom-painted widgets, and anything visible but unclear in the tree.
- A drag starts only when its start coordinate lands on the draggable handle itself: slider thumb, scrollbar grip, or drag source, not the surrounding track.
- Use `duration_ms` around 400-800 for sliders and drag-and-drop so paced motion tracks reliably.
- Read back the result after a drag.
- Use semantic value setting for text fields only with a proven semantic write path and readback.
- Otherwise click to focus, select all with `Cmd+A` or `Ctrl+A` as appropriate, type, and verify with a fresh snapshot.
- In a native file chooser, use focus/select for a file entry and activate Open/Choose separately. If only activate is available and it commits/closes the chooser, treat that activation as the commit and proceed directly to fresh closure/page verification rather than attempting a second commit.
- Do not pre-call activate-window before targeted desktop capture.
- Targeted capture already activates and focus-verifies where the backend can prove it.
- Use activate-window only when focus is needed without a fresh image.

## Completion

- Finish with success only when a fresh observation, capture, or readback proves the requested final state.
- Desktop evidence may verify native permission chrome and file choosers, but browser page content and upload results must be verified on the browser surface; if that surface is unavailable, stop unverified rather than substituting a desktop screenshot.
- For a toggle or other requested state change, compare the fresh readback with the initial state and require the requested final value; an unchanged readback is not success.
- Do not treat a request, a pre-action screenshot, stale coordinates, or a prose diagnostic as completion evidence.
- Treat `structuredContent.error` or `isError=true` as action failure, not success.
- Report failure when no documented fallback or fresh readback can establish the requested state.

## App Guidance

For app-specific behavior, check the snapshot's `app_guidance` field or load the matching file from `references/apps/index.json`.
