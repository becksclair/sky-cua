---
name: browser-use
description: Route Browser work between direct sky_cua tools and persistent node_repl Browser JavaScript.
---

# Browser Use

Use direct `sky_cua` Browser tools for short, latency-sensitive page actions. Use persistent `node_repl` JavaScript for multi-step workflows, retained tab objects, extraction, file/image composition, or Browser plus OCR/PDF/Computer/Phone work.

When the user explicitly asks for the Codex in-app Browser, use persistent
`node_repl` Browser JavaScript. Initialize the runtime with
`await setupBrowserRuntime({ globals: globalThis })`, inspect
`await agent.browsers.list()`, and select `await agent.browsers.get("iab")`.
The selected list entry must have `type === "iab"` and
`transport === "host_provided_iab"`. An `extension_native_host` entry is the
Chrome-family extension bridge, even if stale compatibility metadata labels it
as IAB; do not use it for an explicit in-app Browser request.

For JavaScript, read `references/node-repl.md`, then `references/browser.md`. Call `await browser.documentation()` before relying on an unfamiliar `agent.browsers.*` command. A browser tab's `tab.playwright` controls that claimed tab; standalone Playwright launches a separate system Chrome-family browser and is documented in `references/toolbox.md`.

Task recipes: `recipes/browser-workflows.md` and `recipes/composed-workflows.md`.
