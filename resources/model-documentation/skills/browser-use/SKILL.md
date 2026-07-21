---
name: browser-use
description: Route Browser work between direct sky_cua tools and persistent node_repl Browser JavaScript.
---

# Browser Use

Use direct `sky_cua` Browser tools for short, latency-sensitive page actions. Use persistent `node_repl` JavaScript for multi-step workflows, retained tab objects, extraction, file/image composition, or Browser plus OCR/PDF/Computer/Phone work.

When the user explicitly asks to use the Browser plugin (by name, mention, or
plugin reference) or the Codex in-app Browser, this overrides the short-action
rule: use persistent `node_repl` Browser JavaScript and do not probe direct
Browser tools or the Chrome extension bridge first. Initialize the runtime with
`await setupBrowserRuntime({ globals: globalThis })`; it installs
`globalThis.agent` and returns no Agent, so never assign its result. Then inspect
`await agent.browsers.list()` and select `await agent.browsers.get("iab")`.
The selected list entry must have `type === "iab"` and
`transport === "host_provided_iab"`. An `extension_native_host` entry is the
Chrome-family extension bridge, even if stale compatibility metadata labels it
as IAB. Do not select, open, or test it for an explicit Browser plugin or in-app
Browser request.

For routine list/get/tab/navigation/screenshot work, use the tested happy path in `recipes/browser-workflows.md` directly. Read `references/node-repl.md` and `references/browser.md`, or call `await browser.documentation()`, only for an unfamiliar command or after the happy path fails. A browser tab's `tab.playwright` controls that claimed tab; standalone Playwright launches a separate system Chrome-family browser and is documented in `references/toolbox.md`.

Task recipes: `recipes/browser-workflows.md` and `recipes/composed-workflows.md`.
