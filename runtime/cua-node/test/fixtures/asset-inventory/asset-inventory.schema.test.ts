const assert = require("node:assert/strict");

const { readFileSync } = require("node:fs");

const path = require("node:path");

const Ajv2020 = require("ajv/dist/2020").default;

const { test } = require("bun:test");

const fixtureRoot = __dirname;
const runtimeSchema = JSON.parse(
  readFileSync(
    path.resolve(fixtureRoot, "../../../runtime-lock.schema.json"),
    "utf8",
  ),
);
const nativeAssetsSchema = JSON.parse(
  readFileSync(
    path.resolve(fixtureRoot, "../../../native-assets.lock.schema.json"),
    "utf8",
  ),
);

function readFixture(name: string): Record<string, unknown> {
  return JSON.parse(readFileSync(path.join(fixtureRoot, "fixtures", name), "utf8"));
}

function validator(schema: Record<string, unknown>) {
  const ajv = new Ajv2020({ allErrors: true, strict: true });
  return ajv.compile(schema);
}

function fixtureEntry(
  value: unknown,
  index: number,
  label: string,
): Record<string, unknown> {
  if (!Array.isArray(value)) {
    throw new TypeError(`${label} must be an array`);
  }
  const entry = value[index];
  assert.ok(
    entry !== null && typeof entry === "object" && !Array.isArray(entry),
    `${label}[${index}] must be an object`,
  );
  return entry as Record<string, unknown>;
}

function fixtureRecord(value: unknown, label: string): Record<string, unknown> {
  assert.ok(
    value !== null && typeof value === "object" && !Array.isArray(value),
    `${label} must be an object`,
  );
  return value as Record<string, unknown>;
}

test("valid runtime and native asset fixtures satisfy their lock schemas", () => {
  const runtime = readFixture("runtime-lock.valid.json");
  const nativeAssets = readFixture("native-assets-lock.valid.json");

  const runtimeValidate = validator(runtimeSchema);
  const nativeAssetsValidate = validator(nativeAssetsSchema);

  assert.equal(runtimeValidate(runtime), true, JSON.stringify(runtimeValidate.errors));
  assert.equal(
    nativeAssetsValidate(nativeAssets),
    true,
    JSON.stringify(nativeAssetsValidate.errors),
  );
  assert.equal(runtime.local_assembly_ready, true);
  assert.equal(runtime.release_ready, false);
  assert.equal(nativeAssets.local_assembly_ready, true);
  assert.equal(nativeAssets.release_ready, false);
});

test("runtime locks reject unresolved, wrong-version, and unchecksummed inputs", () => {
  const runtime = readFixture("runtime-lock.valid.json");
  const validate = validator(runtimeSchema);

  const missingSha256 = structuredClone(runtime);
  delete (missingSha256.node as Record<string, unknown>).sha256;
  assert.equal(validate(missingSha256), false);

  const wrongNode = structuredClone(runtime);
  (wrongNode.node as Record<string, unknown>).version = "24.14.1";
  assert.equal(validate(wrongNode), false);

  const unresolvedSource = structuredClone(runtime);
  const unresolvedPackage = fixtureEntry(
    unresolvedSource.packages,
    0,
    "runtime packages",
  );
  fixtureRecord(unresolvedPackage.source, "runtime package source").resolved = false;
  assert.equal(validate(unresolvedSource), false);
});

test("native asset locks require platform ABI, audit, licensing, and destination fields", () => {
  const nativeAssets = readFixture("native-assets-lock.valid.json");
  const validate = validator(nativeAssetsSchema);

  const missingLicense = structuredClone(nativeAssets);
  const firstAsset = fixtureEntry(missingLicense.assets, 0, "native assets");
  delete fixtureRecord(firstAsset.license, "native asset license")
    .redistribution_status;
  assert.equal(validate(missingLicense), false);

  const badAbi = structuredClone(nativeAssets);
  const secondAsset = fixtureEntry(badAbi.assets, 1, "native assets");
  fixtureRecord(secondAsset.platform, "native asset platform").arch = "arm64";
  assert.equal(validate(badAbi), false);

  const missingAudit = structuredClone(nativeAssets);
  delete fixtureEntry(missingAudit.assets, 0, "native assets")
    .native_dependency_audit;
  assert.equal(validate(missingAudit), false);
});

test("inventory report is deterministic and records zero new downloads", () => {
  const report = JSON.parse(
    readFileSync(path.join(fixtureRoot, "reports", "asset-inventory.json"), "utf8"),
  ) as {
    constraints: {
      new_download_bytes: number;
      cumulative_new_download_bytes: number;
      estimated_missing_download_bytes_if_approved: number;
    };
    requirements: Array<{ id: string }>;
    missing_artifacts: Array<{
      id: string;
      estimated_download_bytes: number;
    }>;
  };

  assert.equal(report.constraints.new_download_bytes, 0);
  assert.equal(report.constraints.cumulative_new_download_bytes, 0);
  assert.equal(report.constraints.estimated_missing_download_bytes_if_approved, 103912000);

  const requirementIds = new Set(report.requirements.map((entry) => entry.id));
  for (const id of [
    "node",
    "npm",
    "corepack",
    "playwright",
    "playwright-core",
    "playwright-browser-system-chromium",
    "pdfjs-dist",
    "pdfjs-cmaps",
    "pdfjs-standard-fonts",
    "tesseract-js",
    "tesseract-js-core",
    "tessdata-eng",
    "tesseract-system-engine-reference",
    "image-codecs-via-libvips",
    "sharp",
    "sharp-linux-x64",
    "sharp-libvips-linux-x64",
    "canvas",
    "canvas-linux-x64-gnu",
    "visual-diff-pixelmatch",
  ]) {
    assert.equal(requirementIds.has(id), true, `missing inventory requirement ${id}`);
  }

  assert.ok(report.missing_artifacts.length >= 1);
  for (const artifact of report.missing_artifacts) {
    assert.ok(artifact.id.length > 0);
    assert.ok(Number.isInteger(artifact.estimated_download_bytes));
    assert.ok(artifact.estimated_download_bytes >= 0);
  }

  assert.equal(
    report.missing_artifacts.find((artifact) => artifact.id === "npm-11.9.0-linux-bundle")
      ?.estimated_download_bytes,
    0,
  );
  assert.equal(
    report.missing_artifacts.find((artifact) => artifact.id === "tessdata-eng-approved-bundle")
      ?.estimated_download_bytes,
    0,
  );
});
