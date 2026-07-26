import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

type JsonRecord = Record<string, unknown>;

export interface SourceTruthSnapshot {
  runtimeLock: JsonRecord;
  nativeLock: JsonRecord;
  productionPackage: JsonRecord;
  productionPackageLock: JsonRecord;
  compliancePolicy: JsonRecord;
  complianceProvenance: JsonRecord;
  noticeInventory: JsonRecord;
  distSha256: string;
  distSizeBytes: number;
  sourceBuildSha256: string;
  sourceBuildSizeBytes: number;
  noticeSha256s: Record<string, string>;
}

const LOCKED_DEPENDENCIES: Record<string, string> = {
  "@heliasar/sky-cua":
    "file:../../../vendor/cua-node-cache/@heliasar-sky-cua-0.1.0.tgz",
  "@img/sharp-libvips-linux-x64": "1.2.4",
  "@img/sharp-linux-x64": "0.34.5",
  "@napi-rs/canvas": "0.1.91",
  "@napi-rs/canvas-linux-x64-gnu": "0.1.91",
  acorn: "8.16.0",
  "acorn-walk": "8.3.5",
  "pdfjs-dist": "5.4.624",
  pixelmatch: "7.1.0",
  playwright: "1.57.0",
  "playwright-core": "1.57.0",
  sharp: "0.34.5",
  "tesseract.js": "7.0.0",
  "tesseract.js-core": "7.0.0",
};

const NOTICE_HASHES: Record<string, string> = {
  "notices/pdfjs-cmaps.LICENSE":
    "aa92ab5a472974865a96fd4a4e9c13bb41bf6fe1b309cb6b8da48bc9e19839a2",
  "notices/pdfjs-standard-fonts.LICENSE_FOXIT":
    "b578cdd2345840ada550bd12519533812320d5f1d21cf4c1c7e1b1b0a31c98b7",
  "notices/pdfjs-standard-fonts.LICENSE_LIBERATION":
    "93fed46019c38bbe566b479d22148e2e8a1e85ada614accb0211c37b2c61c19b",
  "notices/pixelmatch-7.1.0.LICENSE":
    "cfec0482fb785fe27e3b368a2d9e84bf7a61b275e83ec582bf288e98cd530bb0",
};

function record(value: unknown, label: string): JsonRecord {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as JsonRecord;
}

function records(value: unknown, label: string): JsonRecord[] {
  if (!Array.isArray(value)) throw new Error(`${label} must be an array`);
  return value.map((entry, index) => record(entry, `${label}[${index}]`));
}

function sha256(bytes: Uint8Array): string {
  return createHash("sha256").update(bytes).digest("hex");
}

async function readJson(path: string): Promise<JsonRecord> {
  return record(JSON.parse(await readFile(path, "utf8")) as unknown, path);
}

function byField(entries: JsonRecord[], field: string, value: string): JsonRecord {
  const match = entries.find((entry) => entry[field] === value);
  if (match === undefined) throw new Error(`missing ${field}=${value}`);
  return match;
}

function stringArray(value: unknown): string[] {
  return Array.isArray(value) && value.every((entry) => typeof entry === "string")
    ? value
    : [];
}

function expectEqual(
  errors: string[],
  label: string,
  actual: unknown,
  expected: unknown,
): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    errors.push(
      `${label} drifted: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

export function validateSourceTruth(snapshot: SourceTruthSnapshot): string[] {
  const errors: string[] = [];
  const runtime = record(snapshot.runtimeLock.runtime, "runtime-lock.runtime");
  const runtimeSource = record(runtime.source, "runtime-lock.runtime.source");
  const runtimePackages = records(
    snapshot.runtimeLock.packages,
    "runtime-lock.packages",
  );
  const outputs = records(snapshot.runtimeLock.outputs, "runtime-lock.outputs");
  const nativeAssets = records(snapshot.nativeLock.assets, "native-assets.assets");
  const dependencies = record(
    snapshot.productionPackage.dependencies,
    "production/package.json dependencies",
  );
  const lockPackages = record(
    snapshot.productionPackageLock.packages,
    "production/package-lock.json packages",
  );
  const lockedRootDependencies = record(
    record(lockPackages[""], "package-lock root").dependencies,
    "package-lock root dependencies",
  );

  for (const [name, version] of Object.entries(LOCKED_DEPENDENCIES)) {
    expectEqual(errors, `production dependency ${name}`, dependencies[name], version);
    expectEqual(
      errors,
      `package-lock dependency ${name}`,
      lockedRootDependencies[name],
      version,
    );
  }

  expectEqual(errors, "runtime-lock release_ready", snapshot.runtimeLock.release_ready, true);
  expectEqual(errors, "runtime-lock release_blockers", snapshot.runtimeLock.release_blockers, []);
  expectEqual(errors, "native-assets release_ready", snapshot.nativeLock.release_ready, true);
  expectEqual(errors, "native-assets release_blockers", snapshot.nativeLock.release_blockers, []);
  expectEqual(errors, "runtime target", snapshot.runtimeLock.target, "linux-x64-glibc");
  expectEqual(errors, "native target", snapshot.nativeLock.target, "linux-x64-glibc");

  expectEqual(errors, "canonical dist SHA-256", runtime.sha256, snapshot.distSha256);
  expectEqual(errors, "canonical dist size", runtime.size_bytes, snapshot.distSizeBytes);
  expectEqual(
    errors,
    "source-built dist SHA-256",
    snapshot.sourceBuildSha256,
    snapshot.distSha256,
  );
  expectEqual(
    errors,
    "source-built dist size",
    snapshot.sourceBuildSizeBytes,
    snapshot.distSizeBytes,
  );
  expectEqual(errors, "runtime source URI", runtimeSource.uri, "runtime/cua-node/dist/cli.js");
  const provenance = typeof runtimeSource.provenance === "string" ? runtimeSource.provenance : "";
  if (!provenance.includes("sky-cua") || provenance.includes("codex-desktop")) {
    errors.push(`runtime provenance is not canonical sky-cua ownership: ${provenance}`);
  }
  const hostOutput = byField(outputs, "name", "node_repl-host-kernel");
  expectEqual(errors, "host output SHA-256", hostOutput.sha256, snapshot.distSha256);
  expectEqual(errors, "host output size", hostOutput.size_bytes, snapshot.distSizeBytes);

  for (const expected of [
    {
      name: "acorn",
      version: "8.16.0",
      integrity:
        "sha512-UVJyE9MttOsBQIDKw1skb9nAwQuR5wuGD3+82K6JgJlm/Y+KI92oNsMNGZCYdDsVtRHSak0pcV5Dno5+4jh9sw==",
      sha256: "e0d357db62f8b138d2e575a2cb92f087fa5d7cdb2f95d7512b9868ff3ef81277",
      sizeBytes: 558_610,
    },
    {
      name: "acorn-walk",
      version: "8.3.5",
      integrity:
        "sha512-HEHNfbars9v4pgpW6SO1KSPkfoS0xVOM/9UzkJltjlsHZmJasxg8aXkuZa7SMf8vKGIBhpUsPluQSqhJFCqebw==",
      sha256: "cd968876dd2797441c7c6de59eaa867e2d804e490e4e6c6e5d9be3d247ea6b0f",
      sizeBytes: 53_765,
    },
  ]) {
    const runtimePackage = byField(runtimePackages, "name", expected.name);
    const license = record(runtimePackage.license, `${expected.name} license`);
    const packageLock = record(
      lockPackages[`node_modules/${expected.name}`],
      `package-lock ${expected.name}`,
    );
    expectEqual(errors, `${expected.name} version`, runtimePackage.version, expected.version);
    expectEqual(errors, `${expected.name} integrity`, runtimePackage.integrity, expected.integrity);
    expectEqual(errors, `${expected.name} tree SHA-256`, runtimePackage.sha256, expected.sha256);
    expectEqual(errors, `${expected.name} tree size`, runtimePackage.size_bytes, expected.sizeBytes);
    expectEqual(errors, `${expected.name} license`, license.expression, "MIT");
    expectEqual(
      errors,
      `${expected.name} notice path`,
      stringArray(license.notice_files),
      [`lib/node_modules/${expected.name}/LICENSE`],
    );
    expectEqual(errors, `package-lock ${expected.name} version`, packageLock.version, expected.version);
    expectEqual(errors, `package-lock ${expected.name} license`, packageLock.license, "MIT");
    expectEqual(
      errors,
      `package-lock ${expected.name} integrity`,
      packageLock.integrity,
      expected.integrity,
    );
  }

  const pixelmatch = byField(runtimePackages, "name", "pixelmatch");
  const pixelmatchLicense = record(pixelmatch.license, "pixelmatch license");
  expectEqual(errors, "pixelmatch version", pixelmatch.version, "7.1.0");
  expectEqual(
    errors,
    "pixelmatch integrity",
    pixelmatch.integrity,
    "sha512-1wrVzJ2STrpmONHKBy228LM1b84msXDUoAzVEl0R8Mz4Ce6EPr+IVtxm8+yvrqLYMHswREkjYFaMxnyGnaY3Ng==",
  );
  expectEqual(errors, "pixelmatch tree SHA-256", pixelmatch.sha256, "6407fbe0821c7b3635924870de1173714beb723161dc033e5421eacdad8c6137");
  expectEqual(errors, "pixelmatch tree size", pixelmatch.size_bytes, 19_428);
  expectEqual(errors, "pixelmatch license", pixelmatchLicense.expression, "ISC");
  expectEqual(
    errors,
    "pixelmatch notice path",
    stringArray(pixelmatchLicense.notice_files),
    ["lib/node_modules/pixelmatch/LICENSE"],
  );
  const pixelmatchPackageLock = record(
    lockPackages["node_modules/pixelmatch"],
    "package-lock pixelmatch",
  );
  expectEqual(errors, "package-lock pixelmatch version", pixelmatchPackageLock.version, "7.1.0");
  expectEqual(errors, "package-lock pixelmatch license", pixelmatchPackageLock.license, "ISC");
  expectEqual(errors, "package-lock pixelmatch integrity", pixelmatchPackageLock.integrity, pixelmatch.integrity);

  const canvas = byField(runtimePackages, "name", "@napi-rs/canvas");
  const canvasLicense = record(canvas.license, "Canvas package license");
  expectEqual(errors, "Canvas declared license", canvasLicense.expression, "MIT");
  expectEqual(
    errors,
    "Canvas package notice paths",
    stringArray(canvasLicense.notice_files),
    [
      "lib/node_modules/@napi-rs/canvas/LICENSE",
      "licenses/notices/canvas-0.1.91.NOTICE.md",
    ],
  );
  const canvasNative = byField(nativeAssets, "id", "canvas-linux-x64-gnu");
  const canvasNativeLicense = record(canvasNative.license, "Canvas native license");
  expectEqual(errors, "Canvas native license", canvasNativeLicense.expression, "MIT");
  expectEqual(
    errors,
    "Canvas native notice paths",
    stringArray(canvasNativeLicense.notice_files),
    [
      "lib/node_modules/@napi-rs/canvas/LICENSE",
      "licenses/notices/canvas-0.1.91-NATIVE-NOTICES.md",
      "licenses/notices/canvas-0.1.91-RUST-NOTICES.md",
      "licenses/notices/canvas-0.1.91.NOTICE.md",
    ],
  );

  const cMaps = byField(nativeAssets, "id", "pdfjs-cmaps");
  const cMapsLicense = record(cMaps.license, "PDF.js CMaps license");
  expectEqual(errors, "PDF.js CMaps license", cMapsLicense.expression, "BSD-3-Clause");
  expectEqual(
    errors,
    "PDF.js CMaps notice path",
    stringArray(cMapsLicense.notice_files),
    ["share/pdfjs/cmaps/LICENSE"],
  );
  const fonts = byField(nativeAssets, "id", "pdfjs-standard-fonts");
  const fontsLicense = record(fonts.license, "PDF.js standard fonts license");
  expectEqual(
    errors,
    "PDF.js standard fonts license",
    fontsLicense.expression,
    "BSD-3-Clause AND OFL-1.1",
  );
  expectEqual(
    errors,
    "PDF.js standard fonts notice paths",
    stringArray(fontsLicense.notice_files),
    [
      "share/pdfjs/standard_fonts/LICENSE_FOXIT",
      "share/pdfjs/standard_fonts/LICENSE_LIBERATION",
    ],
  );

  const policyComponents = records(
    snapshot.compliancePolicy.components,
    "compliance policy components",
  );
  const provenanceRecords = records(
    snapshot.complianceProvenance.records,
    "compliance provenance records",
  );
  const noticeEntries = records(
    snapshot.noticeInventory.entries,
    "compliance notice entries",
  );
  const policyExpectations = [
    ["pdfjs-cmaps", "BSD-3-Clause", cMaps.sha256],
    ["pdfjs-standard-fonts", "BSD-3-Clause AND OFL-1.1", fonts.sha256],
    ["canvas", "MIT", canvas.sha256],
    [
      "canvas-linux-x64-gnu",
      "MIT",
      "edcbca8d43db993a9066974c97ca0e87fb179aaee61d5e56561a19ae171b643d",
    ],
    ["pixelmatch", "ISC", pixelmatch.sha256],
  ] as const;
  for (const [id, licenseExpression, digest] of policyExpectations) {
    const component = byField(policyComponents, "id", id);
    const componentLicense = record(component.license, `${id} policy license`);
    const componentSource = record(component.source, `${id} policy source`);
    expectEqual(errors, `${id} policy license`, componentLicense.spdx_expression, licenseExpression);
    expectEqual(
      errors,
      `${id} policy provenance status`,
      componentSource.provenance_status,
      "artifact-provenance-resolved",
    );
    const componentProvenance = byField(provenanceRecords, "component_id", id);
    expectEqual(errors, `${id} provenance resolved`, componentProvenance.resolved, true);
    expectEqual(errors, `${id} provenance status`, componentProvenance.status, "cleared");
    expectEqual(errors, `${id} provenance SHA-256`, componentProvenance.artifact_sha256, digest);
  }

  for (const [path, expectedHash] of Object.entries(NOTICE_HASHES)) {
    expectEqual(errors, `${path} SHA-256`, snapshot.noticeSha256s[path], expectedHash);
  }
  const noticeExpectations: Record<string, string> = {
    "pdfjs-cmaps-notice": "notices/pdfjs-cmaps.LICENSE",
    "pdfjs-fonts-foxit": "notices/pdfjs-standard-fonts.LICENSE_FOXIT",
    "pdfjs-fonts-liberation": "notices/pdfjs-standard-fonts.LICENSE_LIBERATION",
    "pixelmatch-license": "notices/pixelmatch-7.1.0.LICENSE",
  };
  for (const [id, path] of Object.entries(noticeExpectations)) {
    const entry = byField(noticeEntries, "id", id);
    expectEqual(errors, `${id} collection status`, entry.status, "collected");
    expectEqual(errors, `${id} path`, entry.planned_path, path);
  }

  return errors;
}

export async function loadSourceTruthSnapshot(
  runtimeRoot = dirname(import.meta.dir),
): Promise<SourceTruthSnapshot> {
  const distPath = join(runtimeRoot, "dist/cli.js");
  const distBytes = await readFile(distPath);
  const temporaryRoot = await mkdtemp(join(tmpdir(), "cua-node-source-build-"));
  let sourceBuildBytes: Uint8Array;
  try {
    const result = await Bun.build({
      entrypoints: [join(runtimeRoot, "src/cli.ts")],
      outdir: temporaryRoot,
      target: "node",
    });
    if (!result.success) {
      throw new Error(`canonical source build failed: ${result.logs.join("\n")}`);
    }
    sourceBuildBytes = await readFile(join(temporaryRoot, "cli.js"));
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
  const complianceRoot = join(runtimeRoot, "compliance");
  const noticeSha256s: Record<string, string> = {};
  for (const path of Object.keys(NOTICE_HASHES)) {
    noticeSha256s[path] = sha256(await readFile(join(complianceRoot, path)));
  }
  return {
    runtimeLock: await readJson(join(runtimeRoot, "runtime-lock.json")),
    nativeLock: await readJson(join(runtimeRoot, "native-assets.lock.json")),
    productionPackage: await readJson(join(runtimeRoot, "production/package.json")),
    productionPackageLock: await readJson(
      join(runtimeRoot, "production/package-lock.json"),
    ),
    compliancePolicy: await readJson(join(complianceRoot, "policy.json")),
    complianceProvenance: await readJson(join(complianceRoot, "provenance.json")),
    noticeInventory: await readJson(join(complianceRoot, "notice-inventory.json")),
    distSha256: sha256(distBytes),
    distSizeBytes: (await stat(distPath)).size,
    sourceBuildSha256: sha256(sourceBuildBytes),
    sourceBuildSizeBytes: sourceBuildBytes.byteLength,
    noticeSha256s,
  };
}

export async function assertSourceLocksTruthful(
  runtimeRoot?: string,
): Promise<void> {
  const errors = validateSourceTruth(await loadSourceTruthSnapshot(runtimeRoot));
  if (errors.length > 0) throw new Error(errors.join("\n"));
}
