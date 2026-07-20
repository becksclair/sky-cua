import { createHash } from "node:crypto";
import { readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";
import { test } from "bun:test";
import { strict as assert } from "node:assert";
import { verifyManifest } from "../tools/verifier/manifest-verification.ts";
import type { VerificationCheck } from "../tools/verifier/types.ts";

const Ajv2020 = require("ajv/dist/2020");

const contractsRoot = resolve(import.meta.dir);
const fixtureRoot = join(contractsRoot, "..", "test", "fixtures", "fake-runtime");
const schema = JSON.parse(
  readFileSync(join(contractsRoot, "runtime-manifest.schema.json"), "utf8"),
);
const manifest = JSON.parse(readFileSync(join(fixtureRoot, "manifest.json"), "utf8"));

function sha256(path: string) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

test("runtime manifest schema validates schema v2 compatibility v1 and rejects target drift", () => {
  const validator = new Ajv2020({ allErrors: true, strict: false }).compile(schema);

  assert.equal(validator(manifest), true, JSON.stringify(validator.errors));

  const drifted = { ...manifest, target: "darwin-arm64" };
  assert.equal(validator(drifted), false);
  assert.equal(schema.properties.schema_version.const, 2);
  assert.equal(schema.properties.manifest_version.const, 1);
  assert.deepEqual(manifest.data, {
    playwright: "share/playwright",
    tessdata: "share/tessdata",
    pdfjs: "share/pdfjs",
    licenses: "licenses",
    sbom: "sbom.cdx.json",
  });
  assert.equal(schema.title, "sky-cua cua_node runtime manifest");
  assert.equal(manifest.source.producer, "sky-cua");
  assert.equal(manifest.source.producer_commit.length, 40);
  assert.equal(manifest.source.migration_input.source_tree_sha256.length, 64);
  assert.equal(
    manifest.components.browser_use.package_name,
    "@heliasar/browser-use",
  );
  assert.equal(
    manifest.components.browser_use.entrypoint_sha256,
    manifest.trusted_browser_client_sha256s[0],
  );
});

test("fixture manifest records exact executable and shipped-file checksums", () => {
  assert.equal(manifest.node_sha256, sha256(join(fixtureRoot, manifest.node_path)));
  assert.equal(
    manifest.node_repl_sha256,
    sha256(join(fixtureRoot, manifest.node_repl_path)),
  );
  assert.equal(manifest.checksums.algorithm, "sha256");
  assert.equal(
    manifest.checksums.files.some(
      (entry: { path: string }) => entry.path === "manifest.json",
    ),
    false,
  );

  for (const entry of manifest.checksums.files) {
    const filePath = join(fixtureRoot, entry.path);
    const file = statSync(filePath);
    assert.equal(file.isFile(), true, entry.path);
    assert.equal(entry.size_bytes, file.size, entry.path);
    assert.equal(entry.sha256, sha256(filePath), entry.path);
  }

  assert.notEqual(statSync(join(fixtureRoot, "bin/node")).mode & 0o111, 0);
  assert.notEqual(statSync(join(fixtureRoot, "bin/node_repl")).mode & 0o111, 0);
});

test("production verification rejects unresolved producer provenance", () => {
  const checks: VerificationCheck[] = [];
  verifyManifest(manifest, checks, "linux-x64-glibc", false);
  assert.deepEqual(
    checks
      .filter((check) => check.id.startsWith("manifest:source:"))
      .map(({ id, status }) => ({ id, status })),
    [
      { id: "manifest:source:producer-commit", status: "failed" },
      { id: "manifest:source:migration-evidence", status: "failed" },
      { id: "manifest:source:migration-input", status: "failed" },
    ],
  );
});

test("environment and wrapper contracts freeze the integration seams", () => {
  const environment = JSON.parse(
    readFileSync(join(contractsRoot, "runtime-environment.contract.json"), "utf8"),
  );
  const wrapper = JSON.parse(
    readFileSync(join(contractsRoot, "computer-use-wrapper.contract.json"), "utf8"),
  );

  assert.deepEqual(environment.precedence, [
    "explicit_legacy_fallback",
    "explicit_valid_compatible_environment",
    "bundled_manifest_environment",
    "actionable_missing_runtime_error",
  ]);
  assert.equal(environment.variables.NODE_REPL_NODE_MODULE_DIRS.separator, ":");
  assert.equal(
    environment.variables.NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S.separator,
    ",",
  );
  assert.equal(environment.mode_selection.invalid_bundled_manifest_fails_closed, true);
  assert.equal(wrapper.target_behavior.package_specifier, "@heliasar/sky-cua");
  assert.equal(wrapper.target_behavior.named_export, "sky");
  assert.equal(wrapper.target_behavior.lazy, true);
  assert.equal(wrapper.publication.symbol, 'Symbol.for("openai.computer-use.runtime")');
  assert.equal(wrapper.idempotence.second_apply_byte_identical, true);
  assert.equal(wrapper.target_behavior.oai_sky_alias, "forbidden");
});
