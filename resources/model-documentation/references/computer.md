# Computer Use JavaScript

Import the default facade from `@heliasar/sky-cua`. It connects to the already-running sky-cua service and never starts or restarts the daemon or MCP server. Use capability discovery before platform-specific actions.

The facade covers screenshots, pointer move/click/drag/scroll, keyboard keys and text, and supported window actions. WebP is the default screenshot encoding. Preserve response metadata and emit images through `nodeRepl.emitImage`.

On disconnect, surface the structured transport error. A request rejected before writing is not dispatched; a disconnect after writing is ambiguous and must not be retried automatically.
