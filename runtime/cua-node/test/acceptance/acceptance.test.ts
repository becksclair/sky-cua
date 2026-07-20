import { strict as assert } from "node:assert";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "bun:test";
import { runHarness } from "./orchestrator.ts";
import {
  FIXTURE_ROOT,
  expectedFixtureFiles,
  expectedMcpImageContent,
  generateFixtures,
  fixtureIndex,
} from "./fixtures.ts";
import {
  assertArtifact,
  assertCategoryArtifact,
  stableJson,
  type AcceptanceArtifact,
  type CategoryArtifact,
} from "./types.ts";

test("acceptance fixture bytes are deterministic and check mode is read-only", () => {
  const first = expectedFixtureFiles().map((entry) => [
    entry.path,
    Buffer.from(entry.bytes).toString("hex"),
  ]);
  const second = expectedFixtureFiles().map((entry) => [
    entry.path,
    Buffer.from(entry.bytes).toString("hex"),
  ]);
  assert.deepEqual(first, second);
  assert.doesNotThrow(() => generateFixtures({ checkOnly: true }));
  assert.equal(typeof FIXTURE_ROOT, "string");
  assert.match(stableJson(fixtureIndex()), /cua-node-acceptance\/fixtures-v1/u);
});

test("all deterministic G2/G3/G4 harness tasks produce schema-valid evidence", async () => {
  const outputRoot = mkdtempSync(join(tmpdir(), "cua-node-acceptance-"));
  const result = await runHarness({ category: "all", outputRoot });
  assert.equal(result.outcome, "pending");
  assert.ok(result.artifacts.some((path) => path.includes("/evidence/g2-browser/")));
  assert.ok(result.artifacts.some((path) => path.endsWith("/summary.json")));
  for (const artifactPath of result.artifacts.filter(
    (path) =>
      path.includes("/tasks/") &&
      path.endsWith(".json") &&
      !path.endsWith("summary.json") &&
      !path.endsWith("fixtures-check.json"),
  )) {
    const artifact = JSON.parse(readFileSync(artifactPath, "utf8")) as
      | AcceptanceArtifact
      | CategoryArtifact;
    if ("task_id" in artifact) assertArtifact(artifact);
    else assertCategoryArtifact(artifact);
  }
  const summary = JSON.parse(
    readFileSync(join(outputRoot, "summary.json"), "utf8"),
  ) as Record<string, unknown>;
  assert.equal(summary.installed_acceptance, "pending");
  assert.match(
    String(summary.note),
    /real installed acceptance remains explicitly pending/u,
  );
});

test("artifact schema accepts representative pass/fail/pending results and rejects malformed evidence", () => {
  const evidence = {
    path: "/tmp/evidence.json",
    description: "fixture",
    sha256: "a".repeat(64),
  };
  const base: AcceptanceArtifact = {
    schema_version: "cua-node-acceptance/v1",
    category: "G2",
    task_id: "representative",
    outcome: "pass",
    evidence_kind: "deterministic-fake",
    installed_acceptance: "pending",
    summary: "representative",
    checks: [{ id: "check", outcome: "pass", detail: "ok" }],
    evidence: [evidence],
    adapter: { name: "fake", command_env: "NONE", used_command: false },
  };
  assert.doesNotThrow(() => assertArtifact(base));
  assert.doesNotThrow(() =>
    assertArtifact({
      ...base,
      outcome: "fail",
      checks: [{ id: "check", outcome: "fail", detail: "expected negative" }],
    }),
  );
  assert.doesNotThrow(() =>
    assertArtifact({
      ...base,
      outcome: "pending",
      checks: [{ id: "check", outcome: "pending", detail: "installed later" }],
    }),
  );
  assert.throws(() =>
    assertArtifact({ ...base, evidence: [{ ...evidence, sha256: "bad" }] }),
  );
});

test("final MCP image content preserves upstream metadata and prefix-free base64", () => {
  const image = expectedMcpImageContent();
  assert.deepEqual(image, {
    type: "image",
    data: "iVBORw0KGgo=",
    mimeType: "image/png",
    _meta: { "codex/imageDetail": "original" },
  });
  assert.equal(image.data.startsWith("data:"), false);
});

test("acceptance harness has no production runtime imports", () => {
  const sourceFiles = [
    "types.ts",
    "fixtures.ts",
    "fakes.ts",
    "adapters.ts",
    "orchestrator.ts",
  ];
  for (const file of sourceFiles) {
    const source = readFileSync(join(FIXTURE_ROOT, "../../acceptance", file), "utf8");
    assert.doesNotMatch(
      source,
      /runtime\/cua-node\/src|sky-cua\/packages|sandbox-state|landlock|socket-allowlist/u,
      file,
    );
  }
});
