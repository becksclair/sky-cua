import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

export type JsonObject = Record<string, unknown>;

export type Outcome = "pass" | "fail" | "pending";

export type Evidence = {
  path: string;
  description: string;
  sha256: string;
};

export type AcceptanceArtifact = {
  schema_version: "cua-node-acceptance/v1";
  category: string;
  task_id: string;
  outcome: Outcome;
  evidence_kind: "deterministic-fake" | "external-command";
  installed_acceptance: "pending" | "observed";
  summary: string;
  checks: Array<{
    id: string;
    outcome: Outcome;
    detail: string;
  }>;
  evidence: Evidence[];
  adapter: {
    name: string;
    command_env: string;
    used_command: boolean;
  };
};

export type CategoryArtifact = {
  schema_version: "cua-node-acceptance/category-v1";
  category: string;
  outcome: Outcome;
  installed_acceptance: "pending" | "observed";
  tasks: string[];
  artifacts: string[];
  evidence_root: string;
};

export function sha256(value: Uint8Array): string {
  return createHash("sha256").update(value).digest("hex");
}

export function stableJson(value: unknown): string {
  return `${JSON.stringify(sortJson(value), null, 2)}\n`;
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => sortJson(entry));
  }
  if (value !== null && typeof value === "object") {
    const object = value as Record<string, unknown>;
    const sorted: Record<string, unknown> = {};
    for (const key of Object.keys(object).sort((a, b) => a.localeCompare(b))) {
      sorted[key] = sortJson(object[key]);
    }
    return sorted;
  }
  return value;
}

export function writeText(path: string, value: string): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, value, "utf8");
}

export function writeBytes(path: string, value: Uint8Array): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, value);
}

export function writeJson(path: string, value: unknown): void {
  writeText(path, stableJson(value));
}

export function readJson(path: string): unknown {
  return JSON.parse(readFileSync(path, "utf8")) as unknown;
}

export function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function requireString(value: unknown, label: string): string {
  if (typeof value !== "string") {
    throw new Error(`${label} must be a string`);
  }
  return value;
}

export function assertArtifact(
  value: unknown,
  label = "artifact",
): asserts value is AcceptanceArtifact {
  if (!isObject(value)) {
    throw new Error(`${label} must be an object`);
  }
  if (value.schema_version !== "cua-node-acceptance/v1") {
    throw new Error(`${label}.schema_version is invalid`);
  }
  for (const key of ["category", "task_id", "summary"] as const) {
    requireString(value[key], `${label}.${key}`);
  }
  if (
    value.outcome !== "pass" &&
    value.outcome !== "fail" &&
    value.outcome !== "pending"
  ) {
    throw new Error(`${label}.outcome is invalid`);
  }
  if (
    value.evidence_kind !== "deterministic-fake" &&
    value.evidence_kind !== "external-command"
  ) {
    throw new Error(`${label}.evidence_kind is invalid`);
  }
  if (
    value.installed_acceptance !== "pending" &&
    value.installed_acceptance !== "observed"
  ) {
    throw new Error(`${label}.installed_acceptance is invalid`);
  }
  if (
    !Array.isArray(value.checks) ||
    !Array.isArray(value.evidence) ||
    !isObject(value.adapter)
  ) {
    throw new Error(`${label} has invalid checks, evidence, or adapter`);
  }
  for (const [index, check] of value.checks.entries()) {
    if (
      !isObject(check) ||
      typeof check.id !== "string" ||
      typeof check.detail !== "string"
    ) {
      throw new Error(`${label}.checks[${index}] is invalid`);
    }
    if (
      check.outcome !== "pass" &&
      check.outcome !== "fail" &&
      check.outcome !== "pending"
    ) {
      throw new Error(`${label}.checks[${index}].outcome is invalid`);
    }
  }
  for (const [index, evidence] of value.evidence.entries()) {
    if (
      !isObject(evidence) ||
      typeof evidence.path !== "string" ||
      typeof evidence.description !== "string" ||
      typeof evidence.sha256 !== "string"
    ) {
      throw new Error(`${label}.evidence[${index}] is invalid`);
    }
    if (!/^[a-f0-9]{64}$/u.test(evidence.sha256)) {
      throw new Error(`${label}.evidence[${index}].sha256 is invalid`);
    }
  }
  if (
    typeof value.adapter.name !== "string" ||
    typeof value.adapter.command_env !== "string" ||
    typeof value.adapter.used_command !== "boolean"
  ) {
    throw new Error(`${label}.adapter is invalid`);
  }
}

export function assertCategoryArtifact(
  value: unknown,
  label = "category artifact",
): asserts value is CategoryArtifact {
  if (!isObject(value) || value.schema_version !== "cua-node-acceptance/category-v1") {
    throw new Error(`${label} schema_version is invalid`);
  }
  if (typeof value.category !== "string" || typeof value.evidence_root !== "string") {
    throw new Error(`${label} category or evidence_root is invalid`);
  }
  if (
    value.outcome !== "pass" &&
    value.outcome !== "fail" &&
    value.outcome !== "pending"
  ) {
    throw new Error(`${label}.outcome is invalid`);
  }
  if (
    value.installed_acceptance !== "pending" &&
    value.installed_acceptance !== "observed"
  ) {
    throw new Error(`${label}.installed_acceptance is invalid`);
  }
  if (
    !Array.isArray(value.tasks) ||
    !Array.isArray(value.artifacts) ||
    value.tasks.some((entry) => typeof entry !== "string") ||
    value.artifacts.some((entry) => typeof entry !== "string")
  ) {
    throw new Error(`${label} tasks or artifacts are invalid`);
  }
}
