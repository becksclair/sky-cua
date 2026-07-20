import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import {
  accessSync,
  chmodSync,
  constants,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "bun:test";

type Report = {
  schema: string;
  schema_version: number;
  status: "passed" | "failed";
  checks: Record<string, string | number | boolean>;
  error?: string;
};

const script = resolve(__dirname, "repl-subsystem-acceptance.ts");
const repoRoot = resolve(__dirname, "..");
const runtimeRoot = resolve(repoRoot, "out/linux-x64/cua_node");
function executable(path: string): boolean {
  try {
    accessSync(path, constants.R_OK | constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

const assembledRuntimeExists =
  executable(resolve(runtimeRoot, "bin/node_repl")) &&
  executable(resolve(runtimeRoot, "bin/node")) &&
  existsSync(resolve(runtimeRoot, "lib/node_modules/playwright/package.json")) &&
  existsSync(resolve(runtimeRoot, "share/tessdata/eng.traineddata")) &&
  existsSync(resolve(runtimeRoot, "share/tessdata/osd.traineddata"));

const acceptanceModule = require(script) as {
  parseArgs(arguments_: string[]): {
    runtimeRoot: string;
    timeoutMs: number;
  };
  buildCells(
    options: { runtimeRoot: string; timeoutMs: number },
    tempRoot: string,
  ): ReadonlyArray<{
    label: string;
    tool: "js" | "js_reset";
    code?: string;
    emittedImages?: number;
  }>;
  teardownNodeRepl(
    child: FakeChild,
    tempRoot: string,
    waitMs?: number,
  ): Promise<{ code: number | null; signal: NodeJS.Signals | null }>;
  verifyArtifacts(
    runtimeRootValue: string,
    tempRoot: string,
    cells: Record<string, Record<string, string | number | boolean>>,
  ): Record<string, string | number | boolean>;
};
const fixtureModule = require("./repl-subsystem-media-fixtures") as {
  deterministicPdf(): Buffer;
  prepareMediaFixtures(tempRoot: string): {
    root: string;
    pdf: string;
    malformedImage: string;
    malformedPdf: string;
  };
};

class FakeChild extends EventEmitter {
  readonly signals: NodeJS.Signals[] = [];
  readonly stdin = {
    end: (): void => {
      this.stdinClosed = true;
    },
  };
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;
  stdinClosed = false;

  constructor(private readonly exitOn: NodeJS.Signals) {
    super();
  }

  kill(signal: NodeJS.Signals): boolean {
    this.signals.push(signal);
    if (signal === this.exitOn)
      queueMicrotask(() => {
        this.signalCode = signal;
        this.emit("exit", null, signal);
      });
    return true;
  }
}

function run(
  arguments_: string[],
  timeout = 10_000,
): {
  status: number | null;
  report: Report;
  stderr: string;
} {
  const result = spawnSync(process.execPath, [script, ...arguments_], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout,
    maxBuffer: 4 * 1024 * 1024,
  });
  const stdout = (result.stdout ?? "").trim();
  assert.notEqual(stdout, "", result.stderr);
  return {
    status: result.status,
    report: JSON.parse(stdout) as Report,
    stderr: result.stderr ?? "",
  };
}

test("CLI rejects unknown arguments before spawning node_repl", () => {
  const result = run(["--surprise"]);
  assert.equal(result.status, 1);
  assert.equal(result.report.schema, "com.heliasar.cua-node.repl-subsystem-acceptance");
  assert.equal(result.report.schema_version, 1);
  assert.equal(result.report.status, "failed");
  assert.match(result.report.error ?? "", /unknown argument/u);
});

test("legacy Chromium option does not preflight an unused browser", () => {
  if (!assembledRuntimeExists) return;
  const parsed = acceptanceModule.parseArgs([
    `--runtime-root=${runtimeRoot}`,
    "--chromium-executable=/definitely/missing/chromium",
  ]);
  assert.deepEqual(parsed, { runtimeRoot, timeoutMs: 120_000 });
});

test("artifact verification rejects signature-only output files", () => {
  const root = mkdtempSync(join(tmpdir(), "cua-repl-truncated-artifact-"));
  try {
    const fixtureRoot = join(root, "media fixtures - 日本語");
    mkdirSync(fixtureRoot, { recursive: true });
    writeFileSync(
      join(fixtureRoot, "canvas source π.png"),
      Buffer.from("89504e470d0a1a0a", "hex"),
    );
    assert.throws(
      () => acceptanceModule.verifyArtifacts(runtimeRoot, root, {}),
      /canvas source π\.png is truncated/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("deterministic fixtures use Unicode paths and a two-page vector/image PDF", () => {
  const root = mkdtempSync(join(tmpdir(), "cua-repl-media-fixtures-"));
  try {
    const fixtures = fixtureModule.prepareMediaFixtures(root);
    assert.match(fixtures.root, /media fixtures - 日本語$/u);
    assert.equal(readFileSync(fixtures.pdf).subarray(0, 5).toString("ascii"), "%PDF-");
    const pdfText = fixtureModule.deterministicPdf().toString("latin1");
    assert.match(pdfText, /\/Count 2/u);
    assert.match(pdfText, /\/Subtype \/Image/u);
    assert.equal(readFileSync(fixtures.malformedImage, "utf8"), "not an image");
    assert.equal(readFileSync(fixtures.malformedPdf, "ascii"), "%PDF-not-valid");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("CLI preflight fails when required OSD tessdata is unavailable", () => {
  const root = mkdtempSync(join(tmpdir(), "cua-repl-missing-osd-"));
  try {
    for (const directory of ["bin", "lib/node_modules/playwright", "share/tessdata"])
      mkdirSync(join(root, directory), { recursive: true });
    for (const executableName of ["node", "node_repl"]) {
      const path = join(root, "bin", executableName);
      writeFileSync(path, "#!/bin/sh\nexit 0\n", "utf8");
      chmodSync(path, 0o755);
    }
    writeFileSync(join(root, "lib/node_modules/playwright/package.json"), "{}");
    writeFileSync(join(root, "share/tessdata/eng.traineddata"), "eng");
    assert.throws(
      () => acceptanceModule.parseArgs([`--runtime-root=${root}`]),
      /bundled osd tessdata is missing/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("zero-preamble cells cover the complete WP-05 media matrix", () => {
  const cells = acceptanceModule.buildCells(
    { runtimeRoot, timeoutMs: 120_000 },
    "/tmp/cua acceptance root",
  );
  assert.deepEqual(
    cells.map((cell) => cell.label),
    [
      "pdf-direct-import",
      "canvas",
      "sharp",
      "loopback-server",
      "pdf-inputs",
      "tesseract-inputs",
      "pixelmatch",
      "malformed-and-abort",
      "persistent-cells",
      "loopback-cleanup",
      "reset",
      "after-reset",
    ],
  );
  const code = cells.map((cell) => cell.code ?? "").join("\n");
  assert.doesNotMatch(code, /Object\.assign\s*\(\s*globalThis/u);
  assert.ok(
    (cells[0].code ?? "").indexOf('import("pdfjs-dist/legacy/build/pdf.mjs")') <
      (cells[0].code ?? "").indexOf("nodeRepl.runtime"),
    "PDF.js must import before any runtime configuration and without a global prelude",
  );
  for (const marker of [
    "new DOMPoint",
    "new DOMRect",
    "new Path2D",
    "canvas source π.png",
    "canvas source π.webp",
    "sharpMedia(new Uint8Array",
    "fileURLToPath",
    "url: pdfUrl",
    'loopbackBase + "/ocr.png"',
    "ocrDataUrl",
    'createWorker("osd"',
    "osdWorker.detect",
    "osd_tessdata_exercised",
    "identicalPixels",
    "differingPixels",
    "nodeRepl.emitImage",
    "abortController.abort",
    "aborted_generic_pdf_bytes_fetch",
    "partial_output_removed",
    "bindings_gone",
  ])
    assert.ok(code.includes(marker), marker);
  assert.equal(cells.filter((cell) => cell.tool === "js_reset").length, 1);
  assert.equal(
    cells.reduce((count, cell) => count + (cell.emittedImages ?? 0), 0),
    6,
  );
});

test("acknowledged but hung shutdown escalates through SIGKILL before deletion", async () => {
  const root = mkdtempSync(join(tmpdir(), "cua-repl-hung-shutdown-"));
  const child = new FakeChild("SIGKILL");
  const exit = await acceptanceModule.teardownNodeRepl(child, root, 5);
  assert.equal(child.stdinClosed, true);
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
  assert.deepEqual(exit, { code: null, signal: "SIGKILL" });
  assert.equal(existsSync(root), false);
});

test("failure cleanup awaits SIGTERM exit before deleting its temp root", async () => {
  const root = mkdtempSync(join(tmpdir(), "cua-repl-failure-cleanup-"));
  const child = new FakeChild("SIGTERM");
  child.once("exit", () => assert.equal(existsSync(root), true));
  const exit = await acceptanceModule.teardownNodeRepl(child, root, 5);
  assert.equal(child.stdinClosed, true);
  assert.deepEqual(child.signals, ["SIGTERM"]);
  assert.deepEqual(exit, { code: null, signal: "SIGTERM" });
  assert.equal(existsSync(root), false);
});

test("unconfirmed SIGKILL exit fails bounded and retains the temp root", async () => {
  const root = mkdtempSync(join(tmpdir(), "cua-repl-unconfirmed-kill-"));
  const child = new FakeChild("SIGUSR1");
  try {
    await assert.rejects(
      acceptanceModule.teardownNodeRepl(child, root, 5),
      /did not exit after SIGKILL; temporary root retained/u,
    );
    assert.equal(child.stdinClosed, true);
    assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
    assert.equal(existsSync(root), true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test.skipIf(!assembledRuntimeExists)(
  "assembled node_repl passes WP-05 or returns an exact matrix-row blocker",
  () => {
    const result = run(
      [
        `--runtime-root=${runtimeRoot}`,
        "--timeout-ms=120000",
        "--json",
      ],
      180_000,
    );
    if (result.report.status === "failed") {
      assert.equal(result.status, 1);
      assert.match(
        result.report.error ?? "",
        /^matrix row [a-z-]+ blocked: .+/u,
        JSON.stringify(result.report, null, 2),
      );
      return;
    }
    assert.equal(result.status, 0, JSON.stringify(result.report, null, 2));
    assert.equal(result.report.checks.cell_count, 12);
    assert.equal(result.report.checks.recognized, "ZERO PREAMBLE MEDIA");
    assert.equal(result.report.checks.pdf_text, "ZERO PREAMBLE PDF");
    assert.equal(result.report.checks.pdf_url_contract, "pdfjs-node-direct-url");
    assert.equal(result.report.checks.osd_tessdata_exercised, true);
    assert.equal(result.report.checks.cleanup, true);
    assert.equal(result.report.checks.reset, true);
    for (const key of [
      "canvas_png_bytes",
      "canvas_webp_bytes",
      "sharp_webp_bytes",
      "sharp_png_bytes",
      "pdf_bytes",
      "pdf_png_bytes",
      "pdf_webp_bytes",
      "diff_png_bytes",
    ])
      assert.ok(Number(result.report.checks[key]) > 0, key);
    for (const key of [
      "canvas_png_sha256",
      "sharp_png_sha256",
      "pdf_png_sha256",
      "diff_png_sha256",
    ])
      assert.match(String(result.report.checks[key]), /^[a-f0-9]{64}$/u, key);
  },
);
