const { spawnSync } = require("node:child_process");
const { createHash } = require("node:crypto");
const {
  accessSync,
  constants,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
} = require("node:fs");
const { tmpdir } = require("node:os");
const { join, resolve } = require("node:path");
const { pathToFileURL } = require("node:url");
const { resolveDefaultBrowserExecutable } =
  require("../src/host/runtime-asset-discovery.ts") as typeof import("../src/host/runtime-asset-discovery.ts");

type EvidenceValue = string | number | boolean;
type AcceptanceCheck = {
  id: string;
  status: "passed" | "failed";
  evidence: Record<string, EvidenceValue>;
  error?: string;
};
type AcceptanceReport = {
  schema: "com.heliasar.cua-node.full-subsystem-acceptance";
  schema_version: 1;
  status: "passed" | "failed";
  target: "linux-x64-glibc";
  network: "disabled";
  user_cache: "disposable-empty";
  checks: AcceptanceCheck[];
};
type CliOptions = {
  runtimeRoot: string;
  chromiumExecutable: string;
  target: string;
  networkDisabled: boolean;
  emptyUserCache: boolean;
  internalRun: boolean;
  tempRoot?: string;
};

const SCHEMA = "com.heliasar.cua-node.full-subsystem-acceptance" as const;
const EXPECTED_NODE_VERSION = "v24.14.0";
const PHRASE = "OFFLINE CUA NODE";
const CHECK_IDS = [
  "runtime",
  "canvas-png",
  "canvas-webp",
  "sharp",
  "pdfjs",
  "tesseract",
  "pixelmatch",
  "playwright",
  "cleanup",
] as const;
const PACKAGE_VERSIONS: ReadonlyArray<readonly [string, string]> = [
  ["@napi-rs/canvas", "0.1.91"],
  ["sharp", "0.34.5"],
  ["pdfjs-dist", "5.4.624"],
  ["tesseract.js", "7.0.0"],
  ["tesseract.js-core", "7.0.0"],
  ["pixelmatch", "7.1.0"],
  ["playwright", "1.57.0"],
  ["playwright-core", "1.57.0"],
];

function sha256(value: Uint8Array): string {
  return createHash("sha256").update(value).digest("hex");
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : "unknown acceptance failure";
}

function emptyReport(): AcceptanceReport {
  return {
    schema: SCHEMA,
    schema_version: 1,
    status: "failed",
    target: "linux-x64-glibc",
    network: "disabled",
    user_cache: "disposable-empty",
    checks: [],
  };
}

function failureReport(id: string, error: unknown): AcceptanceReport {
  const report = emptyReport();
  report.checks.push({
    id,
    status: "failed",
    evidence: {},
    error: errorText(error),
  });
  return report;
}

function assertExecutable(path: string, label: string): void {
  if (!resolve(path).startsWith("/")) throw new Error(`${label} must be absolute`);
  try {
    accessSync(path, constants.R_OK | constants.X_OK);
  } catch {
    throw new Error(`${label} is not a readable executable: ${path}`);
  }
}

function optionValue(argument: string, name: string): string | null {
  const prefix = `${name}=`;
  return argument.startsWith(prefix) ? argument.slice(prefix.length) : null;
}

function parseAcceptanceArgs(argv: string[]): CliOptions {
  let runtimeRoot: string | undefined;
  let chromiumExecutable: string | undefined;
  let target: string | undefined;
  let networkDisabled = false;
  let emptyUserCache = false;
  let internalRun = false;
  let tempRoot: string | undefined;
  for (const argument of argv) {
    const rootValue = optionValue(argument, "--runtime-root");
    const chromiumValue = optionValue(argument, "--chromium-executable");
    const targetValue = optionValue(argument, "--target");
    const tempValue = optionValue(argument, "--temp-root");
    if (rootValue !== null) runtimeRoot = rootValue;
    else if (chromiumValue !== null) chromiumExecutable = chromiumValue;
    else if (targetValue !== null) target = targetValue;
    else if (tempValue !== null) tempRoot = tempValue;
    else if (argument === "--network=disabled") networkDisabled = true;
    else if (argument === "--empty-user-cache") emptyUserCache = true;
    else if (argument === "--internal-run") internalRun = true;
    else if (argument === "--json") continue;
    else throw new Error(`unknown argument: ${argument}`);
  }
  if (runtimeRoot === undefined || runtimeRoot.length === 0)
    throw new Error("--runtime-root=PATH is required");
  if (chromiumExecutable === undefined || chromiumExecutable.length === 0) {
    const discovered = resolveDefaultBrowserExecutable();
    if (discovered === null)
      throw new Error(
        "--chromium-executable=PATH is required when no supported browser is discoverable",
      );
    chromiumExecutable = discovered.executablePath;
  }
  if (target !== "linux-x64") throw new Error("--target=linux-x64 is required");
  if (!networkDisabled) throw new Error("--network=disabled is required");
  if (!emptyUserCache) throw new Error("--empty-user-cache is required");
  if (internalRun && (tempRoot === undefined || tempRoot.length === 0))
    throw new Error("internal execution requires --temp-root=PATH");
  return {
    runtimeRoot: resolve(runtimeRoot),
    chromiumExecutable: resolve(chromiumExecutable),
    target,
    networkDisabled,
    emptyUserCache,
    internalRun,
    tempRoot: tempRoot === undefined ? undefined : resolve(tempRoot),
  };
}

function packageRoot(runtimeRoot: string, packageName: string): string {
  return join(runtimeRoot, "lib/node_modules", packageName);
}

function requireBundled(runtimeRoot: string, packageName: string): unknown {
  return require(packageRoot(runtimeRoot, packageName));
}

async function importBundled(
  runtimeRoot: string,
  relativePath: string,
): Promise<unknown> {
  return import(
    pathToFileURL(join(runtimeRoot, "lib/node_modules", relativePath)).href
  );
}

function countFiles(path: string): number {
  return readdirSync(path, { withFileTypes: true }).filter(
    (entry: { isFile(): boolean }) => entry.isFile(),
  ).length;
}

function verifyRuntime(options: CliOptions): Record<string, EvidenceValue> {
  const expectedNode = resolve(options.runtimeRoot, "bin/node");
  assertExecutable(expectedNode, "bundled Node");
  assertExecutable(options.chromiumExecutable, "Chromium executable");
  if (resolve(process.execPath) !== expectedNode)
    throw new Error(
      `acceptance child must run ${expectedNode}; got ${process.execPath}`,
    );
  if (process.version !== EXPECTED_NODE_VERSION)
    throw new Error(
      `bundled Node must be ${EXPECTED_NODE_VERSION}; got ${process.version}`,
    );
  for (const [name, expectedVersion] of PACKAGE_VERSIONS) {
    const packageJsonPath = join(
      packageRoot(options.runtimeRoot, name),
      "package.json",
    );
    const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8")) as {
      name?: unknown;
      version?: unknown;
    };
    if (packageJson.name !== name || packageJson.version !== expectedVersion)
      throw new Error(`expected ${name}@${expectedVersion} at ${packageJsonPath}`);
  }
  const tessdata = join(options.runtimeRoot, "share/tessdata");
  for (const language of ["eng", "osd"])
    if (!existsSync(join(tessdata, `${language}.traineddata`)))
      throw new Error(`bundled tessdata is absent: ${language}.traineddata`);
  const cmaps = join(options.runtimeRoot, "share/pdfjs/cmaps");
  const fonts = join(options.runtimeRoot, "share/pdfjs/standard_fonts");
  const cmapFiles = countFiles(cmaps);
  const fontFiles = countFiles(fonts);
  if (cmapFiles === 0 || fontFiles === 0)
    throw new Error("bundled PDF.js CMaps and standard fonts must be populated");
  return {
    node_version: process.version,
    package_count: PACKAGE_VERSIONS.length,
    tessdata: "eng,osd",
    pdfjs_cmap_files: cmapFiles,
    pdfjs_standard_font_files: fontFiles,
    chromium_explicit: true,
  };
}

function installNetworkGuard(): void {
  const denied = (): never => {
    throw new Error("network access is disabled by full subsystem acceptance");
  };
  globalThis.fetch = denied as typeof globalThis.fetch;
  const http = require("node:http") as { request: unknown; get: unknown };
  const https = require("node:https") as { request: unknown; get: unknown };
  http.request = denied;
  http.get = denied;
  https.request = denied;
  https.get = denied;
}

function makePdf(text: string): Uint8Array {
  const escaped = text.replace(/([()\\])/gu, "\\$1");
  const stream = `BT /F1 24 Tf 40 100 Td (${escaped}) Tj ET`;
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 320 180] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
    `<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let pdf = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((object, index) => {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xrefOffset = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  for (const offset of offsets.slice(1))
    pdf += `${String(offset).padStart(10, "0")} 00000 n \n`;
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`;
  return new Uint8Array(Buffer.from(pdf, "ascii"));
}

async function executeChecks(options: CliOptions): Promise<AcceptanceReport> {
  const report = emptyReport();
  const tempRoot = options.tempRoot;
  if (tempRoot === undefined) return failureReport("cli", "missing internal temp root");
  const add = async (
    id: string,
    operation: () =>
      | Promise<Record<string, EvidenceValue>>
      | Record<string, EvidenceValue>,
  ): Promise<void> => {
    try {
      report.checks.push({ id, status: "passed", evidence: await operation() });
    } catch (error) {
      report.checks.push({
        id,
        status: "failed",
        evidence: {},
        error: errorText(error),
      });
    }
  };

  let canvasModule:
    | {
        createCanvas(
          width: number,
          height: number,
        ): {
          width: number;
          height: number;
          getContext(kind: "2d"): Record<string, unknown>;
          encode(format: "png" | "webp", quality?: number): Promise<Buffer>;
        };
        DOMMatrix: unknown;
        ImageData: unknown;
        Path2D: unknown;
      }
    | undefined;
  let phrasePng: Buffer | undefined;

  await add("runtime", () => verifyRuntime(options));
  if (report.checks[0]?.status !== "passed") {
    for (const id of CHECK_IDS.slice(1, -1))
      report.checks.push({
        id,
        status: "failed",
        evidence: {},
        error: "runtime preflight failed",
      });
  } else {
    installNetworkGuard();
    await add("canvas-png", async () => {
      canvasModule = requireBundled(
        options.runtimeRoot,
        "@napi-rs/canvas",
      ) as typeof canvasModule;
      if (canvasModule === undefined) throw new Error("Canvas module did not load");
      const canvas = canvasModule.createCanvas(720, 160);
      const context = canvas.getContext("2d") as {
        fillStyle: string;
        font: string;
        textBaseline: string;
        fillRect(x: number, y: number, width: number, height: number): void;
        fillText(text: string, x: number, y: number): void;
      };
      context.fillStyle = "#ffffff";
      context.fillRect(0, 0, 720, 160);
      context.fillStyle = "#000000";
      context.font = "bold 64px sans-serif";
      context.textBaseline = "middle";
      context.fillText(PHRASE, 24, 80);
      phrasePng = await canvas.encode("png");
      if (phrasePng.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a")
        throw new Error("Canvas PNG has an invalid signature");
      return {
        width: 720,
        height: 160,
        bytes: phrasePng.length,
        sha256: sha256(phrasePng),
      };
    });
    await add("canvas-webp", async () => {
      if (canvasModule === undefined)
        throw new Error("Canvas PNG check did not load Canvas");
      const canvas = canvasModule.createCanvas(64, 48);
      const context = canvas.getContext("2d") as {
        fillStyle: string;
        fillRect(x: number, y: number, width: number, height: number): void;
      };
      context.fillStyle = "#183153";
      context.fillRect(0, 0, 64, 48);
      context.fillStyle = "#ffb000";
      context.fillRect(16, 12, 32, 24);
      const canvasWebp = await canvas.encode("webp", 90);
      if (
        canvasWebp.subarray(0, 4).toString("ascii") !== "RIFF" ||
        canvasWebp.subarray(8, 12).toString("ascii") !== "WEBP"
      )
        throw new Error("Canvas WebP has an invalid container signature");
      return {
        width: 64,
        height: 48,
        bytes: canvasWebp.length,
        sha256: sha256(canvasWebp),
      };
    });
    await add("sharp", async () => {
      if (phrasePng === undefined) throw new Error("Canvas PNG is unavailable");
      const sharp = requireBundled(options.runtimeRoot, "sharp") as (
        input: Uint8Array,
      ) => {
        resize(
          width: number,
          height: number,
        ): {
          webp(options: { quality: number }): {
            toBuffer(options: { resolveWithObject: true }): Promise<{
              data: Buffer;
              info: { format: string; width: number; height: number };
            }>;
          };
        };
      };
      const result = await sharp(phrasePng)
        .resize(360, 80)
        .webp({ quality: 82 })
        .toBuffer({ resolveWithObject: true });
      if (
        result.info.format !== "webp" ||
        result.info.width !== 360 ||
        result.info.height !== 80
      )
        throw new Error("Sharp resize/WebP metadata is incorrect");
      return {
        input: "canvas-png",
        format: result.info.format,
        width: result.info.width,
        height: result.info.height,
        bytes: result.data.length,
        sha256: sha256(result.data),
      };
    });
    await add("pdfjs", async () => {
      if (canvasModule === undefined) throw new Error("Canvas module is unavailable");
      Object.assign(globalThis, {
        DOMMatrix: canvasModule.DOMMatrix,
        ImageData: canvasModule.ImageData,
        Path2D: canvasModule.Path2D,
      });
      const pdfjs = (await importBundled(
        options.runtimeRoot,
        "pdfjs-dist/legacy/build/pdf.mjs",
      )) as {
        version: string;
        getDocument(options: Record<string, unknown>): {
          promise: Promise<{
            numPages: number;
            getPage(pageNumber: number): Promise<{
              getViewport(options: { scale: number }): {
                width: number;
                height: number;
              };
              render(options: Record<string, unknown>): {
                promise: Promise<void>;
              };
            }>;
            destroy(): Promise<void>;
          }>;
        };
      };
      const cmaps = join(options.runtimeRoot, "share/pdfjs/cmaps/");
      const fonts = join(options.runtimeRoot, "share/pdfjs/standard_fonts/");
      const loadingTask = pdfjs.getDocument({
        data: makePdf("OFFLINE PDF RENDER"),
        cMapUrl: cmaps,
        cMapPacked: true,
        standardFontDataUrl: fonts,
        useSystemFonts: false,
        disableFontFace: true,
      });
      const document = await loadingTask.promise;
      try {
        const page = await document.getPage(1);
        const viewport = page.getViewport({ scale: 1 });
        const canvas = canvasModule.createCanvas(viewport.width, viewport.height);
        const context = canvas.getContext("2d");
        await page.render({ canvasContext: context, viewport, canvas }).promise;
        const png = await canvas.encode("png");
        if (png.length < 500)
          throw new Error("PDF.js produced an unexpectedly small render");
        return {
          version: pdfjs.version,
          pages: document.numPages,
          width: viewport.width,
          height: viewport.height,
          bytes: png.length,
          sha256: sha256(png),
          bundled_cmaps: true,
          bundled_standard_fonts: true,
          canvas: "@napi-rs/canvas",
        };
      } finally {
        await document.destroy();
      }
    });
    await add("tesseract", async () => {
      if (phrasePng === undefined) throw new Error("OCR image is unavailable");
      const tesseract = requireBundled(options.runtimeRoot, "tesseract.js") as {
        OEM: { LSTM_ONLY: number; TESSERACT_ONLY: number };
        PSM: { SINGLE_LINE: string };
        createWorker(
          languages: string,
          oem: number,
          options: Record<string, unknown>,
        ): Promise<{
          setParameters(parameters: Record<string, string>): Promise<unknown>;
          recognize(image: Uint8Array): Promise<{ data: { text: string } }>;
          detect(image: Uint8Array): Promise<{ data: unknown }>;
          terminate(): Promise<unknown>;
        }>;
      };
      const langPath = join(options.runtimeRoot, "share/tessdata");
      const common = {
        langPath,
        gzip: false,
        cacheMethod: "none",
        logger: () => {},
      };
      const worker = await tesseract.createWorker(
        "eng",
        tesseract.OEM.LSTM_ONLY,
        common,
      );
      let recognized = "";
      try {
        await worker.setParameters({
          tessedit_pageseg_mode: tesseract.PSM.SINGLE_LINE,
        });
        const result = await worker.recognize(phrasePng);
        recognized = result.data.text.replace(/\s+/gu, " ").trim().toUpperCase();
      } finally {
        await worker.terminate();
      }
      if (recognized !== PHRASE)
        throw new Error(`OCR mismatch: expected ${PHRASE}; got ${recognized}`);
      const osdWorker = await tesseract.createWorker(
        "osd",
        tesseract.OEM.TESSERACT_ONLY,
        { ...common, legacyCore: true, legacyLang: true },
      );
      try {
        await osdWorker.detect(phrasePng);
      } finally {
        await osdWorker.terminate();
      }
      return {
        version: "7.0.0",
        languages: "eng,osd",
        recognized,
        cache: "disabled",
      };
    });
    await add("pixelmatch", async () => {
      const imported = (await importBundled(
        options.runtimeRoot,
        "pixelmatch/index.js",
      )) as {
        default: (
          a: Uint8Array,
          b: Uint8Array,
          output: Uint8Array,
          width: number,
          height: number,
          options: Record<string, unknown>,
        ) => number;
      };
      const left = new Uint8Array(4 * 4 * 4).fill(255);
      const right = new Uint8Array(left);
      const output = new Uint8Array(left.length);
      right.set([0, 0, 0, 255], (2 * 4 + 1) * 4);
      const differingPixels = imported.default(left, right, output, 4, 4, {
        threshold: 0,
        includeAA: true,
      });
      if (differingPixels !== 1)
        throw new Error(`expected one differing pixel; got ${differingPixels}`);
      return {
        version: "7.1.0",
        width: 4,
        height: 4,
        differing_pixels: differingPixels,
        diff_sha256: sha256(output),
      };
    });
    await add("playwright", async () => {
      const playwright = requireBundled(options.runtimeRoot, "playwright") as {
        chromium: {
          launch(options: Record<string, unknown>): Promise<{
            newPage(): Promise<{
              setContent(html: string): Promise<void>;
              click(selector: string): Promise<void>;
              keyboard: { type(text: string): Promise<void> };
              textContent(selector: string): Promise<string | null>;
            }>;
            close(): Promise<void>;
          }>;
        };
      };
      const browser = await playwright.chromium.launch({
        executablePath: options.chromiumExecutable,
        headless: true,
        args: [
          "--disable-background-networking",
          "--disable-component-update",
          "--disable-default-apps",
          "--disable-sync",
          "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost",
          "--no-first-run",
          "--no-sandbox",
        ],
      });
      try {
        const page = await browser.newPage();
        await page.setContent(
          "<!doctype html><input id='entry'><button id='commit' onclick=\"document.querySelector('#result').textContent=document.querySelector('#entry').value\">Commit</button><output id='result'></output>",
        );
        await page.click("#entry");
        await page.keyboard.type("PLAYWRIGHT OFFLINE INPUT");
        await page.click("#commit");
        const readback = await page.textContent("#result");
        if (readback !== "PLAYWRIGHT OFFLINE INPUT")
          throw new Error(`Playwright readback mismatch: ${readback ?? "null"}`);
        return {
          version: "1.57.0",
          browser: "explicit-system-chromium",
          page_click: true,
          keyboard_type: true,
          button_click: true,
          readback,
        };
      } finally {
        await browser.close();
      }
    });
  }

  try {
    rmSync(tempRoot, { recursive: true, force: true });
    if (existsSync(tempRoot))
      throw new Error("temporary root still exists after cleanup");
    report.checks.push({
      id: "cleanup",
      status: "passed",
      evidence: {
        temp_files_removed: true,
        browser_closed: true,
        ocr_workers_terminated: true,
      },
    });
  } catch (error) {
    report.checks.push({
      id: "cleanup",
      status: "failed",
      evidence: {},
      error: errorText(error),
    });
  }
  report.status = report.checks.every((check) => check.status === "passed")
    ? "passed"
    : "failed";
  assertReport(report, report.checks[0]?.id === "runtime");
  return report;
}

function assertReport(report: AcceptanceReport, requireFullChecks = false): void {
  if (report.schema !== SCHEMA || report.schema_version !== 1)
    throw new Error("acceptance report schema identity is invalid");
  if (report.status !== "passed" && report.status !== "failed")
    throw new Error("acceptance report status is invalid");
  if (!Array.isArray(report.checks) || report.checks.length === 0)
    throw new Error("acceptance report must contain checks");
  const ids = new Set<string>();
  for (const check of report.checks) {
    if (typeof check.id !== "string" || ids.has(check.id))
      throw new Error("acceptance check ids must be unique strings");
    ids.add(check.id);
    if (check.status !== "passed" && check.status !== "failed")
      throw new Error(`invalid status for acceptance check ${check.id}`);
    if (check.status === "failed" && typeof check.error !== "string")
      throw new Error(`failed acceptance check ${check.id} requires an error`);
  }
  if (requireFullChecks) {
    const actual = report.checks.map((check) => check.id).join(",");
    const expected = CHECK_IDS.join(",");
    if (actual !== expected)
      throw new Error(`acceptance check order is invalid: ${actual}`);
  }
}

function cleanEnvironment(tempRoot: string, runtimeRoot: string): NodeJS.ProcessEnv {
  const environment = { ...process.env };
  for (const name of Object.keys(environment))
    if (name.toUpperCase().includes("PROXY") || name.toUpperCase().includes("CACHE"))
      delete environment[name];
  const home = join(tempRoot, "home");
  const cache = join(tempRoot, "cache");
  mkdirSync(home, { recursive: true });
  mkdirSync(cache, { recursive: true });
  environment.HOME = home;
  environment.XDG_CACHE_HOME = cache;
  environment.XDG_CONFIG_HOME = join(tempRoot, "config");
  environment.XDG_DATA_HOME = join(tempRoot, "data");
  environment.TMPDIR = tempRoot;
  environment.TMP = tempRoot;
  environment.TEMP = tempRoot;
  environment.npm_config_cache = join(tempRoot, "npm-cache");
  environment.PLAYWRIGHT_BROWSERS_PATH = "0";
  environment.TESSDATA_PREFIX = join(runtimeRoot, "share/tessdata");
  environment.NO_PROXY = "*";
  return environment;
}

function launchBundledChild(options: CliOptions): AcceptanceReport {
  const nodePath = join(options.runtimeRoot, "bin/node");
  assertExecutable(nodePath, "bundled Node");
  assertExecutable(options.chromiumExecutable, "Chromium executable");
  const tempRoot = mkdtempSync(join(tmpdir(), "cua-node-full-acceptance-"));
  const scriptPath = resolve(__filename);
  const childArguments = [
    scriptPath,
    `--runtime-root=${options.runtimeRoot}`,
    `--chromium-executable=${options.chromiumExecutable}`,
    "--target=linux-x64",
    "--network=disabled",
    "--empty-user-cache",
    "--internal-run",
    `--temp-root=${tempRoot}`,
    "--json",
  ];
  try {
    const result = spawnSync(nodePath, childArguments, {
      encoding: "utf8",
      env: cleanEnvironment(tempRoot, options.runtimeRoot),
      stdio: ["ignore", "pipe", "pipe"],
      timeout: 120_000,
      maxBuffer: 4 * 1024 * 1024,
    });
    const stdout = (result.stdout ?? "").trim();
    if (stdout.length === 0)
      throw new Error(
        (result.stderr ?? "").trim() ||
          `acceptance child exited ${String(result.status)}`,
      );
    let report: AcceptanceReport;
    try {
      report = JSON.parse(stdout) as AcceptanceReport;
    } catch {
      throw new Error(`acceptance child emitted invalid JSON: ${stdout.slice(0, 200)}`);
    }
    assertReport(report, true);
    if (result.status !== (report.status === "passed" ? 0 : 1))
      throw new Error(
        `acceptance child exit/report mismatch: exit ${String(result.status)}, status ${report.status}`,
      );
    return report;
  } finally {
    rmSync(tempRoot, { recursive: true, force: true });
  }
}

async function main(argv: string[]): Promise<number> {
  let options: CliOptions;
  try {
    options = parseAcceptanceArgs(argv);
  } catch (error) {
    process.stdout.write(`${JSON.stringify(failureReport("cli", error), null, 2)}\n`);
    return 1;
  }
  try {
    const report = options.internalRun
      ? await executeChecks(options)
      : launchBundledChild(options);
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return report.status === "passed" ? 0 : 1;
  } catch (error) {
    process.stdout.write(
      `${JSON.stringify(failureReport("launcher", error), null, 2)}\n`,
    );
    return 1;
  }
}

exports.assertReport = assertReport;
exports.executeChecks = executeChecks;
exports.launchBundledChild = launchBundledChild;
exports.parseAcceptanceArgs = parseAcceptanceArgs;

if (require.main === module)
  void main(process.argv.slice(2)).then((exitCode) => {
    process.exitCode = exitCode;
  });
