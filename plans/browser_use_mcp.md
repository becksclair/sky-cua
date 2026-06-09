# First-Class Browser Use MCP Tools

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document follows `/home/bex/.agents/PLANS.md` and `plans/AGENTS.md`.

## Purpose / Big Picture

After this work, agents such as OpenCode, OpenClaw, Pi, and Codex Desktop can use browser automation as a first-class `sky-cua` capability instead of treating browser control as a Codex-only side plugin. The portable host contract is the MCP server exposed by `sky-cua-client mcp`. Codex Desktop still receives a compatibility projection that looks like OpenAI's bundled `chrome` and `browser-use` plugins, but those adapters must not define the core behavior.

The first demonstrable behavior was intentionally small and concrete: when browser MCP tools are explicitly enabled, `tools/list` advertises browser tools, `browser_status` returns structured browser readiness diagnostics, and OpenCode/Pi can be configured to expose that tool group without changing Codex Desktop's default plugin path. The current shipped `user_chrome` slice now includes tab listing, session-owned tab creation/navigation, existing-tab claiming, page snapshots, screenshots, cursor movement, click/type/key actions, and page scrolling. The remaining forward-looking work is managed isolated browser/profile lifecycle and Codex Desktop adapter delegation.

## Progress

- [x] (2026-06-05T15:01Z) Confirmed current architecture: desktop MCP tools exist; Codex browser-use/chrome plugins and Chrome native host exist; first-class `browser_*` MCP tools do not exist.
- [x] (2026-06-05T15:01Z) Fixed adjacent OpenCode selector-default bug before starting browser work: blank `app_id`, `window_title`, and `name` no longer prevent `get_app_state` from matching a non-empty `desktop_file_id`.
- [x] (2026-06-05T15:01Z) Add host-neutral browser model structs in `crates/sky-cua-platform/src/model/browser.rs` and service request/response variants in `crates/sky-cua-platform/src/model/service.rs`.
- [x] (2026-06-05T15:01Z) Add service handling for `BrowserStatus` and `BrowserListTabs` in `crates/sky-cua-service/src/daemon.rs`.
- [x] (2026-06-05T15:01Z) Add gated MCP tools `browser_status` and `browser_list_tabs` in `crates/sky-cua-client/src/mcp_tools.rs`.
- [x] (2026-06-05T15:01Z) Extend installer/config output so OpenCode and Pi opt into browser MCP tools without enabling them for Codex Desktop by default.
- [x] (2026-06-05T15:01Z) Install Pi config under `~/.pi/agent/` and copy sky-cua skills into `~/.pi/agent/skills`.
- [x] (2026-06-05T15:01Z) Verify OpenCode can call `sky_cua_browser_status`; output was `{"ok":true,"tool_used":"sky_cua_browser_status","enabled":true,"targets_count":2,"diagnostics_count":1}`.
- [x] (2026-06-05T15:58Z) Attempt Pi browser-status smoke. The first PTY run timed out with zero output, but later direct `pi -p` and interactive runs proved Pi can call `sky_cua_browser_status`; the timeout was Pi print-mode behavior, not a sky-cua MCP failure.
- [x] (2026-06-05T16:08Z) Add a service-side browser bridge client for `browser_list_tabs` that discovers Chrome native-host sockets, calls `getUserTabs` for `user_chrome`, handles ping frames, maps tab payloads into `BrowserTab`, and preserves `BrowserBridgeDisconnected` diagnostics when no bridge is connected.
- [x] (2026-06-05T15:55Z) Prove installed release `browser_list_tabs` against the user's real Brave profile with `SKY_CUA_BROWSER=brave`; the redacted probe returned 141 Brave tabs, no diagnostics, and did not query Chrome.
- [x] (2026-06-05T16:35Z) Reinstall the release after the final `browser_status` diagnostic fix and prove the installed MCP binary directly: both browser tools were advertised, `browser_status` returned zero diagnostics, and `browser_list_tabs(user_chrome)` returned 141 tabs with zero diagnostics.
- [x] (2026-06-05T16:40Z) Add durable feature documentation in `docs/features/browser-mcp-tools.md` and move the first shipped browser MCP milestone into `ROADMAP.md` while keeping the action/snapshot milestones attached to this active ExecPlan.
- [x] (2026-06-05T16:55Z) Address focused review findings: `browser_status` now probes socket connectability instead of trusting path presence, stale socket status has a regression test, `browser_list_tabs` exposes only `user_chrome` until managed lifecycle is implemented, and docs describe the actual `ServiceResponse::BrowserListTabs { response }` shape.
- [x] (2026-06-05T17:25Z) Prove Pi/OpenCode text fallback for `get_app_state`: Pi receives an `Elements (23)` section for the Brave Certificate Manager window, but the live screenshot at `/run/user/1000/sky-cua/captures/7e4f1b6c-eecc-45fc-9c05-48b886fcb7ab.jpg` shows the visible local-user certificate page itself reports `No certificates` under Trusted, Intermediate, and Distrusted certificate sections.
- [x] (2026-06-05T19:55Z) Add `browser_open` as the first session-owned tab lifecycle slice: the MCP tool creates a new `user_chrome` tab through `createTab`, attaches it to the sky-cua browser session, enables Page CDP, optionally navigates to `http://`, `https://`, or `about:blank`, and returns the created tab metadata.
- [x] (2026-06-05T20:05Z) Live-smoke `browser_open` through MCP against real Brave with fresh debug binaries and an isolated service socket. The call opened `about:blank`, returned `isError=false`, and `browser_list_tabs(user_chrome)` found the opened tab id.
- [x] (2026-06-05T20:47Z) Fix native-host client ownership for first-class browser MCP calls: `sky-cua-mcp` socket clients are request-only and no longer evict or steal routing from the primary Browser Use client.
- [x] (2026-06-05T20:47Z) Bound service-side browser socket discovery: cache browser-family lookups briefly, skip recently disconnected sockets, cap candidates to 32 newest socket paths, cap concurrent probes at 8, and extract this logic into `crates/sky-cua-service/src/browser/sockets.rs`.
- [x] (2026-06-05T20:55Z) Live-smoke native-host ownership after rebuilding debug binaries. `scripts/live_chrome_host_client_smoke.py --install-temp-native-manifest --mcp-list-tabs-proof --skip-cursor-proof --skip-turn-ended-proof` kept the primary client connected, proved MCP `browser_list_tabs(user_chrome)` found the expected `sky-cua-mcp` tab, then proved the extension heartbeat still routed to the primary client. The final run loaded a temporary copy of the extension and wrote `artifacts/chrome-host-smoke/20260605T205549Z/result.json`.
- [x] (2026-06-06T00:00Z) Add browser action tools for session-owned or claimed `user_chrome` tabs: `browser_claim_tab`, `browser_move_mouse`, `browser_navigate`, `browser_snapshot`, `browser_screenshot`, `browser_click`, `browser_type_text`, `browser_press_key`, and `browser_scroll`.
- [x] (2026-06-06T00:00Z) Live-smoke the full browser MCP action surface through release binaries, real Brave, and an isolated service socket. The smoke opened a local HTTP fixture in tab `221675114`, captured a snapshot and PNG screenshot, moved/clicked/typed/pressed/scrolled, then navigated the tab to `about:blank`.
- [x] (2026-06-06T00:00Z) Fix existing-tab critical path: `browser_claim_tab` now recovers tabs stuck in stale `sky-cua-*` sessions, refuses to reclaim non-sky-cua sessions, and attaches/Page-enables claimed tabs so CDP-backed actions work immediately.
- [ ] Add managed browser launch/profile ownership and cleanup.

## Surprises & Discoveries

- Observation: The browser-use and chrome plugin resources are Codex plugin payloads, not an MCP server and not a safe host-neutral API by themselves.
  Evidence: `/home/bex/projects/heliasar-marketplace/plugins/sky-cua/resources/plugins/openai-bundled/plugins/browser-use/scripts/browser-client.mjs` contains bundled command handlers that expect a Codex plugin runtime context; it is not a simple CLI.
- Observation: The existing doctor report already contains a browser integration slot with Chrome-family binary checks and native-host manifest status.
  Evidence: `BrowserIntegrationReport` in `crates/sky-cua-platform/src/model.rs` and `browser_report()` in `crates/sky-cua-linux/src/doctor.rs`.
- Observation: Browser MCP tools are now a core `sky-cua` MCP surface and are always advertised by `sky-cua-client mcp`.
  Evidence: Browser readiness now lives in `browser_status` diagnostics instead of a host-specific enable flag.
- Observation: The Chrome native-host socket already exposes the usable transport for `browser_list_tabs`.
  Evidence: `crates/sky-cua-chrome-host/src/frame.rs` uses native 4-byte length-prefixed JSON frames, and `scripts/live_chrome_host_client_smoke.py` already proves `getTabs` over the Unix socket.
- Observation: Explicit browser socket-dir env vars must be authoritative.
  Evidence: A service test with an empty `SKY_CUA_BROWSER_USE_SOCKET_DIR` initially fell through to `/tmp/codex-browser-use` and found an unrelated live socket; discovery now uses the default only when neither `SKY_CUA_BROWSER_USE_SOCKET_DIR` nor `CODEX_BROWSER_USE_SOCKET_DIR` is set.
- Observation: `getTabs` and `getUserTabs` are different extension semantics.
  Evidence: `getTabs` only returned session-owned Codex tabs, while the user's real browser tabs appeared through `getUserTabs`; `browser_list_tabs(user_chrome)` now calls `getUserTabs`.
- Observation: Multiple Chrome-family sockets can be live at once, and querying the wrong one can trigger a browser remote-control/debugger prompt.
  Evidence: Brave and Chrome both exposed `/tmp/codex-browser-use/extension-*.sock` listeners. Unfiltered enumeration queried both; `SKY_CUA_BROWSER=brave` restricts socket selection to Brave's native-host parent process.
- Observation: `get_app_state` text fallback is enough for text-only hosts to see element summaries, but not enough to recover DOM text when Brave exposes only a KWin native-window fallback.
  Evidence: Pi received `Elements (23)` for `Certificate Manager - Brave` with `AccessibilityCoverageLimited`; the fallback elements were geometric anchors, not AT-SPI rows. Direct image inspection of the capture showed the page was on `chrome://certificate-manager/localcerts/usercerts` and visibly listed `No certificates` in all displayed local-user sections.
- Observation: The current Chrome extension bridge refuses CDP access to existing `user_chrome` tabs that were not created for the caller's browser session.
  Evidence: Raw `executeCdp` and `attach` probes against the visible Certificate Manager tab returned `Tab 221671319 is not part of browser session sky-cua-cert-proof`, even though `getUserTabs` could enumerate that tab. Later browser action milestones need an explicit adopt-existing-tab design or must operate only on session-owned tabs.
- Observation: A useful first action slice can reuse the user's selected Chrome-family browser without pretending existing tabs are controllable.
  Evidence: `browser_open` uses the bridge's `createTab` path, then `attach` and `executeCdp` against that newly session-owned tab. Focused tests prove the request order and MCP response shaping without requiring arbitrary existing-tab adoption.
- Observation: The native-host Unix socket is not one homogeneous client channel. Codex Browser Use needs one primary client for extension-originated requests such as heartbeats, while sky-cua MCP browser tools need short-lived request-only clients.
  Evidence: `crates/sky-cua-chrome-host/src/host.rs` previously drained all clients on each new connection. Tests now prove a `session_id="sky-cua-mcp"` client becomes `Ephemeral`, does not evict the primary client, and extension-originated requests route to the primary client only.
- Observation: Browser socket discovery can become expensive from stale socket accumulation even though each individual socket request has a timeout.
  Evidence: `crates/sky-cua-service/src/browser/sockets.rs` now has regression tests proving discovery caps candidate count and skips a recently disconnected socket before probing it again.
- Observation: `cargo test` is not enough before live-smoking this seam, because the runnable `target/debug/sky-cua-chrome-host` binary can remain stale even when the test harness uses fresh source.
  Evidence: The first live MCP ownership proof failed with the old native-host log `evicting stale browser client 1 after a newer client connected`. After `cargo build -p sky-cua-chrome-host -p sky-cua-client -p sky-cua-service`, the same smoke passed; the final temp-extension run wrote `artifacts/chrome-host-smoke/20260605T205549Z/result.json`.
- Observation: Live Chrome-family smokes should not load the tracked extension directory directly.
  Evidence: Brave removed `_metadata` files from `resources/chrome-extension/codex/1.1.4_0` during an ownership smoke. `scripts/live_chrome_host_client_smoke.py` now copies the extension into the temporary smoke directory before launch, and the repo fallback now points at `resources/chrome-extension/codex/1.1.5_0`.
- Observation: CDP mouse-wheel dispatch can hang through the live extension bridge even when other CDP calls work.
  Evidence: The first full MCP action smoke timed out on `Input.dispatchMouseEvent` with `type="mouseWheel"`; `browser_scroll` now uses `Runtime.evaluate(window.scrollBy(...))`, and the rerun passed.
- Observation: Successful `claimUserTab` is not sufficient for CDP-backed actions.
  Evidence: After reclaiming Chamber tab `221674306`, `browser_snapshot` initially failed with `Debugger unattached`; `browser_claim_tab` now follows successful claim/reclaim with `attach` and `executeCdp(Page.enable)`.
- Observation: Stale sky-cua session ownership can block user-tab control even though the tab is safe to recover.
  Evidence: Chamber tab `221674306` was rejected as already owned by `sky-cua-cursor-proof`. Calling `finalizeTabs` as that stale `sky-cua-*` session with `keep=[]` releases the user-tab lease without closing the tab; the service then retries `claimUserTab` once.

## Decision Log

- Decision: Browser automation becomes a `sky-cua` core capability exposed through the existing `sky_cua` MCP server and always advertised.
  Rationale: One server preserves install simplicity and product identity; hosts choose the `browser-use` workflow when page/tab automation is appropriate.
  Date/Author: 2026-06-05 / Sky
- Decision: Do not wrap Codex's bundled `browser-client.mjs` directly as the portable runtime.
  Rationale: That would bind the public sky-cua contract to Codex cache layout, minified JS internals, and OpenAI plugin runtime assumptions. The bundled plugins remain adapters, not the product boundary.
  Date/Author: 2026-06-05 / Sky
- Decision: The first implementation milestone is status-only plus tab listing with honest diagnostics, not mutating browser actions.
  Rationale: Browser automation has higher security and lifecycle risk than desktop actions; readiness needs to be observable before navigation/click/type tools are safe to expose.
  Date/Author: 2026-06-05 / Sky
- Decision: The MCP client owns browser-tool exposure policy; the service reports browser readiness whenever asked.
  Rationale: The service may be spawned or cached with an environment different from the MCP host. Keeping the gate in the MCP client prevents stale service env from lying about tool availability.
  Date/Author: 2026-06-05 / Sky
- Decision: Implement first tab listing as a small service-local Unix-socket client instead of depending on `sky-cua-chrome-host` internals.
  Rationale: The existing bridge protocol is tiny (native length prefix plus JSON-RPC) and already proven by the smoke harness. Sharing it as a crate boundary can wait until more browser actions need the transport.
  Date/Author: 2026-06-05 / Sky
- Decision: Add `SKY_CUA_BROWSER` for `user_chrome` socket selection.
  Rationale: Hosts can have Brave, Chrome, and Chromium connected simultaneously. A browser preference prevents sky-cua from querying unwanted browsers and avoids remote-control prompts in the wrong profile. Supported values are `brave`, `chrome`, `chromium`, and `all`/unset.
  Date/Author: 2026-06-05 / Sky
- Decision: Keep one primary Browser Use client in the native host and treat sky-cua MCP bridge clients as ephemeral when their request params use `session_id="sky-cua-mcp"`.
  Rationale: Browser MCP calls must be able to send JSON-RPC requests to the extension without disconnecting Codex's companion Browser Use client or becoming the receiver for extension-originated heartbeat and browser-event requests.
  Date/Author: 2026-06-05 / Sky
- Decision: Bound browser socket discovery with a small daemon-local inventory instead of creating a broad browser-manager subsystem now.
  Rationale: The concrete performance failure is repeated filesystem and `/proc` work plus unbounded probe fanout. A local cache, stale-socket suppression, candidate cap, and probe semaphore fix that mechanism while preserving the current tool contract.
  Date/Author: 2026-06-05 / Sky
- Decision: Keep browser scroll page-oriented for the first action surface instead of forcing CDP mouse-wheel input.
  Rationale: The live bridge timed out on `Input.dispatchMouseEvent(type="mouseWheel")`; `window.scrollBy(...)` proves useful page scroll behavior now without adding a flaky transport dependency.
  Date/Author: 2026-06-06 / Sky
- Decision: Keep the active plan open only for managed browser lifecycle and Codex Desktop adapter delegation after the `user_chrome` action slice shipped.
  Rationale: `plans/` is for forward-looking work. The shipped `user_chrome` contract now lives in `docs/features/browser-mcp-tools.md` and `ROADMAP.md`; this ExecPlan remains only because process/profile ownership is still unresolved.
  Date/Author: 2026-06-06 / Sky
- Decision: `browser_claim_tab` may reclaim only stale sessions whose ids start with `sky-cua-`.
  Rationale: Recovering our own stale MCP/proof sessions fixes the critical path without stealing tabs from Codex Browser Use or other browser automation sessions. Non-sky-cua owners still surface as diagnostics.
  Date/Author: 2026-06-06 / Sky

## Outcomes & Retrospective

The first browser MCP milestone is implemented, installed, documented, and listed on the roadmap. Browser tools are always advertised by `sky-cua-client mcp`. `browser_status` reports managed and user Chrome availability from existing doctor checks and now suppresses disconnected diagnostics when a matching native-host socket is connected. `browser_list_tabs(user_chrome)` connects to Chrome-family extension/native-host Unix sockets and calls `getUserTabs` for real user tabs. `SKY_CUA_BROWSER=brave` selects the user's preferred Brave profile without poking Chrome. When no matching bridge is connected, the tool returns an explicit `BrowserBridgeDisconnected` diagnostic. The `user_chrome` action slice is also complete for session-owned or successfully claimed tabs: open, claim, move cursor, navigate, snapshot, screenshot, click, type text, press key, and scroll all route through the shared sky-cua MCP/service boundary.

## Context and Orientation

`sky-cua` is a Rust workspace plus Python harnesses. The platform-neutral data contracts live in `crates/sky-cua-platform/src/model.rs` and `crates/sky-cua-platform/src/model/service.rs`. The long-lived service daemon lives in `crates/sky-cua-service/src/daemon.rs` and handles `ServiceRequest` values. The MCP stdio client lives in `crates/sky-cua-client/src/mcp_tools.rs` and translates MCP tool calls into service requests.

The Chrome browser bridge is separate. `crates/sky-cua-chrome-host` implements a native messaging host for Chrome-family browsers. It talks to the vendored Codex Chrome extension through Chrome's native messaging protocol and exposes a Unix socket to browser clients. The Codex compatibility helper in `resources/chrome_preflight.py` stages OpenAI-compatible `chrome` and `browser-use` plugins into Codex Desktop's plugin cache and writes native host manifests. That compatibility helper is not the portable MCP runtime.

An MCP tool is a JSON-described function that a host such as OpenCode or Pi can call. `sky-cua-client mcp` currently exposes desktop tools such as `list_apps`, `get_app_state`, `click`, `type_text`, and `press_key`. This work adds browser tools beside those desktop tools.

## Plan of Work

Completed `user_chrome` work added browser contracts to the platform model. `BrowserTargetKind` identifies whether a browser context is a managed isolated browser or the user's Chrome-family browser. `user_chrome` is an already-running Brave/Chrome/Chromium profile reached through the extension/native-host bridge. `managed` is a planned sky-cua-owned browser process/profile/tab lifecycle and must not be treated as available until launch and cleanup ownership exist. `BrowserStatusReport` summarizes readiness, available targets, diagnostics, and known tab count. Browser response structs carry tabs, navigation status, snapshots, screenshots, and action outcomes. These types are serializable and platform-neutral.

Completed service work added browser `ServiceRequest`/`ServiceResponse` variants for status, tab listing, open, claim, cursor movement, navigation, snapshot, screenshot, click, text entry, key dispatch, and scroll. The service daemon handles these requests without using the desktop action lane. The service derives status from the existing doctor report when the desktop lane is available, reports deferred browser-integration checks when that lane is busy, and uses a small native-host socket bridge for tab enumeration and actions. If no browser bridge is connected, it returns explicit diagnostics rather than faking tab state.

Completed MCP client work added always-advertised `browser_*` tool definitions in `crates/sky-cua-client/src/mcp_tools.rs`. Browser readiness and bridge availability are reported through tool responses and diagnostics rather than a host-specific feature flag.

Completed installer work updated OpenCode and Pi support. The generated OpenCode config includes optional browser selection. Pi's generated wrapper preserves `SKY_CUA_BROWSER` when it is set during installation. Installation into `~/.pi/agent/` preserves existing user config and merges only the `sky_cua` MCP entry.

Completed verification covered Rust tests, direct MCP probing, Pi, and live Brave browser smokes. Pi verification calls the installed MCP server through Pi and asks for `browser_status`, then inspects that the tool is available and returns structured readiness rather than using shell/browser fallbacks.

The remaining milestone builds on the proven user Chrome bridge and session-owned `browser_open` path. `sky-cua-service/src/browser.rs` owns the current one-shot `getUserTabs`, `claimUserTab`, `createTab`, `moveMouse`, and CDP bridge calls over Chrome native-host sockets. Remaining work starts with managed browser launch/profile ownership, cleanup, and Codex Desktop adapter delegation.

## Concrete Steps

Run all commands from `/home/bex/projects/sky-cua` unless stated otherwise.

For regression coverage on the shipped `user_chrome` surface, run:

    cargo test -p sky-cua-platform
    cargo test -p sky-cua-service
    cargo test -p sky-cua-client
    cargo fmt --check
    cargo clippy -p sky-cua-platform --all-targets -- -D warnings
    cargo clippy -p sky-cua-service --all-targets -- -D warnings
    cargo clippy -p sky-cua-client --all-targets -- -D warnings

After building and installing, run:

    cargo build --release -p sky-cua-client -p sky-cua-service
    python3 scripts/install_mcp_server.py --target-dir "$HOME/.local/share/sky-cua" --host opencode --bin-dir "$HOME/.local/bin"
    python3 scripts/install_mcp_server.py --target-dir "$HOME/.local/share/sky-cua" --host pi --bin-dir "$HOME/.local/bin"

If Pi installation is implemented as a merge into `~/.pi/agent/mcp.json`, inspect that file and verify it contains the `sky_cua` entry and wrapper path without deleting unrelated MCP servers.

For the remaining managed lifecycle milestone, add acceptance around a runtime-owned browser process/profile instead of a user profile. The managed smoke should launch the managed context, open a local HTML fixture, run `browser_snapshot`, `browser_screenshot`, `browser_click`, `browser_type_text`, `browser_press_key`, `browser_scroll`, and `browser_navigate`, then prove the process/profile cleanup happened.

## Validation and Acceptance

The shipped `user_chrome` surface is accepted when browser MCP tools are always observable. `tools/list` must include `browser_status`, `browser_list_tabs`, `browser_open`, `browser_claim_tab`, `browser_move_mouse`, `browser_navigate`, `browser_snapshot`, `browser_screenshot`, `browser_click`, `browser_type_text`, `browser_press_key`, and `browser_scroll`. Calling `browser_status` must return `isError: false`, text describing readiness, and structured content with `enabled: true`, `available_targets`, `diagnostics`, and `tabs_known`.

Pi acceptance is: after installing into `~/.pi/agent/`, a Pi prompt can use `sky_cua` to call `browser_status` and returns browser readiness instead of trying to launch a shell browser or using unrelated browser automation.

Current Pi status: browser-status calls work in direct print mode and interactive mode. The earlier 420 second PTY timeout was not reproduced after cache/warmup and tool-scope narrowing.

Current browser-open status: `browser_open` is implemented for `target=user_chrome` only. It probes matching browser sockets with lightweight `getInfo`, creates a session-owned tab in the selected Chrome-family browser, and optionally navigates to `http://`, `https://`, or `about:blank`. If tab creation succeeds but attach, page enable, or navigation fails, the response returns the created tab with a `BrowserOpenPartial` diagnostic and MCP `isError: true`. `managed` remains unavailable.

Current browser-action status: `browser_claim_tab`, `browser_move_mouse`, `browser_navigate`, `browser_snapshot`, `browser_screenshot`, `browser_click`, `browser_type_text`, `browser_press_key`, and `browser_scroll` are implemented for `target=user_chrome` tabs that are session-owned or successfully adopted by `browser_claim_tab`. `browser_claim_tab` can reclaim tabs stuck in stale `sky-cua-*` sessions, refuses non-sky-cua owners, and attaches/Page-enables claimed tabs for CDP actions. Pointer and scroll coordinates are browser screenshot pixels; the service converts them through `window.devicePixelRatio` before sending CDP or extension input. `browser_scroll` currently scrolls the page viewport through `Runtime.evaluate(window.scrollBy(...))`.

Current bridge-ownership status: the native host distinguishes primary clients from ephemeral sky-cua MCP clients. A client whose requests carry `session_id="sky-cua-mcp"` can issue bridge requests but does not evict the primary Browser Use client. Extension-originated requests route to the primary client and ignore ephemeral MCP clients.

Current socket-inventory status: discovery is bounded to 32 newest live Unix socket candidates and bridge probes are limited to eight concurrent tasks. Browser-family lookup results are cached briefly, and sockets that recently disconnected or timed out are skipped until the stale-failure TTL expires.

Live native-host ownership status: artifact `artifacts/chrome-host-smoke/20260605T205549Z/result.json` proves a second MCP bridge client can call `browser_list_tabs(user_chrome)` through the live Brave native-host socket without evicting the primary Browser Use client. The same run confirmed the extension-originated heartbeat reached the original primary client after the MCP client exited.

Live browser-open status: the MCP smoke used `target/debug/sky-cua-client mcp`,
`target/debug/sky-cua-service`, `SKY_CUA_BROWSER=brave`, and an
isolated `/tmp/opencode/sky-cua-browser-open-live.sock` service socket. It
opened a real Brave `about:blank` tab and confirmed that the tab appeared in
`browser_list_tabs(user_chrome)`. The isolated debug service process was killed
after the smoke.

Full completion of the whole feature requires additional acceptance: a managed browser can be opened to a local HTML page, the existing snapshot/screenshot/action tools work in that owned context, the managed process/profile is cleaned up deterministically, and Codex Desktop still sees the compatibility `chrome` and `browser-use` plugins without duplicate browser MCP tools unless explicitly enabled.

Current full action proof: the 2026-06-06 release-binary smoke used `target/release/sky-cua-client mcp`, `target/release/sky-cua-service`, `SKY_CUA_BROWSER=brave`, and isolated socket `/tmp/sky-cua-browser-full-mcp-bex-1154686.sock`. It advertised all browser tools, opened local fixture tab `221675114`, captured title `sky-cua browser action fixture`, returned a 37,664-byte base64 PNG screenshot, moved/clicked/typed/pressed/scrolled, and navigated to `about:blank` with `isError=false` for each tool.

Current existing-tab reclaim proof: the 2026-06-06 release-binary smoke used isolated socket `/tmp/sky-cua-browser-reclaim-bex-1294027.sock` against real Brave. Existing Chamber tab `221674306` was claimed through `browser_claim_tab` after stale `sky-cua-cursor-proof` ownership, then `browser_snapshot` succeeded with title `Dot Agents | OpenChamber` and URL `https://chamber.heliasar.com/`.

Current Brave-only isolation proof: the installed MCP binary was run with isolated socket `/tmp/sky-cua-brave-only-installed-bex-1329657.sock` and `SKY_CUA_BROWSER=brave`. `browser_list_tabs(user_chrome)` found exactly one `chamber.heliasar.com` tab, `browser_claim_tab` claimed tab `221674306`, and `browser_snapshot` returned title `Dot Agents | OpenChamber` plus URL `https://chamber.heliasar.com/`.

Remaining completion requires managed browser acceptance: sky-cua can launch an isolated browser/profile, open a local HTML page, run the same snapshot/screenshot/action sequence in that owned context, clean up the process/profile deterministically, and keep Codex Desktop on the compatibility `chrome` and `browser-use` plugins without duplicate browser MCP tools unless explicitly enabled.

## Idempotence and Recovery

The code changes are additive and can be repeated safely. Installer changes must preserve existing MCP config keys. If installation fails with `Text file busy`, terminate only running `sky-cua-client` or `sky-cua-service` processes and retry. Do not delete or replace unrelated user config under `~/.pi/agent/`.

If browser tool exposure causes a host to see duplicate browser capability, route through the `browser-use` skill and prefer the shared `sky_cua` MCP tools. Codex Desktop compatibility adapter delegation remains follow-up work.

## Artifacts and Notes

The adjacent OpenCode selector bug was already proven by this passing test in `crates/sky-cua-client/src/mcp_tools.rs`:

    app_selector_ignores_opencode_blank_default_fields ... ok

Browser MCP tests now cover tool visibility, response shaping, argument parsing,
and representative service routing. Keep adding targeted tests beside new
managed lifecycle requests instead of relying only on live smokes.

## Interfaces and Dependencies

In `crates/sky-cua-platform/src/model.rs`, define browser model structs and enums such as:

    pub enum BrowserTargetKind { Managed, UserChrome }
    pub struct BrowserTargetAvailability { ... }
    pub struct BrowserStatusReport { ... }
    pub struct BrowserTab { ... }
    pub struct BrowserListTabsResponse { ... }
    pub struct BrowserOpenResponse { ... }

In `crates/sky-cua-platform/src/model/service.rs`, add:

    ServiceRequest::BrowserStatus
    ServiceRequest::BrowserListTabs { target: Option<BrowserTargetKind> }
    ServiceRequest::BrowserOpen { target: Option<BrowserTargetKind>, url: Option<String> }
    ServiceRequest::BrowserClaimTab { target: Option<BrowserTargetKind>, tab_id: String }
    ServiceRequest::BrowserMoveMouse { target: Option<BrowserTargetKind>, tab_id: String, x: f64, y: f64, wait_for_arrival: bool }
    ServiceRequest::BrowserNavigate { target: Option<BrowserTargetKind>, tab_id: String, url: String }
    ServiceRequest::BrowserSnapshot { target: Option<BrowserTargetKind>, tab_id: String }
    ServiceRequest::BrowserScreenshot { target: Option<BrowserTargetKind>, tab_id: String }
    ServiceRequest::BrowserClick { target: Option<BrowserTargetKind>, tab_id: String, x: f64, y: f64 }
    ServiceRequest::BrowserTypeText { target: Option<BrowserTargetKind>, tab_id: String, text: String }
    ServiceRequest::BrowserPressKey { target: Option<BrowserTargetKind>, tab_id: String, key: String }
    ServiceRequest::BrowserScroll { target: Option<BrowserTargetKind>, tab_id: String, delta_x: f64, delta_y: f64, x: f64, y: f64 }
    ServiceResponse::BrowserStatus { report: BrowserStatusReport }
    ServiceResponse::BrowserListTabs { response: BrowserListTabsResponse }
    ServiceResponse::BrowserOpen { response: BrowserOpenResponse }
    ServiceResponse::BrowserClaimTab { response: BrowserClaimTabResponse }
    ServiceResponse::BrowserMoveMouse { response: BrowserMoveMouseResponse }
    ServiceResponse::BrowserNavigate { response: BrowserNavigateResponse }
    ServiceResponse::BrowserSnapshot { response: BrowserSnapshotResponse }
    ServiceResponse::BrowserScreenshot { response: BrowserScreenshotResponse }
    ServiceResponse::BrowserClick { response: BrowserActionResponse }
    ServiceResponse::BrowserTypeText { response: BrowserActionResponse }
    ServiceResponse::BrowserPressKey { response: BrowserActionResponse }
    ServiceResponse::BrowserScroll { response: BrowserActionResponse }

In `crates/sky-cua-client/src/mcp_tools.rs`, add gated tools:

    browser_status
    browser_list_tabs
    browser_open
    browser_claim_tab
    browser_move_mouse
    browser_navigate
    browser_snapshot
    browser_screenshot
    browser_click
    browser_type_text
    browser_press_key
    browser_scroll

Tool visibility is no longer controlled by an environment flag; browser MCP tools are always advertised.

Current browser tools accept `target=user_chrome` only. `managed` is status-only: `browser_status` may report it as a known future target, but every browser tool rejects it until managed launch/profile ownership and cleanup are implemented.

Revision note 2026-06-05: Initial plan created to guide the first-class Browser Use MCP implementation while preserving Codex Desktop compatibility.
Revision note 2026-06-05: Updated after first milestone implementation. The status/list-tabs MCP surface, installer gating, and OpenCode proof are complete; Pi proof and real browser bridge connection remain.
Revision note 2026-06-05: Added the first service-side browser bridge client for `getTabs`. Pi proof was attempted and timed out with zero output, so it remains inconclusive due to Pi CLI responsiveness.
Revision note 2026-06-05: Installed release proof is complete after the final status-diagnostic fix. The first browser MCP milestone now has a feature doc and roadmap entry; at that point this ExecPlan remained active for managed lifecycle, snapshots, and browser actions.
Revision note 2026-06-05: Focused review found stale-socket status and premature managed-target exposure; both are fixed and covered/documented.
Revision note 2026-06-05: Added Pi text-fallback proof and Certificate Manager follow-up. The visible local-user certificate page reports no certificates; CDP probing of the existing user tab is blocked by the extension's session ownership guard.
Revision note 2026-06-05: Added `browser_open` for session-owned `user_chrome` tabs. Snapshot and action tools should build on returned tab ids rather than trying to control arbitrary existing user tabs.
Revision note 2026-06-05: Live-smoked `browser_open` against real Brave through MCP; it opened `about:blank` and the returned tab id was visible through `browser_list_tabs(user_chrome)`.
Revision note 2026-06-05: Fixed bridge client ownership and bounded socket discovery after ultra-review. The native host now keeps sky-cua MCP clients ephemeral, and service socket inventory lives in `crates/sky-cua-service/src/browser/sockets.rs`.
Revision note 2026-06-05: Live-smoked MCP native-host ownership with artifact `artifacts/chrome-host-smoke/20260605T205549Z/result.json`. Rebuild debug binaries before this smoke; stale `target/debug` binaries can reproduce the old eviction behavior.
Revision note 2026-06-06: Added and live-smoked the full `user_chrome` browser action surface. This plan now remains active for managed browser/profile lifecycle and Codex Desktop adapter delegation, not for session-owned `user_chrome` actions.
Revision note 2026-06-06: Updated durable docs and `ROADMAP.md` so shipped `user_chrome` capability is documented in `docs/features/browser-mcp-tools.md`, while this plan owns only managed browser/profile lifecycle and Codex Desktop adapter delegation.
Revision note 2026-06-06: Fixed existing-tab critical path. `browser_claim_tab` now reclaims stale `sky-cua-*` tab ownership, refuses non-sky-cua owners, attaches/Page-enables claimed tabs, and live-proves CDP snapshot against the existing Chamber tab.
Revision note 2026-06-06: Reproved the installed critical path with `SKY_CUA_BROWSER=brave` and an isolated service socket so the verification targets Brave only and does not broadly probe Chrome-family sockets.
