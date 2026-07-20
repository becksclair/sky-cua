# Browser workflows

For a quick click or snapshot, use direct Browser MCP. For a multi-step task, initialize `@heliasar/browser-use` once, retain the tab binding, inspect `browser.documentation()`, and use `agent.browsers.*`. Prefer `tab.playwright` for DOM work on that same owned tab. Emit screenshots only when the model needs pixels.
