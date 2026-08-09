# Browser MCP tools

## Status

Shipped for `user_chrome`, the only browser target. The managed/isolated
browser target was retired by decision on 2026-06-11 (controlling the user's
real logged-in browser is the product) and its contract stub has been removed
from the wire contract. Last verified: 2026-08-06 with focused browser,
client, and platform `cargo nextest` suites; the full Rust workspace; all-target
Clippy; the Python harness suite; a complete portable plugin build whose six
Linux runtimes passed the x86-64-v3 instruction-floor validator; and a live
cold-start LinkedIn smoke against those exact portable binaries. That smoke
opened a session-owned tab, returned a stable populated destination AppShot,
re-observed the heavy SPA with bounded element projection, captured the visible
viewport, exercised an AppShot-fenced action, and confirmed filtered tab totals.
The installed runtime was left untouched by using a private service socket.

## Summary

`sky-cua` exposes browser readiness, real user-tab listing, session-owned tab
creation, existing-tab claiming, browser snapshots/screenshots, and basic
browser actions through the canonical grouped MCP surface for hosts such as
OpenCode and Pi. The browser capability is advertised by default, but
`[surfaces].browser = false` removes the complete browser MCP projection.
`browser_eval` is independently enabled by default only when the browser surface
exists, and is disabled by an explicit `SKY_CUA_BROWSER_EVAL=off`.

## Contract surface

Current MCP browser entrypoints:

- `status(component="browser")` returns structured browser readiness,
  available targets, diagnostics, and optional known-tab count.
- `list_resources(surface="browser", resource="tabs")` accepts optional
  `target`, currently `user_chrome`, plus optional `url_contains` and
  `title_contains` filters for the human-readable MCP summary and structured
  response. When filters are present, only matching tabs are returned to avoid
  leaking unrelated tab titles/URLs through `structuredContent`; call without
  filters only when a broad tab inventory is actually needed. It returns tabs,
  a `total` matching-count before any explicit result limit, plus diagnostics.
  `user_chrome` is the only target value.
- `browser_open` accepts optional `target=user_chrome` and optional `url`. It
  creates a new session-owned tab through the Chrome-family bridge, attaches it
  to the sky-cua browser session, enables CDP Page events, and navigates when a
  URL is provided. Allowed URL forms are `http://`, `https://`, and
  `about:blank`.
- `browser_claim_tab` accepts `target=user_chrome` and `tab_id` from the
  browser tab resource list. It asks the extension to adopt that existing user
  tab into the sky-cua browser session for subsequent browser actions.
- `browser_move_mouse` accepts `target=user_chrome`, `tab_id`, `x`, `y`, and
  optional `wait_for_arrival`. Coordinates are CSS pixels, the same space as
  browser capture image pixels and browser observation element bounds.
  When omitted, `wait_for_arrival` defaults to true in both the MCP argument
  parser and the service request contract.
- `browser_navigate` accepts `target=user_chrome`, `tab_id`, and `url`. Allowed
  URL forms are `http://`, `https://`, and `about:blank`.
- `observe(surface="browser")` accepts `target=user_chrome`, `tab_id`, and optional
  `element_query`, `element_offset`, `element_limit`, `text_limit`, and
  `capture_timeout_ms`, then
  returns the current page title, URL, and a structured DOM snapshot payload
  when CDP access succeeds. `text_limit` defaults to 4000 visible-text
  characters for MCP calls, accepts 0 to omit page text, and allows up to 20000
  for full text review. The response records `snapshot.textCharCount`,
  `snapshot.textLimit`, and `snapshot.textTruncated`; `textCharCount` is
  `null` when text extraction is skipped or the service stops counting after
  the cap. The service applies element projection before returning the CDP
  payload so agents can surface controls deep in dense sidebars without
  dumping every unrelated element. The MCP tool requests only the projected
  default slice, so the structured `elements` array defaults to at most 200;
  `snapshot.elementCount` always reports the full total, and `element_limit`
  raises or lowers the cap up to the service maximum of 5000. Each returned
  element carries an opaque `ref` — an element reference the agent passes back
  to `browser_input(operation="click"|"type_text")` to target the element by
  identity instead of coordinates. The agent never parses the `ref`; it is a
  self-contained token the service re-resolves against the live page at action
  time. `capture_timeout_ms` is an optional 1000..30000 ms aggregate budget for
  this AppShot. Without it, the service scales a bounded 6..15 second budget
  with requested text, element, and image work. The returned browser AppShot
  carries `capture_outcome`; `deadline_exceeded` with `retryable=true` is safe
  to retry once, while bridge attachment failures are a separate non-retryable
  state. The browser capture also carries `readiness`: `ready` means the
  document had an interactive/complete readyState and a body at capture time,
  `loading` means it did not, and `unknown` means the metadata probe did not
  complete. Readiness does not claim network idle or SPA hydration stability.
- `capture_screen(surface="browser")` accepts `target=user_chrome` and `tab_id`, then captures
  the visible viewport, normalizes the image to CSS-pixel dimensions, and
  re-encodes it with the shared model-screenshot knobs (WebP by default, JPEG
  via `SKY_CUA_MODEL_SCREENSHOT_FORMAT=jpeg`). The MCP result attaches the
  image as an MCP image content block when the session's model supports image
  input. For text-only sessions, the service still persists the capture but
  omits response image data and returns `screenshot_path`, `mime_type`, and
  `width`/`height` only. The base64 payload is never repeated inside
  `structuredContent`.
- `browser_input(operation="click")` accepts `target=user_chrome`, `tab_id`,
  and either `x`/`y` in CSS pixels or an opaque `ref` — mutually exclusive
  targeting. Coordinates match browser capture pixels and browser observation
  element bounds. Before dispatching the CDP click, the service moves the
  browser agent cursor to the target point and waits for arrival. When `ref` is
  supplied, it is an element reference obtained from an
  `observe(surface="browser")` element; the service re-resolves that element's
  live position at action time (re-finding it by signature, scrolling it into
  view when needed, and hit-testing that nothing covers it), then dispatches the
  same trusted click at the element's current center. It never falls back to a
  synthetic `element.click()`. Because resolution is fresh at action time, `ref`
  avoids the stale-coordinate miss that a re-rendered or reflowed page produces
  between observation and click. If the ref cannot be resolved, the call returns
  a `BrowserElementUnresolved` or `BrowserElementNotActionable` diagnostic (see
  below) and dispatches no input.
- `browser_input(operation="type_text")` accepts `target=user_chrome`,
  `tab_id`, non-empty `text`, and optionally a `ref`. Without `ref` it inserts
  text into the already-focused page control. With `ref` it focuses the
  referenced field and types into it in one step — the same live re-resolution
  as `click`, so a separate focus click is unnecessary — and returns the same
  `BrowserElementUnresolved`/`BrowserElementNotActionable` diagnostics on a
  resolution failure.
- `browser_input(operation="press_key")` accepts `target=user_chrome`,
  `tab_id`, and non-empty `key`, then dispatches the key to the page. Single
  CDP key names and modifier chords such as `Ctrl+K`, `Ctrl+L`, `Shift+Tab`,
  and `Meta+K` are accepted. Recognized keys carry their DOM `code` and Windows
  virtual key code, so editing and navigation keys perform their default action:
  `Backspace`/`Delete` delete, `Ctrl+A` selects all, and `Enter`, `Tab`,
  `Escape`, the arrow keys, and `Home`/`End`/`PageUp`/`PageDown` all take effect.
- `browser_scroll` accepts `target=user_chrome`, `tab_id`, `delta_x`, `delta_y`,
  and optional CSS-pixel `x`/`y` context fields. At least one delta must be
  non-zero, and `x` and `y` must be provided together. When they are provided,
  sky-cua first moves the browser agent cursor to that point, then scrolls the
  nearest scrollable DOM container under it when possible and falls back to the
  page viewport. When `x`/`y` are omitted, it scrolls the page viewport
  directly.

Opt-in diagnostic tool:

- `browser_eval` accepts `target=user_chrome`, `tab_id`, and `expression`, then
  evaluates JavaScript in the page with CDP `Runtime.evaluate`, awaits promises,
  and returns the serializable result by value. It is intended for diagnostics
  and controlled page-level fallbacks when visible UI automation is blocked.
  The tool is enabled by default. Running arbitrary JavaScript in real
  signed-in user tabs crosses a stronger trust boundary than visible UI
  automation (hidden DOM, storage, same-origin requests), so an operator who
  wants it off sets `SKY_CUA_BROWSER_EVAL=off` (or `0`/`false`), enforced at
  both layers: when disabled the client does not advertise it in `tools/list`
  and rejects direct calls, and the service — the real CDP execution boundary
  — independently rejects `BrowserRequest::Eval` with a `BrowserEvalDisabled`
  diagnostic so the client and service always agree on the boundary. A thrown
  or rejected expression surfaces as a `BrowserEvalException` diagnostic
  instead of a silent `null` value.

Browser targets:

- `user_chrome` means an already-running user Chrome-family browser: Brave,
  Google Chrome, or Chromium. sky-cua reaches it through the Codex Chrome
  extension and native-host socket, then calls `getUserTabs` to enumerate the
  user's real tabs. Any other `target` string is rejected at argument parsing
  with an explicit error.

`browser_claim_tab` is the explicit adoption seam for existing `user_chrome`
tabs from `list_resources(surface="browser", resource="tabs")`. Browser
actions should target tabs returned by `browser_open` or successfully adopted by
`browser_claim_tab`; callers should not assume every listed tab is already
controllable.

All browser tool coordinates are CSS pixels and share one space:
`capture_screen(surface="browser")` image pixels, `observe(surface="browser")`
element bounds, and `browser_input`/`browser_move_mouse`/`browser_scroll`
coordinates line up one-to-one. They are not desktop screen coordinates and
they are not coordinates from desktop observations. The service normalizes
high-DPI captures to CSS-pixel dimensions at capture time, so agents never
divide by `window.devicePixelRatio` and the center of a returned snapshot
element can be passed directly to `browser_input(operation="click")`.
Screenshots cover the currently visible viewport only; scroll and re-capture
when the target is off-screen. Coordinate targeting stays valid and is the
right tool when the agent genuinely has a pixel target with no discrete
element — a canvas or a map region; for actionable DOM controls on a dynamic
page, prefer the `ref` path, which re-resolves the live position and cannot go
stale between observation and action.

Recommended agent flow:

- Use `status(component="browser")` first when diagnosing bridge readiness.
- Use `browser_open` for a new controllable tab, or
  `list_resources(surface="browser", resource="tabs")` followed by
  `browser_claim_tab` for an existing user tab. When many tabs are open, pass
  `url_contains` or `title_contains` to make the text response list relevant tab
  ids instead of only a count.
- Use `observe(surface="browser")` to inspect page title, URL, viewport, visible text,
  and common actionable element summaries with click-ready CSS-pixel bounds and
  a per-element `ref`. Pass `element_query: "update"` or an `element_offset`/`element_limit`
  window when a page contains many controls. Its MCP text summary also includes
  these details for text-only hosts. Because Pi and some other hosts keep
  `structuredContent` outside model context, text-only sessions additionally
  append the canonical browser semantic snapshot to the model-facing text as
  compact JSON, bounded to 32 KiB. The complete projection remains available
  in `structuredContent` and the persisted AppShot artifact when that text
  fallback is truncated.
- Use `browser_input`, `browser_scroll`, and `browser_move_mouse` against the
  tab returned by `browser_open` or `browser_claim_tab`. Prefer a `ref` from the
  latest `observe` when clicking or typing on an actionable control; reserve
  `x`/`y` for pixel targets without a discrete element. The service
  self-recovers once when the native-host bridge reports lost session ownership
  or a detached debugger. In `legacy` the compatibility session is
  `sky-cua-mcp`; in `hybrid`/`strict` the daemon uses the canonical control-plane
  session and applies scheduler/settlement rules.
- Use `capture_screen(surface="browser")` when visual proof is needed; the image arrives as an
  MCP image content block for image-capable sessions and is also persisted to
  the file named in `structuredContent.screenshot_path`. Text-only agents should
  prefer `observe(surface="browser")`.

Environment variables:

- `SKY_CUA_BROWSER` restricts real-browser socket selection for `user_chrome`;
  accepted values are `brave`, `chrome`, `chromium`, and `all`/unset. Runtime
  and installer machine-config seeding normalize legacy `brave-origin`,
  `chrome-origin`, and `chromium-origin` aliases to those canonical values.
- `SKY_CUA_BROWSER_CONTROL_MODE` and
  `SKY_CUA_CODEX_BROWSER_SOCKET_PATH` override the corresponding
  `[browser_control]` machine-config fields independently. Resolution is
  environment, then machine config, then legacy/unset behavior.
- `SKY_CUA_BROWSER_USE_SOCKET_DIR` and `CODEX_BROWSER_USE_SOCKET_DIR` override
  native-host socket discovery. If either explicit directory is set, the
  default `/tmp/codex-browser-use` fallback is not used.

Service IPC variants:

- Browser IPC uses a single service envelope:
  `ServiceRequest::Browser { request: BrowserRequest }`.
- `BrowserRequest` is internally tagged with `type` and currently includes
  `Status`, `ListTabs { target }`, `Open { target, url }`,
  `ClaimTab { target, tab_id }`,
  `MoveMouse { target, tab_id, x, y, wait_for_arrival }`,
  `Navigate { target, tab_id, url }`,
  `Snapshot { target, tab_id, text_limit, element_offset, element_limit,
  element_query }`,
  `Screenshot { target, tab_id, include_image_data }`,
  `Click { target, tab_id, x, y }`,
  `ClickElement { target, tab_id, element_ref }`,
  `TypeText { target, tab_id, text }`,
  `TypeTextElement { target, tab_id, element_ref, text }`,
  `PressKey { target, tab_id, key }`,
  `Scroll { target, tab_id, delta_x, delta_y, x: Option, y: Option }`, and
  `Eval { target, tab_id, expression }`.
- Browser IPC responses use the matching envelope:
  `ServiceResponse::Browser { response: BrowserResponse }`.
- `BrowserResponse` is internally tagged with `type` and currently includes
  `Status { report }`, `ListTabs { response }`, `Open { response }`,
  `ClaimTab { response }`, `MoveMouse { response }`, `Navigate { response }`,
  `Snapshot { response }`, `Screenshot { response }`, `Click { response }`,
  `TypeText { response }`, `PressKey { response }`, `Scroll { response }`, and
  `Eval { response }`.
- `ListTabs` responses contain `target`, `tabs`, and `diagnostics`.
- `Open` responses contain `target`, optional created `tab`, and `diagnostics`.
- `ClaimTab` responses contain `target`, optional adopted `tab`, and
  `diagnostics`.
- `MoveMouse` responses contain `target`, `tab_id`, the requested CSS-pixel
  coordinates, `wait_for_arrival`, and `diagnostics`.
- `Navigate` responses contain `target`, `tab_id`, `url`, and `diagnostics`.
- `Snapshot` responses contain `target`, `tab_id`, optional `title`, optional
  `url`, optional `snapshot`, and `diagnostics`.
- `Screenshot` responses contain `target`, `tab_id`, `mime_type`,
  `data_base64`, optional `screenshot_path`, optional `width`/`height`, and
  `diagnostics`. The client drops `data_base64` from `structuredContent` and
  forwards the image as an MCP image content block instead.
- `Click`, `TypeText`, `PressKey`, and `Scroll` return
  `BrowserActionResponse` with `target`, `tab_id`, `action`, and
  `diagnostics`.
- `Eval` responses contain `target`, `tab_id`, optional serializable `value`,
  and `diagnostics`.

Install outputs:

- `scripts/install_mcp_server.py --host opencode` writes an OpenCode config
  that preserves `SKY_CUA_BROWSER` when set during install.
- `scripts/install_mcp_server.py --host pi` writes `pi_mcp_wrapper.sh` and a
  copyable MCP snippet. If `~/.pi/agent` exists, it also merges the `sky_cua`
  entry into `~/.pi/agent/mcp.json` and copies sky-cua skills into
  `~/.pi/agent/skills` without replacing unrelated Pi MCP servers. The wrapper
  uses an absolute `/bin/bash` interpreter so Pi's reduced MCP subprocess PATH
  cannot break startup, and the Pi override pins `args: []` so arguments from a
  lower-precedence shared `sky_cua` config cannot leak into the wrapper call.
- `python3 install.py install` supplies OpenClaw through native Codex
  compatibility plugins, registers global `node_repl`, projects an existing
  Pi installation's fixed-root MCP wrapper/config, and enables no-prompt
  full-auto Codex policy. The Pi wrapper captures the installer's `PATH` because
  Pi's lazy MCP subprocess environment may omit utilities required by the
  packaged architecture launcher. The former `openclaw_mcp.json` / standalone
  `sky_cua` MCP registration was retired.
- `scripts/install_mcp_server.py --host claude-code` writes
  `claude_code_mcp.json`, registers the `sky-cua` stdio server (Claude Code
  reserves the name `computer-use`) through
  `claude mcp add-json --scope user` when the `claude` CLI is on `PATH`, and
  copies sky-cua skills into `~/.claude/skills` when `~/.claude` exists.
  Claude Code stdio servers inherit the parent environment, so no env-var
  passthrough list is required. The repository also ships a Claude Code plugin
  manifest (`.claude-plugin/plugin.json` with an inline `mcpServers` entry
  rooted at `${CLAUDE_PLUGIN_ROOT}`) plus `.claude-plugin/marketplace.json`, so
  a built checkout or staged bundle can be installed directly as a Claude Code
  plugin.
- `scripts/install_mcp_server.py --restart-runtime` is an opt-in development
  deploy helper. After copying new installed binaries, it attempts to refresh
  the user AT-SPI accessibility bus on Linux desktop sessions and stops sky-cua runtime
  processes rooted under the install target so OpenCode, Pi, or another MCP host
  can respawn from the updated `sky-cua-client`/`sky-cua-service` on the next
  tool call. If the host does not reconnect automatically, reload the host
  session; for Pi, use `/reload` or restart Pi.

## Behavior

The browser status branch combines the existing runtime doctor browser report
with browser-bridge diagnostics. When a matching native-host socket is
connected, status does not emit a disconnected diagnostic. If the browser
selection env is invalid, the report returns an explicit diagnostic instead of
guessing. If the desktop request lane is already busy, status still returns
bridge diagnostics and marks browser integration checks as deferred instead of
waiting behind the desktop action.


The browser tabs resource discovers Unix sockets from the Chrome-family native
messaging host, filters them by `SKY_CUA_BROWSER`, and calls the Codex
extension's `getUserTabs` method. This is intentionally different from
`getTabs`, which lists session-owned Codex tabs rather than the user's real
browser tabs. Tab titles and URLs are structured runtime data. MCP text output
shows at most a small bounded set of tabs. `url_contains`/`title_contains` also
filter `structuredContent.tabs`, so a targeted lookup does not expose hundreds
of unrelated tab titles and URLs to text-only agents, logs, or transcripts.

Transport depends on `SKY_CUA_BROWSER_CONTROL_MODE`. In the default `legacy`
mode, MCP browser-tool calls use short-lived ephemeral clients with
`session_id="sky-cua-mcp"`, while a separate heartbeat connection receives
extension-originated pings. In `hybrid` or `strict`, browser-tool calls enter
the daemon scheduler and use one persistent native-host `control_plane` role
with canonical extension identity; real MCP caller identity remains private to
the daemon. See
[`unified-browser-bridge-control-plane.md`](unified-browser-bridge-control-plane.md).

`browser_open(user_chrome)` uses the same socket discovery and browser-family
filtering, then sends `createTab`, `attach`, and `executeCdp(Page.enable)` over
the native-host bridge. If `url` is provided, it sends
`executeCdp(Page.navigate)` with that URL and returns the created tab with its
URL set to the requested navigation target. A bridge disconnect, unsupported
target, invalid URL, missing tab id, or CDP navigation error is reported through
structured diagnostics; the MCP response is marked as an error when no tab was
created. If `createTab` succeeds but attach, page enable, or navigation fails,
the response returns the created tab plus a `BrowserOpenPartial` diagnostic and
marks the MCP call as an error so callers can see the side effect explicitly.

`browser_claim_tab(user_chrome)` probes matching sockets, then sends
`claimUserTab` with `session_id="sky-cua-mcp"` and the requested tab id. A
successful response returns the adopted tab metadata. If the extension reports
that the tab belongs to another non-sky-cua session, that bridge error is
surfaced as a diagnostic instead of being hidden. If the tab belongs to a stale
`sky-cua-*` session, sky-cua finalizes that stale session with an empty keep list,
which releases user-tab leases without closing the user tab, then retries
`claimUserTab` once. After successful claim or reclaim, sky-cua sends `attach`
and `executeCdp(Page.enable)` so CDP-backed browser actions can target the tab
immediately. If Page enable reports that the debugger is not attached, sky-cua
sends a best-effort `detach` to clear the extension's stale debugger bookkeeping,
then retries attach/Page enable once. If claim still succeeds but attach/Page
enable fails, the response returns the adopted tab plus `BrowserClaimPartial` and
the MCP call is marked as an error.

`browser_move_mouse(user_chrome)` sends `moveMouse` with the same sky-cua MCP
session id, target tab id, the CSS-pixel coordinates as provided, and
`waitForArrival`. It moves the visible browser agent cursor, not the sky-cua
desktop synthetic cursor used for portal screenshots. If the bridge
reports stale session ownership, an unattached debugger, or a CDP command
timeout, sky-cua reclaims, detaches, re-attaches, enables Page, and retries
once (the move is an absolute position, so a replay is safe).

Browser navigation, observation, capture, input, and enabled `browser_eval` use
extension `executeCdp` requests against tabs that are part of the sky-cua
browser session.
Navigation uses `Page.navigate`. Snapshot uses `Runtime.evaluate` to return the
page title, URL, viewport, body text up to 20,000 characters, total actionable
element count, and up to 5,000 common actionable elements matching anchors,
buttons, inputs, textareas, selects, button/link roles, and editable content.
Screenshot first evaluates the viewport metrics (`innerWidth`,
`innerHeight`, `devicePixelRatio`), then uses `Page.captureScreenshot` with
`fromSurface` to capture the visible viewport as PNG. The service normalizes
the capture to CSS-pixel dimensions (resampling when DPR is not 1), re-encodes
it as JPEG or WebP per `SKY_CUA_MODEL_SCREENSHOT_FORMAT`/`*_QUALITY`, writes it
under the runtime captures directory (`$XDG_RUNTIME_DIR/sky-cua/captures`,
pruned to the eight most recent captures per tab), and reports the path and
dimensions alongside the encoded data. Click, type, and key actions use CDP
`Input.*` events with CSS-pixel coordinates passed through unchanged. Each of
those actions first issues a best-effort `Emulation.setFocusEmulationEnabled`
so the tab's renderer treats itself as focused: sky-cua usually drives a
background tab whose `document.hasFocus()` is false, where Blink otherwise drops
click-to-focus and does not deliver `Input.insertText`. This makes clicks focus
their target and `type_text` land on an already-focused field even with no
preceding click. It does not make synthetic key events (`Input.dispatchKeyEvent`
for Backspace/arrows/Ctrl+A) edit a field that was only focused
programmatically — Blink requires the activation and caret a real click
establishes — so key-driven editing still expects a preceding click, the normal
focus-then-edit flow. The enabled `browser_eval` path uses `Runtime.evaluate`
with `awaitPromise=true` and `returnByValue=true`.

Element-targeted click and type (`ref`) resolve the element against the live
page immediately before acting. Each snapshot element carries an opaque `ref`
that self-contains how to re-find the element; at action time the service
re-locates it by signature, scrolls it into view when it is outside the
viewport, and hit-tests its center so a covered control is not clicked blindly.
On success it dispatches exactly the coordinate path's trusted input at the
element's freshly resolved center — the agent-cursor move, focus emulation, and
`Input.dispatchMouseEvent` sequence for a click, or focus plus `Input.insertText`
for a type — never a synthetic `element.click()`. Resolution is stateless and
side-effect-free apart from the intended scroll and the click/type, so a failed
resolution dispatches no input and leaves no residue. Two structured diagnostics
report failure: `BrowserElementUnresolved` when the `ref` matches no element on
the current page (the page changed since it was observed), and
`BrowserElementNotActionable` when the element is found but is zero-sized,
off-screen after a scroll attempt, or covered by another element. Both carry the
remedy of re-observing with `observe(surface="browser")` to obtain fresh refs
and retrying, rather than pixel-guessing the target.
Snapshot element values are suppressed for password/hidden/token/API-key/auth/
credential/session/code/PIN-like fields; use desktop computer-use or explicit
user-directed workflows for sensitive form inspection instead of relying on raw
browser snapshots.
If the first CDP request reports `Debugger is not attached`, `not part of
browser session`, or a bridge-side CDP command timeout (`Timed out after …
waiting for CDP command …`), sky-cua sends `claimUserTab`, `detach`, `attach`,
and `executeCdp(Page.enable)` on the same bridge socket. Command timeouts take
this recovery path because the extension abandons a timed-out CDP command
without cancelling it; the stuck command wedges every later command on that
tab's debugger session, and only a detach/attach cycle clears it. After the
reset, the original action is retried once — but only when replaying it cannot
mutate the page twice. Snapshot, screenshot, and absolute cursor moves are
always replayed. Mutating actions (click, type, key, navigate, eval, scroll)
are replayed only when none of their compounding sub-commands took effect:
each such sub-command (input dispatch, evaluated scroll/eval, navigation)
raises a mutation flag before dispatch and lowers it only when the extension
rejected the command up front with `Debugger unattached` — its session
bookkeeping refused the target before executing, so the command provably never
ran. That is the post-idle-detach signature (see the keepalive section), and
those operations now recover and replay transparently instead of surfacing a
spurious input error. Once any compounding sub-command has landed — or failed
in a way that cannot prove non-execution (a command timeout, a
`Detached while handling command`) — the operation is not replayed, because a
click/press dispatches several CDP commands on one stream (mouseMoved →
mousePressed → mouseReleased) and replaying would re-dispatch committed
sub-commands and double the input; those calls surface the failure diagnostic
(with a note that the session was reset and steering toward desktop-control
tools). Failures after the retry are
surfaced as diagnostics rather than looping indefinitely.

**Discarded (asleep) tabs.** A tab the browser has discarded (Brave marks these
with a sleeping icon) attaches browser-side, but renderer-bound commands
(`Page.enable`, `Runtime.evaluate`, input dispatch) hang until the extension's
`timeoutMs` expires because no renderer process exists. The cure is
`Page.bringToFront`: handled in the browser process, it succeeds without a
renderer, and activating a discarded tab makes Chrome reload it
(live-verified against Brave sleeping tabs, 2026-07-08). Session recovery
wakes the tab through two routes. When the triggering failure was itself a
CDP command timeout (the discarded-tab signature), the wake runs right after
the session reset, before the re-enable. When recovery was entered another
way — typically `Debugger unattached`, because a discarded tab gets a fresh
tab id and has never been attached — the wake is discovered lazily: the
recovery `Page.enable` is capped at 4 s (`RECOVERY_ENABLE_TIMEOUT_CAP_MS`; a
live tab answers in milliseconds, so a hang past the cap is the discarded
signal), and when that capped enable times out the session is reset once more
(the timed-out enable wedges it), the tab woken, and the enable retried — all
inside one operation deadline. The claim path's first enable is capped the
same way. Healthy background tabs are never brought to the foreground: their
enable succeeds before any wake is reachable. When the post-wake `Page.enable`
still fails, the diagnostic details name the likely discarded state and steer
toward retrying or reopening the URL with `browser_open`. The bridge exposes
no `tabs.reload`/`tabs.update` relay and its tab payloads carry no `discarded`
flag, so the hang-then-wake dance is the only detection the extension surface
allows.

The extension runs a **driver-liveness heartbeat**: every 30 seconds it sends a
`ping` and detaches every debugger if no reply arrives within 3 seconds. The
legacy path answers through `browser/keepalive.rs`; browser activity keeps that
daemon alive for 30 minutes after the latest request. The `hybrid`/`strict`
path folds heartbeat into the persistent bridge actor, so browser operations
and liveness share one canonical control connection rather than competing
primary and ephemeral lifecycles. Ordinary MCP requests and raw Codex requests
both mark 30-minute browser activity, preventing the daemon's five-minute idle
exit from tearing down the actor between active browser operations. Installed
acceptance proves two simultaneous Codex connections; overlap with direct
MCP/OpenClaw/OpenCode/Pi remains pending. Each `executeCdp` request carries a
`timeoutMs` derived from the remaining call deadline (capped at the extension's
10-second default, and shrunk below the 250 ms floor when the deadline is
nearly exhausted), so the bridge returns a structured timeout before the
service abandons the socket read.

`browser_scroll` uses `Runtime.evaluate` rather than CDP
`Input.dispatchMouseEvent(type="mouseWheel")`, because the live extension bridge
timed out on the mouse-wheel CDP command during the 2026-06-06 full MCP smoke.
The client and service both reject zero-delta scroll calls. When `x`/`y` are
provided, the service first moves the visible browser agent cursor to that
point. The evaluated script then finds
`document.elementFromPoint(x, y)`, walks to the nearest scrollable ancestor, and
scrolls that container. If no scrollable element is found, it scrolls the page
viewport. When `x`/`y` are omitted, the evaluated script does not call
`elementFromPoint(0, 0)`; it scrolls the page viewport directly.

Socket discovery uses `/tmp/codex-browser-use/extension-<pid>-<nonce>.sock` by
default. The service inspects `/proc` process ancestry to classify sockets as
Brave, Chrome, or Chromium before querying them. Discovery keeps a short-lived
daemon-local inventory of socket family lookups and recently disconnected
sockets, considers at most 32 newest live socket paths per call, and caps bridge
probes at eight concurrent socket tasks. Stale socket diagnostics are suppressed
when at least one matching live socket responds; if no bridge is connected, the
response returns an empty tab list with a `BrowserBridgeDisconnected` diagnostic.

Tab-bound requests use daemon-global tab-to-socket affinity. Bridge tab ids
are per-browser integers, so the same id can name unrelated tabs on two
connected bridges; every path that learns which socket owns a tab
(`browser_open`, `browser_claim_tab`, `browser_list_tabs`, and each successful
tab-bound operation) records the mapping, and later operations on that tab run
against the owning socket only. If the recorded owner itself answers that the
tab does not exist, the call fails, the stale mapping is dropped, and the next
call rediscovers the owner. A tab id listed by more than one socket in a
single sweep is ambiguous and gets no mapping, and each sweep also prunes
entries a listed socket owns for tabs that no longer appear in its listing —
the close-a-tab case no other prune path covers. Without a mapping, the service
probes all candidate sockets, but a mutating request may move to another
socket only when a bridge answers `No tab with id` — proof the tab is not on
that browser; a not-found from a non-owner never erases an existing mapping.
Any other failure is terminal for the call: retrying it on another bridge
could drive an unrelated tab that happens to share the id. Read-only
operations (snapshot, screenshot, cursor moves) are exempt from that
terminality and may still fall through, since retrying them cannot
double-apply input; the terminal diagnostic, when one exists, is what the
call surfaces even if a non-owner's not-found arrived first. Genuinely
colliding ids stay
unmapped, and a bound operation or claim on an unmapped colliding id may
engage either browser; pinning `SKY_CUA_BROWSER` (or the machine config
`browser` key) to one browser family is the operator mitigation.

## Source paths

- `crates/sky-cua-platform/src/model/browser.rs` — browser contracts and gate
  helpers.
- `crates/sky-cua-platform/src/model/service.rs` — service request/response
  variants.
- `crates/sky-cua-service/src/browser.rs` — native-host bridge client,
  browser-open/list-tab flows, tab mapping, and CDP-backed actions.
- `crates/sky-cua-service/src/browser/sockets.rs` — socket discovery,
  browser-family filtering, inventory caching, and stale-socket suppression.
- `crates/sky-cua-service/src/daemon.rs` — service handlers for browser
  requests.
- `crates/sky-cua-client/src/mcp_tools/browser.rs` and
  `crates/sky-cua-client/src/mcp_tools/browser/{args,schema,response}.rs` —
  browser MCP handlers, argument parsing, tool definitions, and summaries.
- `scripts/install_mcp_server.py` — OpenCode/Pi browser-tool installation.
- `scripts/live_chrome_host_client_smoke.py` — bridge and MCP smoke helper.
- `resources/chrome_preflight.py` — native-host manifest and env allowlist
  support.
- `.mcp.json` — packaged env allowlist includes `SKY_CUA_BROWSER`.

Browser tools no longer require a host-specific opt-in flag; they are enabled by
the all-on default surface policy and can be removed with `[surfaces].browser =
false`. Codex Desktop may
still use the companion Browser Use and Chrome plugins until the adapter
delegates browser actions through the shared runtime.

## Verification

Unified control-plane installed acceptance from 2026-07-19 proves navigation,
click, typing, scroll, screenshots, and two simultaneous Codex connections.
This does not prove direct-caller overlap, rollback, restart reconciliation,
performance targets, or the VM `all` profile; those remain tracked in the
unified-control-plane feature doc.

Focused screenshot-wedge hardening from 2026-06-12:

```bash
cargo fmt --check
cargo test -p sky-cua-service
```

- Service regression tests prove a bridge-side CDP command timeout
  (`Timed out after … waiting for CDP command …`) takes the
  claim/detach/attach/enable recovery path and the retried screenshot
  succeeds; that a timed-out input action (click) resets the session but is
  not replayed and surfaces the timeout diagnostic instead — including on a
  second bridge socket; and that `executeCdp` derives `timeoutMs` from the
  remaining call deadline (250 ms–10 s with a 750 ms response margin,
  shrinking below the floor near an exhausted deadline so the command timer
  never outlives the read).
- Affinity regression tests prove tab-bound requests route only to the
  recorded owning socket, that `No tab with id` drops a stale mapping (only
  when the owner itself answers it) and is the sole failure that lets a
  mutating request fall through to another socket, that an unknown tab still
  reaches the socket that has it, that a stored terminal diagnostic outranks
  an earlier non-owner not-found in the surfaced error, and that a listing
  sweep prunes entries for tabs closed while their browser stays connected.
- Live smoke: `browser_open` + `browser_screenshot` against Brave through the
  installed MCP client, including capture of a background tab and a minimized
  Brave window.

Resilience hardening from 2026-07-01 (`cargo nextest run -p sky-cua-service
-p sky-cua-chrome-host`):

- `cdp_detach_resets_session_without_replaying_input_action` proves a
  mid-sequence `Detached while handling command` on a click resets the session
  but does not replay the input — the same no-double-apply guarantee the CDP
  timeout already had, now covering the detach/unattached recoverable classes.
- `send_bridge_request_skips_belated_reply_to_prior_own_request` /
  `send_bridge_request_rejects_foreign_response` prove a belated reply to one of
  our own earlier requests on a reused stream is dropped, while a genuinely
  foreign frame still hard-fails.
- `chrome_response_is_forwarded_to_the_matching_client_after_unlock` proves the
  native host routes a Chrome response to its client with the socket write
  performed after the `HostState` lock is dropped, so a non-reading (wedged)
  service worker cannot freeze every host thread on a blocked write.
- The service bridge frame cap is 64 MiB (below the host's 100 MiB) so a large
  or high-DPI `Page.captureScreenshot` response is not rejected mid-relay, and
  the probe/IO timeout honors `SKY_CUA_BROWSER_REQUEST_TIMEOUT_MS` (floored at
  3s) so a slow-but-healthy relay is not quarantined as stale.

Focused browser reliability checks from 2026-06-15:

```bash
cargo fmt --check
cargo test -p sky-cua-platform -p sky-cua-service -p sky-cua-client
cargo test
uv run ruff format --check scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
python3 scripts/build_plugin.py
python3 scripts/install_plugin.py --bundle-root dist/plugin/sky-cua
python3 scripts/live_agentic_loop_smoke.py
```

- Installed-MCP agent-loop liveness smoke passed with artifact
  `artifacts/pi-agentic-loop-smoke/20260615T211914Z`; this proves the
  installed MCP server is reachable through an external agent, not
  browser-specific tool behavior.
- Service regression tests prove CDP action recovery still handles
  `Debugger is not attached` and stale session ownership, `browser_click` moves
  the browser agent cursor before dispatching the CDP click, targeted
  `browser_scroll` moves the cursor before scrolling the nearest scrollable DOM
  container under `x`/`y`, untargeted `browser_scroll` scrolls the viewport
  without synthesizing an `(0,0)` target, enabled `browser_eval` returns the
  CDP runtime value, and `browser_press_key` dispatches keys with CDP modifier
  bits plus the DOM `code` and Windows virtual key code, using `rawKeyDown` when
  the press carries no text so editing/navigation keys (Backspace, Delete,
  Ctrl+A, arrows) run their default action instead of no-opping.
- Client regression tests prove `browser_snapshot` advertises and applies
  `element_query`/`element_offset`/`element_limit`, including a dense
  OpenChamber-style sidebar case where `Update Available` is deep in the element
  list, and prove `text_limit` defaults to 4000, accepts 0 for controls-only
  snapshots, rejects values above 20000, and preserves truncation metadata.
- Client regression tests prove text-only `browser_screenshot` calls request a
  path-backed capture without response image data while image-capable sessions
  still receive an MCP image content block.
- Platform contract tests prove direct service requests preserve omitted
  `browser_scroll` target coordinates and default omitted
  `browser_move_mouse.wait_for_arrival` to true. Service regression tests prove
  zero scroll deltas are rejected before CDP dispatch.
- Client registry tests prove `browser_eval` is advertised and routed through
  the Browser MCP service request/response envelope by default, is unadvertised
  and rejected when `SKY_CUA_BROWSER_EVAL=off`, and stays in sync between the
  client and service boundaries; a service test proves thrown expressions
  become `BrowserEvalException` diagnostics.

Focused hardening checks from 2026-06-08:

```bash
cargo nextest run -p sky-cua-service
cargo clippy -p sky-cua-service --all-targets -- -D warnings
cargo fmt --check && cargo nextest run
cargo nextest run -p sky-cua-client
```

- Service regression tests prove CDP actions and `browser_move_mouse` recover
  once from `Debugger is not attached` and `not part of browser session` bridge
  errors by reclaiming, attaching, enabling Page, and retrying.
- Client regression tests prove `browser_list_tabs` text summaries expose bounded
  tab ids/title/URL data and respect `url_contains`/`title_contains` filters.
- Client regression tests prove `browser_snapshot` text summaries expose title,
  URL, viewport, visible text, actionable elements, and element bounds for
  text-only agents. Service regression tests pin snapshot bounds to CSS pixels
  and prove screenshot captures are normalized to CSS-pixel dimensions,
  re-encoded with the model-screenshot knobs, and persisted to disk.
- Client regression tests prove `browser_screenshot` results attach an MCP
  image content block for image-capable sessions, omit it for text-only
  sessions, and never repeat `data_base64` inside `structuredContent`.
- Direct installed MCP smoke used `SKY_CUA_BROWSER=brave` and isolated service
  socket `/tmp/sky-cua-recover-smoke-465994.sock`. It completed
  `browser_open`, `browser_snapshot`, `browser_screenshot`, `browser_click`,
  `browser_snapshot`, `browser_press_key Escape`, and final `browser_snapshot`
  without manual `browser_claim_tab` recovery.

Focused checks from 2026-06-06:

```bash
cargo test -p sky-cua-platform
cargo test -p sky-cua-client
cargo test -p sky-cua-service
cargo fmt --check
cargo clippy -p sky-cua-platform -p sky-cua-client -p sky-cua-service --all-targets -- -D warnings
```

Isolated live MCP proof from 2026-06-06:

- Built `target/release/sky-cua-client` and `target/release/sky-cua-service`.
- Used isolated `SKY_CUA_SERVICE_SOCKET_PATH=/tmp/sky-cua-browser-full-mcp-bex-1154686.sock`.
- Direct MCP `tools/list` advertised `browser_status`, `browser_list_tabs`,
  `browser_open`, `browser_claim_tab`, `browser_move_mouse`,
  `browser_navigate`, `browser_snapshot`, `browser_screenshot`,
  `browser_click`, `browser_type_text`, `browser_press_key`, and
  `browser_scroll`.
- MCP `browser_open({"target":"user_chrome","url":"http://127.0.0.1:42575/"})`
  returned `isError=false` and opened Brave tab `221675114` against a local HTTP
  fixture.
- MCP `browser_snapshot` returned title `sky-cua browser action fixture`.
- MCP `browser_move_mouse`, `browser_click`, `browser_type_text`,
  `browser_press_key`, and `browser_scroll` returned `isError=false` against the
  same tab.
- MCP `browser_screenshot` returned `mime_type="image/png"` with 37,664 base64
  bytes.
- MCP `browser_navigate({"url":"about:blank"})` returned `isError=false` and
  `url="about:blank"`.

Installed OpenCode MCP proof from 2026-06-06:

- Rebuilt `target/release/sky-cua-client` and `target/release/sky-cua-service`.
- Reinstalled with `python3 scripts/install_mcp_server.py --target-dir "$HOME/.local/share/sky-cua" --host opencode --bin-dir "$HOME/.local/bin"`.
- Direct installed-binary MCP probe used isolated
  `SKY_CUA_SERVICE_SOCKET_PATH=/tmp/sky-cua-installed-browser-tools-bex-1190776.sock`.
- `tools/list` advertised the full browser tool set.
- The installed `browser_scroll` description read: `Scroll the page viewport
  within a claimed or session-owned user_chrome tab. Positive delta_y scrolls
  down.`

Stale-session reclaim proof from 2026-06-06:

- Regression tests cover a tab owned by stale session `sky-cua-cursor-proof`:
  service calls `finalizeTabs` as that stale session with `keep=[]`, retries
  `claimUserTab`, then attaches and enables Page CDP for the reclaimed tab.
- Negative regression coverage proves tabs owned by non-sky-cua sessions, such
  as `codex-browser-use`, are not finalized or stolen.
- Regression tests also cover stale extension debugger bookkeeping: when
  `Page.enable` returns `Debugger is not attached`, service sends best-effort
  `detach`, then retries attach/Page enable once.
- Live MCP proof used `target/release/sky-cua-client mcp`, real Brave,
  `SKY_CUA_BROWSER=brave`, and isolated socket
  `/tmp/sky-cua-browser-reclaim-bex-1294027.sock`.
- The existing Chamber tab `221674306` was claimed through `browser_claim_tab`
  after earlier stale ownership, then `browser_snapshot` succeeded through CDP
  with title `Dot Agents | OpenChamber` and URL `https://chamber.heliasar.com/`.
- Installed OpenCode MCP proof used
  `/home/bex/.local/share/sky-cua/bin/sky-cua-client` and isolated socket
  `/tmp/sky-cua-installed-reclaim-bex-1326250.sock`; the same Chamber claim plus
  snapshot sequence succeeded from the installed binaries.
- Brave-only isolation proof used the installed MCP binary with
  `SKY_CUA_SERVICE_SOCKET_PATH=/tmp/sky-cua-brave-only-installed-bex-1329657.sock`
  and `SKY_CUA_BROWSER=brave`. `browser_list_tabs(user_chrome)` found
  exactly one `chamber.heliasar.com` tab, `browser_claim_tab` claimed tab
  `221674306`, and `browser_snapshot` returned title `Dot Agents | OpenChamber`
  plus URL `https://chamber.heliasar.com/` without broad Chrome-family probing.

Broader release checks from 2026-06-05:

```bash
cargo test -p sky-cua-platform -p sky-cua-client -p sky-cua-service
cargo clippy -p sky-cua-platform -p sky-cua-client -p sky-cua-service --all-targets -- -D warnings
uv run ruff format --check scripts resources/chrome_preflight.py
uv run ruff check scripts resources/chrome_preflight.py
uv run basedpyright
uv run pytest
cargo fmt --check
git diff --check
python3 scripts/build_plugin.py
```

Installed release proof from 2026-06-05:

- `/home/bex/.local/share/sky-cua/opencode.json` exports
  `SKY_CUA_BROWSER=brave` when that selector is present during installation.
- `/home/bex/.local/share/sky-cua/pi_mcp_wrapper.sh` preserves the same Brave
  filter when present during installation.
- `~/.pi/agent/mcp.json` contains the merged `sky_cua` entry while preserving
  existing MCP servers.
- Direct MCP probing through `/home/bex/.local/share/sky-cua/bin/sky-cua-client`
  listed both browser tools, returned `browser_status.isError=false` with zero
  diagnostics, and returned `browser_list_tabs(user_chrome).isError=false` with
  141 tabs and zero diagnostics.

Local Pi text-only AppShot proof from 2026-08-09:

- Pi 0.84.1 selected `opencode/deepseek-v4-flash-free`, whose model catalog
  reports `images: no`, and called `sky_cua_observe` once with
  `{"surface":"desktop"}`.
- The installed Pi wrapper connected through the merged local MCP config and
  returned a desktop AppShot with text content only: zero MCP image blocks,
  zero base64 image fields, no tool error, and empty stderr.
- The standalone installer also projects the managed
  `sky-cua-image-capability.ts` Pi extension. It reads the active Pi model's
  `input` modalities on every tool result and removes image content blocks
  before provider submission unless that model explicitly lists `image`.
  Semantic text remains unchanged, and unknown modality metadata fails closed.
- Evidence: `artifacts/pi-text-only-appshot-local-smoke/20260809T1946explicit/`
  (`pi-steady.stdout.log`, `pi-steady.stderr.log`).

Browser observe model-capability proof from 2026-08-09:

- Pi 0.84.1 with `opencode/deepseek-v4-flash-free` claimed an existing Chrome
  tab and completed `observe(surface="browser")` with text content only, zero
  image blocks, and no base64 field. The post-change installed run exposed the
  literal bounded model-facing semantic snapshot, including 298 actionable
  elements with names, bounds, and opaque refs; its 32 KiB fallback truncated
  safely while the complete AppShot remained in structured/artifact storage.
- Codex CLI 0.147.0 does not advertise a model id or image capability in its MCP
  initialize request, but it does send the active model in
  `_meta.x-codex-turn-metadata.model` on each tool call. sky-cua uses that
  per-turn model after explicit capability and process overrides. A local
  an image-capable Codex browser observe automatically returned one image content block
  while structured content remained free of base64 and the redundant text-only
  semantic fallback was absent; no environment override was used.
- Evidence: `artifacts/model-image-gating-local-smoke/20260809T2014/`
  (`pi-browser-claim-observe.stdout.log`,
  `codex-browser-claim-observe.stdout.log`, and
  `codex-browser-auto-capability.stdout.log`).

Live `browser_open` proof from 2026-06-05:

- Fresh debug binaries `target/debug/sky-cua-client` and
  `target/debug/sky-cua-service` were used with isolated
  `SKY_CUA_SERVICE_SOCKET_PATH=/tmp/opencode/sky-cua-browser-open-live.sock`.
- MCP `tools/list` advertised `browser_open`.
- MCP `browser_open({"target":"user_chrome","url":"about:blank"})`
  returned `isError=false`, a `user_chrome` tab id, `active=true`, and
  `url="about:blank"`.
- A follow-up MCP `browser_list_tabs({"target":"user_chrome"})` returned
  `isError=false` and included the opened tab id in the real Brave tab list.
- The isolated debug service process was killed after the smoke.

Live native-host ownership proof from 2026-06-05:

- Artifact: `artifacts/chrome-host-smoke/20260605T205549Z/result.json`.
- Command: `python3 scripts/live_chrome_host_client_smoke.py --install-temp-native-manifest --mcp-list-tabs-proof --skip-cursor-proof --skip-turn-ended-proof` after rebuilding `target/debug/sky-cua-chrome-host`, `target/debug/sky-cua-client`, and `target/debug/sky-cua-service`.
- The smoke kept a primary browser client connected, created a tab visible to
  `session_id="sky-cua-mcp"`, called MCP `browser_list_tabs(user_chrome)` as a
  second client, and found the expected tab id with zero diagnostics.
- After the MCP client exited, the extension-originated heartbeat still routed
  to the original primary client and received `pong`; the temporary native
  manifest was restored.

## Known limitations

- Browser tools currently expose readiness, `user_chrome` tab listing,
  creation/navigation of session-owned `user_chrome` tabs, existing-tab
  claiming, browser cursor movement, snapshots, screenshots, click, text entry,
  key dispatch, and page scrolling. Unified-control-plane installed Codex
  acceptance covers navigation, click, typing, scroll, screenshots, and two
  simultaneous Codex connections. Direct-caller overlap and the remaining
  control-plane gates are follow-up work (the Codex adapter is owned by the
  codex-desktop repo per
  [`docs/runtime/compat-plugin-contract.md`](../runtime/compat-plugin-contract.md));
  managed browser launch was retired on 2026-06-11 and removed from the
  contract.
- Real-browser tab listing depends on the Codex Chrome extension/native-host
  socket already being connected in the selected Chrome-family browser.
- `browser_claim_tab(user_chrome)` can adopt existing tabs and can recover tabs
  stuck in stale `sky-cua-*` sessions. It intentionally does not reclaim tabs
  owned by non-sky-cua browser sessions. Browser actions require a tab returned
  by `browser_open` or successfully adopted by `browser_claim_tab`.
- `SKY_CUA_BROWSER=brave` is a host preference, not a security
  sandbox. The runtime avoids querying unmatched browser families, but the
  selected browser still controls what its extension exposes.

## Related

- [`ROADMAP.md`](../../ROADMAP.md) — Host portability phase: Codex Desktop
  compat materialization follow-up.
- [`docs/features/codex-desktop-compat.md`](codex-desktop-compat.md) — Codex
  companion Browser Use and Chrome plugin compatibility.
- [`Unified browser bridge control plane`](unified-browser-bridge-control-plane.md)
  — persistent routing, identity, scheduling, recovery, migration, accepted
  Codex evidence, and remaining gates.
