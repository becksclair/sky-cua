{
  const { createWorker } = await import("tesseract.js");
  if (!nodeRepl.runtime?.tesseract) throw new Error("Bundled Tesseract assets are unavailable");
  const worker = await createWorker("eng", 1, {
    langPath: nodeRepl.runtime.tesseract.tessdataRoot,
    gzip: false,
    cacheMethod: "none",
  });
  try {
    const { data } = await worker.recognize(nodeRepl.env.SKY_CUA_EXAMPLE_IMAGE);
    nodeRepl.write({ text: data.text, confidence: data.confidence });
  } finally {
    await worker.terminate();
  }
}
