const { spawnSync } = require("node:child_process");
const { createHash } = require("node:crypto");
const {
  accessSync,
  constants,
  existsSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} = require("node:fs");
const { tmpdir } = require("node:os");
const { basename, join, resolve } = require("node:path");
const { prepareMediaFixtures } = require("./repl-subsystem-media-fixtures");
const {
  startMcpSession,
  stopChildProcess,
} = require("./web-workbench-acceptance-helper");

type JsonObject = Record<string, unknown>;
type McpResponse = {
  jsonrpc?: unknown;
  id?: unknown;
  result?: unknown;
  error?: unknown;
};
type AcceptanceOptions = {
  runtimeRoot: string;
  timeoutMs: number;
};
type CellEvidence = Record<string, string | number | boolean>;
type AcceptanceCell = {
  label: string;
  tool: "js" | "js_reset";
  code?: string;
  emittedImages?: number;
};
type AcceptanceReport = {
  schema: "com.heliasar.cua-node.repl-subsystem-acceptance";
  schema_version: 1;
  status: "passed" | "failed";
  checks: CellEvidence;
  error?: string;
};
type ChildExit = {
  code: number | null;
  signal: NodeJS.Signals | null;
};
type NodeReplChild = {
  stdin: { end(): void };
  exitCode: number | null;
  signalCode: NodeJS.Signals | null;
  kill(signal: NodeJS.Signals): boolean;
  once(
    event: "exit",
    listener: (code: number | null, signal: NodeJS.Signals | null) => void,
  ): unknown;
  removeListener(
    event: "exit",
    listener: (code: number | null, signal: NodeJS.Signals | null) => void,
  ): unknown;
};

const SCHEMA = "com.heliasar.cua-node.repl-subsystem-acceptance" as const;
const OCR_PHRASE = "ZERO PREAMBLE MEDIA";
const DEFAULT_TIMEOUT_MS = 120_000;
const DEFAULT_TEARDOWN_WAIT_MS = 1_000;

function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : "unknown acceptance failure";
}

async function teardownNodeRepl(
  child: NodeReplChild,
  tempRoot: string,
  waitMs = DEFAULT_TEARDOWN_WAIT_MS,
): Promise<ChildExit> {
  let exit: ChildExit;
  try {
    exit = await stopChildProcess(child, waitMs, "node_repl");
  } catch (error) {
    throw new Error(
      `node_repl did not exit after SIGKILL; temporary root retained: ${tempRoot}`,
      { cause: error },
    );
  }
  rmSync(tempRoot, { recursive: true, force: true });
  if (existsSync(tempRoot))
    throw new Error(`temporary root was not removed: ${tempRoot}`);
  return exit;
}

function executable(path: string): boolean {
  try {
    accessSync(path, constants.R_OK | constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function assertExecutable(path: string, label: string): void {
  if (!executable(path)) throw new Error(`${label} is not executable: ${path}`);
}

function parseArgs(argv: string[]): AcceptanceOptions {
  let runtimeRoot = resolve(__dirname, "../out/linux-x64/cua_node");
  let timeoutMs = DEFAULT_TIMEOUT_MS;
  for (const argument of argv) {
    if (argument.startsWith("--runtime-root="))
      runtimeRoot = resolve(argument.slice("--runtime-root=".length));
    else if (argument.startsWith("--chromium-executable=")) continue;
    else if (argument.startsWith("--timeout-ms=")) {
      timeoutMs = Number(argument.slice("--timeout-ms=".length));
      if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1)
        throw new Error("--timeout-ms must be a positive integer");
    } else if (argument !== "--json") throw new Error(`unknown argument: ${argument}`);
  }
  assertExecutable(join(runtimeRoot, "bin/node"), "bundled Node");
  assertExecutable(join(runtimeRoot, "bin/node_repl"), "assembled node_repl");
  if (!existsSync(join(runtimeRoot, "lib/node_modules/playwright/package.json")))
    throw new Error(`assembled module directory is incomplete: ${runtimeRoot}`);
  for (const language of ["eng", "osd"])
    if (!existsSync(join(runtimeRoot, `share/tessdata/${language}.traineddata`)))
      throw new Error(`bundled ${language} tessdata is missing`);
  return { runtimeRoot, timeoutMs };
}

function sha256(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function fileEvidence(
  path: string,
  signature?: string,
  minimumBytes = 32,
): CellEvidence {
  const bytes = readFileSync(path);
  if (bytes.length < minimumBytes)
    throw new Error(`${basename(path)} is truncated: ${bytes.length} bytes`);
  if (
    signature !== undefined &&
    !bytes.subarray(0, signature.length / 2).equals(Buffer.from(signature, "hex"))
  )
    throw new Error(`${basename(path)} has an invalid signature`);
  return { bytes: bytes.length, sha256: sha256(path) };
}

type ImageExpectation = {
  path: string;
  format: "png" | "webp";
  width: number;
  height: number;
  nonblank: boolean;
};

function verifyDecodedImages(
  runtimeRoot: string,
  expectations: readonly ImageExpectation[],
): void {
  const modules = join(runtimeRoot, "lib/node_modules");
  const probe = String.raw`
const { createRequire } = require("node:module");
const { join } = require("node:path");
const expectations = JSON.parse(process.argv[1]);
const modules = process.argv[2];
const requireFrom = createRequire(join(modules, "repl-acceptance-probe.cjs"));
const sharp = requireFrom("sharp");
(async () => {
  const results = [];
  for (const expectation of expectations) {
    const image = sharp(expectation.path);
    const metadata = await image.metadata();
    const stats = await image.stats();
    results.push({
      path: expectation.path,
      format: metadata.format,
      width: metadata.width,
      height: metadata.height,
      channels: metadata.channels,
      varying: stats.channels.some(channel => channel.max > channel.min),
    });
  }
  process.stdout.write(JSON.stringify(results));
})().catch(error => { console.error(error); process.exitCode = 1; });
`;
  const result = spawnSync(
    join(runtimeRoot, "bin/node"),
    ["-e", probe, JSON.stringify(expectations), modules],
    {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 30_000,
    },
  );
  if (result.status !== 0)
    throw new Error(
      `bundled image decoder failed: ${(result.stderr || result.stdout).trim()}`,
    );
  const decoded = JSON.parse(result.stdout) as Array<{
    path?: unknown;
    format?: unknown;
    width?: unknown;
    height?: unknown;
    channels?: unknown;
    varying?: unknown;
  }>;
  if (!Array.isArray(decoded) || decoded.length !== expectations.length)
    throw new Error("bundled image decoder returned incomplete evidence");
  expectations.forEach((expectation, index) => {
    const actual = decoded[index];
    if (
      actual?.path !== expectation.path ||
      actual.format !== expectation.format ||
      actual.width !== expectation.width ||
      actual.height !== expectation.height ||
      typeof actual.channels !== "number" ||
      actual.channels < 3 ||
      (expectation.nonblank && actual.varying !== true)
    )
      throw new Error(
        `decoded image evidence mismatch for ${basename(expectation.path)}: ${JSON.stringify(actual)}`,
      );
  });
}

function parseCellResult(
  response: McpResponse,
  label: string,
  expectedImages = 0,
): CellEvidence {
  if (response.error !== undefined)
    throw new Error(`${label} MCP error: ${JSON.stringify(response.error)}`);
  if (!isObject(response.result)) throw new Error(`${label} returned no result`);
  if (response.result.isError === true) {
    const content = Array.isArray(response.result.content)
      ? response.result.content
      : [];
    const item = content.find(isObject);
    throw new Error(
      `${label} failed: ${typeof item?.text === "string" ? item.text : "unknown tool error"}`,
    );
  }
  const content = Array.isArray(response.result.content) ? response.result.content : [];
  const imageCount = content.filter(
    (candidate) => isObject(candidate) && candidate.type === "image",
  ).length;
  if (imageCount !== expectedImages)
    throw new Error(
      `${label} emitted ${imageCount} images; expected ${expectedImages}`,
    );
  const item = content.find(
    (candidate) => isObject(candidate) && candidate.type === "text",
  );
  if (!isObject(item) || typeof item.text !== "string")
    throw new Error(`${label} returned no text evidence`);
  const lines = item.text.trim().split("\n");
  const finalLine = lines.at(-1);
  if (finalLine === undefined) throw new Error(`${label} evidence is empty`);
  const parsed: unknown = JSON.parse(finalLine);
  if (!isObject(parsed)) throw new Error(`${label} evidence is not an object`);
  for (const value of Object.values(parsed))
    if (
      typeof value !== "string" &&
      typeof value !== "number" &&
      typeof value !== "boolean"
    )
      throw new Error(`${label} evidence contains a non-scalar value`);
  return { ...(parsed as CellEvidence), emitted_images: imageCount };
}

function buildCells(
  options: AcceptanceOptions,
  tempRoot: string,
): ReadonlyArray<AcceptanceCell> {
  const fixtureRoot = join(tempRoot, "media fixtures - 日本語");
  const path = (name: string): string => join(fixtureRoot, name);
  const pngPath = path("canvas source π.png");
  const webpPath = path("canvas source π.webp");
  const sharpPngPath = path("sharp output Ω.png");
  const sharpWebpPath = path("sharp output Ω.webp");
  const pdfPath = path("input vector + image.pdf");
  const pdfPngPath = path("pdf text page.png");
  const pdfWebpPath = path("pdf image page.webp");
  const malformedImagePath = path("malformed image.png");
  const malformedPdfPath = path("malformed document.pdf");
  const diffPath = path("pixel diff Δ.png");
  const transientPath = path("transient partial output.png");
  return [
    {
      label: "pdf-direct-import",
      tool: "js",
      code: String.raw`
var pdfjsMedia = await import("pdfjs-dist/legacy/build/pdf.mjs");
if (!nodeRepl.runtime || !nodeRepl.runtime.pdfjs || !nodeRepl.runtime.tesseract) throw new Error("nodeRepl.runtime media assets are unavailable");
pdfjsMedia.GlobalWorkerOptions.workerSrc = nodeRepl.runtime.pdfjs.workerSrc;
nodeRepl.write(JSON.stringify({ version: pdfjsMedia.version, runtime_version: nodeRepl.runtime.version, direct_import: true }));`,
    },
    {
      label: "canvas",
      tool: "js",
      emittedImages: 2,
      code: String.raw`
var fsMedia = await import("node:fs/promises");
var canvasMedia = await import("@napi-rs/canvas");
var canvasGlobals = DOMMatrix === canvasMedia.DOMMatrix && ImageData === canvasMedia.ImageData && Path2D === canvasMedia.Path2D && Image === canvasMedia.Image;
var canvasPoint = new DOMPoint(3, 4);
var canvasRect = new DOMRect(1, 2, 20, 10);
var canvasPath = new Path2D();
canvasPath.rect(canvasRect.x, canvasRect.y, canvasRect.width, canvasRect.height);
var sourceCanvas = canvasMedia.createCanvas(720, 160);
var sourceContext = sourceCanvas.getContext("2d");
sourceContext.fillStyle = "#ffffff";
sourceContext.fillRect(0, 0, 720, 160);
sourceContext.fillStyle = "#1769aa";
sourceContext.fill(canvasPath);
sourceContext.fillStyle = "#000000";
sourceContext.font = "bold 56px sans-serif";
sourceContext.textBaseline = "middle";
sourceContext.fillText(${JSON.stringify(OCR_PHRASE)}, 20, 80);
var canvasPngBytes = await sourceCanvas.encode("png");
await fsMedia.writeFile(${JSON.stringify(pngPath)}, canvasPngBytes);
var webpCanvas = canvasMedia.createCanvas(64, 48);
var webpContext = webpCanvas.getContext("2d");
webpContext.fillStyle = "#183153";
webpContext.fillRect(0, 0, 64, 48);
webpContext.fillStyle = "#ffb000";
webpContext.fillRect(16, 12, 32, 24);
var canvasWebpBytes = await webpCanvas.encode("webp", 90);
await fsMedia.writeFile(${JSON.stringify(webpPath)}, canvasWebpBytes);
var reopenedCanvasImage = await canvasMedia.loadImage(${JSON.stringify(pngPath)});
await nodeRepl.emitImage(canvasPngBytes);
await nodeRepl.emitImage({ bytes: canvasWebpBytes, mimeType: "image/webp" });
nodeRepl.write(JSON.stringify({ globals: canvasGlobals, point: canvasPoint.x + canvasPoint.y, png_width: reopenedCanvasImage.width, png_height: reopenedCanvasImage.height, webp_width: webpCanvas.width, webp_height: webpCanvas.height }));`,
    },
    {
      label: "sharp",
      tool: "js",
      code: String.raw`
var sharpMedia = (await import("sharp")).default;
var urlMedia = await import("node:url");
var sharpPathMeta = await sharpMedia(${JSON.stringify(pngPath)}).metadata();
var sharpFileUrlPath = urlMedia.fileURLToPath(urlMedia.pathToFileURL(${JSON.stringify(pngPath)}));
var sharpBuffer = await fsMedia.readFile(sharpFileUrlPath);
var sharpBufferMeta = await sharpMedia(sharpBuffer).metadata();
var sharpArrayMeta = await sharpMedia(new Uint8Array(sharpBuffer)).metadata();
var sharpOverlay = Buffer.from("<svg width='40' height='20'><rect width='40' height='20' fill='#ff00aa'/></svg>");
await sharpMedia(sharpBuffer).resize(180, 80, { fit: "cover" }).composite([{ input: sharpOverlay, left: 5, top: 5 }]).png().toFile(${JSON.stringify(sharpPngPath)});
var sharpWrite = await sharpMedia(new Uint8Array(sharpBuffer)).extract({ left: 0, top: 0, width: 360, height: 160 }).resize(180, 80).webp({ quality: 82 }).toFile(${JSON.stringify(sharpWebpPath)});
var sharpOutputMeta = await sharpMedia(${JSON.stringify(sharpWebpPath)}).metadata();
nodeRepl.write(JSON.stringify({ version: sharpMedia.versions.sharp, path_width: sharpPathMeta.width, file_url_width: sharpPathMeta.width, buffer_width: sharpBufferMeta.width, uint8_width: sharpArrayMeta.width, output_format: sharpOutputMeta.format, output_width: sharpOutputMeta.width, output_height: sharpOutputMeta.height, output_size: sharpWrite.size }));`,
    },
    {
      label: "loopback-server",
      tool: "js",
      code: String.raw`
var httpMedia = await import("node:http");
var loopbackRequests = [];
var loopbackServer = httpMedia.createServer(async function (request, response) {
  loopbackRequests.push(request.url || "");
  if (request.url === "/slow" || request.url === "/slow.pdf") { setTimeout(function () { response.end("late"); }, 2000); return; }
  var target = request.url === "/fixture.pdf" ? ${JSON.stringify(pdfPath)} : request.url === "/ocr.png" ? ${JSON.stringify(pngPath)} : null;
  if (target === null) { response.writeHead(404); response.end(); return; }
  response.end(await fsMedia.readFile(target));
});
await new Promise(function (resolvePromise, rejectPromise) { loopbackServer.once("error", rejectPromise); loopbackServer.listen(0, "127.0.0.1", resolvePromise); });
var loopbackAddress = loopbackServer.address();
if (!loopbackAddress || typeof loopbackAddress === "string" || loopbackAddress.address !== "127.0.0.1") throw new Error("media server did not bind IPv4 loopback");
var loopbackBase = "http://127.0.0.1:" + loopbackAddress.port;
nodeRepl.write(JSON.stringify({ host: loopbackAddress.address, family: loopbackAddress.family, loopback_only: true }));`,
    },
    {
      label: "pdf-inputs",
      tool: "js",
      emittedImages: 2,
      code: String.raw`
var canvasPdf = await import("@napi-rs/canvas");
var pdfBytes = await fsMedia.readFile(${JSON.stringify(pdfPath)});
var pdfFileBytes = await fsMedia.readFile(urlMedia.fileURLToPath(urlMedia.pathToFileURL(${JSON.stringify(pdfPath)})));
var pdfOptions = { cMapUrl: nodeRepl.runtime.pdfjs.cMapUrl, cMapPacked: true, standardFontDataUrl: nodeRepl.runtime.pdfjs.standardFontDataUrl, wasmUrl: nodeRepl.runtime.pdfjs.wasmUrl || undefined, useSystemFonts: false, disableFontFace: true, disableWorker: true };
var openPdfBytes = async function (data) {
  return await pdfjsMedia.getDocument({ ...pdfOptions, data }).promise;
};
var pdfFromBytes = await openPdfBytes(new Uint8Array(pdfBytes));
var pdfFromFile = await openPdfBytes(new Uint8Array(pdfFileBytes));
var pdfUrl = loopbackBase + "/fixture.pdf";
var pdfFromUrl = await pdfjsMedia.getDocument({ ...pdfOptions, url: pdfUrl }).promise;
var pdfPageOne = await pdfFromBytes.getPage(1);
var pdfText = (await pdfPageOne.getTextContent()).items.map(function (item) { return item.str || ""; }).join(" ").trim();
var pdfViewport = pdfPageOne.getViewport({ scale: 1 });
var pdfCanvas = canvasPdf.createCanvas(pdfViewport.width, pdfViewport.height);
await pdfPageOne.render({ canvasContext: pdfCanvas.getContext("2d"), viewport: pdfViewport, canvas: pdfCanvas }).promise;
var pdfPngBytes = await pdfCanvas.encode("png");
await fsMedia.writeFile(${JSON.stringify(pdfPngPath)}, pdfPngBytes);
var pdfPageTwo = await pdfFromUrl.getPage(2);
var pdfImageViewport = pdfPageTwo.getViewport({ scale: 1 });
var pdfImageCanvas = canvasPdf.createCanvas(pdfImageViewport.width, pdfImageViewport.height);
await pdfPageTwo.render({ canvasContext: pdfImageCanvas.getContext("2d"), viewport: pdfImageViewport, canvas: pdfImageCanvas }).promise;
var pdfWebpBytes = await pdfImageCanvas.encode("webp", 90);
await fsMedia.writeFile(${JSON.stringify(pdfWebpPath)}, pdfWebpBytes);
await nodeRepl.emitImage(pdfPngBytes);
await nodeRepl.emitImage({ bytes: pdfWebpBytes, mimeType: "image/webp" });
var pdfPages = [pdfFromBytes.numPages, pdfFromFile.numPages, pdfFromUrl.numPages];
await Promise.all([pdfFromBytes.destroy(), pdfFromFile.destroy(), pdfFromUrl.destroy()]);
nodeRepl.write(JSON.stringify({ bytes_pages: pdfPages[0], file_pages: pdfPages[1], direct_url_pages: pdfPages[2], direct_pdfjs_url: true, text: pdfText, width: pdfViewport.width, height: pdfViewport.height, image_page: true }));`,
    },
    {
      label: "tesseract-inputs",
      tool: "js",
      code: String.raw`
var tesseractMedia = await import("tesseract.js");
var ocrBytes = await fsMedia.readFile(${JSON.stringify(pngPath)});
var ocrDataUrl = "data:image/png;base64," + ocrBytes.toString("base64");
var ocrOptions = { langPath: nodeRepl.runtime.tesseract.tessdataRoot, gzip: false, cacheMethod: "none", logger: function () {}, errorHandler: function () {} };
var ocrWorker = await tesseractMedia.createWorker("eng", tesseractMedia.OEM.LSTM_ONLY, ocrOptions);
var osdWorker = await tesseractMedia.createWorker("osd", tesseractMedia.OEM.TESSERACT_ONLY, ocrOptions);
var ocrResults = [];
var malformedOcrError = false;
var osdResult;
try {
  await ocrWorker.setParameters({ tessedit_pageseg_mode: tesseractMedia.PSM.SINGLE_LINE });
  for (var ocrInput of [${JSON.stringify(pngPath)}, ocrBytes, ocrDataUrl, loopbackBase + "/ocr.png"]) ocrResults.push((await ocrWorker.recognize(ocrInput)).data);
  osdResult = (await osdWorker.detect(ocrBytes)).data;
  try { await ocrWorker.recognize(${JSON.stringify(malformedImagePath)}); } catch { malformedOcrError = true; }
} finally { await Promise.all([ocrWorker.terminate(), osdWorker.terminate()]); }
var normalizedOcr = ocrResults.map(function (result) { return result.text.replace(/\s+/gu, " ").trim().toUpperCase(); });
var minimumConfidence = Math.min.apply(null, ocrResults.map(function (result) { return result.confidence; }));
nodeRepl.write(JSON.stringify({ version: tesseractMedia.default && tesseractMedia.default.version ? tesseractMedia.default.version : "7", path: normalizedOcr[0], bytes: normalizedOcr[1], data_url: normalizedOcr[2], localhost_url: normalizedOcr[3], minimum_confidence: minimumConfidence, local_tessdata: true, osd_tessdata_exercised: osdResult !== undefined, malformed_error: malformedOcrError }));`,
    },
    {
      label: "pixelmatch",
      tool: "js",
      emittedImages: 1,
      code: String.raw`
var pixelmatchMedia = (await import("pixelmatch")).default;
var pixelLeft = Buffer.alloc(4 * 4 * 4, 255);
var pixelRight = Buffer.from(pixelLeft);
var pixelDiff = Buffer.alloc(pixelLeft.length);
var identicalPixels = pixelmatchMedia(pixelLeft, pixelLeft, pixelDiff, 4, 4, { threshold: 0, includeAA: true });
pixelRight[(2 * 4 + 1) * 4] = 0;
pixelRight[(2 * 4 + 1) * 4 + 1] = 0;
pixelRight[(2 * 4 + 1) * 4 + 2] = 0;
var differingPixels = pixelmatchMedia(pixelLeft, pixelRight, pixelDiff, 4, 4, { threshold: 0, includeAA: true });
await sharpMedia(pixelDiff, { raw: { width: 4, height: 4, channels: 4 } }).png().toFile(${JSON.stringify(diffPath)});
var diffBytes = await fsMedia.readFile(${JSON.stringify(diffPath)});
await nodeRepl.emitImage(diffBytes);
var sizeMismatch = false;
try { pixelmatchMedia(pixelLeft, Buffer.alloc(3), pixelDiff, 4, 4); } catch { sizeMismatch = true; }
nodeRepl.write(JSON.stringify({ identical_pixels: identicalPixels, differing_pixels: differingPixels, size_mismatch_error: sizeMismatch }));`,
    },
    {
      label: "malformed-and-abort",
      tool: "js",
      code: String.raw`
var missingFileError = false;
var malformedImageError = false;
var malformedPdfError = false;
var abortedFetch = false;
var abortedPdfBytesFetch = false;
try { await sharpMedia(${JSON.stringify(path("missing image.png"))}).metadata(); } catch { missingFileError = true; }
try { await sharpMedia(${JSON.stringify(malformedImagePath)}).metadata(); } catch { malformedImageError = true; }
try { await (await openPdfBytes(new Uint8Array(await fsMedia.readFile(${JSON.stringify(malformedPdfPath)})))).destroy(); } catch { malformedPdfError = true; }
var abortController = new AbortController();
var abortPromise = fetch(loopbackBase + "/slow", { signal: abortController.signal });
abortController.abort(new Error("acceptance abort"));
try { await abortPromise; } catch (error) { abortedFetch = error && (error.name === "AbortError" || error.message === "acceptance abort"); }
var pdfAbortController = new AbortController();
var abortPdfBytesFetch = fetch(loopbackBase + "/slow.pdf", { signal: pdfAbortController.signal });
pdfAbortController.abort(new Error("PDF bytes fetch acceptance abort"));
try { await abortPdfBytesFetch; } catch (error) { abortedPdfBytesFetch = error && (error.name === "AbortError" || error.message === "PDF bytes fetch acceptance abort"); }
try { await sharpMedia(${JSON.stringify(malformedImagePath)}).png().toFile(${JSON.stringify(transientPath)}); } catch {}
await fsMedia.rm(${JSON.stringify(transientPath)}, { force: true });
var transientRemoved = false;
try { await fsMedia.access(${JSON.stringify(transientPath)}); } catch { transientRemoved = true; }
nodeRepl.write(JSON.stringify({ missing_file_error: missingFileError, malformed_image_error: malformedImageError, malformed_pdf_error: malformedPdfError, aborted_url: abortedFetch, aborted_generic_pdf_bytes_fetch: abortedPdfBytesFetch, partial_output_removed: transientRemoved }));`,
    },
    {
      label: "persistent-cells",
      tool: "js",
      code: String.raw`
var persistedFiles = (await Promise.all([${JSON.stringify(pngPath)}, ${JSON.stringify(sharpWebpPath)}, ${JSON.stringify(pdfPngPath)}, ${JSON.stringify(diffPath)}].map(async function (file) { try { await fsMedia.access(file); return true; } catch { return false; } }))).every(Boolean);
var repeatedSharp = (await import("sharp")).default === sharpMedia;
var repeatedPdf = await import("pdfjs-dist/legacy/build/pdf.mjs") === pdfjsMedia;
nodeRepl.write(JSON.stringify({ files: persistedFiles, sharp_identity: repeatedSharp, pdf_identity: repeatedPdf, request_count: loopbackRequests.length }));`,
    },
    {
      label: "loopback-cleanup",
      tool: "js",
      code: String.raw`
await new Promise(function (resolvePromise, rejectPromise) { loopbackServer.close(function (error) { if (error) rejectPromise(error); else resolvePromise(); }); });
var onlyLoopbackRoutes = loopbackRequests.every(function (requestPath) { return requestPath === "/fixture.pdf" || requestPath === "/ocr.png" || requestPath === "/slow" || requestPath === "/slow.pdf"; });
nodeRepl.write(JSON.stringify({ closed: true, only_loopback_routes: onlyLoopbackRoutes, requests: loopbackRequests.length }));`,
    },
    { label: "reset", tool: "js_reset" },
    {
      label: "after-reset",
      tool: "js",
      emittedImages: 1,
      code: String.raw`
var fsAfterReset = await import("node:fs/promises");
var resetBindingsGone = typeof sourceCanvas === "undefined" && typeof sharpMedia === "undefined" && typeof pdfjsMedia === "undefined" && typeof loopbackServer === "undefined";
var resetFileBytes = await fsAfterReset.readFile(${JSON.stringify(diffPath)});
await nodeRepl.emitImage(resetFileBytes);
nodeRepl.write(JSON.stringify({ bindings_gone: resetBindingsGone, files_survive: resetFileBytes.length > 32, runtime_survives: nodeRepl.runtime.version === 1 }));`,
    },
  ];
}

function verifyArtifacts(
  runtimeRoot: string,
  tempRoot: string,
  cells: Record<string, CellEvidence>,
): CellEvidence {
  const root = join(tempRoot, "media fixtures - 日本語");
  const pngPath = join(root, "canvas source π.png");
  const canvasWebpPath = join(root, "canvas source π.webp");
  const sharpPngPath = join(root, "sharp output Ω.png");
  const sharpWebpPath = join(root, "sharp output Ω.webp");
  const pdfPath = join(root, "input vector + image.pdf");
  const pdfPngPath = join(root, "pdf text page.png");
  const pdfWebpPath = join(root, "pdf image page.webp");
  const diffPath = join(root, "pixel diff Δ.png");
  const png = fileEvidence(pngPath, "89504e470d0a1a0a");
  const canvasWebp = readFileSync(canvasWebpPath);
  if (
    canvasWebp.subarray(0, 4).toString("ascii") !== "RIFF" ||
    canvasWebp.subarray(8, 12).toString("ascii") !== "WEBP"
  )
    throw new Error("canvas.webp has an invalid WebP signature");
  const sharpWebp = readFileSync(sharpWebpPath);
  if (
    sharpWebp.subarray(0, 4).toString("ascii") !== "RIFF" ||
    sharpWebp.subarray(8, 12).toString("ascii") !== "WEBP"
  )
    throw new Error("sharp-output.webp has an invalid WebP signature");
  const pdf = readFileSync(pdfPath);
  if (pdf.subarray(0, 5).toString("ascii") !== "%PDF-")
    throw new Error("input.pdf has an invalid signature");
  const pdfPng = fileEvidence(pdfPngPath, "89504e470d0a1a0a");
  const pdfWebp = readFileSync(pdfWebpPath);
  if (
    pdfWebp.subarray(0, 4).toString("ascii") !== "RIFF" ||
    pdfWebp.subarray(8, 12).toString("ascii") !== "WEBP"
  )
    throw new Error("pdf image page.webp has an invalid WebP signature");
  const diff = fileEvidence(diffPath, "89504e470d0a1a0a");
  const sharpPng = fileEvidence(sharpPngPath, "89504e470d0a1a0a");
  verifyDecodedImages(runtimeRoot, [
    {
      path: pngPath,
      format: "png",
      width: 720,
      height: 160,
      nonblank: true,
    },
    {
      path: canvasWebpPath,
      format: "webp",
      width: 64,
      height: 48,
      nonblank: true,
    },
    {
      path: sharpPngPath,
      format: "png",
      width: 180,
      height: 80,
      nonblank: true,
    },
    {
      path: sharpWebpPath,
      format: "webp",
      width: 180,
      height: 80,
      nonblank: true,
    },
    {
      path: pdfPngPath,
      format: "png",
      width: 320,
      height: 180,
      nonblank: true,
    },
    {
      path: pdfWebpPath,
      format: "webp",
      width: 320,
      height: 180,
      nonblank: true,
    },
    {
      path: diffPath,
      format: "png",
      width: 4,
      height: 4,
      nonblank: true,
    },
  ]);
  const directPdf = cells["pdf-direct-import"];
  const canvas = cells.canvas;
  const sharp = cells.sharp;
  const pdfjs = cells["pdf-inputs"];
  const tesseract = cells["tesseract-inputs"];
  const pixelmatch = cells.pixelmatch;
  const malformed = cells["malformed-and-abort"];
  const persistent = cells["persistent-cells"];
  const loopback = cells["loopback-cleanup"];
  const afterReset = cells["after-reset"];
  if (
    directPdf?.direct_import !== true ||
    directPdf.runtime_version !== 1 ||
    canvas?.globals !== true ||
    canvas.png_width !== 720 ||
    canvas.png_height !== 160 ||
    canvas.webp_width !== 64 ||
    canvas.webp_height !== 48 ||
    canvas.emitted_images !== 2
  )
    throw new Error("zero-preamble Canvas or direct PDF import evidence is incomplete");
  if (
    sharp?.path_width !== 720 ||
    sharp.file_url_width !== 720 ||
    sharp.buffer_width !== 720 ||
    sharp.uint8_width !== 720 ||
    sharp.output_format !== "webp" ||
    sharp.output_width !== 180 ||
    sharp.output_height !== 80
  )
    throw new Error("Sharp input/output matrix evidence is incomplete");
  if (
    pdfjs?.bytes_pages !== 2 ||
    pdfjs.file_pages !== 2 ||
    pdfjs.direct_url_pages !== 2 ||
    pdfjs.direct_pdfjs_url !== true ||
    pdfjs.text !== "ZERO PREAMBLE PDF" ||
    pdfjs.width !== 320 ||
    pdfjs.height !== 180 ||
    pdfjs.image_page !== true ||
    pdfjs.emitted_images !== 2
  )
    throw new Error("PDF.js input/render matrix evidence is incomplete");
  if (
    tesseract?.path !== OCR_PHRASE ||
    tesseract.bytes !== OCR_PHRASE ||
    tesseract.data_url !== OCR_PHRASE ||
    tesseract.localhost_url !== OCR_PHRASE ||
    typeof tesseract.minimum_confidence !== "number" ||
    tesseract.minimum_confidence < 50 ||
    tesseract.local_tessdata !== true ||
    tesseract.osd_tessdata_exercised !== true ||
    tesseract.malformed_error !== true
  )
    throw new Error(
      `Tesseract input matrix mismatch: ${String(tesseract?.path ?? "missing")}`,
    );
  if (
    pixelmatch?.identical_pixels !== 0 ||
    pixelmatch.differing_pixels !== 1 ||
    pixelmatch.size_mismatch_error !== true ||
    pixelmatch.emitted_images !== 1
  )
    throw new Error("pixelmatch output/error evidence is incomplete");
  if (
    malformed?.missing_file_error !== true ||
    malformed.malformed_image_error !== true ||
    malformed.malformed_pdf_error !== true ||
    malformed.aborted_url !== true ||
    malformed.aborted_generic_pdf_bytes_fetch !== true ||
    malformed.partial_output_removed !== true
  )
    throw new Error("malformed input, abort, or cleanup evidence is incomplete");
  if (
    persistent?.files !== true ||
    persistent.sharp_identity !== true ||
    persistent.pdf_identity !== true ||
    loopback?.closed !== true ||
    loopback.only_loopback_routes !== true ||
    afterReset?.bindings_gone !== true ||
    afterReset.files_survive !== true ||
    afterReset.runtime_survives !== true ||
    afterReset.emitted_images !== 1
  )
    throw new Error("persistence, reset, cleanup, or loopback evidence is incomplete");
  return {
    cell_count: Object.keys(cells).length,
    canvas_png_bytes: png.bytes ?? 0,
    canvas_webp_bytes: canvasWebp.length,
    sharp_webp_bytes: sharpWebp.length,
    sharp_png_bytes: sharpPng.bytes ?? 0,
    pdf_bytes: pdf.length,
    pdf_png_bytes: pdfPng.bytes ?? 0,
    pdf_webp_bytes: pdfWebp.length,
    diff_png_bytes: diff.bytes ?? 0,
    canvas_png_sha256: png.sha256 ?? "",
    sharp_png_sha256: sharpPng.sha256 ?? "",
    pdf_png_sha256: pdfPng.sha256 ?? "",
    diff_png_sha256: diff.sha256 ?? "",
    recognized: OCR_PHRASE,
    pdf_text: "ZERO PREAMBLE PDF",
    pdf_url_contract: "pdfjs-node-direct-url",
    pdfjs_version: String(directPdf.version ?? ""),
    sharp_version: String(sharp.version ?? ""),
    tesseract_version: String(tesseract.version ?? ""),
    osd_tessdata_exercised: true,
    loopback_requests: Number(loopback.requests ?? 0),
    cleanup: true,
    reset: true,
  };
}

async function runAcceptance(options: AcceptanceOptions): Promise<AcceptanceReport> {
  const tempRoot = mkdtempSync(join(tmpdir(), "cua-node-repl-acceptance-"));
  prepareMediaFixtures(tempRoot);
  const nodeRepl = join(options.runtimeRoot, "bin/node_repl");
  const session = startMcpSession({
    executable: nodeRepl,
    cwd: tempRoot,
    timeoutMs: options.timeoutMs + 5_000,
    env: {
      ...process.env,
      HOME: tempRoot,
      XDG_CACHE_HOME: join(tempRoot, "cache"),
      NODE_REPL_NODE_PATH: join(options.runtimeRoot, "bin/node"),
      NODE_REPL_NODE_MODULE_DIRS: join(options.runtimeRoot, "lib/node_modules"),
      PLAYWRIGHT_BROWSERS_PATH: "0",
      TESSDATA_PREFIX: join(options.runtimeRoot, "share/tessdata"),
      NO_PROXY: "*",
      no_proxy: "*",
    },
  });
  const child = session.child;
  let initialized = false;
  let expectCleanExit = false;
  const request = session.request;
  try {
    const initialize = await request("initialize", {
      protocolVersion: "2025-11-25",
      capabilities: {},
      clientInfo: { name: "repl-subsystem-acceptance", version: "1" },
    });
    if (initialize.error !== undefined || !isObject(initialize.result))
      throw new Error(`initialize failed: ${JSON.stringify(initialize.error)}`);
    initialized = true;
    const tools = await request("tools/list", {});
    if (
      tools.error !== undefined ||
      !isObject(tools.result) ||
      !Array.isArray(tools.result.tools)
    )
      throw new Error(`tools/list failed: ${JSON.stringify(tools.error)}`);
    const names = new Set(
      tools.result.tools
        .filter(isObject)
        .map((tool) => tool.name)
        .filter((name): name is string => typeof name === "string"),
    );
    if (!names.has("js") || !names.has("js_reset"))
      throw new Error("assembled node_repl is missing js or js_reset");
    const cells: Record<string, CellEvidence> = {};
    for (const cell of buildCells(options, tempRoot)) {
      try {
        const response = await request(
          "tools/call",
          cell.tool === "js_reset"
            ? { name: "js_reset", arguments: {} }
            : {
                name: "js",
                arguments: {
                  code: cell.code,
                  timeout_ms: options.timeoutMs,
                  title: `Accept ${cell.label}`,
                },
              },
          options.timeoutMs,
        );
        if (cell.tool === "js_reset") {
          if (response.error !== undefined || !isObject(response.result))
            throw new Error(`MCP error: ${JSON.stringify(response.error)}`);
          const content = Array.isArray(response.result.content)
            ? response.result.content
            : [];
          const resetText = content.find(
            (candidate) => isObject(candidate) && candidate.type === "text",
          );
          if (!isObject(resetText) || resetText.text !== "true")
            throw new Error("did not return true");
          cells[cell.label] = { reset: true };
        } else {
          cells[cell.label] = parseCellResult(
            response,
            cell.label,
            cell.emittedImages ?? 0,
          );
        }
      } catch (error) {
        throw new Error(`matrix row ${cell.label} blocked: ${errorText(error)}`);
      }
    }
    const checks = verifyArtifacts(options.runtimeRoot, tempRoot, cells);
    const shutdown = await request("shutdown", {});
    initialized = false;
    if (shutdown.error !== undefined || shutdown.result !== null)
      throw new Error(`shutdown failed: ${JSON.stringify(shutdown.error)}`);
    expectCleanExit = true;
    return { schema: SCHEMA, schema_version: 1, status: "passed", checks };
  } finally {
    if (initialized && child.exitCode === null) {
      try {
        await request("shutdown", {}, 5_000);
      } catch {}
    }
    let exit: ChildExit;
    try {
      exit = await session.close(DEFAULT_TEARDOWN_WAIT_MS);
    } catch (error) {
      throw new Error(
        `node_repl did not exit after SIGKILL; temporary root retained: ${tempRoot}`,
        { cause: error },
      );
    }
    rmSync(tempRoot, { recursive: true, force: true });
    if (existsSync(tempRoot))
      throw new Error(`temporary root was not removed: ${tempRoot}`);
    if (expectCleanExit && (exit.code !== 0 || exit.signal !== null))
      throw new Error(
        `node_repl exit was ${JSON.stringify(exit)}: ${session.stderr().trim()}`,
      );
  }
}

async function main(argv = process.argv.slice(2)): Promise<number> {
  try {
    const report = await runAcceptance(parseArgs(argv));
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return 0;
  } catch (error) {
    const report: AcceptanceReport = {
      schema: SCHEMA,
      schema_version: 1,
      status: "failed",
      checks: {},
      error: errorText(error),
    };
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return 1;
  }
}

exports.buildCells = buildCells;
exports.parseArgs = parseArgs;
exports.runAcceptance = runAcceptance;
exports.teardownNodeRepl = teardownNodeRepl;
exports.verifyArtifacts = verifyArtifacts;

if (require.main === module)
  void main().then((exitCode) => {
    process.exitCode = exitCode;
  });
