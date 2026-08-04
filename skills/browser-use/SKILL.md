---
name: browser-use
description: "Use sky-cua browser MCP tools for page content in a claimed/open desktop Chrome-family tab, including tab control, snapshots, screenshots, clicks, typing, keys, and scrolling; do not use for browser chrome/toolbars, extension pages, browser permission popups, OS/native UI, desktop windows, or Android/phone browser tasks."
---

# Browser Use

Use browser tools only for page content in a claimed or opened desktop
Chrome-family tab. Use `computer-use` for browser chrome or toolbars, Chrome
internal pages, extension pages or permission UI, native file pickers, OS
dialogs, and desktop windows. Use `phone-use` for Android or phone-browser UI.

## Ownership

- List browser tabs with `list_resources(surface="browser", resource="tabs")`.
- Read page state with `observe(surface="browser", tab_id=...)`.
- Capture browser screenshots with `capture_screen(surface="browser", tab_id=...)`.
- Send page input with `browser_input`.
- Target only `user_chrome`, the user's running Chrome-family browser.
- Pin a browser with `SKY_CUA_BROWSER=brave`, `chrome`, or `chromium`.
- Prefer reusing an existing tab.
- `browser_open` creates a new session-owned tab on every call, so repeated opens leave a pile of Codex tabs in the user's browser.
- List tabs before opening a new tab.
- Claim a relevant existing tab such as active/about:blank.
- Navigate the claimed tab and keep reusing its `tab_id`.
- `browser_open` and navigation return the destination AppShot. Before any other state-changing page input, retain the latest exact `appshot_id` for that tab/document and pass it to the action.
- Open a fresh tab only when no existing tab can serve.
- Never claim or drive privileged internal pages such as `chrome://*`, `devtools://*`, `view-source:*`, or the extensions page.
- Privileged internal pages are not CDP page targets, and claiming or navigating them can hang and wedge the debugger transport.
- Open a fresh tab when only internal tabs exist.
- The runtime retries stale session/debugger attachment once per action; treat later attachment failures as real.
- On the exact `Cannot access a chrome-extension:// URL of different extension` attach diagnostic, read [foreign-extension-attach.md](references/foreign-extension-attach.md) and follow its conditional recovery.

## Coordinates

- Treat browser coordinates as normalized CSS pixels: screenshot pixels, observation bounds, and action coordinates line up one-to-one, so never divide them by `devicePixelRatio`.
- Screenshots show only the visible viewport; scroll and re-capture for off-screen targets.
- Any scroll, resize, navigation, or tab switch invalidates coordinates from the prior observation or capture.
- An element `ref` does not go stale from page movement because the service re-resolves it live; prefer it after a re-render or scroll.
- Desktop coordinates are a different space; never reuse desktop screenshot or `observe(surface="desktop")` coordinates in browser actions.

## Observe and finish

- Prefer `observe(surface="browser", tab_id=...)` for the screenshot, title, URL, viewport, visible text, actionable bounds, per-element `ref` values, document generation, and canonical `appshot_id` captured together.
- Defaults are compact; on dense pages use element query, offset, and limit controls, use `text_limit: 0` for controls-only snapshots, and raise text limits only when page text is the task.
- Use the image attached to browser `observe` for visual layout and pixel targeting; `capture_screen` remains a focused screenshot call but does not replace the AppShot action fence.
- Pass the latest matching `appshot_id` to every state-changing browser input. `AppShotRequired` proves the rejected side effect did not run and includes a fresh recovery AppShot; continue from that new id.
- Tool success means only that input was dispatched, so verify consequential changes with a fresh observation or screenshot.
- If fresh evidence shows no intended change, re-observe, correct the current field or target state, and retry the action once; if it is still unchanged, stop and report the failure.
- Stop only after fresh browser evidence confirms the requested page state, and report URL, text, or visual state from that evidence.

## Actions

- Keep `tab_id`, `appshot_id`, `operation`, coordinates, deltas, and text as top-level tool fields; never pack JSON or action fields into an id string.
- `browser_input(operation="click")` moves the visible browser agent cursor before clicking; call `browser_move_mouse` first only for hover or cursor placement without a click.
- Prefer a `ref` from the latest `observe(surface="browser")` over `x`/`y` for actionable controls.
- `browser_input(operation="click", ref=...)` re-finds, scrolls into view, hit-tests, and dispatches a real trusted click at the element's current center.
- `browser_input(operation="type_text", ref=..., text=...)` re-finds, scrolls into view, hit-tests, and types into the element at its current position.
- A `ref` is opaque and must be passed verbatim; never parse or build one.
- Reserve `x`/`y` for pixel targets with no discrete element, such as a canvas or map region.
- Typing by `ref` focuses the field and types in one step, and `type_text` with `ref` requires non-empty `text`.
- On `BrowserElementUnresolved` or `BrowserElementNotActionable`, re-observe for fresh refs and retry.
- Do not discard a `ref` merely because a re-render may have moved it; the service re-resolves it live. Re-observe only when the diagnostic says it is unresolved or non-actionable.
- `BrowserElementUnresolved` means the page changed and the ref matches nothing; `BrowserElementNotActionable` means the element is hidden, off-screen, or covered.
- Do not pixel-guess an unresolved or non-actionable target, and do not use `browser_eval` to find and click elements.
- `browser_scroll` requires non-zero `delta_x` or `delta_y`; omit x/y for viewport scroll, or provide both to move the cursor there and scroll the browser-selected container with viewport fallback.
- `browser_scroll` is scripted DOM scrolling, not a real wheel event.
- `browser_input(operation="type_text")` inserts literal text into the focused control; without a `ref`, focus the control first.
- `browser_input(operation="press_key")` handles focused controls and page shortcuts; use literal keys such as `Enter`, `Escape`, `Tab`, `Ctrl+K`, and `Ctrl+L`.
- To replace field contents, focus the field and press `Ctrl+A` before typing or pressing `Backspace`.
- Browser URLs must be HTTP(S) or exactly `about:blank`.
