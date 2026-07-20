---
name: computer-use
description: Route desktop work between direct sky_cua tools and persistent Computer Use JavaScript.
---

# Computer Use

Use direct `sky_cua` Computer tools for a short observe/action sequence. Use persistent `node_repl` with `@heliasar/sky-cua` when state, files, image transforms, OCR, or composed Browser/Phone work must survive across calls.

Read `references/node-repl.md`, then `references/computer.md`. Emit screenshots with `nodeRepl.emitImage`; for `file://` screenshots read bytes with `node:fs/promises` first.

Task recipes: `recipes/computer-workflows.md` and `recipes/composed-workflows.md`.
