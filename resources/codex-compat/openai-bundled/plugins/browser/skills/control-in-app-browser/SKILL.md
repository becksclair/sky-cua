---
name: control-in-app-browser
description: "Control Codex Desktop's host-provided in-app Browser through the sky-cua Browser client and persistent node_repl."
---

# Browser

Use this plugin for Codex Desktop's in-app Browser. Run setup through the
`node_repl` JavaScript tool and import this plugin's
`scripts/browser-client.mjs` by absolute path:

```js
if (globalThis.agent?.browsers == null) {
  const { setupBrowserRuntime } = await import(
    "<plugin root>/scripts/browser-client.mjs"
  );
  await setupBrowserRuntime({ globals: globalThis });
}
const available = await agent.browsers.list();
const iab = available.find(
  (entry) =>
    entry.type === "iab" &&
    entry.transport === "host_provided_iab"
);
if (iab == null) throw new Error("No host-provided in-app Browser is available");
globalThis.browser = await agent.browsers.get("iab");
```

For this plugin, require both `type === "iab"` and
`transport === "host_provided_iab"`. Never substitute an
`extension_native_host` entry: that transport is the shared client's
Chrome-family bridge for non-IAB consumers such as OpenClaw.

Reuse an appropriate existing tab before opening a new tab. Browser coordinates
are CSS pixels and must never be reused as desktop coordinates. Re-observe after
navigation, scrolling, resizing, or other visible transitions, and verify
consequential actions from fresh Browser evidence.
