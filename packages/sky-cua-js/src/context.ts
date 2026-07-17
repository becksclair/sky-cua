import { invalidContext } from "./errors";
import type { RequestContext } from "./protocol/generated";

export type NodeReplBridge = {
  readonly requestMeta?: unknown;
  withSuspendedTimeout?<T>(fn: () => Promise<T> | T): Promise<T> | T;
  setResponseMeta?(meta: Record<string, unknown>): void;
  emitImage?(dataUrl: string): Promise<unknown> | unknown;
};

declare global {
  // The runtime injects this object into trusted Node REPL modules.
  var nodeRepl: NodeReplBridge | undefined;
}

function deepClone(value: unknown): unknown {
  if (value === undefined) {
    return undefined;
  }
  return JSON.parse(JSON.stringify(value)) as unknown;
}

function deepFreeze<T>(value: T): T {
  if (typeof value !== "object" || value === null || Object.isFrozen(value)) {
    return value;
  }
  Object.freeze(value);
  for (const child of Object.values(value as Record<string, unknown>)) {
    deepFreeze(child);
  }
  return value;
}

function requestMetaValue(): unknown {
  const value = globalThis.nodeRepl?.requestMeta;
  if (value !== undefined && value !== null) {
    return value;
  }
  const serialized = process.env.NODE_REPL_REQUEST_META;
  if (serialized === undefined || serialized.length === 0) {
    return undefined;
  }
  try {
    return JSON.parse(serialized) as unknown;
  } catch {
    return undefined;
  }
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function metadataWithTurnFields(value: unknown): Record<string, unknown> {
  const record = asRecord(value);
  if (record === undefined) {
    return {};
  }
  const header = asRecord(record["x-codex-turn-metadata"]);
  return { ...record, ...(header ?? {}) };
}

function nonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function optionalDeadlineMs(value: unknown): number | undefined {
  if (value === undefined) {
    return undefined;
  }
  if (typeof value !== "number" || !Number.isInteger(value) || value < 1 || value > 30_000) {
    throw invalidContext("deadline_ms metadata must be an integer between 1 and 30000.");
  }
  return value;
}

export function readRequestMetadata(): Readonly<Record<string, unknown>> {
  const cloned = deepClone(metadataWithTurnFields(requestMetaValue()));
  return deepFreeze((cloned ?? {}) as Record<string, unknown>);
}

export function requestContext(): RequestContext {
  const meta = readRequestMetadata();
  const sessionId = meta.session_id ?? meta.sessionId;
  const turnId = meta.turn_id ?? meta.turnId;
  if (!nonEmptyString(sessionId) || !nonEmptyString(turnId)) {
    throw invalidContext("sky-cua mutations require non-empty session_id and turn_id metadata.");
  }
  const deadlineMs = optionalDeadlineMs(meta.deadline_ms);
  return {
    session_id: sessionId,
    turn_id: turnId,
    ...(deadlineMs === undefined ? {} : { deadline_ms: deadlineMs })
  };
}

export async function withSuspendedTimeout<T>(operation: () => Promise<T>): Promise<T> {
  const suspend = globalThis.nodeRepl?.withSuspendedTimeout;
  if (typeof suspend !== "function") {
    return operation();
  }
  return await suspend(operation);
}

export function setComputerUseResponseMeta(): void {
  globalThis.nodeRepl?.setResponseMeta?.({
    "codex/toolSurface": { app: null, kind: "computerUse" }
  });
}
