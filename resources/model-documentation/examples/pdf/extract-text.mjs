{
  const { readFile } = await import("node:fs/promises");
  const pdfjs = await import("pdfjs-dist/legacy/build/pdf.mjs");
  if (!nodeRepl.runtime?.pdfjs) throw new Error("Bundled PDF.js assets are unavailable");
  pdfjs.GlobalWorkerOptions.workerSrc = nodeRepl.runtime.pdfjs.workerSrc;
  const data = new Uint8Array(await readFile(nodeRepl.env.SKY_CUA_EXAMPLE_PDF));
  const pdf = await pdfjs.getDocument({
    data,
    cMapUrl: nodeRepl.runtime.pdfjs.cMapUrl,
    cMapPacked: true,
    standardFontDataUrl: nodeRepl.runtime.pdfjs.standardFontDataUrl,
    wasmUrl: nodeRepl.runtime.pdfjs.wasmUrl || undefined,
    useSystemFonts: false,
    disableFontFace: true,
    disableWorker: true,
  }).promise;
  const page = await pdf.getPage(1);
  const content = await page.getTextContent();
  nodeRepl.write({ pages: pdf.numPages, text: content.items.map((item) => item.str ?? "").join(" ") });
}
