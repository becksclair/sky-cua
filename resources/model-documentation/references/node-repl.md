# node_repl contract

`js` evaluates JavaScript in one persistent Node 24 VM. Top-level bindings and awaited results survive subsequent calls. `js_reset` creates a fresh VM and discards those bindings. `js_add_node_module_dir` adds an absolute installed module directory without resetting state.

Use `await import(...)` for Node built-ins and packages. The host supplies `nodeRepl.write(value)` for text or structured output, `nodeRepl.emitImage(value)` for supported image bytes/data URLs, and `nodeRepl.requestMeta` for the current session, turn, caller provenance, and supplied host metadata. Preserve that metadata; do not manufacture Codex metadata.

Use ordinary Node globals including `Buffer`, `URL`, `Blob`, `fetch`, streams, timers, and `AbortController`. A timeout or cancellation interrupts the current call; reset if application state may be incomplete. Mutation calls are not automatically retried.

## Environment and installed runtime assets

`nodeRepl.env` is a frozen, public environment view. It always exposes `SKY_CUA_CODEX_BROWSER_SOCKET_PATH` and `SKY_CUA_MCP_CALLER_PROVENANCE` when set, plus the comma-separated names selected by the MCP process in `NODE_REPL_PUBLIC_ENV`. The installed example runner selects `SKY_CUA_EXAMPLE_INPUT_FILE`, `SKY_CUA_EXAMPLE_IMAGE`, and `SKY_CUA_EXAMPLE_PDF`; when copying those examples into an ordinary session, replace the values with your own absolute paths/file URL or ensure the MCP process explicitly publishes the same names.

`nodeRepl.runtime` is the frozen verified runtime inventory. Its stable v1 keys are `version`, `root`, `node.{version,execPath}`, `modules.root`, `browser.{playwrightRoot,executablePath,executableKind}`, `pdfjs.{root,cMapUrl,standardFontDataUrl,wasmUrl,workerSrc}`, `tesseract.{tessdataRoot,languages}`, `licenses.root`, and `sbomPath`. Check an optional asset such as `browser.executablePath` or `pdfjs.wasmUrl` before use. The PDF and OCR examples use these generation-bound paths instead of checkout or downloaded assets.

See `references/toolbox.md` for files, buffers, images, PDF, OCR, Canvas, pixel comparison, and standalone Playwright.
