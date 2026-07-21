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
  await tab.goto("https://example.com/");
  await tab.playwright.waitForLoadState({ timeoutMs: 15_000 });
  await nodeRepl.emitImage(await tab.screenshot());
}
