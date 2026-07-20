import { strict as assert } from "node:assert";
import { createHash } from "node:crypto";
import {
  cpSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, test } from "bun:test";
import { findWrongPlatformOptionalDependencies } from "../tools/verifier/package-metadata-audit.ts";
import { verifyCuaNode } from "../tools/verifier/runtime-verification.ts";
import type { JsonRecord } from "../tools/verifier/types.ts";

const FIXTURE_ROOT = join(import.meta.dir, "fixtures/package-metadata");
const FAKE_RUNTIME_ROOT = join(import.meta.dir, "fixtures/fake-runtime");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0))
    rmSync(root, { recursive: true, force: true });
});

function cloneFakeRuntime(): string {
  const root = mkdtempSync(join(tmpdir(), "cua-node-metadata-audit-"));
  temporaryRoots.push(root);
  cpSync(FAKE_RUNTIME_ROOT, root, { recursive: true });
  return root;
}

function updateManifestChecksum(root: string, relativePath: string): void {
  const manifestPath = join(root, "manifest.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as JsonRecord;
  const checksums = manifest.checksums as JsonRecord;
  const files = checksums.files as JsonRecord[];
  const entry = files.find((candidate) => candidate.path === relativePath);
  if (entry === undefined)
    throw new Error(`manifest checksum entry is missing: ${relativePath}`);
  const bytes = readFileSync(join(root, relativePath));
  entry.sha256 = createHash("sha256").update(bytes).digest("hex");
  entry.size_bytes = statSync(join(root, relativePath)).size;
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
}

function inspect(optionalDependencies: Record<string, string>) {
  return findWrongPlatformOptionalDependencies(
    { name: "native-facade", optionalDependencies },
    "native-facade/package.json",
  );
}

describe("native optional dependency metadata", () => {
  test("accepts only the Linux x64 glibc Sharp and Canvas variants", () => {
    assert.deepEqual(
      inspect({
        "@img/sharp-linux-x64": "0.34.5",
        "@img/sharp-libvips-linux-x64": "1.2.4",
        "@napi-rs/canvas-linux-x64-gnu": "0.1.91",
      }),
      [],
    );
  });

  test("rejects wrong-platform keys across native package naming schemes", () => {
    assert.deepEqual(
      inspect({
        "@img/sharp-darwin-arm64": "0.34.5",
        "@img/sharp-libvips-linuxmusl-x64": "1.2.4",
        "@napi-rs/canvas-win32-x64-msvc": "0.1.91",
        "@swc/core-linux-arm": "1.0.0",
        "@rollup/rollup-windows-x64-msvc": "1.0.0",
      }),
      [
        { dependency: "@img/sharp-darwin-arm64", platform: "darwin" },
        {
          dependency: "@img/sharp-libvips-linuxmusl-x64",
          platform: "musl",
        },
        {
          dependency: "@napi-rs/canvas-win32-x64-msvc",
          platform: "windows",
        },
        { dependency: "@rollup/rollup-windows-x64-msvc", platform: "windows" },
        { dependency: "@swc/core-linux-arm", platform: "arm" },
      ],
    );
  });

  test("does not infer package target from portable source filenames", () => {
    const fixture = JSON.parse(
      readFileSync(
        join(FIXTURE_ROOT, "portable-win32-source-package.json"),
        "utf8",
      ),
    ) as JsonRecord;
    assert.deepEqual(
      findWrongPlatformOptionalDependencies(fixture, "portable/package.json"),
      [],
    );
  });

  test("runtime preflight accepts a shipped portable win32.js source file", () => {
    const root = cloneFakeRuntime();
    const relativePackagePath =
      "lib/node_modules/@heliasar/sky-cua/package.json";
    const packagePath = join(root, relativePackagePath);
    const packageJson = JSON.parse(
      readFileSync(packagePath, "utf8"),
    ) as JsonRecord;
    const portableFixture = JSON.parse(
      readFileSync(
        join(FIXTURE_ROOT, "portable-win32-source-package.json"),
        "utf8",
      ),
    ) as JsonRecord;
    packageJson.files = portableFixture.files;
    packageJson.optionalDependencies = portableFixture.optionalDependencies;
    writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`);
    const portableSourceDirectory = join(
      root,
      "lib/node_modules/@heliasar/sky-cua/lib",
    );
    mkdirSync(portableSourceDirectory);
    copyFileSync(
      join(FIXTURE_ROOT, "win32.js"),
      join(portableSourceDirectory, "win32.js"),
    );
    updateManifestChecksum(root, relativePackagePath);

    const report = verifyCuaNode({ root, allowFixtureValues: true });
    const platformCheck = report.checks.find(
      (check) =>
        check.id === "dependencies:no-wrong-platform-optional-dependencies",
    );

    assert.equal(report.status, "passed");
    assert.equal(platformCheck?.status, "passed");
  });

  test("does not match marker substrings inside portable dependency names", () => {
    assert.deepEqual(
      inspect({
        armature: "1.0.0",
        darwining: "1.0.0",
        muslin: "1.0.0",
        windowsill: "1.0.0",
      }),
      [],
    );
  });

  test("rejects malformed optional dependency metadata", () => {
    assert.throws(
      () =>
        findWrongPlatformOptionalDependencies(
          { optionalDependencies: ["@img/sharp-linux-x64"] },
          "malformed/package.json",
        ),
      /malformed\/package\.json\.optionalDependencies must be an object/u,
    );
  });

  test("runtime preflight rejects a shipped package with a wrong-platform key", () => {
    const root = cloneFakeRuntime();
    const relativePackagePath =
      "lib/node_modules/@heliasar/sky-cua/package.json";
    const packagePath = join(root, relativePackagePath);
    const packageJson = JSON.parse(
      readFileSync(packagePath, "utf8"),
    ) as JsonRecord;
    packageJson.optionalDependencies = {
      "@napi-rs/canvas-win32-x64-msvc": "0.1.91",
    };
    writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`);
    updateManifestChecksum(root, relativePackagePath);

    const report = verifyCuaNode({ root, allowFixtureValues: true });
    const platformCheck = report.checks.find(
      (check) =>
        check.id === "dependencies:no-wrong-platform-optional-dependencies",
    );

    assert.equal(report.status, "failed");
    assert.equal(platformCheck?.status, "failed");
    assert.match(platformCheck?.detail ?? "", /canvas-win32-x64-msvc/u);
    assert.equal(
      report.checks.some((check) => check.id === "identity:node-version"),
      false,
    );
  });
});
