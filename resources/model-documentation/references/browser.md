# Browser JavaScript

Import `setupBrowserRuntime` from `@heliasar/browser-use`, call it once with `globalThis`, then retain `agent` and a selected `browser` from `agent.browsers.*`. Call `await browser.documentation()` to discover the installed command surface; use `agent.documentation.get(name)` for a named guidance document. `agent.browsers.*` owns browser discovery, tabs, groups, navigation, snapshots, screenshots, interaction, and tab-scoped `playwright`.

The installed Browser package uses the same daemon scheduler and extension bridge as direct Browser MCP. Caller provenance and browser group ownership are preserved. Codex IAB remains host-provided; Chrome, Chromium, and Brave use the daemon-owned Web Store extension bridge.

The compatibility projections contain the exact canonical Browser bytes and routing pointers; they are not a second implementation.
