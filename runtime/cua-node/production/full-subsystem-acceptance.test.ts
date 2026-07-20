import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import { accessSync, constants, existsSync } from "node:fs";
import { resolve } from "node:path";
import { test } from "bun:test";

type Report = {
  schema: string;
  schema_version: number;
  status: "passed" | "failed";
  checks: Array<{
    id: string;
    status: "passed" | "failed";
    evidence: Record<string, string | number | boolean>;
    error?: string;
  }>;
};

const script = resolve(__dirname, "full-subsystem-acceptance.ts");
const repoRoot = resolve(__dirname, "..");
const runtimeRoot = resolve(repoRoot, "out/linux-x64/cua_node");
const chromiumCandidates = [
  process.env.CUA_NODE_CHROMIUM_EXECUTABLE,
  "/opt/brave-origin-bin/brave",
  "/usr/bin/brave-origin",
  "/usr/bin/brave-browser",
  "/usr/bin/brave",
  "/usr/bin/chromium",
  "/usr/bin/google-chrome-stable",
  "/opt/google/chrome/chrome",
].filter((value): value is string => value !== undefined);

function executableExists(path: string): boolean {
  try {
    accessSync(path, constants.R_OK | constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

const chromiumExecutable = chromiumCandidates.find(executableExists);
const assembledRuntimeExists =
  existsSync(resolve(runtimeRoot, "bin/node")) &&
  existsSync(resolve(runtimeRoot, "lib/node_modules/playwright/package.json"));
const acceptanceModule = require(script) as {
  parseAcceptanceArgs(arguments_: string[]): {
    chromiumExecutable: string;
  };
};

function run(
  arguments_: string[],
  timeout = 10_000,
): { status: number | null; report: Report; stderr: string } {
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

test("CLI fails closed when required offline inputs are missing", () => {
  const result = run(["--json"]);
  assert.equal(result.status, 1);
  assert.equal(result.report.schema, "com.heliasar.cua-node.full-subsystem-acceptance");
  assert.equal(result.report.schema_version, 1);
  assert.equal(result.report.status, "failed");
  assert.deepEqual(
    result.report.checks.map((check) => check.id),
    ["cli"],
  );
  assert.match(result.report.checks[0]?.error ?? "", /runtime-root/u);
});

test.skipIf(!assembledRuntimeExists || chromiumExecutable === undefined)(
  "CLI discovers Brave Origin or another supported browser when omitted",
  () => {
    const options = acceptanceModule.parseAcceptanceArgs([
      `--runtime-root=${runtimeRoot}`,
      "--target=linux-x64",
      "--network=disabled",
      "--empty-user-cache",
      "--json",
    ]);
    assert.ok(executableExists(options.chromiumExecutable));
    assert.equal(options.chromiumExecutable, resolve(chromiumExecutable));
  },
);

test("CLI rejects enabled network and unknown arguments before launching Node", () => {
  const result = run([
    `--runtime-root=${runtimeRoot}`,
    "--chromium-executable=/usr/bin/chromium",
    "--target=linux-x64",
    "--network=enabled",
    "--empty-user-cache",
    "--surprise",
  ]);
  assert.equal(result.status, 1);
  assert.equal(result.report.status, "failed");
  assert.equal(result.report.checks[0]?.id, "cli");
  assert.match(result.report.checks[0]?.error ?? "", /unknown argument/u);
});

test.skipIf(!assembledRuntimeExists || chromiumExecutable === undefined)(
  "assembled runtime passes every real offline subsystem operation under bundled Node",
  () => {
    assert.ok(chromiumExecutable);
    const result = run(
      [
        `--runtime-root=${runtimeRoot}`,
        `--chromium-executable=${chromiumExecutable}`,
        "--target=linux-x64",
        "--network=disabled",
        "--empty-user-cache",
        "--json",
      ],
      120_000,
    );
    assert.equal(result.status, 0, JSON.stringify(result.report, null, 2));
    assert.equal(
      result.report.status,
      "passed",
      JSON.stringify(result.report, null, 2),
    );
    assert.deepEqual(
      result.report.checks.map((check) => check.id),
      [
        "runtime",
        "canvas-png",
        "canvas-webp",
        "sharp",
        "pdfjs",
        "tesseract",
        "pixelmatch",
        "playwright",
        "cleanup",
      ],
    );
    assert.ok(result.report.checks.every((check) => check.status === "passed"));
    const runtime = result.report.checks.find((check) => check.id === "runtime");
    const ocr = result.report.checks.find((check) => check.id === "tesseract");
    const browser = result.report.checks.find((check) => check.id === "playwright");
    assert.equal(runtime?.evidence.node_version, "v24.14.0");
    assert.equal(ocr?.evidence.recognized, "OFFLINE CUA NODE");
    assert.equal(browser?.evidence.readback, "PLAYWRIGHT OFFLINE INPUT");
  },
  150_000,
);
