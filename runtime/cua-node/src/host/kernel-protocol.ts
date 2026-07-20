import type { PersistentBinding } from "./cell-analysis.ts";

export const KERNEL_PROTOCOL_VERSION = "cua-kernel-control-v2" as const;

export type KernelRequest =
  | {
      version: typeof KERNEL_PROTOCOL_VERSION;
      type: "exec";
      id: string;
      code: string;
      bindings: PersistentBinding[];
      request_meta: Record<string, unknown> | null;
    }
  | {
      version: typeof KERNEL_PROTOCOL_VERSION;
      type: "cancel" | "shutdown";
      exec_id?: string;
    };

export type KernelMessage = {
  version?: string;
  type: string;
  id?: string;
  exec_id?: string;
  ok?: boolean;
  output?: string;
  error?: string;
  images?: string[];
  response_meta?: Record<string, unknown> | null;
  token?: string;
  generation?: string;
  op?: string;
  native_op?: string;
  connection_id?: string;
  path?: string;
  data_base64?: string;
  image_url?: string;
  result?: unknown;
  added?: boolean;
  request?: Record<string, unknown>;
  input?: string;
  init?: Record<string, unknown>;
  config_op?: string;
};

export interface KernelExecResult {
  ok: boolean;
  output: string;
  error: string | null;
  images: string[];
  responseMeta: Record<string, unknown> | null;
}

export function encodeKernelMessage(
  message: KernelRequest | Record<string, unknown>,
): string {
  return `${JSON.stringify(message)}\n`;
}

export function parseKernelMessage(value: unknown): KernelMessage {
  const parsed: unknown = typeof value === "string" ? JSON.parse(value) : value;
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error("kernel control message must be an object");
  }
  const message = parsed as Record<string, unknown>;
  if (message.version !== KERNEL_PROTOCOL_VERSION) {
    throw new Error("unsupported kernel control protocol version");
  }
  if (typeof message.type !== "string") {
    throw new Error("kernel control message type is required");
  }
  return message as KernelMessage;
}

export function parseExecResult(message: KernelMessage): KernelExecResult {
  if (message.type !== "exec_result" || typeof message.id !== "string") {
    throw new Error("invalid kernel execution result");
  }
  const responseMeta = parseJsonObject(message.response_meta, "response metadata");
  return {
    ok: message.ok === true,
    output: typeof message.output === "string" ? message.output : "",
    error: typeof message.error === "string" ? message.error : null,
    images: Array.isArray(message.images)
      ? message.images.filter((image): image is string => typeof image === "string")
      : [],
    responseMeta,
  };
}

function parseJsonObject(
  value: unknown,
  label: string,
): Record<string, unknown> | null {
  if (value === null || value === undefined) return null;
  if (!isJsonValue(value)) throw new Error(`${label} must be JSON-safe`);
  if (typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be a JSON object`);
  }
  return value as Record<string, unknown>;
}

function isJsonValue(value: unknown, seen = new WeakSet<object>()): boolean {
  if (value === null || typeof value === "string" || typeof value === "boolean")
    return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (typeof value !== "object") return false;
  if (seen.has(value)) return false;
  seen.add(value);
  let valid = true;
  if (Array.isArray(value)) valid = value.every((item) => isJsonValue(item, seen));
  else if (
    Object.getPrototypeOf(value) !== Object.prototype &&
    Object.getPrototypeOf(value) !== null
  )
    valid = false;
  else valid = Object.values(value).every((item) => isJsonValue(item, seen));
  seen.delete(value);
  return valid;
}
