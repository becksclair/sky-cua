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
- Prefer reusing a tab over opening new ones. `browser_open` creates a new
  session-owned tab on every call, so repeatedly opening leaves a growing pile
  of Codex tabs in the user's browser. The default workflow is:
  `list_resources(surface="browser", resource="tabs")`, `browser_claim_tab(tab_id)`
  the relevant existing tab (e.g. the active/`about:blank` tab), then
  `browser_navigate(tab_id, url)` it and keep reusing that `tab_id`. Only
  `browser_open` when you genuinely need a separate new tab that no existing tab
  can serve. Page actions require an opened or claimed tab.
- Never claim or drive privileged internal pages — `chrome://*`,
  `devtools://*`, `view-source:*`, or the extensions page. They are not CDP
  page targets, so claim and navigate hang and can wedge the debugger
  transport for later tabs. If the only existing tab is one of these,
  `browser_open` a fresh tab instead of claiming it.
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
  `Ctrl+K`, and `Ctrl+L`. Editing and navigation keys act on the focused field:
  `Backspace`/`Delete` delete, `Ctrl+A` selects all, and the arrow keys and
  `Home`/`End`/`PageUp`/`PageDown` move the caret. To replace a field's
  contents, focus it, `Ctrl+A`, then `type_text` (or `Backspace`).

### Read-tool argument shapes

`list_resources(surface="browser", resource="tabs")` lists tabs. It accepts
only the optional keys `target` (`user_chrome`), `url_contains`,
`title_contains`, and `limit`. `limit` caps the returned tab list to at most
that many entries (a `0` or absent limit returns all tabs):

```json
{
  "surface": "browser",
  "resource": "tabs",
  "limit": 20
}
```

`observe(surface="browser", tab_id=...)` reads page state. It accepts only the
optional keys `target`, `text_limit`, `element_query`, `element_offset`, and
`element_limit`. It never returns an image; `capture_screen`/
`screenshot_delivery` are desktop-only. For a tab image call
`capture_screen(surface="browser", tab_id=...)` instead:

```json
{
  "surface": "browser",
  "tab_id": "tab-1",
  "text_limit": 0,
  "element_limit": 200
}
```

### Browser argument shape

Every browser action after `browser_open` or `browser_claim_tab` carries the
claimed `tab_id` as a top-level key. Do not put `tab_id`, coordinates, deltas,
or operation fields inside another string value.

Valid open and navigate:

```json
{
  "url": "about:blank"
}
```

```json
{
  "tab_id": "tab-1",
  "url": "https://example.com"
}
```

Valid viewport scroll:

```json
{
  "tab_id": "tab-1",
  "delta_y": 600
}
```

Valid targeted scroll:

```json
{
  "tab_id": "tab-1",
  "x": 500,
  "y": 700,
  "delta_y": 600
}
```

Valid clicks and typing:

```json
{
  "operation": "click",
  "tab_id": "tab-1",
  "x": 500,
  "y": 700
}
```

```json
{
  "operation": "type_text",
  "tab_id": "tab-1",
  "text": "hello"
}
```

For `browser_scroll`, omit both `x` and `y` for viewport scroll, or provide
both as top-level numbers for targeted scroll. Browser URLs must be HTTP(S) or
exactly `about:blank`; reuse the returned/claimed `tab_id` on every later call.
