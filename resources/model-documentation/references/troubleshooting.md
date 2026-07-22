# Troubleshooting

Confirm `SKY_CUA_DOCUMENTATION_ROOT`, `NODE_REPL_NODE_PATH`, and `NODE_REPL_NODE_MODULE_DIRS` resolve inside the fixed sky-cua installation. Browser Use is loaded from the co-installed fixed package path; callers do not supply a Browser-client trust hash.

Use structured service error codes and response metadata. Do not retry a mutation after a post-write disconnect. Reset `node_repl` only when persistent VM state is suspect. Reconnect Phone sessions explicitly after disconnect. If standalone Playwright fails, verify a supported system Chrome-family browser exists; do not substitute it for `tab.playwright` when the intended tab is scheduler-owned.

Unsupported v1 targets are Linux arm64, Linux musl, macOS node_repl beyond a placeholder, Windows node_repl, public npm publication, and `@heliasar/sky-cua/advanced`.
