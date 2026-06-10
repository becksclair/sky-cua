---
name: browser-use
description: "Use when operating browser tabs or web pages through sky-cua browser MCP tools: browser readiness, tab listing/opening/claiming, snapshots, screenshots, clicks, typing, keypresses, scrolling, and Brave/Chrome-family native-host bridge debugging."
---

# Browser Use

Use this skill when the installed sky-cua runtime exposes `browser_*` MCP tools
and the task is inside a browser page or tab.

## Core Stance

Treat browser automation as a separate lane from desktop computer use.

- Use `browser_status` first when browser readiness is unclear.
- Use `browser_list_tabs` to discover existing `user_chrome` tabs. When many tabs are open, pass `url_contains` or `title_contains` so the text response includes the tab ids you need.
- Use `browser_open` for a new session-owned tab.
- Use `browser_claim_tab` before acting on an existing user tab.
- Use `browser_snapshot` for page title, URL, viewport, visible text, and common element summaries. This is the best first inspection tool for text-only agents. Pass `element_query` or `element_offset`/`element_limit` to keep responses lean on control-heavy pages; some hosts cap tool-output size.
- Use `browser_screenshot` when target coordinates or visual proof matter. The image is attached to the result when the session's model supports image input, and is also saved to the file named in `structuredContent.screenshot_path`. If you cannot see images at all, use `browser_snapshot` instead of guessing.
- Use `browser_click`, `browser_move_mouse`, `browser_type_text`, `browser_press_key`, and `browser_scroll` against tabs from `browser_open` or `browser_claim_tab`; sky-cua repairs stale browser-session/debugger attachment once per action.
- Treat tool success as transport success. Re-check with `browser_snapshot` or `browser_screenshot` after meaningful actions.

## Targets And Ownership

- `user_chrome` means the user's already-running Chrome-family browser reached through the extension/native-host bridge.
- `managed` is reserved for a future sky-cua-owned browser lifecycle. Until implemented, expect it to report unsupported.
- Existing tabs may be owned by another browser session. `browser_claim_tab` may reclaim stale owners whose session id starts with `sky-cua-`, but it must not steal tabs owned by other agents' browser sessions.
- When proving Brave behavior, pin selection with `SKY_CUA_BROWSER=brave` so the runtime does not probe every Chrome-family browser.

## Coordinate Contract

Browser coordinates are not desktop coordinates.

- All browser tools share one coordinate space: CSS pixels. `browser_screenshot` image pixels, `browser_snapshot` element bounds, and `browser_click`/`browser_move_mouse`/`browser_scroll` coordinates line up one-to-one.
- Screenshots show the currently visible viewport only; scroll first, then re-capture, when the target is off-screen.
- Do not use coordinates from `get_app_state` screenshots with browser tools.
- Do not manually divide by `window.devicePixelRatio`; sky-cua already normalizes high-DPI captures to CSS pixels.
- If a page is scaled or zoomed, verify by clicking one visible target, then reacquire `browser_snapshot` or `browser_screenshot` before continuing.

## Action Loop

1. Check readiness with `browser_status` if needed.
2. Find or open the target tab with `browser_list_tabs` or `browser_open`. Use `url_contains`/`title_contains` on `browser_list_tabs` when the tab count is large.
3. Claim existing tabs with `browser_claim_tab`.
4. Inspect page state with `browser_snapshot`.
5. Use `browser_screenshot` for visual targeting only when you can inspect image output.
6. Perform one action.
7. Re-check page state before the next non-trivial action.

## When To Use Desktop Computer Use Instead

Use `computer-use` instead of this skill when the task targets browser chrome,
native file pickers, OS dialogs, extension permission UI, desktop windows, or any
surface not reachable through the web page's browser tab.

## Read Next When Needed

- For browser MCP contracts and known limitations, read `docs/features/browser-mcp-tools.md`.
- For the host/runtime boundary, read `docs/runtime/mcp-boundary.md`.
- For installed MCP server or plugin deployment runbooks, read `docs/operations/`.
