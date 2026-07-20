import { mkdir } from "node:fs/promises";
import { dirname, join } from "node:path";

import noticeInventoryJson from "./notice-inventory.json";
import policyJson from "./policy.json";
import provenanceJson from "./provenance.json";

export type Disposition =
  | "routine-notice-clearance"
  | "provenance-only-gate"
  | "unresolved-evidence-gate";

export type GateType = "provenance-only" | "unresolved-evidence";

export interface LicenseRecord {
  spdx_expression: string;
  designation: string;
}

export interface SourceRecord {
  uri: string;
  kind: string;
  provenance_status:
    | "planned-authoritative-source"
    | "artifact-provenance-resolved"
    | "provenance-unresolved"
    | "evidence-unresolved";
}

export interface PolicyComponent {
  id: string;
  name: string;
  version: string;
  kind: string;
  purl: string;
  disposition: Disposition;
  release_effect: "notice-clearance-required" | "blocking-gate-open";
  license: LicenseRecord;
  source: SourceRecord;
  notice_ids: string[];
  blocking_gate_ids: string[];
  notes: string;
}

export interface EvidenceRecord {
  id: string;
  kind: string;
  status: "missing";
  acceptance: string;
}

export interface BlockingGate {
  id: string;
  type: GateType;
  status: "open";
  blocking: true;
  component_ids: string[];
  required_evidence: EvidenceRecord[];
  source_offer_required: boolean;
  source_offer_template?: string;
  acceptance: string[];
  reason: string;
}

export interface DispositionPolicy {
  id: Disposition;
  summary: string;
  release_blocking: boolean;
  required_evidence: string[];
}

export interface CompliancePolicy {
  schema_version: 1;
  policy_id: "linux-cua-node-compliance";
  target: "linux-x64-glibc";
  release_status: "blocked" | "clear";
  determinism: {
    fixed_timestamp: "1970-01-01T00:00:00Z";
    component_sort_key: "id";
    network_access: "not-used";
  };
  external_requirements: Array<{
    id: string;
    kind: "runtime-dependency";
    platform: "linux";
    requirement: string;
    redistribution: "not-redistributed";
    sbom_distribution_component: false;
    notes: string;
  }>;
  dispositions: DispositionPolicy[];
  components: PolicyComponent[];
  blocking_gates: BlockingGate[];
}

export interface NoticeEntry {
  id: string;
  component_ids: string[];
  kind: string;
  status: "planned" | "provenance-open" | "evidence-open" | "collected";
  planned_path: string;
  source_uri: string;
  release_required: true;
  gate_ids: string[];
}

export interface NoticeInventory {
  schema_version: 1;
  policy_id: "linux-cua-node-compliance";
  target: "linux-x64-glibc";
  status: "pending-clearance";
  notice_root: "notices";
  entries: NoticeEntry[];
}

export interface ProvenanceRecord {
  component_id: string;
  source_uri: string;
  source_kind: string;
  expected_platform: "linux-x64-glibc";
  status:
    | "planned"
    | "provenance-open"
    | "evidence-open"
    | "artifact-resolved"
    | "cleared";
  resolved: boolean;
  artifact_sha256: string | null;
  artifact_sha512?: string;
  sha256_scope: string;
  sha512_scope?: string;
  native_addon_sha256?: string;
  native_addon_sha256_scope?: string;
  slsa_source_commit?: string;
  skia_submodule_commit?: string;
  evidence_record_uri?: string;
  source_offer_uri?: string;
  residual_gate_reason?: string;
  evidence: string[];
}

export interface ProvenanceManifest {
  schema_version: 1;
  policy_id: "linux-cua-node-compliance";
  target: "linux-x64-glibc";
  status: "open";
  network_access: "not-used";
  records: ProvenanceRecord[];
}

export interface ComplianceInputs {
  policy: CompliancePolicy;
  notices: NoticeInventory;
  provenance: ProvenanceManifest;
}

export interface CycloneDxDocument {
  bomFormat: "CycloneDX";
  specVersion: "1.6";
  serialNumber: string;
  version: 1;
  metadata: {
    timestamp: "1970-01-01T00:00:00Z";
    tools: Array<{ vendor: string; name: string; version: string }>;
    component: {
      type: "application";
      bomRef: string;
      name: string;
      version: string;
      purl: string;
    };
    properties: Array<{ name: string; value: string }>;
  };
  components: Array<{
    type: "library" | "data";
    bomRef: string;
    name: string;
    version: string;
    scope: "required";
    purl: string;
    licenses: Array<
      { license: { id?: string; name?: string } } | { expression: string }
    >;
    hashes?: Array<{ alg: "SHA-256" | "SHA-512"; content: string }>;
    properties: Array<{ name: string; value: string }>;
  }>;
}

export interface SpdxDocument {
  spdxVersion: "SPDX-2.3";
  dataLicense: "CC0-1.0";
  SPDXID: "SPDXRef-DOCUMENT";
  name: string;
  documentNamespace: string;
  creationInfo: {
    created: "1970-01-01T00:00:00Z";
    creators: string[];
  };
  packages: Array<{
    SPDXID: string;
    name: string;
    versionInfo: string;
    downloadLocation: string;
    filesAnalyzed: false;
    licenseConcluded: string;
    licenseDeclared: string;
    licenseComments?: string;
    packageComment: string;
    checksums?: Array<{
      algorithm: "SHA256" | "SHA512";
      checksumValue: string;
    }>;
    externalRefs: Array<{
      referenceCategory: "PACKAGE-MANAGER";
      referenceType: "purl";
      referenceLocator: string;
    }>;
    attributionTexts: string[];
  }>;
  hasExtractedLicensingInfos: Array<{
    licenseId: string;
    extractedText: string;
  }>;
  relationships: Array<{
    spdxElementId: string;
    relationshipType: "DESCRIBES" | "DEPENDS_ON";
    relatedSpdxElement: string;
  }>;
}

export interface ComplianceArtifacts {
  cyclonedx: CycloneDxDocument;
  spdx: SpdxDocument;
}

const policy = policyJson as CompliancePolicy;
const notices = noticeInventoryJson as NoticeInventory;
const provenance = provenanceJson as ProvenanceManifest;

export const inputs: ComplianceInputs = { policy, notices, provenance };

const COMPONENT_BOM_REF_PREFIX = "pkg:generic/cua-node-component/";
const ROOT_BOM_REF = "pkg:generic/cua-node@0.1.0";
const ROOT_SPDX_ID = "SPDXRef-cua-node";

function sortedComponents(components: PolicyComponent[]): PolicyComponent[] {
  return components.slice().sort((left, right) => left.id.localeCompare(right.id));
}

function sortedGates(gates: BlockingGate[]): BlockingGate[] {
  return gates.slice().sort((left, right) => left.id.localeCompare(right.id));
}

function componentBomRef(component: PolicyComponent): string {
  return `${COMPONENT_BOM_REF_PREFIX}${component.id}`;
}

function componentSpdxId(component: PolicyComponent): string {
  return `SPDXRef-${component.id}`;
}

function findProvenanceRecord(
  component: PolicyComponent,
  manifest: ProvenanceManifest,
): ProvenanceRecord {
  const record = manifest.records.find(
    (candidate) => candidate.component_id === component.id,
  );
  if (record === undefined) {
    throw new Error(`Missing provenance record for ${component.id}`);
  }
  return record;
}

function licenseValue(component: PolicyComponent): { id?: string; name?: string } {
  const expression = component.license.spdx_expression;
  if (expression.startsWith("LicenseRef-") || expression.includes("LicenseRef-")) {
    return { name: component.license.designation };
  }
  return { id: expression };
}

function componentLicenses(
  component: PolicyComponent,
): CycloneDxDocument["components"][number]["licenses"] {
  const expression = component.license.spdx_expression;
  if (/\s(?:AND|OR|WITH)\s/u.test(expression)) return [{ expression }];
  return [{ license: licenseValue(component) }];
}

function componentType(component: PolicyComponent): "library" | "data" {
  return component.kind === "data" || component.kind === "language-data"
    ? "data"
    : "library";
}

function componentProperties(
  component: PolicyComponent,
  manifest: ProvenanceManifest,
): Array<{ name: string; value: string }> {
  const record = findProvenanceRecord(component, manifest);
  const properties = [
    { name: "compliance:disposition", value: component.disposition },
    { name: "compliance:release-effect", value: component.release_effect },
    { name: "compliance:provenance-status", value: record.status },
    { name: "compliance:notice-ids", value: component.notice_ids.join(",") },
    {
      name: "compliance:blocking-gate-ids",
      value: component.blocking_gate_ids.join(","),
    },
  ];
  const evidenceProperties: Array<[string, string | undefined]> = [
    ["compliance:native-addon-sha256", record.native_addon_sha256],
    ["compliance:slsa-source-commit", record.slsa_source_commit],
    ["compliance:skia-submodule-commit", record.skia_submodule_commit],
    ["compliance:evidence-record-uri", record.evidence_record_uri],
    ["compliance:source-offer-uri", record.source_offer_uri],
    ["compliance:residual-gate-reason", record.residual_gate_reason],
  ];
  for (const [name, value] of evidenceProperties) {
    if (value !== undefined) {
      properties.push({ name, value });
    }
  }
  return properties;
}

function componentHashes(
  component: PolicyComponent,
  manifest: ProvenanceManifest,
): Array<{ alg: "SHA-256" | "SHA-512"; content: string }> {
  const record = findProvenanceRecord(component, manifest);
  const hashes: Array<{ alg: "SHA-256" | "SHA-512"; content: string }> = [];
  if (record.artifact_sha256 !== null) {
    hashes.push({ alg: "SHA-256", content: record.artifact_sha256 });
  }
  if (record.artifact_sha512 !== undefined) {
    hashes.push({ alg: "SHA-512", content: record.artifact_sha512 });
  }
  return hashes;
}

export function createCycloneDxDocument(
  source: ComplianceInputs = inputs,
): CycloneDxDocument {
  const components = sortedComponents(source.policy.components);
  const gates = sortedGates(source.policy.blocking_gates);
  return {
    bomFormat: "CycloneDX",
    specVersion: "1.6",
    serialNumber: "urn:uuid:00000000-0000-4000-8000-000000000001",
    version: 1,
    metadata: {
      timestamp: source.policy.determinism.fixed_timestamp,
      tools: [
        { vendor: "@heliasar", name: "cua-node-compliance-scaffold", version: "1.0.0" },
      ],
      component: {
        type: "application",
        bomRef: ROOT_BOM_REF,
        name: "cua_node",
        version: "0.1.0",
        purl: ROOT_BOM_REF,
      },
      properties: [
        { name: "compliance:policy-id", value: source.policy.policy_id },
        { name: "compliance:target", value: source.policy.target },
        { name: "compliance:release-status", value: source.policy.release_status },
        {
          name: "compliance:open-blocking-gates",
          value: gates.map((gate) => gate.id).join(","),
        },
        {
          name: "compliance:network-access",
          value: source.policy.determinism.network_access,
        },
        {
          name: "compliance:external-system-browser",
          value: source.policy.external_requirements[0]?.requirement ?? "",
        },
      ],
    },
    components: components.map((component) => {
      const hashes = componentHashes(component, source.provenance);
      return {
        type: componentType(component),
        bomRef: componentBomRef(component),
        name: component.name,
        version: component.version,
        scope: "required",
        purl: component.purl,
        licenses: componentLicenses(component),
        ...(hashes.length > 0 ? { hashes } : {}),
        properties: componentProperties(component, source.provenance),
      };
    }),
  };
}

function extractedLicenses(
  source: ComplianceInputs,
): SpdxDocument["hasExtractedLicensingInfos"] {
  const customLicenseIds = new Set<string>();
  for (const component of source.policy.components) {
    const expression = component.license.spdx_expression;
    if (expression.includes("LicenseRef-Node.js")) {
      customLicenseIds.add("LicenseRef-Node.js");
    }
    if (expression.includes("LicenseRef-PDF.js-Standard-Fonts")) {
      customLicenseIds.add("LicenseRef-PDF.js-Standard-Fonts");
    }
    if (expression.includes("LicenseRef-Heliasar-Proprietary")) {
      customLicenseIds.add("LicenseRef-Heliasar-Proprietary");
    }
  }
  const descriptions: Record<string, string> = {
    "LicenseRef-Heliasar-Proprietary":
      "First-party @heliasar/sky-cua; project designation LicenseRef-Heliasar-Proprietary/UNLICENSED.",
    "LicenseRef-Node.js":
      "Node.js release license text must be copied from the exact official release archive.",
    "LicenseRef-PDF.js-Standard-Fonts":
      "PDF.js standard-font component notices must be collected for the exact standard_fonts tree.",
  };
  return Array.from(customLicenseIds)
    .sort((left, right) => left.localeCompare(right))
    .map((licenseId) => ({
      licenseId,
      extractedText: descriptions[licenseId] ?? "Pending extracted license text.",
    }));
}

export function createSpdxDocument(source: ComplianceInputs = inputs): SpdxDocument {
  const components = sortedComponents(source.policy.components);
  const packages: SpdxDocument["packages"] = [
    {
      SPDXID: ROOT_SPDX_ID,
      name: "cua_node",
      versionInfo: "0.1.0",
      downloadLocation: "NOASSERTION",
      filesAnalyzed: false,
      licenseConcluded: "NOASSERTION",
      licenseDeclared: "UNLICENSED",
      packageComment: `Compliance policy ${source.policy.policy_id}; release status ${source.policy.release_status}.`,
      externalRefs: [
        {
          referenceCategory: "PACKAGE-MANAGER",
          referenceType: "purl",
          referenceLocator: ROOT_BOM_REF,
        },
      ],
      attributionTexts: ["First-party Linux cua_node runtime."],
    },
  ];
  for (const component of components) {
    const hashes = componentHashes(component, source.provenance);
    const provenanceRecord = findProvenanceRecord(component, source.provenance);
    const evidenceUris = [
      provenanceRecord.evidence_record_uri,
      provenanceRecord.source_offer_uri,
    ].filter((value): value is string => value !== undefined);
    const packageRecord: SpdxDocument["packages"][number] = {
      SPDXID: componentSpdxId(component),
      name: component.name,
      versionInfo: component.version,
      downloadLocation: component.source.uri,
      filesAnalyzed: false,
      licenseConcluded: component.license.spdx_expression,
      licenseDeclared: component.license.spdx_expression,
      packageComment: `${component.disposition}; ${component.notes}`,
      externalRefs: [
        {
          referenceCategory: "PACKAGE-MANAGER",
          referenceType: "purl",
          referenceLocator: component.purl,
        },
      ],
      attributionTexts: [...component.notice_ids, ...evidenceUris].sort((left, right) =>
        left.localeCompare(right),
      ),
    };
    if (hashes.length > 0) {
      packageRecord.checksums = hashes.map((hash) => ({
        algorithm: hash.alg === "SHA-256" ? "SHA256" : "SHA512",
        checksumValue: hash.content,
      }));
    }
    if (component.license.designation !== component.license.spdx_expression) {
      packageRecord.licenseComments = `Project designation: ${component.license.designation}`;
    }
    packages.push(packageRecord);
  }

  const relationships: SpdxDocument["relationships"] = [
    {
      spdxElementId: "SPDXRef-DOCUMENT",
      relationshipType: "DESCRIBES",
      relatedSpdxElement: ROOT_SPDX_ID,
    },
  ];
  for (const component of components) {
    relationships.push({
      spdxElementId: ROOT_SPDX_ID,
      relationshipType: "DEPENDS_ON",
      relatedSpdxElement: componentSpdxId(component),
    });
  }

  return {
    spdxVersion: "SPDX-2.3",
    dataLicense: "CC0-1.0",
    SPDXID: "SPDXRef-DOCUMENT",
    name: "cua-node-linux-x64-glibc-compliance",
    documentNamespace:
      "https://codex.local/sbom/cua-node/linux-x64-glibc/compliance-v1",
    creationInfo: {
      created: source.policy.determinism.fixed_timestamp,
      creators: ["Tool: @heliasar/cua-node-compliance-scaffold-1.0.0"],
    },
    packages,
    hasExtractedLicensingInfos: extractedLicenses(source),
    relationships,
  };
}

export function createArtifacts(
  source: ComplianceInputs = inputs,
): ComplianceArtifacts {
  return {
    cyclonedx: createCycloneDxDocument(source),
    spdx: createSpdxDocument(source),
  };
}

export function renderJson(value: unknown): string {
  const rendered = JSON.stringify(value, null, 2);
  if (rendered === undefined) {
    throw new Error("Cannot render undefined JSON");
  }
  return `${rendered}\n`;
}

export async function writeArtifacts(outputDirectory: string): Promise<void> {
  await mkdir(outputDirectory, { recursive: true });
  const artifacts = createArtifacts();
  await Bun.write(
    join(outputDirectory, "sbom.cdx.json"),
    renderJson(artifacts.cyclonedx),
  );
  await Bun.write(join(outputDirectory, "sbom.spdx.json"), renderJson(artifacts.spdx));
}

if (import.meta.main) {
  const outputDirectory = dirname(new URL(import.meta.url).pathname);
  await writeArtifacts(outputDirectory);
}
