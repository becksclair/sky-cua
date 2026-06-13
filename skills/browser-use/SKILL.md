---
name: browser-use
description: "Use when operating web pages through sky-cua browser MCP tools: browser readiness, tab listing/opening/claiming, snapshots, screenshots, clicks, typing, keypresses, and scrolling."
---

# Browser Use

For browser chrome, native file pickers, OS dialogs, extension permission UI,
or desktop windows, use the `computer-use` skill instead — those surfaces are
not reachable through the page.

## Ownership

- `user_chrome` is the user's already-running Chrome-family browser. It is
  the only browser target; other target values are rejected.
- Use `browser_open` for a new controllable tab. Use `browser_list_tabs`,
  then `browser_claim_tab`, before acting on an existing tab.
- Page actions only target controllable tabs from `browser_open` or
  `browser_claim_tab`.
- The runtime repairs stale session/debugger attachment once per action;
  beyond that, failures are real.
- To pin a specific browser (e.g. Brave) instead of probing every
  Chrome-family install, set `SKY_CUA_BROWSER=brave`.

## Coordinates

- One shared space, CSS pixels: `browser_screenshot` image pixels,
  `browser_snapshot` element bounds, and `browser_click` /
  `browser_move_mouse` / `browser_scroll` coordinates line up one-to-one.
  Never divide by `devicePixelRatio` — high-DPI captures are already
  normalized.
- Screenshots show only the visible viewport; scroll, then re-capture, for
  off-screen targets.
- Desktop `get_app_state` coordinates are a different space; never reuse
  them here.

## State

- Prefer `browser_snapshot` for state: title, URL, viewport, bounded visible
  text, and actionable element bounds. Defaults are token-lean: 4000 text
  chars and 200 elements.
- For dense pages, use `element_query` first; use
  `element_offset`/`element_limit` for paging up to 5000 captured controls.
  Use `text_limit: 0` for controls-only snapshots; this skips page text
  extraction. A `null` `textCharCount` means the exact count is intentionally
  unknown because text was omitted or truncated early. Raise `text_limit` up to
  20000 only when page text is the task.
- Use `browser_screenshot` when visual layout or pixel targeting matters.
  Image-capable sessions get an image block; text-only sessions get
  `screenshot_path`, dimensions, and metadata without inline image data.
- Tool success means the input was dispatched, not that the page reacted;
  verify consequential actions with a fresh snapshot or screenshot.

## Actions

- `browser_click` moves the visible browser agent cursor before dispatching
  the click; do not pre-call `browser_move_mouse` unless you need a hover or
  visual cursor move without clicking.
- `browser_scroll` without x/y scrolls the viewport. With x/y, it moves the
  visible browser agent cursor there and scrolls the nearest scrollable
  container, falling back to the viewport. It uses scripted DOM scrolling via
  `Runtime.evaluate`, not a real wheel event.
- `browser_type_text` inserts literal text into the focused control. Click or
  otherwise focus the control first.
- `browser_press_key` is for focused controls and page shortcuts such as
  Enter, Escape, Tab, Ctrl+K, and Ctrl+L.
