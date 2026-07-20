# Troubleshooting

Confirm `SKY_CUA_RELEASE_ROOT` and `SKY_CUA_DOCUMENTATION_ROOT` resolve inside the same immutable generation. For `node_repl`, also confirm `NODE_REPL_NODE_PATH`, `NODE_REPL_NODE_MODULE_DIRS`, and `NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S` point to that generation. A Browser trust mismatch must be fixed at release/install resolution; it is rejected before socket connection.

Use structured service error codes and response metadata. Do not retry a mutation after a post-write disconnect. Reset `node_repl` only when persistent VM state is suspect. Reconnect Phone sessions explicitly after disconnect. If standalone Playwright fails, verify a supported system Chrome-family browser exists; do not substitute it for `tab.playwright` when the intended tab is scheduler-owned.

Unsupported v1 targets are Linux arm64, Linux musl, macOS node_repl beyond a placeholder, Windows node_repl, public npm publication, and `@heliasar/sky-cua/advanced`.
