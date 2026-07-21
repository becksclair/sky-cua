# Browser workflows

For a quick click or snapshot, use direct Browser MCP unless the user explicitly requested the Browser plugin or in-app Browser; those requests go directly to persistent `node_repl` and `host_provided_iab`. Initialize `@heliasar/browser-use` once without assigning the setup result, then retain the installed global `agent`, browser, and tab bindings. Prefer `tab.playwright` for DOM work on that same owned tab. Emit screenshot bytes with `nodeRepl.emitImage`; image emission does not require marking the tab deliverable.

For an explicit Browser plugin screenshot, use one `node_repl` `js` call
(`mcp__node_repl__js` when that qualified name is shown) and replace `url`:

```js
{
  const { setupBrowserRuntime } = await import("@heliasar/browser-use");
  if (!globalThis.agent) await setupBrowserRuntime({ globals: globalThis });
  const available = await agent.browsers.list();
  const matches = available.filter(
    (entry) => entry.type === "iab" && entry.transport === "host_provided_iab",
  );
  if (matches.length !== 1) {
    nodeRepl.write({ availableBrowsers: available });
    throw new Error(`Expected one host-provided IAB, found ${matches.length}`);
  }
  globalThis.browser = await agent.browsers.get(matches[0].id);
  globalThis.tab = (await browser.tabs.selected()) ?? (await browser.tabs.new());
  await tab.goto(url);
  await tab.playwright.waitForLoadState({ timeoutMs: 15_000 });
  await nodeRepl.emitImage(await tab.screenshot());
}
```

If no exact match exists, report the emitted browser list and stop; never probe
the extension or direct Browser tools as a fallback. If setup leaves no global
`agent`, report a setup failure. Consult `browser.documentation()` once only
when an unfamiliar command is needed. Reset the VM only when persistent state
may be incomplete.
