import { readFile } from "node:fs/promises";
import { join } from "node:path";

import { strict as assert } from "node:assert";
import { test } from "bun:test";

import {
  createArtifacts,
  inputs,
  renderJson,
  type CompliancePolicy,
  type Disposition,
} from "./generate.ts";
import { productionPin as pin } from "../production/production-pins.ts";

const complianceRoot = import.meta.dir;
const libvipsSourceOffer = `sharp-libvips-${pin("@img/sharp-libvips-linux-x64")}-source-offer.json`;

const expectedComponents = new Map<
  string,
  { version: string; disposition: Disposition }
>([
  ["acorn", { version: pin("acorn"), disposition: "routine-notice-clearance" }],
  ["acorn-walk", { version: pin("acorn-walk"), disposition: "routine-notice-clearance" }],
  ["node-runtime", { version: "24.14.0", disposition: "routine-notice-clearance" }],
  ["npm", { version: "11.9.0", disposition: "routine-notice-clearance" }],
  ["corepack", { version: "0.34.6", disposition: "routine-notice-clearance" }],
  ["playwright", { version: pin("playwright"), disposition: "routine-notice-clearance" }],
  ["playwright-core", { version: pin("playwright-core"), disposition: "routine-notice-clearance" }],
  ["pdfjs-dist", { version: pin("pdfjs-dist"), disposition: "routine-notice-clearance" }],
  ["pdfjs-cmaps", { version: pin("pdfjs-dist"), disposition: "routine-notice-clearance" }],
  [
    "pdfjs-standard-fonts",
    { version: pin("pdfjs-dist"), disposition: "routine-notice-clearance" },
  ],
  ["tesseract-js", { version: pin("tesseract.js"), disposition: "routine-notice-clearance" }],
  ["tesseract-js-core", { version: pin("tesseract.js-core"), disposition: "routine-notice-clearance" }],
  ["tessdata-eng", { version: "eng", disposition: "routine-notice-clearance" }],
  ["tessdata-osd", { version: "osd", disposition: "routine-notice-clearance" }],
  ["sharp", { version: pin("sharp"), disposition: "routine-notice-clearance" }],
  [
    "sharp-linux-x64-addon",
    { version: pin("@img/sharp-linux-x64"), disposition: "routine-notice-clearance" },
  ],
  [
    "sharp-libvips-linux-x64",
    { version: pin("@img/sharp-libvips-linux-x64"), disposition: "routine-notice-clearance" },
  ],
  ["canvas", { version: pin("@napi-rs/canvas"), disposition: "routine-notice-clearance" }],
  [
    "canvas-linux-x64-gnu",
    { version: pin("@napi-rs/canvas-linux-x64-gnu"), disposition: "routine-notice-clearance" },
  ],
  ["pixelmatch", { version: pin("pixelmatch"), disposition: "routine-notice-clearance" }],
  ["sky-cua", { version: "0.1.0", disposition: "routine-notice-clearance" }],
]);

const expectedGates = new Map<
  string,
  { type: "provenance-only" | "unresolved-evidence"; evidence: string[] }
>();

function sorted(values: string[]): string[] {
  return values.slice().sort((left, right) => left.localeCompare(right));
}

test("every planned component has the settled disposition and provenance record", () => {
  const policy = inputs.policy;
  assert.equal(policy.release_status, "clear");
  assert.deepEqual(
    new Set(policy.components.map((component) => component.id)),
    new Set(expectedComponents.keys()),
  );
  assert.equal(policy.components.length, expectedComponents.size);

  const dispositionIds = new Set(
    policy.dispositions.map((disposition) => disposition.id),
  );
  const provenanceByComponent = new Map(
    inputs.provenance.records.map((record) => [record.component_id, record]),
  );
  const noticeIds = new Set(inputs.notices.entries.map((entry) => entry.id));
  assert.equal(provenanceByComponent.size, expectedComponents.size);

  for (const component of policy.components) {
    const expected = expectedComponents.get(component.id);
    assert.ok(expected, `unexpected or unplanned component: ${component.id}`);
    assert.ok(
      dispositionIds.has(component.disposition),
      `missing disposition policy: ${component.disposition}`,
    );
    assert.equal(component.version, expected.version);
    assert.equal(component.disposition, expected.disposition);
    assert.ok(
      component.notice_ids.length > 0,
      `${component.id} has no notice inventory reference`,
    );
    for (const noticeId of component.notice_ids) {
      assert.ok(
        noticeIds.has(noticeId),
        `${component.id} references missing notice ${noticeId}`,
      );
    }
    const provenance = provenanceByComponent.get(component.id);
    assert.ok(provenance, `missing provenance record for ${component.id}`);
    assert.equal(provenance.source_uri, component.source.uri);
    assert.equal(provenance.expected_platform, "linux-x64-glibc");
    if (component.source.provenance_status === "artifact-provenance-resolved") {
      assert.ok(
        provenance.status === "artifact-resolved" || provenance.status === "cleared",
      );
      assert.equal(provenance.resolved, true);
    }
    if (component.disposition === "routine-notice-clearance") {
      assert.deepEqual(component.blocking_gate_ids, []);
      assert.ok(
        provenance.status === "planned" ||
          provenance.status === "artifact-resolved" ||
          provenance.status === "cleared",
      );
    } else {
      assert.ok(
        component.blocking_gate_ids.length > 0,
        `${component.id} has no blocking gate`,
      );
      assert.notEqual(provenance.status, "planned");
    }
  }
});

test("all release-compliance gates are closed", () => {
  const policy = inputs.policy;
  assert.deepEqual(
    new Set(policy.blocking_gates.map((gate) => gate.id)),
    new Set(expectedGates.keys()),
  );
  assert.equal(policy.blocking_gates.length, expectedGates.size);
  const componentIds = new Set(policy.components.map((component) => component.id));

  for (const gate of policy.blocking_gates) {
    const expected = expectedGates.get(gate.id);
    assert.ok(expected, `unexpected gate ${gate.id}`);
    assert.equal(gate.status, "open");
    assert.equal(gate.blocking, true);
    assert.equal(gate.type, expected.type);
    assert.deepEqual(
      sorted(gate.required_evidence.map((evidence) => evidence.id)),
      sorted(expected.evidence),
    );
    assert.ok(
      gate.required_evidence.every((evidence) => evidence.status === "missing"),
    );
    assert.ok(gate.component_ids.every((componentId) => componentIds.has(componentId)));
    assert.ok(gate.acceptance.length > 0);
    for (const componentId of gate.component_ids) {
      const component = policy.components.find(
        (candidate) => candidate.id === componentId,
      );
      assert.ok(component);
      assert.ok(component.blocking_gate_ids.includes(gate.id));
    }
  }
});

test("system browser is external and excluded from distribution SBOM components", () => {
  const policy = inputs.policy;
  assert.equal(policy.external_requirements.length, 1);
  const requirement = policy.external_requirements[0];
  assert.ok(requirement);
  assert.equal(requirement.id, "system-browser");
  assert.equal(requirement.redistribution, "not-redistributed");
  assert.equal(requirement.sbom_distribution_component, false);
  assert.equal(
    policy.components.some(
      (component) => component.kind === "browser" || component.kind === "codec",
    ),
    false,
  );

  const artifacts = createArtifacts();
  assert.equal(
    artifacts.cyclonedx.components.some((component) =>
      component.name.includes("Chromium"),
    ),
    false,
  );
  assert.equal(
    artifacts.spdx.packages.some((component) => component.name.includes("Chromium")),
    false,
  );
});

test("resolved Node, sharp-libvips, and Canvas artifact evidence is exact", () => {
  const provenanceByComponent = new Map(
    inputs.provenance.records.map((record) => [record.component_id, record]),
  );
  const node = provenanceByComponent.get("node-runtime");
  assert.ok(node);
  assert.equal(node.status, "artifact-resolved");
  assert.equal(node.resolved, true);
  assert.equal(
    node.artifact_sha256,
    "41cd79bb7877c81605a9e68ec4c91547774f46a40c67a17e34d7179ef11729df",
  );

  const sharpLibvips = provenanceByComponent.get("sharp-libvips-linux-x64");
  assert.ok(sharpLibvips);
  assert.equal(sharpLibvips.status, "cleared");
  assert.equal(sharpLibvips.resolved, true);
  assert.equal(
    sharpLibvips.artifact_sha256,
    "4aa73553408c3964071728f4a231a8e86918e39bc49f1fbae75355f7ea933ea9",
  );
  assert.equal(
    sharpLibvips.artifact_sha512,
    "189fff4925db9f04837acd36ba60779c324b15c433c3c6b5f15f1fca1aabead579d79b4e10c7489a38f18f502869f311cf9e85dcf1e07ed9e3d5010a494d7393",
  );
  assert.equal(
    sharpLibvips.slsa_source_commit,
    "ebb95f8add54eee8bed840e3fb587e4cbec857d7",
  );

  const canvas = provenanceByComponent.get("canvas-linux-x64-gnu");
  assert.ok(canvas);
  assert.equal(canvas.status, "cleared");
  assert.equal(canvas.resolved, true);
  assert.equal(
    canvas.artifact_sha256,
    "edcbca8d43db993a9066974c97ca0e87fb179aaee61d5e56561a19ae171b643d",
  );
  assert.equal(canvas.native_addon_sha256, canvas.artifact_sha256);
  assert.equal(canvas.artifact_sha512, undefined);
  assert.equal(canvas.slsa_source_commit, "6661e25b9520bfc2df1e4c9820717fee5dd304fd");
  assert.equal(
    canvas.skia_submodule_commit,
    "ee20d565acb08dece4a32e3f209cdd41119015ca",
  );

  const artifacts = createArtifacts();
  const canvasCdx = artifacts.cyclonedx.components.find(
    (component) => component.name === "@napi-rs/canvas Linux x64 gnu native addon",
  );
  assert.ok(canvasCdx);
  assert.deepEqual(canvasCdx.hashes, [
    { alg: "SHA-256", content: canvas.artifact_sha256 },
  ]);
  const canvasSpdx = artifacts.spdx.packages.find(
    (packageRecord) => packageRecord.name === canvasCdx.name,
  );
  assert.ok(canvasSpdx);
  assert.deepEqual(canvasSpdx.checksums, [
    { algorithm: "SHA256", checksumValue: canvas.artifact_sha256 },
  ]);
});

test("PDF data, Canvas package, and pixelmatch licenses match shipped bytes", async () => {
  const policyById = new Map(
    inputs.policy.components.map((component) => [component.id, component]),
  );
  const provenanceById = new Map(
    inputs.provenance.records.map((record) => [record.component_id, record]),
  );
  const expected = new Map([
    [
      "acorn",
      {
        license: "MIT",
        sha256: "838d0d6330ffdcf8bd15c3d31af77100f12a69535342256657a5c4fd7aef3118",
      },
    ],
    [
      "acorn-walk",
      {
        license: "MIT",
        sha256: "cd968876dd2797441c7c6de59eaa867e2d804e490e4e6c6e5d9be3d247ea6b0f",
      },
    ],
    [
      "pdfjs-cmaps",
      {
        license: "BSD-3-Clause",
        sha256: "8e7e37a6ab3d1b0d7d24ed62ef49e7374c6f79378b8189dc23bf90a6955b6de6",
      },
    ],
    [
      "pdfjs-standard-fonts",
      {
        license: "BSD-3-Clause AND OFL-1.1",
        sha256: "c8b927c60a75d97c4f75deb1ffc04eaa700b598106db47c67422ecb7dbe974cf",
      },
    ],
    [
      "canvas",
      {
        license: "MIT",
        sha256: "110b2f6fbf3234fea13a14fce94284f1383eba1aa958447bf4ba06e3e8d29be8",
      },
    ],
    [
      "pixelmatch",
      {
        license: "ISC",
        sha256: "6407fbe0821c7b3635924870de1173714beb723161dc033e5421eacdad8c6137",
      },
    ],
  ]);
  for (const [id, truth] of expected) {
    const component = policyById.get(id);
    const provenance = provenanceById.get(id);
    assert.ok(component);
    assert.ok(provenance);
    assert.equal(component.license.spdx_expression, truth.license);
    assert.equal(component.source.provenance_status, "artifact-provenance-resolved");
    assert.equal(provenance.status, "cleared");
    assert.equal(provenance.resolved, true);
    assert.equal(provenance.artifact_sha256, truth.sha256);
  }

  const exactNotices = new Map([
    [
      "notices/acorn-8.19.0.LICENSE",
      "76a876cf886ff9be2a8b5e2e86514fed06223c8c9f0c1e9ee9606e93841e00b7",
    ],
    [
      "notices/acorn-walk-8.3.5.LICENSE",
      "c2d8184ea9becd063aa40be243e8c6ec2c4f72828fdfe4b0752664ef73e96ed3",
    ],
    [
      "notices/pdfjs-cmaps.LICENSE",
      "aa92ab5a472974865a96fd4a4e9c13bb41bf6fe1b309cb6b8da48bc9e19839a2",
    ],
    [
      "notices/pdfjs-standard-fonts.LICENSE_FOXIT",
      "b578cdd2345840ada550bd12519533812320d5f1d21cf4c1c7e1b1b0a31c98b7",
    ],
    [
      "notices/pdfjs-standard-fonts.LICENSE_LIBERATION",
      "93fed46019c38bbe566b479d22148e2e8a1e85ada614accb0211c37b2c61c19b",
    ],
    [
      "notices/pixelmatch-7.1.0.LICENSE",
      "cfec0482fb785fe27e3b368a2d9e84bf7a61b275e83ec582bf288e98cd530bb0",
    ],
  ]);
  for (const [path, digest] of exactNotices) {
    assert.equal(
      new Bun.CryptoHasher("sha256")
        .update(await readFile(join(complianceRoot, path)))
        .digest("hex"),
      digest,
    );
  }

  const standardFonts = createArtifacts().cyclonedx.components.find(
    (component) => component.name === "PDF.js standard fonts",
  );
  assert.ok(standardFonts);
  assert.deepEqual(standardFonts.licenses, [
    { expression: "BSD-3-Clause AND OFL-1.1" },
  ]);
});

test("sharp source offer and cleared Canvas self-build record are complete", async () => {
  const sourceOffer = JSON.parse(
    await readFile(
      join(complianceRoot, libvipsSourceOffer),
      "utf8",
    ),
  ) as {
    status: string;
    corresponding_source: { revision: string };
    build: { patches: Array<{ uri: string; sha256: string }> };
    components: Array<{
      id: string;
      version: string;
      source_uri: string;
      notice_uri: string;
    }>;
  };
  assert.equal(sourceOffer.status, "operational");
  assert.equal(
    sourceOffer.corresponding_source.revision,
    "ebb95f8add54eee8bed840e3fb587e4cbec857d7",
  );
  assert.ok(sourceOffer.build.patches.length > 0);
  assert.equal(
    new Set(sourceOffer.build.patches.map((patch) => patch.uri)).size,
    sourceOffer.build.patches.length,
  );
  assert.ok(
    sourceOffer.build.patches.every(
      (patch) => patch.uri.startsWith("https://") && /^[0-9a-f]{64}$/.test(patch.sha256),
    ),
  );
  assert.equal(sourceOffer.components.length, 29);
  assert.equal(
    new Set(sourceOffer.components.map((component) => component.id)).size,
    29,
  );
  assert.ok(sourceOffer.components.every((component) => component.version.length > 0));
  assert.ok(
    sourceOffer.components.every((component) =>
      component.source_uri.startsWith("https://"),
    ),
  );
  assert.ok(
    sourceOffer.components.every((component) =>
      component.notice_uri.startsWith("https://"),
    ),
  );
  assert.equal(
    sourceOffer.components.find((component) => component.id === "libvips")?.version,
    "8.18.7",
  );

  const canvasEvidence = JSON.parse(
    await readFile(join(complianceRoot, "canvas-0.1.91-evidence.json"), "utf8"),
  ) as {
    status: string;
    slsa: { source_commit: string; workflow: string; skia_submodule_commit: string };
    artifact: {
      sha256: string;
      bytes: number;
      elf_build_id: string;
      maximum_glibc_symbol_version: string;
    };
    source_inventory: {
      cargo_lock_present: boolean;
      resolved_package_count: number;
      rust_direct_dependencies: string[];
    };
    static_composition: {
      static_archives: Array<{ name: string; sha256: string }>;
      archive_policy: string;
      byte_reproducible: boolean;
      second_build_sha256: string;
      second_build_id: string;
    };
    irreducible_missing_evidence: string[];
  };
  assert.equal(canvasEvidence.status, "cleared");
  assert.equal(
    canvasEvidence.slsa.source_commit,
    "6661e25b9520bfc2df1e4c9820717fee5dd304fd",
  );
  assert.equal(canvasEvidence.slsa.workflow, ".github/workflows/CI.yaml");
  assert.equal(
    canvasEvidence.slsa.skia_submodule_commit,
    "ee20d565acb08dece4a32e3f209cdd41119015ca",
  );
  assert.equal(canvasEvidence.source_inventory.cargo_lock_present, true);
  assert.equal(canvasEvidence.source_inventory.resolved_package_count, 84);
  assert.equal(canvasEvidence.source_inventory.rust_direct_dependencies.length, 20);
  assert.equal(canvasEvidence.static_composition.static_archives.length, 10);
  assert.equal(
    new Set(
      canvasEvidence.static_composition.static_archives.map((archive) => archive.name),
    ).size,
    10,
  );
  assert.ok(
    canvasEvidence.static_composition.static_archives.every((archive) =>
      /^[a-f0-9]{64}$/.test(archive.sha256),
    ),
  );
  assert.equal(canvasEvidence.static_composition.byte_reproducible, true);
  assert.deepEqual(canvasEvidence.artifact, {
    uri: "vendor/cua-node-cache/canvas-build/build-glibc228-final4/target/x86_64-unknown-linux-gnu/release/libcanvas.so",
    package_destination: "skia.linux-x64-gnu.node",
    sha256: "edcbca8d43db993a9066974c97ca0e87fb179aaee61d5e56561a19ae171b643d",
    bytes: 31_261_376,
    elf_build_id: "0b4378841be7f5b7f0c9c894217f820dfaf2cfe3",
    maximum_glibc_symbol_version: "GLIBC_2.28",
    build_record: "canvas-0.1.91-build-record.json",
    source_offer: "canvas-0.1.91-source-offer.json",
  });
  assert.match(
    canvasEvidence.static_composition.archive_policy,
    /no deduplicated archives/u,
  );
  assert.equal(
    canvasEvidence.static_composition.second_build_sha256,
    canvasEvidence.artifact.sha256,
  );
  assert.equal(
    canvasEvidence.static_composition.second_build_id,
    canvasEvidence.artifact.elf_build_id,
  );
  assert.equal(canvasEvidence.irreducible_missing_evidence.length, 0);

  const buildRecord = JSON.parse(
    await readFile(join(complianceRoot, "canvas-0.1.91-build-record.json"), "utf8"),
  ) as {
    command: string;
    environment: Record<string, string>;
    artifact: {
      sha256: string;
      link_map_sha256: string;
      maximum_glibc_symbol_version: string;
    };
    reproducibility: {
      build_1_sha256: string;
      build_2_sha256: string;
      build_1_build_id: string;
      build_2_build_id: string;
    };
  };
  assert.equal(
    buildRecord.command,
    "cargo +1.92.0 zigbuild --manifest-path $ROOT/source/Cargo.toml --target x86_64-unknown-linux-gnu.2.28 --release",
  );
  assert.equal(buildRecord.environment.SOURCE_DATE_EPOCH, undefined);
  assert.equal(buildRecord.environment.CFLAGS, undefined);
  assert.equal(buildRecord.environment.CXXFLAGS, undefined);
  assert.equal(buildRecord.artifact.sha256, canvasEvidence.artifact.sha256);
  assert.equal(buildRecord.artifact.maximum_glibc_symbol_version, "GLIBC_2.28");
  assert.equal(
    buildRecord.artifact.link_map_sha256,
    "bd898ccb8249e189b5bb18058d7ec4e8d4c5a3c49c88c0789f51095e85c927bb",
  );
  assert.equal(
    buildRecord.reproducibility.build_1_sha256,
    buildRecord.reproducibility.build_2_sha256,
  );
  assert.equal(
    buildRecord.reproducibility.build_1_build_id,
    buildRecord.reproducibility.build_2_build_id,
  );

  const smoke = JSON.parse(
    await readFile(join(complianceRoot, "canvas-0.1.91-smoke.json"), "utf8"),
  ) as {
    artifact_sha256: string;
    topology: string;
    status: string;
    checks_passed: number;
    checks: string[];
  };
  assert.equal(smoke.artifact_sha256, canvasEvidence.artifact.sha256);
  assert.equal(smoke.topology, "one-package prepared production subsystem");
  assert.equal(smoke.status, "passed");
  assert.equal(smoke.checks_passed, 9);
  assert.equal(smoke.checks.length, 9);

  const artifacts = createArtifacts();
  const sharpCdx = artifacts.cyclonedx.components.find(
    (component) => component.name === "sharp bundled libvips Linux x64",
  );
  const canvasCdx = artifacts.cyclonedx.components.find(
    (component) => component.name === "@napi-rs/canvas Linux x64 gnu native addon",
  );
  assert.ok(sharpCdx);
  assert.ok(canvasCdx);
  assert.equal(
    sharpCdx.properties.find(
      (property) => property.name === "compliance:source-offer-uri",
    )?.value,
    libvipsSourceOffer,
  );
  assert.equal(
    canvasCdx.properties.find(
      (property) => property.name === "compliance:evidence-record-uri",
    )?.value,
    "canvas-0.1.91-evidence.json",
  );
  assert.equal(
    canvasCdx.properties.find(
      (property) => property.name === "compliance:source-offer-uri",
    )?.value,
    "canvas-0.1.91-source-offer.json",
  );
  assert.equal(
    canvasCdx.properties.find(
      (property) => property.name === "compliance:residual-gate-reason",
    ),
    undefined,
  );
});

test("signed Arch tessdata provenance clears eng and osd release gates", () => {
  const provenanceByComponent = new Map(
    inputs.provenance.records.map((record) => [record.component_id, record]),
  );
  const expected = new Map([
    [
      "tessdata-eng",
      "daa0c97d651c19fba3b25e81317cd697e9908c8208090c94c3905381c23fc047",
    ],
    [
      "tessdata-osd",
      "e19f2ae860792fdf372cf48d8ce70ae5da3c4052962fe22e9de1f680c374bb0e",
    ],
  ]);
  for (const [componentId, digest] of expected) {
    const component = inputs.policy.components.find(
      (candidate) => candidate.id === componentId,
    );
    const provenance = provenanceByComponent.get(componentId);
    assert.ok(component);
    assert.ok(provenance);
    assert.equal(component.source.provenance_status, "artifact-provenance-resolved");
    assert.deepEqual(component.blocking_gate_ids, []);
    assert.equal(provenance.status, "artifact-resolved");
    assert.equal(provenance.resolved, true);
    assert.equal(provenance.artifact_sha256, digest);
    assert.ok(
      provenance.evidence.some((entry) =>
        entry.includes("pacman -Qkk reports 0 altered"),
      ),
    );
    assert.ok(provenance.evidence.some((entry) => entry.includes("990fffb9b7a9b52d")));
  }
});

test("notice inventory covers every component in both directions", async () => {
  const policy = inputs.policy;
  const componentIds = new Set(policy.components.map((component) => component.id));
  const entriesById = new Map(inputs.notices.entries.map((entry) => [entry.id, entry]));
  for (const entry of inputs.notices.entries) {
    assert.ok(entry.release_required);
    assert.ok(entry.planned_path.startsWith("notices/"));
    assert.ok(
      entry.component_ids.every((componentId) => componentIds.has(componentId)),
    );
    if (entry.status === "collected") {
      assert.ok(await Bun.file(join(complianceRoot, entry.planned_path)).exists());
    }
  }
  for (const component of policy.components) {
    for (const noticeId of component.notice_ids) {
      const entry = entriesById.get(noticeId);
      assert.ok(entry);
      assert.ok(entry.component_ids.includes(component.id));
    }
  }
});

test("sky-cua keeps its proprietary UNLICENSED designation", () => {
  const skyCua = inputs.policy.components.find(
    (component) => component.id === "sky-cua",
  );
  assert.ok(skyCua);
  assert.equal(skyCua.name, "@heliasar/sky-cua");
  assert.equal(
    skyCua.license.designation,
    "LicenseRef-Heliasar-Proprietary/UNLICENSED",
  );
  assert.equal(skyCua.license.spdx_expression, "LicenseRef-Heliasar-Proprietary");
  assert.equal(skyCua.source.provenance_status, "artifact-provenance-resolved");
  const provenance = inputs.provenance.records.find(
    (record) => record.component_id === "sky-cua",
  );
  assert.ok(provenance);
  assert.equal(provenance.status, "artifact-resolved");
  assert.equal(provenance.resolved, true);
  assert.equal(
    provenance.artifact_sha256,
    "fecb9426aa44de2abdaaf4bc081d9c1a0cba7a2021dfbdc8c9c0d8572f8c86c2",
  );
});

test("CycloneDX and SPDX output is deterministic and contains every component", async () => {
  const first = createArtifacts();
  const second = createArtifacts();
  assert.deepEqual(first, second);

  const cdxPath = join(complianceRoot, "sbom.cdx.json");
  const spdxPath = join(complianceRoot, "sbom.spdx.json");
  assert.equal(await readFile(cdxPath, "utf8"), renderJson(first.cyclonedx));
  assert.equal(await readFile(spdxPath, "utf8"), renderJson(first.spdx));
  assert.equal(first.cyclonedx.components.length, expectedComponents.size);
  assert.equal(first.spdx.packages.length, expectedComponents.size + 1);
  assert.deepEqual(
    sorted(
      first.cyclonedx.components.map(
        (component) =>
          component.properties.find(
            (property) => property.name === "compliance:disposition",
          )?.value ?? "",
      ),
    ),
    sorted(inputs.policy.components.map((component) => component.disposition)),
  );
  const skyCuaPackage = first.spdx.packages.find(
    (packageRecord) => packageRecord.name === "@heliasar/sky-cua",
  );
  assert.ok(skyCuaPackage);
  assert.match(
    skyCuaPackage.licenseComments ?? "",
    /LicenseRef-Heliasar-Proprietary\/UNLICENSED/,
  );
});

test("policy schema and templates expose the compliance contract", async () => {
  const schema = JSON.parse(
    await readFile(join(complianceRoot, "policy.schema.json"), "utf8"),
  ) as { required: string[]; $defs: Record<string, unknown> };
  assert.ok(schema.required.includes("components"));
  assert.ok(schema.required.includes("blocking_gates"));
  assert.ok(schema.$defs.component);
  assert.ok(schema.$defs.gate);

  const provenanceTemplate = await readFile(
    join(complianceRoot, "provenance.template.json"),
    "utf8",
  );
  const sourceOfferTemplate = await readFile(
    join(complianceRoot, "source-offer.template.md"),
    "utf8",
  );
  const sourceOfferRecordTemplate = await readFile(
    join(complianceRoot, "source-offer-record.template.json"),
    "utf8",
  );
  for (const requiredField of [
    "component_id",
    "source_uri",
    "artifact_sha256",
    "sha256_scope",
    "license_source_uri",
  ]) {
    assert.match(provenanceTemplate, new RegExp(requiredField));
  }
  for (const requiredField of [
    "Corresponding source",
    "Build and composition record",
    "License and notices",
    "Binary/archive SHA-256",
  ]) {
    assert.match(sourceOfferTemplate, new RegExp(requiredField));
  }
  for (const requiredField of [
    "corresponding_source",
    "configure_flags",
    "component_notices",
    "valid_until",
  ]) {
    assert.match(sourceOfferRecordTemplate, new RegExp(requiredField));
  }
});

const _policyTypeCheck: CompliancePolicy | undefined = inputs.policy;
void _policyTypeCheck;
