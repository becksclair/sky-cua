import { strict as assert } from "node:assert";

import { test } from "bun:test";

import {
  loadSourceTruthSnapshot,
  validateSourceTruth,
  type SourceTruthSnapshot,
} from "../tools/source-lock-truth.ts";

function clone(snapshot: SourceTruthSnapshot): SourceTruthSnapshot {
  return structuredClone(snapshot);
}

test("source locks match the canonical first-party build, dependencies, and licenses", async () => {
  const snapshot = await loadSourceTruthSnapshot();
  assert.deepEqual(validateSourceTruth(snapshot), []);
});

test("source lock truth rejects stale built output", async () => {
  const snapshot = clone(await loadSourceTruthSnapshot());
  snapshot.sourceBuildSha256 = "0".repeat(64);
  assert.ok(
    validateSourceTruth(snapshot).some((error) =>
      error.startsWith("source-built dist SHA-256 drifted:"),
    ),
  );
});

test("source lock truth rejects dependency drift", async () => {
  const snapshot = clone(await loadSourceTruthSnapshot());
  const dependencies = snapshot.productionPackage.dependencies as Record<
    string,
    unknown
  >;
  dependencies.pixelmatch = "7.0.0";
  assert.ok(
    validateSourceTruth(snapshot).some((error) =>
      error.startsWith("production dependency pixelmatch drifted:"),
    ),
  );
});

test("source lock truth rejects license and notice drift", async () => {
  const snapshot = clone(await loadSourceTruthSnapshot());
  const assets = snapshot.nativeLock.assets as Array<Record<string, unknown>>;
  const cMaps = assets.find((asset) => asset.id === "pdfjs-cmaps");
  assert.ok(cMaps);
  (cMaps.license as Record<string, unknown>).expression = "Apache-2.0";
  snapshot.noticeSha256s["notices/pdfjs-cmaps.LICENSE"] = "0".repeat(64);
  const errors = validateSourceTruth(snapshot);
  assert.ok(errors.some((error) => error.startsWith("PDF.js CMaps license drifted:")));
  assert.ok(
    errors.some((error) =>
      error.startsWith("notices/pdfjs-cmaps.LICENSE SHA-256 drifted:"),
    ),
  );
});
