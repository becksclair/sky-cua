import { randomUUID } from "node:crypto";
import { Buffer } from "node:buffer";
import { lstat, readFile, readlink, realpath, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { stringify as stringifyToml } from "smol-toml";
import runtimeManifestSchema from "../../contracts/runtime-manifest.schema.json";
import {
  KernelChild,
  type FetchBridgeResult,
  type KernelChildOptions,
} from "./kernel-child.ts";
import { createKernelId } from "./kernel-child.ts";
import type { KernelExecResult } from "./kernel-protocol.ts";
import {
  discoverRuntimeAssets,
  resolveDefaultBrowserExecutable,
  type RuntimeAssetMetadata,
  type ValidatedRuntimeManifestRecord,
} from "./runtime-asset-discovery.ts";

export const DEFAULT_TIMEOUT_MS = 30_000;
export const TIMEOUT_ERROR = "js execution timed out; kernel reset, rerun your request";
export const RESET_ERROR = "js execution reset";
export const CANCEL_ERROR = "js execution cancelled; kernel reset";
export const MAX_CANCELLATION_TOMBSTONES = 1_024;

type AjvError = { instancePath?: string; message?: string };
type AjvValidator = ((value: unknown) => boolean) & {
  errors?: AjvError[] | null;
};
type AjvInstance = {
  compile: (schema: Record<string, unknown>) => AjvValidator;
};
type AjvConstructor = new (options: {
  allErrors: boolean;
  strict: boolean;
}) => AjvInstance;

const Ajv2020 = require("ajv/dist/2020").default as AjvConstructor;
const validateRuntimeManifest = new Ajv2020({
  allErrors: true,
  strict: true,
}).compile(runtimeManifestSchema as unknown as Record<string, unknown>);

export type RuntimeRequestId = string | number;

export interface RuntimeManagerOptions {
  nodePath?: string;
  runtimeRoot?: string;
  runtimeMetadata?: RuntimeAssetMetadata | null;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
  allowHostNode?: boolean;
  defaultTimeoutMs?: number;
  onStderr?: (text: string) => void;
  onElicitation?: (
    request: Record<string, unknown>,
    signal: AbortSignal,
  ) => Promise<unknown>;
}

export interface RuntimeExecOptions {
  timeoutMs?: number;
  requestMeta: Record<string, unknown> | null;
  requestId: RuntimeRequestId;
}

export interface RuntimeExecResult extends KernelExecResult {
  execId: string;
}

interface ActiveExecution {
  id: string;
  requestId: RuntimeRequestId;
  timer: ReturnType<typeof setTimeout> | null;
  deadline: number;
  remainingMs: number;
  suspended: number;
  settled: boolean;
  timeoutPending: boolean;
  resolve: (result: RuntimeExecResult) => void;
  reject: (error: Error) => void;
}

export function resolveBundledNodePath(
  explicitPath?: string,
  allowHostNode = false,
): string {
  const configured = explicitPath ?? process.env.NODE_REPL_NODE_PATH;
  if (configured !== undefined && configured.length > 0) {
    if (!isAbsolute(configured))
      throw new Error("NODE_REPL_NODE_PATH must be absolute");
    return configured;
  }
  if (allowHostNode) return process.execPath;
  throw new Error(
    "bundled Node runtime is unavailable; set NODE_REPL_NODE_PATH to an absolute path",
  );
}

export function normalizeModuleDir(path: string, cwd = process.cwd()): string {
  if (!isAbsolute(path)) throw new Error("node module directory path must be absolute");
  const normalized = resolve(cwd, path);
  return normalized.endsWith("/node_modules")
    ? normalized.slice(0, -"/node_modules".length)
    : normalized;
}

export function normalizeModuleRoots(
  value: string | undefined,
  cwd = process.cwd(),
): string[] {
  const delimiter = process.platform === "win32" ? ";" : ":";
  const result: string[] = [];
  for (const entry of (value ?? "").split(delimiter)) {
    const trimmed = entry.trim();
    if (trimmed.length === 0) continue;
    const absolute = resolve(cwd, trimmed);
    const root = absolute.endsWith("/node_modules")
      ? absolute.slice(0, -"/node_modules".length)
      : absolute;
    if (!result.includes(root)) result.push(root);
  }
  return result;
}

export class RuntimeManager {
  private readonly options: Required<
    Pick<RuntimeManagerOptions, "cwd" | "defaultTimeoutMs">
  > &
    RuntimeManagerOptions;
  private readonly moduleDirs: string[];
  private readonly moduleDirSet: Set<string>;
  private child: KernelChild | null = null;
  private active: ActiveExecution | null = null;
  private termination: Promise<void> | null = null;
  private startup: Promise<void> | null = null;
  private startupGeneration: number | null = null;
  private runtimeMetadata: RuntimeAssetMetadata | null | undefined;
  private generation = 0;
  private readonly cancellationTombstones = new Set<string>();
  private readonly completedRequestIds = new Set<string>();
  private shuttingDown = false;

  public constructor(options: RuntimeManagerOptions = {}) {
    const cwd = options.cwd ?? process.cwd();
    this.options = {
      ...options,
      cwd,
      defaultTimeoutMs: options.defaultTimeoutMs ?? DEFAULT_TIMEOUT_MS,
      allowHostNode: options.allowHostNode ?? false,
    };
    this.moduleDirs = normalizeModuleRoots(
      options.env?.NODE_REPL_NODE_MODULE_DIRS ?? process.env.NODE_REPL_NODE_MODULE_DIRS,
      cwd,
    );
    this.moduleDirSet = new Set(this.moduleDirs);
  }

  public get kernelPid(): number | undefined {
    return this.child?.pid;
  }

  public get moduleSearchRoots(): readonly string[] {
    return this.moduleDirs;
  }

  public async execute(
    code: string,
    options: RuntimeExecOptions,
  ): Promise<RuntimeExecResult> {
    if (this.shuttingDown)
      throw new Error(
        "js execution unavailable because the runtime manager is shutting down",
      );
    if (this.active !== null) throw new Error("another js execution is already active");
    const timeoutMs = options.timeoutMs ?? this.options.defaultTimeoutMs;
    validateTimeout(timeoutMs);
    this.completedRequestIds.delete(requestIdKey(options.requestId));
    const execId = createKernelId();
    const promise = new Promise<RuntimeExecResult>((resolvePromise, rejectPromise) => {
      const active: ActiveExecution = {
        id: execId,
        requestId: options.requestId,
        timer: null,
        deadline: Date.now() + timeoutMs,
        remainingMs: timeoutMs,
        suspended: 0,
        settled: false,
        timeoutPending: false,
        resolve: resolvePromise,
        reject: rejectPromise,
      };
      this.active = active;
      if (this.cancellationTombstones.delete(requestIdKey(options.requestId))) {
        active.settled = true;
        this.active = null;
        rejectPromise(new Error(CANCEL_ERROR));
        return;
      }
      this.startTimer(active);
      void this.beginExecution(active, code, options.requestMeta);
    });
    return promise;
  }

  public cancel(requestId: RuntimeRequestId): void {
    if (this.shuttingDown) return;
    const active = this.active;
    if (active === null) {
      const key = requestIdKey(requestId);
      if (!this.completedRequestIds.has(key)) this.addCancellationTombstone(key);
      return;
    }
    if (
      !sameRequestId(active.requestId, requestId) ||
      (active.settled && !active.timeoutPending)
    )
      return;
    active.settled = true;
    active.timeoutPending = false;
    this.clearTimer(active);
    this.child?.cancel(active.id);
    const oldChild = this.child;
    this.child = null;
    if (oldChild !== null) this.startTermination(oldChild, CANCEL_ERROR);
    this.active = null;
    this.rememberCompletedRequestId(active.requestId);
    active.reject(new Error(CANCEL_ERROR));
  }

  public cancelActive(): void {
    const active = this.active;
    if (active !== null) this.cancel(active.requestId);
  }

  public async reset(): Promise<true> {
    if (this.shuttingDown)
      throw new Error(
        "js reset unavailable because the runtime manager is shutting down",
      );
    this.generation += 1;
    const active = this.active;
    if (active !== null && !active.settled) {
      active.settled = true;
      this.clearTimer(active);
      active.reject(new Error(RESET_ERROR));
      this.active = null;
    }
    await this.restart(RESET_ERROR);
    await this.ensureChild();
    return true;
  }

  public async addNodeModuleDir(path: string): Promise<boolean> {
    if (this.shuttingDown)
      throw new Error(
        "js add node module directory unavailable because the runtime manager is shutting down",
      );
    const normalized = normalizeModuleDir(path, this.options.cwd);
    if (this.moduleDirSet.has(normalized)) return false;
    this.moduleDirs.push(normalized);
    this.moduleDirSet.add(normalized);
    const child = await this.ensureChild();
    if (this.shuttingDown || this.child !== child) {
      throw new Error(
        "js add node module directory unavailable because the kernel generation changed",
      );
    }
    return child.addNodeModuleDir(normalized);
  }

  public async close(): Promise<void> {
    this.shuttingDown = true;
    this.generation += 1;
    this.cancellationTombstones.clear();
    this.completedRequestIds.clear();
    const active = this.active;
    if (active !== null && !active.settled) {
      active.settled = true;
      this.clearTimer(active);
      active.reject(
        new Error(
          "js execution unavailable because the runtime manager is shutting down",
        ),
      );
      this.active = null;
    }
    const child = this.child;
    this.child = null;
    if (child !== null)
      this.startTermination(child, "node_repl runtime manager shutdown");
    await this.startup?.catch(() => undefined);
    this.startup = null;
    await this.awaitTermination();
  }

  private addCancellationTombstone(requestId: string): void {
    this.cancellationTombstones.delete(requestId);
    this.cancellationTombstones.add(requestId);
    while (this.cancellationTombstones.size > MAX_CANCELLATION_TOMBSTONES) {
      const oldest = this.cancellationTombstones.values().next().value;
      if (typeof oldest !== "string") break;
      this.cancellationTombstones.delete(oldest);
    }
  }

  private rememberCompletedRequestId(requestId: RuntimeRequestId): void {
    const key = requestIdKey(requestId);
    this.completedRequestIds.delete(key);
    this.completedRequestIds.add(key);
    while (this.completedRequestIds.size > MAX_CANCELLATION_TOMBSTONES) {
      const oldest = this.completedRequestIds.values().next().value;
      if (typeof oldest !== "string") break;
      this.completedRequestIds.delete(oldest);
    }
  }

  private async ensureChild(shouldStart?: () => boolean): Promise<KernelChild> {
    while (true) {
      await this.awaitTermination();
      if (shouldStart !== undefined && !shouldStart()) {
        throw new Error("kernel execution admission was cancelled");
      }
      if (this.child !== null) return this.child;
      if (this.shuttingDown) throw new Error("runtime manager is shutting down");
      const generation = this.generation;
      if (this.startup === null) {
        const startup = this.startChild(generation);
        this.startup = startup;
        this.startupGeneration = generation;
        const clearStartup = (): void => {
          if (this.startup === startup) {
            this.startup = null;
            this.startupGeneration = null;
          }
        };
        void startup.then(clearStartup, clearStartup);
      }
      const startup = this.startup;
      const startupGeneration = this.startupGeneration;
      try {
        await startup;
      } catch (error) {
        if (startupGeneration === this.generation) throw error;
      }
    }
  }

  private async startChild(generation: number): Promise<void> {
    const nodePath = resolveBundledNodePath(
      this.options.nodePath,
      this.options.allowHostNode,
    );
    const runtimeMetadata = await this.resolveRuntimeMetadata(nodePath);
    if (this.shuttingDown || generation !== this.generation) {
      throw new Error("runtime manager generation changed during kernel startup");
    }
    this.seedImplicitRuntimeModuleRoot(runtimeMetadata);
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      ...this.options.env,
      NODE_REPL_NODE_MODULE_DIRS: this.moduleDirs.join(
        process.platform === "win32" ? ";" : ":",
      ),
    };
    if (runtimeMetadata !== null)
      env.NODE_REPL_RUNTIME_METADATA = JSON.stringify(runtimeMetadata);
    else delete env.NODE_REPL_RUNTIME_METADATA;
    const childOptions: KernelChildOptions = {
      nodePath,
      cwd: this.options.cwd,
      env,
      onSuspension: (kind, execId) => this.handleSuspension(kind, execId),
      onAuthenticatedFetch: (input, init, signal) =>
        this.authenticatedFetch(input, init, signal),
      onConfig: (operation, payload, signal) =>
        this.configBridge(operation, payload, signal),
      ...(this.options.onStderr === undefined
        ? {}
        : { onStderr: this.options.onStderr }),
      ...(this.options.onElicitation === undefined
        ? {}
        : { onElicitation: this.options.onElicitation }),
    };
    const child = new KernelChild(childOptions);
    this.child = child;
    try {
      await child.start();
      if (this.shuttingDown || generation !== this.generation || this.child !== child) {
        await child.terminate("runtime manager shutdown during kernel startup");
        throw new Error("runtime manager generation changed during kernel startup");
      }
    } catch (error) {
      if (this.child === child) this.child = null;
      await child.terminate("kernel startup failed").catch(() => undefined);
      throw error;
    }
  }

  private async resolveRuntimeMetadata(
    nodePath: string,
  ): Promise<RuntimeAssetMetadata | null> {
    if (this.runtimeMetadata !== undefined) return this.runtimeMetadata;
    if (this.options.runtimeMetadata !== undefined) {
      this.runtimeMetadata = this.options.runtimeMetadata;
      return this.runtimeMetadata;
    }
    const explicitRoot = this.options.runtimeRoot;
    const runtimeRoot = explicitRoot ?? dirname(dirname(nodePath));
    let parsed: unknown;
    try {
      parsed = JSON.parse(await readFile(join(runtimeRoot, "manifest.json"), "utf8"));
    } catch (error) {
      if (
        explicitRoot === undefined &&
        this.usesHostNodeFallback() &&
        isErrnoException(error) &&
        error.code === "ENOENT"
      ) {
        this.runtimeMetadata = null;
        return null;
      }
      throw error;
    }
    if (!validateRuntimeManifest(parsed)) {
      const schemaErrors = (validateRuntimeManifest.errors ?? []).map((error) => {
        const location = error.instancePath?.length ? error.instancePath : "$";
        return `${location} ${error.message ?? "does not satisfy canonical schema"}`;
      });
      throw new Error(
        `runtime manifest does not satisfy canonical schema: ${join(runtimeRoot, "manifest.json")}: ${schemaErrors.join("; ")}`,
      );
    }
    const env = { ...process.env, ...this.options.env };
    this.runtimeMetadata = await discoverRuntimeAssets({
      runtimeRoot,
      manifest: parsed as ValidatedRuntimeManifestRecord,
      resolveBrowserExecutable: () => resolveDefaultBrowserExecutable(env),
    });
    return this.runtimeMetadata;
  }

  private seedImplicitRuntimeModuleRoot(
    runtimeMetadata: RuntimeAssetMetadata | null,
  ): void {
    if (this.options.runtimeMetadata !== undefined || runtimeMetadata === null)
      return;
    const normalized = normalizeModuleDir(
      runtimeMetadata.modules.root,
      this.options.cwd,
    );
    if (this.moduleDirSet.has(normalized)) return;
    this.moduleDirs.unshift(normalized);
    this.moduleDirSet.add(normalized);
  }

  private usesHostNodeFallback(): boolean {
    return this.options.allowHostNode === true;
  }

  private async beginExecution(
    active: ActiveExecution,
    code: string,
    requestMeta: Record<string, unknown> | null,
  ): Promise<void> {
    let child: KernelChild | null = null;
    try {
      child = await this.ensureChild(
        () => this.active === active && !active.settled && !this.shuttingDown,
      );
      if (
        this.active !== active ||
        active.settled ||
        this.shuttingDown ||
        this.child !== child
      )
        return;
      const result = await child.execute(active.id, code, requestMeta);
      this.finishExecution(active, { ...result, execId: active.id });
    } catch (error) {
      if (active.settled) return;
      let terminationError: Error | undefined;
      if (child !== null && this.child === child) {
        this.child = null;
        this.startTermination(child, "kernel execution failed");
        try {
          await this.awaitTermination();
        } catch (terminationFailure) {
          terminationError = toError(terminationFailure);
        }
      }
      this.finishExecution(active, undefined, terminationError ?? toError(error));
    }
  }

  private async restart(reason: string): Promise<void> {
    await this.awaitTermination();
    const child = this.child;
    this.child = null;
    if (child === null) return;
    this.startTermination(child, reason);
    await this.awaitTermination();
  }

  private startTimer(active: ActiveExecution): void {
    active.deadline = Date.now() + active.remainingMs;
    active.timer = setTimeout(() => void this.timeout(active), active.remainingMs);
  }

  private clearTimer(active: ActiveExecution): void {
    if (active.timer !== null) clearTimeout(active.timer);
    active.timer = null;
  }

  private handleSuspension(kind: "suspend" | "resume", execId: string): void {
    const active = this.active;
    if (active === null || active.id !== execId || active.settled) return;
    if (kind === "suspend") {
      if (active.suspended === 0) {
        active.remainingMs = Math.max(1, active.deadline - Date.now());
        this.clearTimer(active);
      }
      active.suspended += 1;
      return;
    }
    if (active.suspended === 0) return;
    active.suspended -= 1;
    if (active.suspended === 0) this.startTimer(active);
  }

  private async timeout(active: ActiveExecution): Promise<void> {
    if (this.active !== active || active.settled) return;
    active.settled = true;
    active.timeoutPending = true;
    this.clearTimer(active);
    this.child?.cancel(active.id);
    const child = this.child;
    this.child = null;
    if (child !== null) this.startTermination(child, TIMEOUT_ERROR);
    let terminationError: Error | null = null;
    try {
      await this.awaitTermination();
    } catch (error) {
      terminationError = toError(error);
    }
    if (this.active !== active || !active.timeoutPending) return;
    active.timeoutPending = false;
    this.active = null;
    this.rememberCompletedRequestId(active.requestId);
    active.reject(terminationError ?? new Error(TIMEOUT_ERROR));
  }

  private finishExecution(
    active: ActiveExecution,
    result?: RuntimeExecResult,
    error?: Error,
  ): void {
    if (this.active !== active || active.settled) return;
    active.settled = true;
    this.clearTimer(active);
    this.active = null;
    this.rememberCompletedRequestId(active.requestId);
    if (error !== undefined) active.reject(error);
    else if (result !== undefined) active.resolve(result);
    else active.reject(new Error("node_repl execution ended without a result"));
  }

  private async authenticatedFetch(
    input: string,
    init: Record<string, unknown> | undefined,
    signal: AbortSignal,
  ): Promise<FetchBridgeResult> {
    const requestInit: Record<string, unknown> = { ...init, signal };
    if (
      typeof requestInit?.body === "string" &&
      requestInit.body.startsWith("base64:")
    ) {
      requestInit.body = Buffer.from(
        requestInit.body.slice("base64:".length),
        "base64",
      );
    }
    const response = await fetch(input, requestInit as RequestInit);
    const bytes = new Uint8Array(await response.arrayBuffer());
    const headers: Record<string, string> = {};
    response.headers.forEach((value, key) => {
      headers[key] = value;
    });
    return {
      status: response.status,
      statusText: response.statusText,
      headers,
      body_base64: Buffer.from(bytes).toString("base64"),
    };
  }

  private startTermination(child: KernelChild, reason: string): Promise<void> {
    const termination = child.terminate(reason);
    this.termination = termination;
    return termination;
  }

  private async awaitTermination(): Promise<void> {
    const termination = this.termination;
    if (termination === null) return;
    await termination;
    if (this.termination === termination) this.termination = null;
  }

  private async configBridge(
    operation: string,
    payload: Record<string, unknown>,
    signal: AbortSignal,
  ): Promise<unknown> {
    if (operation === "readToml" && typeof payload.path === "string") {
      return readFile(payload.path, { encoding: "utf8", signal });
    }
    if (
      operation === "writeToml" &&
      typeof payload.path === "string" &&
      isPlainObject(payload.value)
    ) {
      if (await isProtectedConfigDestination(payload.path)) {
        throw new Error("config.writeToml cannot write ~/.codex/config.toml");
      }
      signal.throwIfAborted();
      const serialized = stringifyToml(payload.value);
      await writeFile(
        payload.path,
        serialized.endsWith("\n") ? serialized : `${serialized}\n`,
        { encoding: "utf8", signal },
      );
      return true;
    }
    if (operation === "read") return {};
    if (operation === "readRequirements") return {};
    if (operation === "writeValue" && isPlainObject(payload.request)) return true;
    if (
      operation === "batchWrite" &&
      isPlainObject(payload.request) &&
      Object.keys(payload.request).length > 0
    )
      return true;
    throw new Error(`unsupported config operation: ${operation}`);
  }
}

export function validateTimeout(value: number): void {
  if (!Number.isInteger(value) || value < 1)
    throw new Error("timeout_ms must be a positive integer");
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function toError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

function isErrnoException(error: unknown): error is NodeJS.ErrnoException {
  return error instanceof Error && "code" in error;
}

export function makeRequestMeta(value: unknown): Record<string, unknown> | null {
  if (value === null || value === undefined) return null;
  if (!isPlainObject(value)) throw new Error("request _meta must be a plain object");
  return structuredClone(value);
}

export function makeResponseMeta(value: unknown): Record<string, unknown> | null {
  if (value === null || value === undefined) return null;
  if (!isPlainObject(value))
    throw new Error("response metadata must be a plain object");
  return cloneJsonObject(value, "response metadata");
}

export function createClientRequestId(): string {
  return `node-repl-client-${randomUUID()}`;
}

function requestIdKey(value: RuntimeRequestId): string {
  return `${typeof value}:${String(value)}`;
}

function sameRequestId(left: RuntimeRequestId, right: RuntimeRequestId): boolean {
  return typeof left === typeof right && left === right;
}

function cloneJsonObject(
  value: Record<string, unknown>,
  label: string,
): Record<string, unknown> {
  const cloned = cloneJsonValue(value, label, new WeakSet<object>());
  if (!isPlainObject(cloned)) throw new Error(`${label} must be a plain object`);
  return cloned;
}

function cloneJsonValue(
  value: unknown,
  label: string,
  ancestors: WeakSet<object>,
): unknown {
  if (value === null || typeof value === "string" || typeof value === "boolean")
    return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error(`${label} must be JSON-safe`);
    return value;
  }
  if (typeof value !== "object") throw new Error(`${label} must be JSON-safe`);
  if (ancestors.has(value)) throw new Error(`${label} must not contain cycles`);
  ancestors.add(value);
  let result: unknown;
  if (Array.isArray(value)) {
    result = value.map((item) => cloneJsonValue(item, label, ancestors));
  } else {
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null)
      throw new Error(`${label} must contain only JSON values`);
    const object: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value))
      object[key] = cloneJsonValue(item, label, ancestors);
    result = object;
  }
  ancestors.delete(value);
  return result;
}

async function isProtectedConfigDestination(path: string): Promise<boolean> {
  const protectedPath = resolve(homedir(), ".codex", "config.toml");
  const requestedPath = resolve(path);
  if (requestedPath === protectedPath) return true;
  const protectedRealPath = await canonicalPath(protectedPath);
  const requestedRealPath = await canonicalPath(requestedPath);
  if (requestedRealPath === protectedRealPath) return true;
  try {
    const stat = await lstat(requestedPath);
    if (!stat.isSymbolicLink()) return false;
    const target = await readlink(requestedPath);
    const resolvedTarget = resolve(dirname(requestedPath), target);
    return (
      resolvedTarget === protectedPath ||
      (await canonicalPath(resolvedTarget)) === protectedRealPath
    );
  } catch {
    return false;
  }
}

async function canonicalPath(path: string): Promise<string> {
  try {
    return await realpath(path);
  } catch {
    try {
      return resolve(await realpath(dirname(path)), basename(path));
    } catch {
      return resolve(path);
    }
  }
}
