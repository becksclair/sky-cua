# Composed workflows

Keep Browser, Computer, and Phone bindings in the same persistent VM. Use Browser APIs for page content, Computer Use for native windows and browser chrome, and Phone for Android surfaces. Pass screenshots through Sharp/Canvas/pixelmatch or Tesseract, and PDF files through PDF.js. Write intermediate and final files with standard Node APIs and report exact paths plus response metadata.

Choose direct MCP only for an isolated low-latency action. Choose `node_repl` when objects, modules, files, or cross-surface state must persist.
