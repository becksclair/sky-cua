# Sky enhancements on the native Browser baseline

The helper findings below remain valid. The versioned native-overlay deployment
described here was superseded on 2026-11-06: ChatGPT startup reconciles its
reserved Browser package to the packaged baseline. The current
[deployment contract](../features/codex-desktop-compat.md) keeps the helper and
its explicit activation guidance under Sky's ownership.

## Context

The native Linux package `chatgpt-desktop 26.1027.51404-1` provides a working
Browser RPC client/service pair. Sky's older native-pipe facade cannot run in
that runtime. The revised objective is to retain useful Sky behavior on the
maintained OpenAI implementation, rather than merely bypass Sky or port every
historical difference.

## Investigation

The comparison uses the current installed native plugin's `docs/api.json`,
capability documentation, and parsed client/service implementation under
`/usr/lib/chatgpt/resources/plugins/openai-bundled/plugins/browser`. Package
ownership and integrity were established in the
[regression investigation](2026-11-native-browser-plugin-regression.md).
The retired Desktop rebuild repository was excluded.

| Behavior | Sky source / evidence | Current native equivalent | Decision |
| --- | --- | --- | --- |
| Browser/session selection, tab lists and retention marks | `packages/browser-use/src/runtime.ts`, `transport.ts` | Native API schema: browser selection, session naming, `markDeliverable` / `markHandoff`; native host owns session routing | Use native behavior; do not port the older facade or its `transport` field onto native records |
| Clipboard, content, dialogs, downloads, file chooser, screenshots | `runtime.ts`, `wire-runtime.ts` | Native API schema and tab methods already cover these; native also adds AX and MCP Apps | Superseded upstream; preserve the complete native Agent and API filtering |
| Buffered CDP events, bot reports, auth handoff, page assets | `runtime.ts` capability facade and `wire-runtime.ts` notification state | Native advertised `cdp`, `botDetection`, `browserAuth`, `pageAssets` capabilities and docs | Use native capability implementations when advertised |
| Strict fresh top-level navigation wait | `runtime.ts::PlaywrightAPI.expectNavigation` | Native helper waits for URL/load readiness; a live no-op on a loaded fixture resolved in 33 ms with a 200 ms timeout | Retain additively as `tab.sky.expectNavigation`, using public native CDP events |
| WebP capture | `wire-runtime.ts`, `tab_screenshot` | Native screenshots are documented JPEG; public options do not select format | Do not port without measured need; keep native capture and metadata |
| Explicit stale synthetic-session recovery | `wire-runtime.ts`, `browser_user_claim_tab`; `transport.ts::finalizeStaleNodeReplSession` | No public native reclaim operation found | Retain for external extension consumers; native port blocked by missing supported ownership/recovery seam |
| Exact-match → Jev → Terra advisory ranking with ref revalidation | `crates/sky-cua-client/src/mcp_tools/browser_ranking.rs` | No equivalent ranked target selection found; native AX indices / DOM results are not Sky appshot-bound refs | Retain existing opt-in external ranking; native adapter requires observation/ref/actionability and provider integration, not just a decorator |
| Scheduler, fenced leases, reconnect/recovery and browser cursor | `crates/sky-cua-service/src/browser/bridge.rs` and control-plane feature doc | Native app owns IAB service/session lifecycle | Retain on Sky's extension path; no evidence these ran through native IAB |

Ranking is dispatched through Sky's MCP handler, observes `user_chrome` via
`ObserveAppShot`, and re-observes the same document generation and opaque ref
before returning an actionable result. It was not integrated with native IAB.
Similarly, source presence of Sky's cursor or daemon is not evidence that those
features execute in the native Browser. No model requests were made here.

The native client exports `setupBrowserRuntime(options)` and supports a public
`decorateTab` callback. Its return value is ignored, so the Sky layer attaches
an additive namespace to the returned tab rather than substituting another
facade. Setup still returns the original native Agent; caller options and any
existing decorator are composed. No private transport or service patch is needed.

An isolated native rehearsal established the retained behavior:

- The materialized wrapper connected through native RPC and decorated tabs;
  native AX and ordinary methods remained available, and a caller decorator ran.
- The Sky helper rejected a no-op that native `expectNavigation` accepted.
- A fresh wrapper-created tab, navigated to the intended local HTTP fixture
  before CDP use, passed same-origin cross-document and same-document waits,
  returning the action's result.
- One reused tab carried across setup contexts reported truncated history.
  The helper rejected it. Native service source discards event evidence on
  detach and security-origin changes; an uninitialized/stale origin can also
  trigger that condition. Public timeout waiters do not themselves evict events.
  The exact initiating state of that one invalidation was not captured.

Offline tests cover already-buffered events, iframe-only navigation, event loss,
callback rejection, capability errors, and one deadline across setup, action,
URL, and load waits. They also check that timeout ends observation without
claiming to cancel an already-running callback. Live evidence is retained at
`artifacts/native-browser-layer/`.

## Conclusion

Use a thin materialized overlay of the complete current native Browser plugin.
Only the bootstrap wrapper, Sky helper, appended skill guidance, and derived
version metadata differ. Baseline client, service, docs, hooks, assets, runtime,
and built-in methods remain native. Native package and Sky layer bytes determine
the overlay identity; no maintained copy of the native implementation is added.

The first supported native enhancement is strict navigation evidence, justified
by an observed behavioral gap. Ranking and external ownership/recovery remain
real Sky features on their existing path; claiming them for native IAB would
require further implementation and acceptance. That additional work is not
silently included in this layer.

## Implications

Standalone installation and preflight share the same overlay materializer.
The native helper is a separate artifact from the existing external
`browser_client`, so the external native-pipe contract is preserved. The retired
`browser-use` identity stays disabled. Native CDP capability/origin restrictions
and documentation gates still apply; lost event history is a failure, not an
excuse to skip validation or retry a mutating callback.

The bypass-only rollout was superseded and must not be applied. The enhancement
rollout remains held; active cache replacement, global installation, app restart,
and a fresh-chat catalog check require the separately authorized live step.
