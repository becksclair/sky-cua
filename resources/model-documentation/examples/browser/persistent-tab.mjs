{
  const { setupBrowserRuntime } = await import("@heliasar/browser-use");
  if (!globalThis.agent) await setupBrowserRuntime({ globals: globalThis });
  globalThis.browser ??= await agent.browsers.getDefault();
  const documentation = await browser.documentation();
  nodeRepl.write({ documentation, provenance: nodeRepl.requestMeta });
}
