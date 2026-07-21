import { readdir } from "node:fs/promises";
import { endianness } from "node:os";
import type { BrowserCommand, CommandEnvelope } from "./commands.ts";
import { callerContext, type BrowserGlobals, type NativePipeConnection } from "./globals.ts";
import { executeBrowserCommand } from "./wire-runtime.ts";

const MAX_FRAME_BYTES = 100 * 1024 * 1024;
const GET_INFO_TIMEOUT_MS = 5_000;
const FRAME_LITTLE_ENDIAN = endianness() === "LE";
const IAB_SOCKET_DIRECTORY = "/tmp/codex-browser-use";
const IAB_SOCKET_NAME = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\.sock$/u;
const EXTENSION_SOCKET_NAME = /^extension-[a-z0-9._-]+\.sock$/iu;

export type BrowserTransport = "extension_native_host" | "host_provided_iab";

export type BrowserInfo = {
  id: string;
  type: "iab" | "extension" | "cdp";
  transport: BrowserTransport;
  name: string;
  capabilities: {
    browser?: Array<{ id: string; description: string }>;
    tab?: Array<{ id: string; description: string }>;
  };
  apiSupportOverrides?: Record<string, boolean>;
  metadata?: Record<string, string>;
};

type BrowserCandidate = {
  path: string;
  transport: BrowserTransport;
};

type JsonRpcResponse = {
  id?: number;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: { code?: number; message?: string; data?: unknown };
};

function frame(value: unknown): Uint8Array {
  const payload = new TextEncoder().encode(JSON.stringify(value));
  if (payload.byteLength > MAX_FRAME_BYTES) throw new Error("BROWSER_PIPE_FRAME_TOO_LARGE");
  const result = new Uint8Array(payload.byteLength + 4);
  new DataView(result.buffer).setUint32(0, payload.byteLength, FRAME_LITTLE_ENDIAN);
  result.set(payload, 4);
  return result;
}

class JsonRpcConnection {
  private buffer: Uint8Array<ArrayBufferLike> = new Uint8Array(4_096);
  private bufferedBytes = 0;
  private nextId = 1;
  private readonly pending = new Map<number, {
    resolve: (value: unknown) => void;
    reject: (error: Error) => void;
  }>();
  private readonly notificationListeners = new Map<string, Set<(params: unknown) => void>>();

  private constructor(private readonly connection: NativePipeConnection) {
    connection.on("data", (chunk) => this.onData(new Uint8Array(chunk)));
    connection.on("error", (error) => this.failAll(error));
    connection.on("close", () => this.failAll(new Error("BROWSER_PIPE_EARLY_CLOSE")));
  }

  static async connect(globals: BrowserGlobals, path: string): Promise<JsonRpcConnection> {
    const createConnection = globals.nodeRepl?.nativePipe?.createConnection;
    if (typeof createConnection !== "function") {
      throw new Error("Trusted Browser native-pipe access is unavailable");
    }
    return new JsonRpcConnection(await createConnection(path));
  }

  request(method: string, params: Record<string, unknown>): Promise<unknown> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      try {
        this.connection.write(frame({ jsonrpc: "2.0", id, method, params }));
      } catch (error) {
        this.pending.delete(id);
        reject(asError(error));
      }
    });
  }

  close(): void {
    this.connection.end();
    this.failAll(new Error("Browser connection closed"));
  }

  onNotification(method: string, listener: (params: unknown) => void): () => void {
    let listeners = this.notificationListeners.get(method);
    if (listeners === undefined) {
      listeners = new Set();
      this.notificationListeners.set(method, listeners);
    }
    listeners.add(listener);
    return () => {
      listeners?.delete(listener);
      if (listeners?.size === 0) this.notificationListeners.delete(method);
    };
  }

  private onData(chunk: Uint8Array): void {
    const requiredBytes = this.bufferedBytes + chunk.byteLength;
    if (requiredBytes > MAX_FRAME_BYTES + 4) {
      this.failAll(new Error("BROWSER_PIPE_FRAME_TOO_LARGE"));
      return;
    }
    if (requiredBytes > this.buffer.byteLength) {
      const maximumCapacity = MAX_FRAME_BYTES + 4;
      let capacity = this.buffer.byteLength;
      while (capacity < requiredBytes) capacity = Math.min(maximumCapacity, capacity * 2);
      const grown = new Uint8Array(capacity);
      grown.set(this.buffer.subarray(0, this.bufferedBytes));
      this.buffer = grown;
    }
    this.buffer.set(chunk, this.bufferedBytes);
    this.bufferedBytes = requiredBytes;

    let offset = 0;
    while (this.bufferedBytes - offset >= 4) {
      const size = new DataView(this.buffer.buffer, this.buffer.byteOffset + offset, 4)
        .getUint32(0, FRAME_LITTLE_ENDIAN);
      if (size > MAX_FRAME_BYTES) {
        this.failAll(new Error("BROWSER_PIPE_FRAME_TOO_LARGE"));
        return;
      }
      if (this.bufferedBytes - offset < size + 4) break;
      const payload = this.buffer.slice(offset + 4, offset + size + 4);
      offset += size + 4;
      this.onMessage(JSON.parse(new TextDecoder().decode(payload)) as JsonRpcResponse);
    }
    if (offset > 0) {
      this.buffer.copyWithin(0, offset, this.bufferedBytes);
      this.bufferedBytes -= offset;
    }
  }

  private onMessage(message: JsonRpcResponse): void {
    if (typeof message.id !== "number") {
      if (typeof message.method === "string") {
        for (const listener of this.notificationListeners.get(message.method) ?? []) {
          listener(message.params);
        }
      }
      return;
    }
    if (message.method === "ping") {
      this.connection.write(frame({ jsonrpc: "2.0", id: message.id, result: "pong" }));
      return;
    }
    if (typeof message.method === "string") {
      this.connection.write(frame({
        jsonrpc: "2.0",
        id: message.id,
        error: { code: -32601, message: `No handler registered for method: ${message.method}` },
      }));
      return;
    }
    const pending = this.pending.get(message.id);
    if (pending === undefined) return;
    this.pending.delete(message.id);
    if (message.error !== undefined) {
      pending.reject(new Error(message.error.message ?? `Browser RPC error ${String(message.error.code)}`));
    } else {
      pending.resolve(message.result);
    }
  }

  private failAll(reason: unknown): void {
    const error = asError(reason);
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
  }
}

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number, message: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error(message)), timeoutMs);
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

function validSocketPath(path: string): boolean {
  return path.startsWith("/") && !path.includes("\0");
}

function requestSessionId(globals: BrowserGlobals): string | undefined {
  const meta = globals.nodeRepl?.requestMeta;
  const direct = meta?.session_id;
  if (typeof direct === "string" && direct !== "") return direct;
  const nested = meta?.["x-codex-turn-metadata"];
  if (nested !== null && typeof nested === "object") {
    const sessionId = (nested as Record<string, unknown>).session_id;
    if (typeof sessionId === "string" && sessionId !== "") return sessionId;
  }
  return undefined;
}

async function discoverBrowserCandidates(globals: BrowserGlobals): Promise<BrowserCandidate[]> {
  const extensionOnly = callerContext(globals).caller_provenance === "openclaw";
  const explicit = globals.nodeRepl?.env?.SKY_CUA_CODEX_BROWSER_SOCKET_PATH?.trim();
  if (explicit !== undefined && explicit !== "" && !validSocketPath(explicit)) {
    throw new Error("SKY_CUA_CODEX_BROWSER_SOCKET_PATH must be an absolute Unix socket path");
  }
  const candidates: BrowserCandidate[] = [];
  let entries: string[] = [];
  try {
    const injectedListDirectory = globals.nodeRepl?.nativePipe?.listDirectory;
    entries = typeof injectedListDirectory === "function"
      ? await injectedListDirectory(IAB_SOCKET_DIRECTORY)
      : (await readdir(IAB_SOCKET_DIRECTORY, { withFileTypes: true }))
        .filter((entry) => entry.isSocket())
        .map((entry) => entry.name);
  } catch {
    // The shared socket directory may not exist yet. An explicit extension
    // candidate can still be usable, so discovery failures stay local.
  }
  const uniqueEntries = [...new Set(entries)];
  for (const name of uniqueEntries.sort((left, right) => left.localeCompare(right))) {
    if (
      !extensionOnly
      && requestSessionId(globals) !== undefined
      && IAB_SOCKET_NAME.test(name)
    ) {
      candidates.push({
        path: `${IAB_SOCKET_DIRECTORY}/${name}`,
        transport: "host_provided_iab",
      });
    }
  }
  if (explicit === undefined || explicit === "") {
    for (const name of uniqueEntries
      .filter((entry) => EXTENSION_SOCKET_NAME.test(entry))
      .sort((left, right) => right.localeCompare(left))) {
      candidates.push({
        path: `${IAB_SOCKET_DIRECTORY}/${name}`,
        transport: "extension_native_host",
      });
    }
  }
  if (explicit !== undefined && explicit !== "") {
    candidates.push({ path: explicit, transport: "extension_native_host" });
  }
  if (candidates.length === 0 && (explicit === undefined || explicit === "")) {
    throw new Error(
      "No external extension or task-scoped Browser socket is available",
    );
  }
  return candidates;
}

export class BrowserBackend {
  private constructor(
    readonly path: string,
    readonly info: BrowserInfo,
    private readonly globals: BrowserGlobals,
    private readonly rpc: JsonRpcConnection,
  ) {}

  static async connect(
    globals: BrowserGlobals,
    path: string,
    transport: BrowserTransport,
    signal?: AbortSignal,
  ): Promise<BrowserBackend> {
    const candidate = { path, transport };
    const rpc = await JsonRpcConnection.connect(globals, path);
    const abort = () => rpc.close();
    try {
      if (signal?.aborted === true) throw new Error("Browser probe cancelled");
      signal?.addEventListener("abort", abort, { once: true });
      const context = callerContext(globals);
      const raw = await withTimeout(
        rpc.request("getInfo", { ...context, _meta: context, request_meta: context }),
        GET_INFO_TIMEOUT_MS,
        "Browser getInfo probe timed out",
      );
      const info = normalizeBrowserInfo(raw, candidate, requestSessionId(globals));
      signal?.removeEventListener("abort", abort);
      return new BrowserBackend(path, info, globals, rpc);
    } catch (error) {
      signal?.removeEventListener("abort", abort);
      rpc.close();
      throw error;
    }
  }

  async execute(type: BrowserCommand, params: Record<string, unknown> = {}): Promise<unknown> {
    const command: CommandEnvelope = { type, browser_id: this.info.id, ...params };
    const requestMeta = callerContext(this.globals);
    const result = await executeBrowserCommand(this, command);
    this.globals.nodeRepl?.setResponseMeta?.({
      browser_use: {
        browser_id: this.info.id,
        browser_type: this.info.type,
        metadata: this.info.metadata ?? {},
        caller_provenance: requestMeta.caller_provenance,
      },
    });
    return result;
  }

  raw(method: string, params: Record<string, unknown> = {}): Promise<unknown> {
    const context = callerContext(this.globals);
    return this.rpc.request(method, { ...params, ...context, _meta: context, request_meta: context });
  }

  onNotification(method: string, listener: (params: unknown) => void): () => void {
    return this.rpc.onNotification(method, listener);
  }

  executeCapability(params: Record<string, unknown>): Promise<unknown> {
    const id = typeof params.capability_id === "string" ? params.capability_id : "unknown";
    throw new Error(`Browser capability ${id} has no executable sky-cua v1 raw-protocol mapping`);
  }

  close(): void {
    this.rpc.close();
  }
}

export class BrowserBackendRegistry {
  private backends: BrowserBackend[] | undefined;

  constructor(private readonly globals: BrowserGlobals) {}

  async list(): Promise<BrowserBackend[]> {
    if (this.backends !== undefined) return this.backends;
    const candidates = await discoverBrowserCandidates(this.globals);
    const hostCandidates = candidates.filter(({ transport }) => transport === "host_provided_iab");
    const extensionCandidates = candidates.filter(
      ({ transport }) => transport === "extension_native_host",
    );
    const [hostResult, extensionResult] = await Promise.all([
      this.connectFirst(hostCandidates),
      this.connectFirstSequential(extensionCandidates),
    ]);
    const settled = [
      ...hostResult.failures.map((reason) => ({ status: "rejected" as const, reason })),
      ...extensionResult.failures.map((reason) => ({ status: "rejected" as const, reason })),
    ];
    this.backends = [
      ...(hostResult.backend === undefined ? [] : [hostResult.backend]),
      ...(extensionResult.backend === undefined ? [] : [extensionResult.backend]),
    ];
    if (this.backends.length === 0) {
      const trustFailure = settled.find((result) =>
        result.status === "rejected"
        && asError(result.reason).message === "Trusted Browser native-pipe access is unavailable"
      );
      if (trustFailure?.status === "rejected") throw asError(trustFailure.reason);
    }
    return this.backends;
  }

  async get(idOrType: string): Promise<BrowserBackend> {
    const backends = await this.list();
    if (idOrType === "iab") {
      const iab = backends.find(({ info }) =>
        info.type === "iab" && info.transport === "host_provided_iab");
      if (iab === undefined) throw new Error("Browser is not available: iab");
      return iab;
    }
    const exact = backends.find(({ info }) => info.id === idOrType);
    if (exact !== undefined) return exact;
    const match = backends.find(({ info }) => info.type === idOrType);
    if (match === undefined) throw new Error(`Browser is not available: ${idOrType}`);
    return match;
  }

  async close(): Promise<void> {
    for (const backend of this.backends ?? []) backend.close();
    this.backends = [];
  }

  private connectFirst(candidates: BrowserCandidate[]): Promise<{
    backend?: BrowserBackend;
    failures: Error[];
  }> {
    if (candidates.length === 0) return Promise.resolve({ failures: [] });
    const controllers = candidates.map(() => new AbortController());
    const failures: Error[] = [];
    return new Promise((resolve) => {
      let remaining = candidates.length;
      let resolved = false;
      candidates.forEach((candidate, index) => {
        BrowserBackend.connect(
          this.globals,
          candidate.path,
          candidate.transport,
          controllers[index]?.signal,
        ).then((backend) => {
          remaining -= 1;
          if (resolved) {
            backend.close();
            return;
          }
          resolved = true;
          controllers.forEach((controller, controllerIndex) => {
            if (controllerIndex !== index) controller.abort();
          });
          resolve({ backend, failures });
        }).catch((error) => {
          remaining -= 1;
          failures.push(asError(error));
          if (!resolved && remaining === 0) {
            resolved = true;
            resolve({ failures });
          }
        });
      });
    });
  }

  private async connectFirstSequential(candidates: BrowserCandidate[]): Promise<{
    backend?: BrowserBackend;
    failures: Error[];
  }> {
    const failures: Error[] = [];
    for (const candidate of candidates) {
      try {
        return {
          backend: await BrowserBackend.connect(this.globals, candidate.path, candidate.transport),
          failures,
        };
      } catch (error) {
        failures.push(asError(error));
      }
    }
    return { failures };
  }
}

function normalizeBrowserInfo(
  raw: unknown,
  candidate: BrowserCandidate,
  expectedSessionId: string | undefined,
): BrowserInfo {
  if (raw === null || typeof raw !== "object") throw new Error("Browser getInfo returned no identity");
  const value = raw as Record<string, unknown>;
  if (value.type !== "iab" && value.type !== "extension" && value.type !== "cdp") {
    throw new Error("Browser getInfo returned an unsupported type");
  }
  const rawMetadata = value.metadata !== null && typeof value.metadata === "object"
    ? value.metadata as Record<string, unknown>
    : {};
  const advertisedTransport = rawMetadata.skyCuaBridgeTransport;
  if (candidate.transport === "host_provided_iab") {
    if (
      value.type !== "iab"
      || advertisedTransport === "extension_native_host"
      || rawMetadata.skyCuaBridgeType === "extension"
      || rawMetadata.provider === "extension"
    ) {
      throw new Error("Task-scoped Browser socket did not return a host-provided IAB");
    }
  }
  const transport = candidate.transport;
  const type = value.type === "iab" && transport === "extension_native_host"
    ? "extension"
    : value.type;
  if (transport === "host_provided_iab") {
    if (type !== "iab") throw new Error("Host-provided IAB returned a non-IAB browser type");
    const actualSessionId = rawMetadata.codexSessionId;
    if (
      expectedSessionId === undefined
      || actualSessionId !== expectedSessionId
    ) {
      throw new Error("Host-provided IAB session does not match the current Codex session");
    }
  }
  const metadata = {
    ...rawMetadata,
    skyCuaBridgeTransport: transport,
    ...(transport === "extension_native_host"
      ? { provider: "extension", skyCuaBridgeType: "extension" }
      : {}),
  } as Record<string, string>;
  const name = typeof value.name === "string" ? value.name : type;
  const rawId = typeof value.id === "string" ? value.id : undefined;
  const id = transport === "extension_native_host" && rawId?.startsWith("iab:") !== false
    ? `extension:${candidate.path.split("/").at(-1)?.replace(/\.sock$/u, "") ?? "browser"}`
    : (rawId ?? `${type}:${candidate.path}`);
  const capabilities = value.capabilities !== null && typeof value.capabilities === "object"
    ? value.capabilities as BrowserInfo["capabilities"]
    : {};
  const apiSupportOverrides = value.apiSupportOverrides !== null && typeof value.apiSupportOverrides === "object"
    ? Object.fromEntries(Object.entries(value.apiSupportOverrides).filter((entry): entry is [string, boolean] =>
      typeof entry[1] === "boolean"))
    : undefined;
  return {
    id,
    type,
    transport,
    name,
    capabilities,
    ...(apiSupportOverrides === undefined ? {} : { apiSupportOverrides }),
    metadata,
  };
}
