---
name: sky-browser-enhancements
description: Add tab.sky.expectNavigation for strict same-origin navigation checks in the native ChatGPT Browser. Use when this helper or fresh navigation evidence is requested; ordinary browsing and sky-cua MCP browser actions use their existing skills.
---

# Sky Browser Enhancements

Use the native Browser skill for its absolute client import, setup, browser
selection, tab ownership, actions, and verification. This supplement adds one
helper to a native `Tab`; it does not provide a browser or transport.
If the native instructions have not been loaded, read
`@NATIVE_BROWSER_SKILL@`. Resolve their relative client path against that native
plugin's root, following the active host's instructions when already supplied.

With an existing native `tab` binding, use the Node REPL `js` tool:

```js
var skyBrowserEnhancements = await import(@SKY_CUA_ROOT_JSON@ + "/browser/sky-enhancements.mjs");
skyBrowserEnhancements.decorateSkyTab(tab);
```

Apply the decorator to each tab that needs the helper. Keep using existing
browser and tab bindings. Import the native client from the active Browser
instructions, rather than copying or pinning a native plugin cache path here.

Before the first helper call, read
`await agent.documentation.get("capabilities/tab/cdp")`. Navigate a fresh tab
to its intended HTTP(S) page before using that capability. On supported
same-origin navigation:

```js
var result = await tab.sky.expectNavigation(action, {
  timeoutMs: 10000,
  waitUntil: "load",
  // url: an optional destination URL filter
});
```

`action` is an async callback that triggers navigation; the helper returns its
result. Success requires a fresh main-frame navigation event after the action's
cursor and any requested URL/load readiness. One deadline covers the entire
operation. A no-op or child-frame event does not satisfy it. Lost event history
rejects the wait. Timeout does not cancel a running callback; do not retry a
mutation automatically.

Confirm the destination through fresh native Browser state. Report helper
failures directly; native `tab.playwright.expectNavigation` remains available
with its own contract. ChatGPT owns its native Browser package and service;
this helper stays in Sky's fixed install root across native reconciliation.
