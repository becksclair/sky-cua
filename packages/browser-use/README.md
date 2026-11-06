# `@heliasar/browser-use`

Browser components for the sky-cua release. The external client remains a
first-party native-pipe implementation; the native Linux Browser layer delegates
to the maintained OpenAI client and service. Bun
builds the package and Node 24 runs the resulting `browser-client.mjs`. The
client installs `agent` and `display` through:

```js
const { setupBrowserRuntime } = await import("@heliasar/browser-use");
await setupBrowserRuntime({ globals: globalThis });
```

The runtime connects over two distinct owner-only native transports. The
explicit `SKY_CUA_CODEX_BROWSER_SOCKET_PATH` is the daemon-owned
`extension_native_host` lane for Chrome-family browsers. In Codex Desktop, the
client also discovers task-scoped pipes under `/tmp/codex-browser-use`, probes
them through trusted `nodeRepl.nativePipe.createConnection`, and accepts a
`host_provided_iab` only when its native type and Codex session identity match
the current trusted request metadata. It does not launch a browser, daemon, MCP
server, or proxy.

`await agent.browsers.list()` reports the concrete `transport` for every
accepted backend. `await agent.browsers.get("iab")` can resolve only
`host_provided_iab`; a daemon extension entry cannot satisfy it even if an older
compatibility response mislabeled the extension as `type: "iab"`.

The effective API is filtered from `api-manifest.json` for the selected `iab`,
`extension`, or `cdp` surface, then adjusted by the daemon's
`apiSupportOverrides`. The canonical 72-command surface is implemented with
existing daemon raw methods, direct CDP calls, and ordered notifications; the
only new daemon ingress reserved by the contract is `reportBotDetection` and
`browserAuthHandoff`. There is no catch-all raw command. Clipboard, content,
dev logs, dialogs, media/download/file-chooser workflows, page assets, and
annotated WebP screenshots use those explicit primitives. `tabs.finalize()` is
one atomic `finalizeTabs` request carrying exact tab/status dispositions.

`bun run build` produces one canonical byte stream. `src/projection.ts`
materializes `browser-use@openai-bundled` and `chrome@openai-bundled` by writing
that exact stream to each projection path, then records the single SHA-256.
There is no alternate implementation or Skynet projection.


## Native Linux enhancement layer

`bun run build` also emits `build/sky-enhancements.mjs`, separately from the
external `browser-client.mjs`. Standalone installation keeps this artifact at
`~/.local/share/sky-cua/browser/sky-enhancements.mjs` and installs the Codex
`sky-browser-enhancements` skill separately. Native Browser keeps its original
client, service, instructions and version. Activate the helper explicitly on
each tab that needs it; ordinary native setup remains unchanged.

```js
const { setupBrowserRuntime } = await import("<native plugin>/scripts/browser-client.mjs");
const agent = await setupBrowserRuntime();
const browser = await agent.browsers.get("iab");
nodeRepl.write(await browser.documentation());
const tab = await browser.tabs.new();
const { decorateSkyTab } = await import("<fixed install root>/browser/sky-enhancements.mjs");
decorateSkyTab(tab);
await tab.goto("http://localhost:3000/");
nodeRepl.write(await agent.documentation.get("capabilities/tab/cdp"));
await tab.sky.expectNavigation(
  () => tab.goto("http://localhost:3000/next"),
  { timeoutMs: 5000, url: "http://localhost:3000/next" },
);
```

This additive helper requires a fresh top-level navigation after its pre-action
event cursor and successful callback completion, then optional URL and load
readiness. One deadline covers the whole wait. It does not change native
`tab.playwright.expectNavigation`, cancel an expired callback, retry actions,
or bypass native capability/origin restrictions. Navigate a fresh tab to its
intended HTTP(S) page before CDP use. Lost/truncated history rejects the wait;
same-origin navigation is the verified scope.

Native clipboard, dialogs, downloads, assets, auth, screenshot and AX behavior
remain upstream. Sky ranking, cursor, scheduling and stale-session recovery
remain on their existing external-browser path; native ranking is not provided
by this layer. See the [evidence-backed inventory](../../docs/research/2026-11-native-browser-enhancement-baseline.md).
