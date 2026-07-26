# Persistent Node toolbox

Files and binary data use standard Node APIs: `node:fs/promises`, `node:path`, `node:url`, `Buffer`, `ArrayBuffer`, typed arrays, `Blob`, streams, and `data:` URLs. Resolve and validate output paths explicitly; emit or report the exact written path.

Installed modules include Acorn 8.16.0 and Acorn Walk 8.3.5 for structural inspection of generated or minified JavaScript; Sharp 0.34.5 with Linux x64 libvips 1.2.4 for WebP/PNG/JPEG transforms; `@napi-rs/canvas` 0.1.91 for Canvas/Skia rendering; pixelmatch 7.1.0 for image comparison; PDF.js 5.4.624 with bundled fonts/cmaps for extraction and rendering; Tesseract.js 7.0.0 with bundled language data for OCR; and Playwright 1.57.0 using system Chrome-family browsers.

Prefer the pinned lazy loaders: `await nodeRepl.loaders.acorn()` and
`await nodeRepl.loaders.acornWalk()`. Parse with an explicit `ecmaVersion` and
`sourceType`, then use walkers such as `walk.simple(...)`. These loaders do not
repair malformed JavaScript or transport-level quoting errors.

Standalone Playwright launches its own browser process and is distinct from `tab.playwright`, which operates on a Browser API tab already owned by the sky-cua scheduler.

Copy-safe examples are listed in `inventories/example-inventory.json` and stored under `examples/`.
