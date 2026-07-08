# Browser element-target interaction (click and type by identity, not pixels)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document must be maintained in accordance with `PLANS.md` at the repository root (`/home/bex/projects/sky-cua/PLANS.md` if present; otherwise the copy at `~/.agents/PLANS.md`), and retired per `plans/AGENTS.md` when the work lands and is proven.


## Purpose / Big Picture

Today an agent driving a web page through sky-cua can only click and type by raw CSS-pixel coordinates. The browser observation tool (`observe(surface="browser")`) already returns a list of actionable page elements, each with a position, a role, an accessible name, and a numeric index — but the input tool (`browser_input`) accepts only `x`/`y` numbers. So to click a button the agent must read the button's rectangle from the observation, compute its center, and send those pixels. On a static page this is fine. On a modern web app whose layout re-renders and reflows constantly (a grocery store, a dashboard, an infinite-scroll grid), the rectangle the agent read a moment ago is stale by the time the click is sent, and the click lands on empty space or the wrong control.

We have direct evidence of the cost. In a real session on 2026-07-08 an agent trying to build a shopping cart on a dynamic grocery site made 37 `browser_input` calls (32 of them coordinate clicks) and 9 `browser_eval` calls. Every one of those 9 eval calls was the agent reimplementing element discovery in raw JavaScript — enumerating elements with their text and `getBoundingClientRect()`, filtering by visibility, and in several cases calling `element.click()` directly by text — because the tool surface gave it no way to say "click that element." Raw `element.click()` is worse than a real click: many sites (login forms, bot checks) reject synthetic clicks that lack a trusted mouse-event sequence.

After this change, an agent can pass an element reference obtained from the observation instead of coordinates. It calls `observe(surface="browser")`, gets back elements that each carry an opaque `ref` token, and then calls `browser_input(operation="click", tab_id=..., ref="<token>")`. The sky-cua service re-locates that element in the *live* page at click time, scrolls it into view if needed, confirms nothing is covering it, and dispatches a real trusted mouse click at the element's current center — the same trusted-input path used today, just aimed by identity instead of by a number the agent had to compute and that may have gone stale. The same applies to typing: `browser_input(operation="type_text", tab_id=..., ref="<token>", text=...)` focuses the referenced field and types into it in one atomic step, removing the "click to focus, then type, and hope focus landed" dance that has misfired before (for example, a two-factor code that "didn't land because focus stayed elsewhere" in an earlier session).

You can see it working by opening any page with sky-cua's browser bridge, calling `observe`, taking a `ref` from the returned elements, and calling `browser_input` with that `ref`; the referenced control activates, and the returned diagnostics are empty. When the element has vanished or is covered, the call returns a clear, structured diagnostic that tells the agent to re-observe, instead of silently clicking nothing.

This plan delivers the capability in two implementation styles, staged as separate milestones so the safer one ships first and the more robust one is an internal upgrade behind the same agent-facing contract:

- Milestone 1 (Option 1, "re-resolve by signature"): the element reference is re-located using only the two CDP capabilities sky-cua already uses today — `Runtime.evaluate` (running JavaScript in the page) and `Input.dispatchMouseEvent` (synthesizing a mouse event). No new browser-extension capability and no new CDP domain. This is the primary deliverable.

- Milestone 2 (Option 2, "resolve by node id"): the element reference is re-located using the CDP DOM domain (`DOM.getBoxModel`, `DOM.scrollIntoViewIfNeeded`) keyed on a Chrome DevTools `backendNodeId`, which is bound to the actual DOM node and survives re-renders that a position-based match cannot. This is an internal robustness upgrade promoted only if Milestone 1's signature matching proves too fragile in live use. Because the agent-facing `ref` is opaque, this swap changes no wire contract and no tool schema.

Term definitions used throughout, in plain language:

- "CDP" is the Chrome DevTools Protocol: the low-level command language a debugger uses to drive a Chrome-family browser (Chrome, Brave, Chromium). Commands look like `Runtime.evaluate` (run JS), `Input.dispatchMouseEvent` (send a mouse event), `DOM.getBoxModel` (get an element's geometry).
- "The bridge" is the local socket transport connecting sky-cua's service to the Codex browser extension's native host. sky-cua speaks JSON-RPC over a Unix socket found under `/tmp/codex-browser-use/`; the extension relays CDP commands to the page. sky-cua does not own the extension; it can only send what the extension forwards.
- "The service" is the sky-cua daemon process (`sky-cua-service`), a long-lived Rust program that receives tool requests and drives the browser over the bridge. Its browser code lives under `crates/sky-cua-service/src/browser/`.
- "The client" is the sky-cua MCP process (`sky-cua-client`), the Rust program that speaks the Model Context Protocol to a host (Claude, Codex, OpenClaw) and forwards structured requests to the service. Its browser tool code lives under `crates/sky-cua-client/src/mcp_tools/`.
- "MCP" is the Model Context Protocol, the JSON tool interface an AI host calls. `browser_input`, `observe`, and `browser_open` are MCP tools sky-cua advertises.
- "A ref" (element reference) is an opaque text token this plan introduces. The observation tool puts one on each returned element; the agent passes it back to the input tool. The agent never parses it. Internally it is a compact, self-describing encoding of how to re-find the element.


## Progress

- [x] (2026-07-08) Milestone 0 — Shared contract and the JavaScript/Rust resolution boundary. Committed as `9779a9c`: `ClickElement`/`TypeTextElement` wire variants carrying an opaque `element_ref`, the `BrowserElementUnresolved`/`BrowserElementNotActionable` diagnostic codes, the `resolve` module with the `resolve_element_center` signature, bridge stubs, daemon routing, client dispatch, protocol ids, and the wire round-trip test.
- [x] (2026-07-08) Milestone 1 — Option 1 element targeting (re-resolve by signature), the primary deliverable. Built as four parallel Opus streams in isolated worktrees, then integrated on main (`3d5b377`).
  - [x] Stream 1A — Snapshot emits a base64url `ref` per element; the stateless resolver `Runtime.evaluate` re-finds an element by signature, scrolls it into view, hit-tests, and returns its live center with a `reason`. (`ef02717`; widened `resolve_element_center` with a `tab_id_value` param, reconciled at integration.)
  - [x] Stream 1B — Service input path gains `BrowserCdpAction::ClickElement`/`TypeTextElement` that resolve then dispatch through the shared `dispatch_click_at` trusted-click helper, surfacing the unresolved/not-actionable diagnostics before any input. (`7390189`)
  - [x] Stream 1C — Client `browser_input` schema accepts a `ref` as a mutually-exclusive alternative to `x`/`y` on `click` (oneOf) and optionally on `type_text`; both golden fixtures regenerated. (`c6bc2b0`)
  - [x] Stream 1D — Feature doc and browser-use skill document element targeting and when to prefer it. (`1e6fb2b`)
  - [x] Milestone 1 integration — all four branches merged cleanly (disjoint files); the one semantic seam (resolver signature) reconciled; full gate green (1075 Rust + 690 Python tests, clippy/fmt clean); deployed; live-smoked on real pages (see Outcomes).
- [ ] Milestone 2 — Option 2 element targeting (resolve by `backendNodeId` via the DOM domain), promoted only if Option 1 proves fragile in live use. Not started; Option 1 is live and working, so this is deferred pending observed fragility.
- [ ] Retirement — feature doc + roadmap update per `plans/AGENTS.md`, delete this plan. Deferred until Milestone 2 is decided (promote or drop).


## Surprises & Discoveries

- Observation: The Codex browser extension imposes no allow/deny list on CDP methods; it is a transparent relay. Any CDP command sky-cua sends is forwarded verbatim to the page's debugger session. This is what makes element resolution feasible entirely on sky-cua's side with no extension change.
  Evidence: the extension's dispatch function (deminified from `resources/chrome-extension/codex/1.1.5_0/background.js`) special-cases only `Target.getTargets` and otherwise calls `chrome.debugger.sendCommand(target, method, commandParams)` directly with the caller's `method`/`commandParams`. sky-cua already exercises three CDP domains through this relay: `Runtime.evaluate` (snapshot/scroll/eval in `crates/sky-cua-service/src/browser/snapshot.rs` and `cdp.rs`), `Page.*` (navigate/capture/enable), and `Input.dispatchMouseEvent` (click). The `DOM.*` domain rides the same relay and is available to Milestone 2 with no extension work.

- Observation: The snapshot already assigns each element an `index`, but that index is the element's position within a fixed CSS selector query at snapshot time, not a durable handle. Re-running the same query after a re-render can shift positions, so the index alone is not a safe re-resolution key.
  Evidence: `crates/sky-cua-service/src/browser/snapshot.rs`, `BROWSER_SNAPSHOT_EXPRESSION_TEMPLATE`, computes `index` as the loop position over `document.querySelectorAll('a,button,input,textarea,select,[role="button"],[role="link"],[contenteditable="true"]')`.

- Observation: sky-cua drives the browser over ephemeral, per-operation bridge connections; there is no per-tab, server-held cache of the last snapshot's elements. Therefore the `ref` must be self-contained: it must carry everything needed to re-find the element, and resolution must be stateless.
  Evidence: each browser operation opens and uses its own `UnixStream` in `crates/sky-cua-service/src/browser/executor.rs` (`run_operation_on_socket`); there is no element registry keyed by tab across calls.


## Decision Log

- Decision: The agent-facing element handle is a single opaque `ref` string, emitted by `observe` on each element and accepted by `browser_input`. The agent never parses it.
  Rationale: Keeping the handle opaque lets Milestone 2 change the internal encoding (from a signature descriptor to a `backendNodeId`) with zero change to the wire contract, the MCP tool schema, or agent behavior. It also prevents agents from constructing refs by hand, which would couple them to internal details.
  Date/Author: 2026-07-08, design owner.

- Decision: Element targeting is added as an optional, mutually-exclusive alternative to `x`/`y` on the existing `click` and `type_text` operations, not as new operations.
  Rationale: The agent's mental model stays "click a thing" / "type into a thing." Coordinates remain valid and backward-compatible for cases where the agent genuinely has a pixel target (a canvas, a map). The schema expresses the exclusivity with a `oneOf` on the operation branch.
  Date/Author: 2026-07-08, design owner.

- Decision: Element resolution re-locates the element and then dispatches a real `Input.dispatchMouseEvent` at the element's current center; it never calls `element.click()` in JavaScript.
  Rationale: Synthetic `element.click()` produces an untrusted event that login forms and bot checks reject, and it bypasses the visible agent-cursor movement. Resolving geometry and dispatching a real event preserves trusted input and the on-screen cursor glide, and is strictly fresher than the agent's manual read-bounds-then-click.
  Date/Author: 2026-07-08, design owner.

- Decision: Resolution is stateless; the `ref` self-contains the re-find recipe. No server-side element cache.
  Rationale: sky-cua's per-operation ephemeral bridge connections have no shared per-tab state, and adding one would introduce staleness and lifecycle bugs. A self-contained ref keeps each call independent.
  Date/Author: 2026-07-08, design owner.

- Decision: Ship Option 1 (signature re-resolution, Runtime.evaluate only) first; treat Option 2 (DOM domain, backendNodeId) as a later, promotable robustness upgrade.
  Rationale: Option 1 uses only CDP sky-cua already exercises, so it carries the least integration risk with the extension we do not control. Option 2 adds a new CDP domain; it should be justified by observed fragility, not adopted speculatively.
  Date/Author: 2026-07-08, design owner.


## Outcomes & Retrospective

Milestone 1 (2026-07-08): delivered and live-proven. An agent can now click and type by element identity. Live smoke against the deployed runtime and a real Chrome-family browser:

    observe(surface=browser) on example.com returned an element with a 387-char opaque `ref`.
    browser_input(operation=click, tab_id, ref) on the "Learn more" link -> no error, no diagnostics;
      the page navigated example.com -> iana.org, proving the click landed on the referenced element
      (not coordinates).
    Reusing the now-stale ref after navigation -> isError=true, diagnostic code BrowserElementUnresolved
      (clean "re-observe" signal, not a silent miss).
    browser_input(operation=type_text, tab_id, ref, text) into DuckDuckGo's search field ->
      "sky-cua element targeting" landed in that field with no separate focus click (confirmed by reading
      the element value back via a second observe).

This matches the Purpose: element-by-identity targeting works, it re-resolves the live position at action time (so it does not suffer the stale-coordinate miss), and failures are structured rather than silent. Whether `browser_eval`-for-element-discovery usage actually drops is a behavioral outcome to observe in future agent sessions.

Process note: the four streams were partitioned by file so parallel agents never collided; the single planned seam (Stream 1B calling Stream 1A's resolver) surfaced exactly as anticipated when 1A widened the resolver signature with a `tab_id_value` parameter, and integration was a one-line reconcile per call site plus removing two now-obsolete `#[allow(dead_code)]` markers. The opaque-`ref` decision held: nothing agent-facing depends on the internal encoding, so Milestone 2 remains a pure internal swap.

Remaining: Milestone 2 (backendNodeId via the DOM domain) is deferred; promote it only if live use shows the signature matcher returning `not_found`/`ambiguous` too often on real pages.


## Context and Orientation

This section names every file and symbol the implementer touches. Read the named files before editing; they are small and cohesive.

The browser subsystem lives in `crates/sky-cua-service/src/browser/`. The relevant files:

- `snapshot.rs` holds `BROWSER_SNAPSHOT_EXPRESSION_TEMPLATE`, a JavaScript string run in the page via `Runtime.evaluate` to enumerate elements, and `snapshot_evaluate_params`, the Rust that fills the template's placeholders (`__TEXT_LIMIT__`, `__ELEMENT_OFFSET__`, `__ELEMENT_LIMIT__`, `__ELEMENT_QUERY__`). Each element object it returns has `index`, `tag`, `role`, `name`, `value`, `href`, `disabled`, and `bounds`. This is where the `ref` field is added (Stream 1A) and where, in Option 2, `backendNodeId` capture would live (Milestone 2).

- `cdp.rs` holds `enum BrowserCdpAction` (the internal action set: `Navigate`, `Snapshot`, `Screenshot`, `Click { x, y }`, `TypeText { text }`, `PressKey { key }`, `Eval`, `Scroll`) and `cdp_action_on_stream`, the async function that turns one action into CDP commands over the bridge. The `Click` arm calls `ensure_focus_emulation` then three `Input.dispatchMouseEvent` commands (`mouseMoved`, `mousePressed`, `mouseReleased`) at `x,y`; the `TypeText` arm calls `ensure_focus_emulation` then `Input.insertText`. New element-targeted variants are added here (Stream 1B).

- `bridge.rs` holds the crate-public entry points `click`, `type_text`, `press_key`, `move_mouse`, `snapshot`, `eval_with_policy`, `scroll`, and helpers that validate arguments and build a `BrowserCdpAction`. Element-targeted entry points are added here (Stream 1B).

- `executor.rs` holds `BrowserBridgeExecutor` and `BoundTabOperation` (the operation wrapper that runs an action on a tab, with the session-recovery and replay-safety logic). It also has `replay_safe`, which decides whether an operation can be re-run after a session reset. Element-targeted click/type are mutating and therefore not replay-safe, exactly like their coordinate counterparts.

- `coordinates.rs` holds `viewport_metrics_until` and the CSS-pixel normalization the screenshot path uses. Element centers returned by the resolver are already in CSS pixels (the same space as `getBoundingClientRect`), matching the coordinate `Click` path, so no scaling conversion is needed.

- `session.rs`, `transport.rs`, `protocol.rs` hold the bridge request plumbing (`execute_cdp_until`, `send_bridge_request_until`, request-id constants). New request-id constants for the resolver evaluate and for element click/type sub-commands are added in `protocol.rs`.

The wire contract lives in `crates/sky-cua-platform/src/model/browser.rs`: `enum BrowserRequest` has `Click { target, tab_id, x, y }`, `TypeText { target, tab_id, text }`, `PressKey { target, tab_id, key }`, and the response types `BrowserActionResponse`, `BrowserSnapshotResponse`. The snapshot element type (the Rust struct the snapshot JSON deserializes into, if any) and the request variants are extended here (Milestone 0). Diagnostic code strings used by the service are also defined in this crate.

The client MCP surface lives in `crates/sky-cua-client/src/mcp_tools/`:

- `definitions/browser.rs` builds the JSON schema for `browser_input` via `browser_input_properties` and `browser_input_constraints`, which currently declare `operation` in `{click, type_text, press_key}` and, for the `click` branch, require `tab_id`, `x`, `y`. The `ref` alternative is added here (Stream 1C).
- `browser.rs` (under `mcp_tools/`) dispatches a parsed `browser_input` call to a service request; `browser_click` parses `x`/`y` via `parse_browser_point`, `browser_type_text` parses `text`. The `ref` routing is added here (Stream 1C).
- `definitions/status.rs` and the golden fixture `crates/sky-cua-client/tests/fixtures/tool_contract.json` pin the advertised tool contract; regenerate the fixture with `SKY_CUA_UPDATE_MCP_FIXTURES=1` after schema changes (Stream 1C).

Documentation: `docs/features/browser-mcp-tools.md` is the feature doc; `skills/browser-use/SKILL.md` is the agent-facing usage guide. Both are updated in Stream 1D.

Test conventions: run Rust tests with `cargo nextest run` (never `cargo test`; the service suite mutates process-global env and binds sockets, and nextest isolates each test in its own process). Browser bridge tests live under `crates/sky-cua-service/src/browser/tests/` and use `UnixListener` fake servers that assert the exact CDP request sequence and feed canned responses; `helpers.rs` there has reusable reply helpers. Python harness tests run with `uv run pytest`. Full local gate: `cargo fmt --check && cargo clippy --workspace --all-targets && cargo nextest run` and `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest`.


## Plan of Work

The work is one small serial contract step followed by a parallel implementation wave, an integration step, and a later optional upgrade. The prose below is the authoritative sequence; the `Parallel Execution Guide` section explains how to farm the wave to independent agents.


### Milestone 0 — Shared contract and the JS/Rust resolution boundary

This milestone lands the interface that every parallel stream compiles and reasons against. It is deliberately small and must land first.

What exists at the end: the wire types, the diagnostic codes, and a written specification of the exact JSON shape that passes between the resolver JavaScript (in the page) and the Rust that calls it. After this, Streams 1A and 1B can be built in parallel against that boundary, and 1C against the wire types.

Concretely:

First, in `crates/sky-cua-platform/src/model/browser.rs`, extend the request contract so `click` and `type_text` can carry an element reference instead of coordinates. Change `BrowserRequest::Click` to make `x`/`y` optional and add an optional `ref` field of type `Option<String>`; add an optional `ref: Option<String>` to `BrowserRequest::TypeText`. Keep the fields additive and `serde(default, skip_serializing_if = "Option::is_none")` so existing coordinate callers and existing serialized forms are unchanged. Add two diagnostic code constants (as string literals used when building `DiagnosticEntry`): `BrowserElementUnresolved` (the ref no longer matches any element on the page) and `BrowserElementNotActionable` (the element was found but is zero-sized, off-screen after a scroll attempt, or covered by another element so a click could not reach it). Document each in a short comment.

Second, add the `ref` field to whatever type represents a snapshot element on the Rust side (if the snapshot JSON is passed through as `serde_json::Value`, no Rust struct change is needed and the `ref` simply appears in the JSON; confirm by reading how `snapshot_from_cdp_response` in `cdp.rs`/`snapshot.rs` handles the element array). The `observe` response must carry `ref` through to the MCP client unchanged.

Third, write the resolver boundary specification directly in this plan (below) and as a doc comment on a new module `crates/sky-cua-service/src/browser/resolve.rs` created with just the constants and function signatures stubbed (returning `unimplemented!()` is not acceptable in committed code; instead commit the signatures with a trivial body that Stream 1A/1B replace, or land the module empty except for the documented contract and let Stream 1A create it — pick one and note it in the Decision Log). The boundary is:

    The ref token (Option 1 encoding): a base64url-encoded, compact JSON object with keys
      { "v": 1, "sel": <string>, "i": <int>, "sig": { "tag": <string>, "role": <string|null>,
        "name": <string|null>, "href": <string|null> }, "b": { "x": <num>, "y": <num>,
        "w": <num>, "h": <num> } }
    where "sel" is the selector base used by the snapshot, "i" is the element's index within
    that selector query at snapshot time, "sig" is the identifying signature, and "b" is the
    element's CSS-pixel bounds at snapshot time (used to disambiguate when the signature matches
    more than one live element). "v" is a version integer so the encoding can evolve.

    The resolver Runtime.evaluate payload receives the decoded ref object as a JSON literal and
    returns, by value:
      { "found": <bool>, "center": { "x": <num>, "y": <num> } | null,
        "reason": "ok" | "not_found" | "ambiguous" | "zero_size" | "offscreen" | "covered",
        "scrolled": <bool> }
    Resolution algorithm: re-run document.querySelectorAll(sel); collect candidates whose
    signature matches "sig" (tag exact; role/name/href equal when present in sig); if none,
    return not_found; if more than one, choose the candidate whose current bounds are closest
    to "b" (Euclidean distance of centers) and only accept it if it is unambiguously closest,
    else return ambiguous; scroll the chosen element into view if its rect is outside the
    viewport (record scrolled=true); recompute its rect; if width or height is ~0 return
    zero_size; if still outside the viewport return offscreen; hit-test document.elementFromPoint
    at the rect center and require the result to be the element or a descendant/ancestor within
    it, else return covered; on success return found=true with the CSS-pixel center.

Fourth, add request-id constants in `crates/sky-cua-service/src/browser/protocol.rs` for the resolver evaluate and the element click/type sub-commands, following the existing `sky-cua-browser-...` naming.

Acceptance for Milestone 0: the workspace compiles (`cargo build`), existing tests pass (`cargo nextest run`), and the wire types serialize round-trip in a unit test that constructs a `BrowserRequest::Click` with a `ref` and no `x`/`y` and asserts it serializes without `x`/`y` and deserializes back. No behavior is user-visible yet.


### Milestone 1 — Option 1 element targeting (re-resolve by signature)

This is the primary deliverable. At the end, an agent can click and type by `ref`, resolution uses only `Runtime.evaluate` and `Input.dispatchMouseEvent`, and the behavior is proven live on a dynamic page. The milestone is four parallel streams plus an integration step.


#### Stream 1A — Snapshot emits refs; resolver JS re-finds an element

In `crates/sky-cua-service/src/browser/snapshot.rs`, extend `BROWSER_SNAPSHOT_EXPRESSION_TEMPLATE` so `elementFor(el, index)` also computes a `ref`: build the ref object described in the Milestone 0 boundary from the element's selector base, `index`, signature (`tag`, `role`, `name`, `href`), and current bounds, JSON-stringify it, and base64url-encode it in the page (a small inline `btoa` with URL-safe substitution; define it once in the payload). Add `ref` to the returned element object. Keep the existing fields.

Create `crates/sky-cua-service/src/browser/resolve.rs` (or fill the stub from Milestone 0). It exposes a Rust function that, given a decoded ref and a mutable bridge stream, runs a `Runtime.evaluate` whose expression is the resolver payload with the ref embedded as a JSON literal, and returns a typed result mirroring the boundary's return shape (`found`, `center`, `reason`, `scrolled`). The resolver JavaScript payload is a string constant in this module implementing the resolution algorithm from the Milestone 0 boundary. Decoding the ref (base64url → JSON) happens in Rust; the decoded object is injected into the payload as a JSON literal so the page code does not need to base64-decode.

Tests: add unit tests that exercise the ref encode/decode round-trip in Rust, and a fake-bridge test (pattern from `crates/sky-cua-service/src/browser/tests/`) that feeds a canned resolver `Runtime.evaluate` response and asserts the Rust maps each `reason` to the right typed outcome. The resolver JavaScript itself is validated end-to-end in the integration step against a real page.


#### Stream 1B — Service input path: element-targeted click and type

In `crates/sky-cua-service/src/browser/cdp.rs`, add element-targeted behavior. Two acceptable shapes; pick one and record it in the Decision Log: either add new `BrowserCdpAction::ClickElement { ref_token }` and `TypeTextElement { ref_token, text }` variants, or thread an optional element target into the existing `Click`/`TypeText` variants. In the action's execution, call the Stream 1A resolver to get the element's live center; if `found` is false, return a `DiagnosticEntry` with code `BrowserElementUnresolved` (reasons `not_found`, `ambiguous`) or `BrowserElementNotActionable` (reasons `zero_size`, `offscreen`, `covered`), including the `reason` and guidance to re-observe in the message. On success, dispatch exactly the existing trusted-input sequence at the resolved center: for click, the same `moveMouse` agent-cursor glide (as the coordinate click does via `bridge.rs`), `ensure_focus_emulation`, then `Input.dispatchMouseEvent` `mouseMoved`/`mousePressed`/`mouseReleased`; for type, focus the element (dispatch a click at the resolved center, or add a focus step) then `Input.insertText`. Reuse the existing helpers rather than duplicating dispatch.

In `crates/sky-cua-service/src/browser/bridge.rs`, add crate-public entry points mirroring `click`/`type_text` that accept a `ref` and build the element-targeted action. Validate that exactly one of `ref` or `x`/`y` is present (the service is the real execution boundary and must reject a malformed pairing even though the client schema also enforces it).

In `crates/sky-cua-service/src/browser/executor.rs`, ensure the element-targeted actions are classified not-replay-safe in `replay_safe` (they mutate the page), matching the coordinate click/type.

In `crates/sky-cua-service/src/daemon/browser.rs`, route the new/extended `BrowserRequest` fields to the new bridge entry points.

Tests: fake-bridge tests asserting the full CDP sequence for a click-by-ref (resolver evaluate → moveMouse → focus emulation → three mouse events) and for a type-by-ref, plus tests for each failure reason mapping to the correct diagnostic code with an empty action effect (no mouse events dispatched when resolution fails).


#### Stream 1C — Client MCP schema and dispatch

In `crates/sky-cua-client/src/mcp_tools/definitions/browser.rs`, extend `browser_input_properties` to declare an optional `ref` string, and `browser_input_constraints` so the `click` branch accepts either `{x, y}` or `{ref}` (a `oneOf` on that branch) and the `type_text` branch accepts an optional `ref` alongside `text`. Describe `ref` in the schema as "an opaque element reference from observe(surface=browser); prefer it over x/y for reliable clicks on dynamic pages."

In `crates/sky-cua-client/src/mcp_tools/browser.rs`, route a `browser_input` call carrying `ref` to the element-targeted service request; keep the `x`/`y` path unchanged when `ref` is absent.

Regenerate the golden contract fixture: `SKY_CUA_UPDATE_MCP_FIXTURES=1 cargo nextest run -p sky-cua-client tool_contract_fixture`, then run the fixture test normally to confirm it passes.

Tests: client tests that a `browser_input(click, ref=...)` call builds the element-targeted service request and that `browser_input(click)` with neither `ref` nor `x`/`y`, or with both, is rejected by schema validation.


#### Stream 1D — Documentation and skill

In `docs/features/browser-mcp-tools.md`, document that `browser_input` `click` and `type_text` accept an opaque `ref` from `observe(surface="browser")` as an alternative to `x`/`y`, how the service re-resolves it (scroll-into-view, hit-test, trusted dispatch at the live center), and the `BrowserElementUnresolved` / `BrowserElementNotActionable` diagnostics with their "re-observe" remedy. In `skills/browser-use/SKILL.md`, add guidance: prefer `ref` over coordinates for clicks and typing on dynamic pages; coordinates remain for canvas/map targets; on an unresolved/not-actionable diagnostic, re-observe and retry rather than falling back to pixel guessing or `browser_eval`.


#### Milestone 1 integration

Merge the streams, run the full local gate, then live-smoke on a real page: with the browser bridge connected, call `observe(surface="browser")` on a page with buttons, take a `ref`, call `browser_input(operation="click", tab_id=..., ref=...)`, and confirm the control activated and diagnostics are empty; scroll the page so the element moves, and confirm a second click-by-ref (using a fresh observe) still lands, where a stale coordinate would have missed; type into a field by `ref` and confirm the text landed without a separate focus click. Deploy with `python3 scripts/deploy_plugin.py` and repeat one live check against the deployed runtime.


### Milestone 2 — Option 2 element targeting (resolve by backendNodeId, DOM domain)

Promote only if Milestone 1's signature matching proves fragile in live use (frequent `ambiguous`/`not_found` on real pages). Because the `ref` is opaque, this is an internal change.

In `snapshot.rs`, capture a `backendNodeId` per element. The snapshot currently returns element data by value from a single `Runtime.evaluate`; obtaining `backendNodeId` requires the DOM domain. One approach: after enabling the DOM domain, use `DOM.getDocument` and `DOM.querySelectorAll` to get node ids for the same selector, or resolve each element object to a node via `DOM.requestNode` from a `Runtime.evaluate` that returns element handles. Encode the `backendNodeId` into the ref (a new ref version `"v": 2`). Keep decoding version-aware so old `v:1` refs still resolve by signature.

In `resolve.rs`, add a DOM-domain resolution path for `v:2` refs: `DOM.scrollIntoViewIfNeeded(backendNodeId)`, `DOM.getBoxModel(backendNodeId)` to get the current quad, compute the center, and hit-test as before. Fall back to the signature path if the node id is stale (the node was removed), so a re-rendered element still resolves.

Tests: fake-bridge tests for the DOM-domain sequence and the stale-node fallback. Live-smoke on a page that re-renders the target between observe and click, confirming the node-id path lands where the signature path previously reported `not_found`.

Acceptance: measurable reduction in unresolved/ambiguous outcomes on the target dynamic pages, with no regression to the `v:1` path.


## Parallel Execution Guide

This plan is structured to be farmed to parallel implementation agents (for example, Opus subagents). The orchestrator owns the serial boundary and the integration; the streams run concurrently.

Sequencing:

1. Land Milestone 0 first, alone. It is small (contract types, diagnostic codes, the resolver boundary spec, request-id constants) and every stream depends on it. Do not parallelize it.

2. After Milestone 0 lands and compiles, launch the Milestone 1 streams concurrently. The safe parallel decomposition, chosen so no two agents edit the same file:
   - Agent A (Stream 1A): edits `crates/sky-cua-service/src/browser/snapshot.rs` and creates `crates/sky-cua-service/src/browser/resolve.rs`. Owns the JavaScript payloads and ref encode/decode.
   - Agent B (Stream 1B): edits `crates/sky-cua-service/src/browser/cdp.rs`, `bridge.rs`, `executor.rs`, and `crates/sky-cua-service/src/daemon/browser.rs`. Owns the input dispatch and diagnostics. Agent B calls the resolver through the Rust function signature fixed in Milestone 0, so it can be written against that signature before Agent A's JavaScript is final; the two meet at integration. To keep them from colliding, Agent A creates `resolve.rs` and its public function signature as part of its first commit, or Milestone 0 creates the empty module with the signature — decide in Milestone 0 and state it so Agent B has a stable import.
   - Agent C (Stream 1C): edits `crates/sky-cua-client/src/mcp_tools/definitions/browser.rs`, `crates/sky-cua-client/src/mcp_tools/browser.rs`, and regenerates `crates/sky-cua-client/tests/fixtures/tool_contract.json`. Depends only on the Milestone 0 wire types, not on A or B.
   - Agent D (Stream 1D): edits `docs/features/browser-mcp-tools.md` and `skills/browser-use/SKILL.md`. Depends on the design, not on code; can start immediately after Milestone 0 and is trivially independent.

   The only true code dependency inside the wave is B→A (B calls A's resolver). Resolve it by fixing the `resolve.rs` public signature in Milestone 0 so both compile independently; integration wires the real JavaScript. A, C, and D share no files with each other or with B.

3. Integration is serial and owned by the orchestrator: merge the four streams, run `cargo fmt --check && cargo clippy --workspace --all-targets && cargo nextest run` and the Python gate, fix any cross-stream seams, then run the live smoke. Only the orchestrator runs the live smoke and deploy.

4. Milestone 2 is a later, separate wave with the same shape (snapshot capture, resolver path, tests) and can itself be split A/B, but it is gated on a decision to promote it.

Guidance for each agent: each stream's tests must pass in isolation before integration. Agents must not weaken the `oneOf` exclusivity, must preserve the opaque-ref contract (no agent-visible parsing of the ref), and must reuse the existing trusted-input dispatch rather than introducing `element.click()`. Any deviation is recorded in the Decision Log by the orchestrator at integration.


## Validation

The change is validated at three levels.

Unit and contract tests (fail before, pass after): the wire round-trip test in Milestone 0; the ref encode/decode and reason-mapping tests in Stream 1A; the CDP-sequence and diagnostic tests in Stream 1B; the schema-and-dispatch tests plus regenerated fixture in Stream 1C. Run with `cargo nextest run` (Rust) and `uv run pytest` (Python harness, for any script-side fixture). The full gate is `cargo fmt --check && cargo clippy --workspace --all-targets && cargo nextest run` and `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest`.

Live behavior (the observable outcome from the Purpose): with the browser bridge connected to a Chrome-family browser, drive the MCP tools directly (the repository's live-smoke pattern spawns `sky-cua-client mcp` and speaks JSON-RPC on stdio; see existing live checks). The passing scenario: call `observe(surface="browser")` on a page with visible controls; pick an element's `ref`; call `browser_input(operation="click", tab_id=<the tab>, ref=<that ref>)`; observe the control activate and the response diagnostics be empty. Then scroll the page, re-observe, and click a moved element by its fresh `ref`, confirming it still lands. Then `browser_input(operation="type_text", tab_id=..., ref=<a text field's ref>, text="hello")` and confirm the field contains the text with no separate focus click. Finally, delete the element via the page (or navigate away), reuse a now-stale `ref`, and confirm the call returns a `BrowserElementUnresolved` diagnostic rather than a silent miss.

Deployed check: after `python3 scripts/deploy_plugin.py`, repeat one click-by-ref against the deployed runtime to confirm the built bundle carries the behavior.


## Idempotence

Every step is additive and re-runnable. The wire fields are optional with `skip_serializing_if`, so re-serializing old requests is unchanged and re-applying the migration is a no-op. The snapshot gains a field; re-running the snapshot is stateless. Resolution is stateless and side-effect-free except the intended scroll-into-view and the click/type it performs, so a failed resolution leaves no residue and can be retried after a re-observe. Regenerating the contract fixture is deterministic (`SKY_CUA_UPDATE_MCP_FIXTURES=1`); running it twice yields no diff. Deploying twice is safe (the deploy reaps and respawns the stack).


## Artifacts

The live-smoke transcripts (the observe→click-by-ref JSON-RPC exchanges) are the evidence of success; capture the key request/response pairs into the `Surprises & Discoveries` or `Outcomes & Retrospective` sections as short indented excerpts, not as committed files. Do not commit raw transcripts or screenshots into the tree. The regenerated `crates/sky-cua-client/tests/fixtures/tool_contract.json` is the one committed artifact that changes shape.


## Interfaces

Agent-facing (stable, opaque): `observe(surface="browser")` elements each gain a `ref` string. `browser_input` `click` accepts either `{x, y}` or `{ref}`; `type_text` accepts optional `ref` alongside `text`. Agents pass `ref` verbatim and never parse it.

Service-internal (may change between Option 1 and Option 2 without agent-facing impact): the `ref` encoding (a versioned base64url JSON object), the resolver `Runtime.evaluate` payload and its `{found, center, reason, scrolled}` return shape, and the `resolve.rs` Rust function that runs it. The diagnostic codes `BrowserElementUnresolved` and `BrowserElementNotActionable` are part of the structured response contract and must remain stable once shipped.
