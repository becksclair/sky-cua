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
  navigation, or tab switch invalidates previous bounds. An element `ref` does
  not go stale this way — the service re-resolves it live — so it is the safer
  target after the page may have moved.
- Desktop coordinates are a different space. Never reuse desktop screenshot or
  `observe(surface="desktop")` coordinates here.

## State

- Prefer `observe(surface="browser", tab_id=...)` for title, URL, viewport,
  visible text, actionable element bounds, and a per-element `ref` for targeting
  clicks and typing. Defaults are compact enough for most pages.
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
- Prefer a `ref` from the latest `observe(surface="browser")` over `x`/`y` when
  clicking or typing on an actionable control, especially on dynamic pages.
  `browser_input(operation="click", ref=...)` and
  `browser_input(operation="type_text", ref=..., text=...)` re-resolve the
  element's live position at action time (re-find, scroll into view, hit-test)
  and dispatch a real trusted click at its current center, so they do not miss
  when a re-render or scroll has moved the target since you observed it. `ref` is
  opaque; pass it verbatim, never parse or build one. Reserve `x`/`y` for pixel
  targets with no discrete element, such as a canvas or a map region.
- Typing by `ref` focuses the field and types in one step, so no separate
  focus click is needed. `type_text` with `ref` still requires non-empty `text`.
- On a `BrowserElementUnresolved` (the page changed; the ref matches nothing) or
  `BrowserElementNotActionable` (found but hidden, off-screen, or covered)
  diagnostic, re-observe to get fresh refs and retry. Do not pixel-guess the
  target or use `browser_eval` to find and click elements.
- `browser_scroll` requires non-zero `delta_x` or `delta_y`. Omit x/y for
  viewport scroll; provide both x/y to move the cursor there and scroll the
  browser-selected container, falling back to the viewport. This is scripted DOM
  scrolling, not a real wheel event.
- `browser_input(operation="type_text")` inserts literal text into the focused
  control; without a `ref`, focus it first (see the `ref` bullets above to focus
  and type in one step).
- `browser_input(operation="press_key")` handles focused controls and page
  shortcuts. Use literal key strings such as `Enter`, `Escape`, `Tab`,
  `Ctrl+K`, and `Ctrl+L`. To replace field contents, focus it, press `Ctrl+A`,
  then type or press `Backspace`.
- Browser URLs must be HTTP(S) or exactly `about:blank`.
