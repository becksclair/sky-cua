---
name: browser-use
description: "Use when operating browser tabs or web pages through sky-cua browser MCP tools: browser readiness, tab listing/opening/claiming, snapshots, screenshots, clicks, typing, keypresses, scrolling, and Brave/Chrome-family native-host bridge debugging."
---

# Browser Use

For browser chrome, native file pickers, OS dialogs, extension permission UI,
or desktop windows, use the `computer-use` skill instead — those surfaces are
not reachable through the page.

## Ownership

- `user_chrome` is the user's already-running Chrome-family browser, reached
  through the extension/native-host bridge. `managed` is reserved and reports
  unsupported.
- Action tools only work on tabs from `browser_open` (new session-owned tab)
  or `browser_claim_tab` (adopt an existing tab). `browser_claim_tab`
  reclaims stale owners whose session id starts with `sky-cua-` but never
  steals tabs owned by other live agent sessions.
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

- `browser_snapshot` (title, URL, viewport, visible text, actionable element
  summaries) is the primary inspection tool, and the only one for sessions
  without image input. On control-heavy pages pass `element_query` or
  `element_offset`/`element_limit` — some hosts cap tool-output size.
- Tool success means the input was dispatched, not that the page reacted;
  verify consequential actions with a fresh snapshot or screenshot.
- `browser_scroll` scrolls the nearest scrollable container under x/y, else
  the page viewport (via `window.scrollBy`, not a real wheel event).
