{
  const { sky } = await import("@heliasar/sky-cua");
  globalThis.computer ??= sky;
  const [shot] = await computer.get_screenshot();
  nodeRepl.write({ filepath: shot.filepath, provenance: nodeRepl.requestMeta });
  await nodeRepl.emitImage(shot.data_url);
}
