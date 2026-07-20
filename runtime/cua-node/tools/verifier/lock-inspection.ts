import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { basename, resolve } from "node:path";
import { record } from "./json";
import {
  BLOCKED_LOCK_SCHEMA_PATH,
  NATIVE_ASSETS_LOCK_SCHEMA_PATH,
  RUNTIME_LOCK_SCHEMA_PATH,
  validateCanonicalSchema,
} from "./schema-validation";
import type { CuaNodeLockInspection, JsonRecord } from "./types";

const LOCK_DEFINITIONS = {
  "runtime-lock.json": {
    kind: "runtime",
    schemaPath: RUNTIME_LOCK_SCHEMA_PATH,
  },
  "native-assets.lock.json": {
    kind: "native-assets",
    schemaPath: NATIVE_ASSETS_LOCK_SCHEMA_PATH,
  },
} as const;

type LockDefinition = (typeof LOCK_DEFINITIONS)[keyof typeof LOCK_DEFINITIONS];

function lockDefinition(path: string): LockDefinition | null {
  const definition = LOCK_DEFINITIONS[basename(path) as keyof typeof LOCK_DEFINITIONS];
  return definition ?? null;
}

function unresolvedProductionLockReasons(lock: JsonRecord): string[] {
  const reasons: string[] = [];
  if (lock.release_ready !== true) reasons.push("release_ready must be true");
  if (Array.isArray(lock.release_blockers) && lock.release_blockers.length > 0)
    reasons.push(`release_blockers contains ${lock.release_blockers.length} item(s)`);

  const redistribution = record(lock.redistribution, "lock.redistribution");
  if (redistribution.allowed === false)
    reasons.push("redistribution.allowed must not be false");
  if (
    new Set(["blocked", "pending", "failed"]).has(
      typeof redistribution.status === "string" ? redistribution.status : "",
    )
  )
    reasons.push(`redistribution.status is ${String(redistribution.status)}`);

  const nativeAudit = record(
    lock.native_dependency_audit,
    "lock.native_dependency_audit",
  );
  if (
    new Set(["pending", "failed"]).has(
      typeof nativeAudit.status === "string" ? nativeAudit.status : "",
    )
  )
    reasons.push(`native_dependency_audit.status is ${String(nativeAudit.status)}`);
  return reasons;
}

export function inspectCuaNodeLock(path: string): CuaNodeLockInspection {
  const absolutePath = resolve(path);
  const definition = lockDefinition(absolutePath);
  const kind = definition?.kind ?? "unknown";
  let raw: string;
  try {
    raw = readFileSync(absolutePath, "utf8");
  } catch (error) {
    const detail =
      error instanceof Error ? error.message : `cannot read lock: ${absolutePath}`;
    return {
      path: absolutePath,
      kind,
      status: "failed",
      sha256: "",
      blockers: [detail],
      detail,
    };
  }
  const digest = createHash("sha256").update(raw).digest("hex");
  if (definition === null) {
    const detail = `unrecognized enforced lock filename: ${basename(absolutePath)}`;
    return {
      path: absolutePath,
      kind,
      status: "failed",
      sha256: digest,
      blockers: [detail],
      detail,
    };
  }

  let lockValue: unknown;
  try {
    lockValue = JSON.parse(raw) as unknown;
  } catch (error) {
    const detail = error instanceof Error ? error.message : "invalid lock JSON";
    return {
      path: absolutePath,
      kind,
      status: "failed",
      sha256: digest,
      blockers: [detail],
      detail,
    };
  }

  const explicitStatus =
    lockValue !== null && typeof lockValue === "object" && !Array.isArray(lockValue)
      ? (lockValue as JsonRecord).status
      : undefined;
  const schemaPath =
    explicitStatus === "blocked" ? BLOCKED_LOCK_SCHEMA_PATH : definition.schemaPath;
  const schemaErrors = validateCanonicalSchema(lockValue, schemaPath);
  if (schemaErrors.length > 0) {
    const detail = `canonical lock schema: ${schemaErrors.join("; ")}`;
    return {
      path: absolutePath,
      kind,
      status: "failed",
      sha256: digest,
      blockers: [detail],
      detail,
    };
  }

  const lock = record(lockValue, absolutePath);
  if (explicitStatus === "blocked") {
    const entries = lock.blockers as Array<JsonRecord>;
    const blockers = entries.map(
      (entry) =>
        `${String(entry.id)}: ${String(entry.approval)}: ${String(entry.reason)}`,
    );
    return {
      path: absolutePath,
      kind,
      status: "blocked",
      sha256: digest,
      blockers,
      detail: `lock is explicitly blocked by ${blockers.length} unresolved input(s)`,
    };
  }
  const unresolvedReasons = unresolvedProductionLockReasons(lock);
  if (unresolvedReasons.length > 0) {
    const detail = `production lock is unresolved: ${unresolvedReasons.join("; ")}`;
    return {
      path: absolutePath,
      kind,
      status: "blocked",
      sha256: digest,
      blockers: unresolvedReasons,
      detail,
    };
  }
  return {
    path: absolutePath,
    kind,
    status: "passed",
    sha256: digest,
    blockers: [],
    detail: "resolved lock satisfies its canonical fail-closed schema",
  };
}
