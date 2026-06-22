---
name: browser-use
description: "Use for sky-cua browser MCP tools: tab control, page snapshots, screenshots, clicks, typing, keys, and scrolling."
---

# Browser Use

For browser chrome, native file pickers, OS dialogs, extension permission UI,
or desktop windows, use the `computer-use` skill instead — those surfaces are
not reachable through the page.

## Ownership

- Browser tab listing is `list_resources(surface="browser", resource="tabs")`,
  page state is `observe(surface="browser", tab_id=...)`, screenshots are
  `capture_screen(surface="browser", tab_id=...)`, and clicks/typing/keys are grouped
  under `browser_input`.
- `user_chrome` is the user's running Chrome-family browser and the only
  target.
- New tab: `browser_open` returns the `tab_id` for later observe/capture/action
  calls. Existing tab: `list_resources(surface="browser", resource="tabs")`,
  then `browser_claim_tab(tab_id)` with the listed `tab_id`. Page actions
  require an opened or claimed tab.
- The runtime retries stale session/debugger attachment once per action; later
  failures are real.
- Pin a browser with `SKY_CUA_BROWSER=brave` (or chrome/chromium).

## Coordinates

- One shared space: CSS pixels. `capture_screen(surface="browser", tab_id=...)` pixels,
  `observe(surface="browser", tab_id=...)` bounds, and action coordinates line up
  one-to-one. Never divide by `devicePixelRatio`; captures are already
  normalized.
- Screenshots show only the visible viewport; scroll, then re-capture, for
  off-screen targets.
- Coordinates are valid for the snapshot/capture moment. Any scroll, resize,
  navigation, or tab switch invalidates previous bounds.
- Desktop `observe(surface="desktop")` coordinates are a different space; never reuse
  them here.

## State

- Prefer `observe(surface="browser", tab_id=...)` for title, URL, viewport, visible text,
  and actionable element bounds. Defaults: 4000 text chars, 200 elements.
- Dense pages: use `element_query`, then `element_offset`/`element_limit`.
  Use `text_limit: 0` for controls-only snapshots. Raise `text_limit` only
  when page text is the task.
- Use `capture_screen(surface="browser", tab_id=...)` for visual layout or pixel targeting.
- Tool success means input was dispatched; verify consequential changes with a
  fresh snapshot or screenshot.

## Actions

- `browser_input(operation="click")` moves the visible browser agent cursor
  before clicking; call `browser_move_mouse` first only for hover or cursor
  placement without click.
- `browser_scroll`: provide non-zero `delta_x` or `delta_y`. Omit x/y for
  viewport scroll; provide x/y to move the cursor there and scroll the
  browser-selected container, falling back to the viewport. This is scripted
  DOM scrolling, not a real wheel event.
- `browser_input(operation="type_text")` inserts literal text into the focused
  control; focus it first.
- `browser_input(operation="press_key")` handles focused controls and page
  shortcuts. Use literal key strings such as `Enter`, `Escape`, `Tab`,
  `Ctrl+K`, and `Ctrl+L`.
