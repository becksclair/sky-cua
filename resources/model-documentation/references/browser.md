# Browser JavaScript

Import `setupBrowserRuntime` from `@heliasar/browser-use`, call
`await setupBrowserRuntime({ globals: globalThis })` once, then retain the
installed global `agent` and a selected `browser` from `agent.browsers.*`.
Setup returns no Agent; never assign its return value. Call
`await browser.documentation()` to discover the installed command surface; use
`agent.documentation.get(name)` for a named guidance document.
`agent.browsers.*` owns browser discovery, tabs, groups, navigation, snapshots,
screenshots, interaction, and tab-scoped `playwright`.

The installed Browser package exposes two distinct native transports. Codex
IAB is a task-scoped `host_provided_iab` pipe owned by Codex Desktop. Chrome,
Chromium, and Brave use the daemon-owned `extension_native_host` pipe and Web
Store extension. Caller provenance and browser group ownership are preserved;
caller provenance does not change one transport into the other.

For an explicit Browser plugin request (including a plugin mention/reference)
or in-app Browser request, go directly to this persistent JavaScript path.
Inspect `await agent.browsers.list()` and select
`await agent.browsers.get("iab")` without probing the Chrome extension bridge
first. The matching list entry must have
`type === "iab"`, `transport === "host_provided_iab"`, and the current Codex
session identity. Never substitute an `extension_native_host` entry. For a
Chrome-family extension task, select `"extension"` or the entry's exact ID.

`await tab.screenshot()` returns image bytes; emit them directly with
`await nodeRepl.emitImage(bytes)`. Screenshot capture or emission does not
require `markDeliverable()` or another tab-retention call.

The compatibility projections contain the exact canonical Browser bytes and routing pointers; they are not a second implementation.
