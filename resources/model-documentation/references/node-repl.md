# node_repl contract

`js` evaluates JavaScript in one persistent Node 24 VM. Top-level bindings and awaited results survive subsequent calls. `js_reset` creates a fresh VM and discards those bindings. `js_add_node_module_dir` adds an absolute installed module directory without resetting state.

Use `await import(...)` for Node built-ins and packages. The host supplies `nodeRepl.write(value)` for text or structured output, `nodeRepl.emitImage(value)` for supported image bytes/data URLs, and `nodeRepl.requestMeta` for the current session, turn, caller provenance, and supplied host metadata. Preserve that metadata; do not manufacture Codex metadata.

Use ordinary Node globals including `Buffer`, `URL`, `Blob`, `fetch`, streams, timers, and `AbortController`. A timeout or cancellation interrupts the current call; reset if application state may be incomplete. Mutation calls are not automatically retried.

See `references/toolbox.md` for files, buffers, images, PDF, OCR, Canvas, pixel comparison, and standalone Playwright.
