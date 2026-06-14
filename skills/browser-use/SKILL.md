---
name: browser-use
description: "Use for sky-cua browser MCP tools: tab control, page snapshots, screenshots, clicks, typing, keys, and scrolling."
---

# Browser Use

For browser chrome, native file pickers, OS dialogs, extension permission UI,
or desktop windows, use the `computer-use` skill instead — those surfaces are
not reachable through the page.

## Ownership

- `user_chrome` is the user's running Chrome-family browser and the only
  target.
- New tab: `browser_open`. Existing tab: `browser_list_tabs` ->
  `browser_claim_tab`. Page actions require an opened or claimed tab.
- The runtime retries stale session/debugger attachment once per action; later
  failures are real.
- Pin a browser with `SKY_CUA_BROWSER=brave` (or chrome/chromium).

## Coordinates

- One shared space: CSS pixels. `browser_screenshot` pixels,
  `browser_snapshot` bounds, and action coordinates line up one-to-one. Never
  divide by `devicePixelRatio`; captures are already normalized.
- Screenshots show only the visible viewport; scroll, then re-capture, for
  off-screen targets.
- Desktop `get_app_state` coordinates are a different space; never reuse
  them here.

## State

- Prefer `browser_snapshot` for title, URL, viewport, visible text, and
  actionable element bounds. Defaults: 4000 text chars, 200 elements.
- Dense pages: use `element_query`, then `element_offset`/`element_limit`.
  Use `text_limit: 0` for controls-only snapshots. Raise `text_limit` only
  when page text is the task.
- Use `browser_screenshot` for visual layout or pixel targeting.
- Tool success means input was dispatched; verify consequential changes with a
  fresh snapshot or screenshot.

## Actions

- `browser_click` moves the visible browser agent cursor before clicking; call
  `browser_move_mouse` first only for hover or cursor placement without click.
- `browser_scroll`: provide non-zero `delta_x` or `delta_y`. Omit x/y for
  viewport scroll; provide x/y to move the cursor there and scroll the nearest
  scrollable container, falling back to the viewport. This is scripted DOM
  scrolling, not a real wheel event.
- `browser_type_text` inserts literal text into the focused control; focus it
  first.
- `browser_press_key` handles focused controls and page shortcuts such as
  Enter, Escape, Tab, Ctrl+K, and Ctrl+L.
