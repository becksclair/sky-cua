---
name: browser-use
description: "Use for sky-cua browser MCP tools: tab control, page snapshots, screenshots, clicks, typing, keys, and scrolling."
---

# Browser Use

For browser chrome, native file pickers, OS dialogs, extension permission UI,
or desktop windows, use `computer-use` instead. Browser tools only reach page
content in a claimed or opened tab.

## Ownership

- Browser tab listing is `list_resources(surface="browser", resource="tabs")`;
  page state is `observe(surface="browser", tab_id=...)`; screenshots are
  `capture_screen(surface="browser", tab_id=...)`; page input is
  `browser_input`.
- `user_chrome` is the user's running Chrome-family browser and the only
  target. Pin a browser with `SKY_CUA_BROWSER=brave` (or chrome/chromium).
- Prefer reusing a tab. `browser_open` creates a new session-owned tab on every
  call, so repeated opens leave a pile of Codex tabs in the user's browser.
  List tabs, claim a relevant existing tab such as active/about:blank, navigate
  it, then keep reusing that `tab_id`. Open a fresh tab only when no existing
  tab can serve.
- Never claim or drive privileged internal pages: `chrome://*`,
  `devtools://*`, `view-source:*`, or the extensions page. They are not CDP
  page targets; claim/navigate can hang and wedge the debugger transport. If
  only internal tabs exist, open a fresh tab instead.
- The runtime retries stale session/debugger attachment once per action; later
  failures are real.
- Password-manager overlays block attach on login pages. Chrome refuses
  debugger access while the tab hosts another extension's frame — Bitwarden's
  inline autofill menu on a credential form is the common case, surfacing as
  `Cannot access a chrome-extension:// URL of different extension`. Claim and
  attach the tab BEFORE navigating it into a login flow (an attached session
  survives the overlay appearing). If attach is refused on a page already
  showing a login form, dismiss the overlay first — press Escape or click a
  neutral spot via desktop input — then retry the browser action once; if the
  refusal persists, drive that step with desktop input.

## Coordinates

- Browser coordinates are CSS pixels. Browser screenshot pixels, browser
  observation bounds, and browser action coordinates line up one-to-one. Never
  divide by `devicePixelRatio`; captures are already normalized.
- Screenshots show only the visible viewport. Scroll, then re-capture, for
  off-screen targets.
- Coordinates are valid for the observation/capture moment. Any scroll, resize,
  navigation, or tab switch invalidates previous bounds.
- Desktop coordinates are a different space. Never reuse desktop screenshot or
  `observe(surface="desktop")` coordinates here.

## State

- Prefer `observe(surface="browser", tab_id=...)` for title, URL, viewport,
  visible text, and actionable element bounds. Defaults are compact enough for
  most pages.
- On dense pages, use element query/offset/limit controls. Use `text_limit: 0`
  for controls-only snapshots, and raise text limits only when page text is the
  task.
- Use `capture_screen(surface="browser", tab_id=...)` for visual layout or
  pixel targeting. Browser `observe` does not return an image.
- Tool success means input was dispatched, not that the page changed as
  intended. Verify consequential changes with a fresh observation or screenshot.

## Actions

- Keep `tab_id`, `operation`, coordinates, deltas, and text as top-level tool
  fields. Never pack JSON or action fields into an id string.
- `browser_input(operation="click")` moves the visible browser agent cursor
  before clicking. Call `browser_move_mouse` first only for hover or cursor
  placement without click.
- `browser_scroll` requires non-zero `delta_x` or `delta_y`. Omit x/y for
  viewport scroll; provide both x/y to move the cursor there and scroll the
  browser-selected container, falling back to the viewport. This is scripted DOM
  scrolling, not a real wheel event.
- `browser_input(operation="type_text")` inserts literal text into the focused
  control; focus it first.
- `browser_input(operation="press_key")` handles focused controls and page
  shortcuts. Use literal key strings such as `Enter`, `Escape`, `Tab`,
  `Ctrl+K`, and `Ctrl+L`. To replace field contents, focus it, press `Ctrl+A`,
  then type or press `Backspace`.
- Browser URLs must be HTTP(S) or exactly `about:blank`.
