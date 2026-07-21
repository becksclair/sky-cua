# `@heliasar/browser-use`

Canonical first-party Browser JavaScript client for the sky-cua release. Bun
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
