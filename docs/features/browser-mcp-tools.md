# Browser MCP tools

## Status

Shipped for `user_chrome`, the only browser target. The managed/isolated
browser target was retired by decision on 2026-06-11 (controlling the user's
real logged-in browser is the product) and its contract stub has been removed
from the wire contract. Last verified: 2026-06-15 with focused Rust browser
MCP tests, root `cargo test`, and plugin build/install. Installed-MCP
agent-loop liveness was separately verified on 2026-06-15; live browser smoke
remains the 2026-06-08 Brave MCP/native-host smoke. With
`SKY_CUA_BROWSER=brave`, a full isolated MCP smoke advertised the browser tools,
opened a session-owned Brave tab, navigated it to a local HTTP fixture, captured
a snapshot and screenshot, moved the browser cursor, clicked, typed, pressed a
key, scrolled, and navigated back to `about:blank`.

## Summary

`sky-cua` exposes browser readiness, real user-tab listing, session-owned tab
creation, existing-tab claiming, browser snapshots/screenshots, and basic
browser actions as first-class MCP tools for hosts such as OpenCode and Pi. The
default browser MCP surface is a core sky-cua capability and is always
advertised by the MCP server; `browser_eval` is an opt-in diagnostic exception.

## Contract surface

MCP browser tools:

- `browser_status` returns structured browser readiness, available targets,
  diagnostics, and optional known-tab count.
- `browser_list_tabs` accepts optional `target`, currently `user_chrome`, plus
  optional `url_contains` and `title_contains` filters for the human-readable MCP
  summary and structured response. When filters are present, only matching tabs
  are returned to avoid leaking unrelated tab titles/URLs through
  `structuredContent`; call without filters only when a broad tab inventory is
  actually needed. It returns tabs plus diagnostics. `user_chrome` is the only
  target value.
- `browser_open` accepts optional `target=user_chrome` and optional `url`. It
  creates a new session-owned tab through the Chrome-family bridge, attaches it
  to the sky-cua browser session, enables CDP Page events, and navigates when a
  URL is provided. Allowed URL forms are `http://`, `https://`, and
  `about:blank`.
- `browser_claim_tab` accepts `target=user_chrome` and `tab_id` from
  `browser_list_tabs`. It asks the extension to adopt that existing user tab into
  the sky-cua browser session for subsequent browser actions.
- `browser_move_mouse` accepts `target=user_chrome`, `tab_id`, `x`, `y`, and
  optional `wait_for_arrival`. Coordinates are CSS pixels, the same space as
  `browser_screenshot` image pixels and `browser_snapshot` element bounds.
  When omitted, `wait_for_arrival` defaults to true in both the MCP argument
  parser and the service request contract.
- `browser_navigate` accepts `target=user_chrome`, `tab_id`, and `url`. Allowed
  URL forms are `http://`, `https://`, and `about:blank`.
- `browser_snapshot` accepts `target=user_chrome`, `tab_id`, and optional
  `element_query`, `element_offset`, `element_limit`, and `text_limit`, then
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
  raises or lowers the cap up to the service maximum of 5000.
- `browser_screenshot` accepts `target=user_chrome` and `tab_id`, then captures
  the visible viewport, normalizes the image to CSS-pixel dimensions, and
  re-encodes it with the shared model-screenshot knobs (JPEG by default, WebP
  via `SKY_CUA_MODEL_SCREENSHOT_FORMAT=webp`). The MCP result attaches the
  image as an MCP image content block when the session's model supports image
  input. For text-only sessions, the service still persists the capture but
  omits response image data and returns `screenshot_path`, `mime_type`, and
  `width`/`height` only. The base64 payload is never repeated inside
  `structuredContent`.
- `browser_click` accepts `target=user_chrome`, `tab_id`, `x`, and `y` in CSS
  pixels, matching `browser_screenshot` image pixels and `browser_snapshot`
  element bounds. Before dispatching the CDP click, the service moves the
  browser agent cursor to the same point and waits for arrival.
- `browser_type_text` accepts `target=user_chrome`, `tab_id`, and non-empty
  `text`, then inserts text into the focused page control.
- `browser_press_key` accepts `target=user_chrome`, `tab_id`, and non-empty
  `key`, then dispatches the key to the page. Single CDP key names and modifier
  chords such as `Ctrl+K`, `Ctrl+L`, `Shift+Tab`, and `Meta+K` are accepted.
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
  The tool is disabled by default: running arbitrary JavaScript in real
  signed-in user tabs crosses a stronger trust boundary than visible UI
  automation (hidden DOM, storage, same-origin requests) and amplifies prompt
  injection. The operator enables it explicitly with `SKY_CUA_BROWSER_EVAL=on`
  (or `1`/`true`), enforced at both layers: when disabled the client does not
  advertise it in `tools/list` and rejects direct calls, and the service — the
  real CDP execution boundary — independently rejects `BrowserRequest::Eval`
  with a `BrowserEvalDisabled` diagnostic so a direct service-socket caller
  cannot bypass the opt-in. A thrown or rejected expression surfaces as a
  `BrowserEvalException` diagnostic instead of a silent `null` value.

Browser targets:

- `user_chrome` means an already-running user Chrome-family browser: Brave,
  Google Chrome, or Chromium. sky-cua reaches it through the Codex Chrome
  extension and native-host socket, then calls `getUserTabs` to enumerate the
  user's real tabs. Any other `target` string is rejected at argument parsing
  with an explicit error.

`browser_claim_tab` is the explicit adoption seam for existing `user_chrome`
tabs from `browser_list_tabs`. Browser actions should target tabs returned by
`browser_open` or successfully adopted by `browser_claim_tab`; callers should
not assume every listed tab is already controllable.

All browser tool coordinates are CSS pixels and share one space:
`browser_screenshot` image pixels, `browser_snapshot` element bounds, and
`browser_click`/`browser_move_mouse`/`browser_scroll` coordinates line up
one-to-one. They are not desktop screen coordinates and they are not
coordinates from `get_app_state` screenshots. The service normalizes high-DPI
captures to CSS-pixel dimensions at capture time, so agents never divide by
`window.devicePixelRatio` and the center of a returned snapshot element can be
passed directly to `browser_click`. Screenshots cover the currently visible
viewport only; scroll and re-capture when the target is off-screen.

Recommended agent flow:

- Use `browser_status` first when diagnosing bridge readiness.
- Use `browser_open` for a new controllable tab, or `browser_list_tabs` followed
  by `browser_claim_tab` for an existing user tab. When many tabs are open, pass
  `url_contains` or `title_contains` to make the text response list relevant tab
  ids instead of only a count.
- Use `browser_snapshot` to inspect page title, URL, viewport, visible text,
  and common actionable element summaries with click-ready CSS-pixel bounds. Pass `element_query: "update"` or an `element_offset`/`element_limit`
  window when a page contains many controls. Its MCP text summary also includes
  these details for text-only hosts.
- Use `browser_click`, `browser_type_text`, `browser_press_key`,
  `browser_scroll`, and `browser_move_mouse` against the tab returned by
  `browser_open` or `browser_claim_tab`. The service self-recovers once when the
  native-host bridge reports that the tab fell out of `sky-cua-mcp` session
  ownership or that the debugger is no longer attached.
- Use `browser_screenshot` when visual proof is needed; the image arrives as an
  MCP image content block for image-capable sessions and is also persisted to
  the file named in `structuredContent.screenshot_path`. Text-only agents should
  prefer `browser_snapshot`.

Environment variables:

- `SKY_CUA_BROWSER` restricts real-browser socket selection for `user_chrome`;
  accepted values are `brave`, `chrome`, `chromium`, and `all`/unset.
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
  `Click { target, tab_id, x, y }`, `TypeText { target, tab_id, text }`,
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
  `~/.pi/agent/skills` without replacing unrelated Pi MCP servers.
- `scripts/install_mcp_server.py --host openclaw` writes `openclaw_mcp.json`,
  registers the `sky_cua` stdio server through `openclaw mcp set`, and copies
  sky-cua skills into `~/.openclaw/workspace/skills`.
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

`browser_status` combines the existing runtime doctor browser report with
browser-bridge diagnostics. When a matching native-host socket is connected,
status does not emit a disconnected diagnostic. If the browser selection env is
invalid, the report returns an explicit diagnostic instead of guessing. If the
desktop request lane is already busy, status still returns bridge diagnostics
and marks browser integration checks as deferred instead of waiting behind the
desktop action.


`browser_list_tabs(user_chrome)` discovers Unix sockets from the Chrome-family
native messaging host, filters them by `SKY_CUA_BROWSER`, and calls the Codex
extension's `getUserTabs` method. This is intentionally different from
`getTabs`, which lists session-owned Codex tabs rather than the user's real
browser tabs. Tab titles and URLs are structured runtime data. MCP text output
shows at most a small bounded set of tabs. `url_contains`/`title_contains` also
filter `structuredContent.tabs`, so a targeted lookup does not expose hundreds
of unrelated tab titles and URLs to text-only agents, logs, or transcripts.

The native host treats clients as either a primary Browser Use client or an
ephemeral sky-cua MCP client. Primary clients receive extension-originated
requests such as heartbeat pings. MCP browser-tool calls connect as short-lived
ephemeral clients using `session_id="sky-cua-mcp"`; they can send requests to the
extension, but they do not evict the primary client and do not become the target
for extension-originated requests.

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

`browser_navigate`, `browser_snapshot`, `browser_screenshot`, `browser_click`,
`browser_type_text`, `browser_press_key`, and enabled `browser_eval` use
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
`Input.*` events with CSS-pixel coordinates passed through unchanged. The
enabled `browser_eval` path uses `Runtime.evaluate` with `awaitPromise=true`
and `returnByValue=true`.
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
reset, the original action is retried once — except after a command timeout
for actions that mutate the page (click, type, key, navigate, eval, scroll):
the timed-out command may still have executed in the browser, so replaying it
could double the input. Those calls surface the timeout diagnostic (with a
note that the session was reset) instead; snapshot, screenshot, and absolute
cursor moves are replayed. Failures after the retry are surfaced as
diagnostics rather than looping indefinitely. Each `executeCdp` request
carries a `timeoutMs` derived from the remaining call deadline (capped at the
extension's 10-second default, and shrunk below the 250 ms floor when the
deadline is nearly exhausted), so the bridge returns a structured timeout
before the service abandons the socket read.

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

Browser tools no longer require a host-specific enable flag. Codex Desktop may
still use the companion Browser Use and Chrome plugins until the adapter
delegates browser actions through the shared runtime.

## Verification

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
  CDP runtime value, and `browser_press_key` dispatches modifier chords with CDP
  modifier bits.
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
- Client registry tests prove `browser_eval` stays unadvertised by default, is
  advertised only with the explicit opt-in, rejects calls when disabled, and is
  routed through the Browser MCP service request/response envelope; a service
  test proves thrown expressions become `BrowserEvalException` diagnostics.

Focused hardening checks from 2026-06-08:

```bash
cargo test -p sky-cua-service
cargo clippy -p sky-cua-service --all-targets -- -D warnings
cargo fmt --check && cargo test
cargo test -p sky-cua-client -- --test-threads=1
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
  key dispatch, and page scrolling. Codex Desktop adapter delegation remains
  follow-up work (owned by the codex-desktop repo per
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
