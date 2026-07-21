# Browser workflows

For a quick click or snapshot, use direct Browser MCP unless the user explicitly requested the Browser plugin or in-app Browser; those requests go directly to persistent `node_repl` and `host_provided_iab`. Initialize `@heliasar/browser-use` once without assigning the setup result, retain the installed global `agent` and tab binding, inspect `browser.documentation()`, and use `agent.browsers.*`. Prefer `tab.playwright` for DOM work on that same owned tab. Emit screenshot bytes with `nodeRepl.emitImage`; image emission does not require marking the tab deliverable.
