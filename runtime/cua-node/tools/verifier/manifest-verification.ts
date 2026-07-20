import { record } from "./json";
import {
  RUNTIME_MANIFEST_SCHEMA_PATH,
  validateCanonicalSchema,
} from "./schema-validation";
import type { JsonRecord, VerificationCheck } from "./types";

function isResolvedHash(value: unknown, allowFixtureValues: boolean): boolean {
  return allowFixtureValues || (typeof value === "string" && !/^0+$/u.test(value));
}

function isResolvedCommit(value: unknown, allowFixtureValues: boolean): boolean {
  return allowFixtureValues || (typeof value === "string" && !/^0+$/u.test(value));
}

export function verifyManifest(
  value: unknown,
  checks: VerificationCheck[],
  expectedTarget: string,
  allowFixtureValues: boolean,
): JsonRecord | null {
  const schemaErrors = validateCanonicalSchema(value, RUNTIME_MANIFEST_SCHEMA_PATH);
  checks.push({
    id: "manifest:schema",
    status: schemaErrors.length === 0 ? "passed" : "failed",
    detail:
      schemaErrors.length === 0
        ? "manifest satisfies the canonical runtime manifest schema"
        : `canonical runtime manifest schema: ${schemaErrors.join("; ")}`,
  });
  if (
    schemaErrors.some((error) => error.startsWith("/trusted_browser_client_sha256s "))
  ) {
    checks.push({
      id: "manifest:trusted-hashes",
      status: "failed",
      detail: "canonical schema requires at least one unique lowercase SHA-256",
    });
  }
  if (schemaErrors.length > 0) return null;

  const manifest = record(value, "manifest.json");

  const source = record(manifest.source, "manifest.source");
  const producerCommitValid = isResolvedCommit(
    source.producer_commit,
    allowFixtureValues,
  );
  checks.push({
    id: "manifest:source:producer-commit",
    status: producerCommitValid ? "passed" : "failed",
    detail: producerCommitValid
      ? "sky-cua producer commit is resolved"
      : "sky-cua producer commit is unresolved",
  });
  if (source.migration_evidence !== undefined) {
    const migrationEvidence = record(
      source.migration_evidence,
      "manifest.source.migration_evidence",
    );
    const migrationCommitValid = isResolvedCommit(
      migrationEvidence.codex_desktop_commit,
      allowFixtureValues,
    );
    checks.push({
      id: "manifest:source:migration-evidence",
      status: migrationCommitValid ? "passed" : "failed",
      detail: migrationCommitValid
        ? "optional Codex migration evidence commit is resolved"
        : "Codex migration evidence commit is unresolved",
    });
  }
  const migrationInput = record(
    source.migration_input,
    "manifest.source.migration_input",
  );
  const migrationInputValid = isResolvedHash(
    migrationInput.source_tree_sha256,
    allowFixtureValues,
  );
  checks.push({
    id: "manifest:source:migration-input",
    status: migrationInputValid ? "passed" : "failed",
    detail: migrationInputValid
      ? "content-addressed migration input is resolved"
      : "migration input tree checksum is unresolved",
  });

  checks.push({
    id: "manifest:target",
    status: manifest.target === expectedTarget ? "passed" : "failed",
    detail: `expected ${expectedTarget}, got ${String(manifest.target)}`,
  });
  for (const key of ["node_sha256", "node_repl_sha256"] as const) {
    const valid = isResolvedHash(manifest[key], allowFixtureValues);
    checks.push({
      id: `manifest:${key}`,
      status: valid ? "passed" : "failed",
      detail: valid ? "resolved lowercase SHA-256" : `${key} is unresolved`,
    });
  }

  const components = record(manifest.components, "manifest.components");
  for (const key of ["host", "kernel"] as const) {
    const component = record(components[key], `manifest.components.${key}`);
    const valid = isResolvedHash(component.sha256, allowFixtureValues);
    checks.push({
      id: `manifest:component:${key}`,
      status: valid ? "passed" : "failed",
      detail: valid
        ? `${String(component.name)} ${String(component.version)}`
        : `${key} checksum is unresolved`,
    });
  }
  const sky = record(components.sky_cua, "manifest.components.sky_cua");
  const skyValid = isResolvedHash(sky.tarball_sha256, allowFixtureValues);
  checks.push({
    id: "manifest:component:sky-cua",
    status: skyValid ? "passed" : "failed",
    detail: skyValid
      ? "@heliasar/sky-cua checksum is resolved"
      : "sky-cua checksum is unresolved",
  });
  const browserUse = record(
    components.browser_use,
    "manifest.components.browser_use",
  );
  const browserUseHash = browserUse.entrypoint_sha256;
  const browserUseValid = isResolvedHash(browserUseHash, allowFixtureValues);
  const trustedBrowserHashes = manifest.trusted_browser_client_sha256s;
  const browserUseTrusted =
    Array.isArray(trustedBrowserHashes) && trustedBrowserHashes.includes(browserUseHash);
  checks.push({
    id: "manifest:component:browser-use",
    status: browserUseValid && browserUseTrusted ? "passed" : "failed",
    detail:
      browserUseValid && browserUseTrusted
        ? "@heliasar/browser-use entrypoint checksum is resolved and trusted"
        : !browserUseValid
          ? "browser-use entrypoint checksum is unresolved"
          : "browser-use entrypoint checksum is not in trusted_browser_client_sha256s",
  });
  return manifest;
}
