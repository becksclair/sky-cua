import type { BrowserCommand, CommandEnvelope } from "./commands.ts";
import { callerContext, type BrowserGlobals, type NativePipeConnection } from "./globals.ts";
import { executeBrowserCommand } from "./wire-runtime.ts";

const MAX_FRAME_BYTES = 8 * 1024 * 1024;

export type BrowserInfo = {
  id: string;
  type: "iab" | "extension" | "cdp";
  name: string;
  capabilities: {
    browser?: Array<{ id: string; description: string }>;
    tab?: Array<{ id: string; description: string }>;
  };
  apiSupportOverrides?: Record<string, boolean>;
  metadata?: Record<string, string>;
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
  new DataView(result.buffer).setUint32(0, payload.byteLength, true);
  result.set(payload, 4);
  return result;
}

function append(left: Uint8Array, right: Uint8Array): Uint8Array {
  const joined = new Uint8Array(left.byteLength + right.byteLength);
  joined.set(left);
  joined.set(right, left.byteLength);
  return joined;
}

class JsonRpcConnection {
  private buffer: Uint8Array<ArrayBufferLike> = new Uint8Array();
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
    this.buffer = append(this.buffer, chunk);
    while (this.buffer.byteLength >= 4) {
      const size = new DataView(this.buffer.buffer, this.buffer.byteOffset, 4).getUint32(0, true);
      if (size > MAX_FRAME_BYTES) {
        this.failAll(new Error("BROWSER_PIPE_FRAME_TOO_LARGE"));
        return;
      }
      if (this.buffer.byteLength < size + 4) return;
      const payload = this.buffer.slice(4, size + 4);
      this.buffer = this.buffer.slice(size + 4);
      this.onMessage(JSON.parse(new TextDecoder().decode(payload)) as JsonRpcResponse);
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

function validSocketPath(path: string): boolean {
  return path.startsWith("/") && !path.includes("\0");
}

async function discoverSocketPaths(globals: BrowserGlobals): Promise<string[]> {
  const explicit = globals.nodeRepl?.env?.SKY_CUA_CODEX_BROWSER_SOCKET_PATH?.trim();
  if (explicit === undefined || explicit === "") {
    throw new Error("SKY_CUA_CODEX_BROWSER_SOCKET_PATH is required");
  }
  if (!validSocketPath(explicit)) {
    throw new Error("SKY_CUA_CODEX_BROWSER_SOCKET_PATH must be an absolute Unix socket path");
  }
  return [explicit];
}

export class BrowserBackend {
  private constructor(
    readonly path: string,
    readonly info: BrowserInfo,
    private readonly globals: BrowserGlobals,
    private readonly rpc: JsonRpcConnection,
  ) {}

  static async connect(globals: BrowserGlobals, path: string): Promise<BrowserBackend> {
    const rpc = await JsonRpcConnection.connect(globals, path);
    const context = callerContext(globals);
    const raw = await rpc.request("getInfo", { ...context, _meta: context, request_meta: context });
    const info = normalizeBrowserInfo(raw, path);
    return new BrowserBackend(path, info, globals, rpc);
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
    const paths = await discoverSocketPaths(this.globals);
    this.backends = await Promise.all(paths.map((path) => BrowserBackend.connect(this.globals, path)));
    return this.backends;
  }

  async get(idOrType: string): Promise<BrowserBackend> {
    const backends = await this.list();
    const match = backends.find(({ info }) => info.id === idOrType || info.type === idOrType);
    if (match === undefined) throw new Error(`Browser is not available: ${idOrType}`);
    return match;
  }

  async close(): Promise<void> {
    for (const backend of this.backends ?? []) backend.close();
    this.backends = [];
  }
}

function normalizeBrowserInfo(raw: unknown, path: string): BrowserInfo {
  if (raw === null || typeof raw !== "object") throw new Error("Browser getInfo returned no identity");
  const value = raw as Record<string, unknown>;
  if (value.type !== "iab" && value.type !== "extension" && value.type !== "cdp") {
    throw new Error("Browser getInfo returned an unsupported type");
  }
  const name = typeof value.name === "string" ? value.name : value.type;
  const id = typeof value.id === "string" ? value.id : `${value.type}:${path}`;
  const capabilities = value.capabilities !== null && typeof value.capabilities === "object"
    ? value.capabilities as BrowserInfo["capabilities"]
    : {};
  const apiSupportOverrides = value.apiSupportOverrides !== null && typeof value.apiSupportOverrides === "object"
    ? Object.fromEntries(Object.entries(value.apiSupportOverrides).filter((entry): entry is [string, boolean] =>
      typeof entry[1] === "boolean"))
    : undefined;
  return {
    id,
    type: value.type,
    name,
    capabilities,
    ...(apiSupportOverrides === undefined ? {} : { apiSupportOverrides }),
    ...(value.metadata !== null && typeof value.metadata === "object"
      ? { metadata: value.metadata as Record<string, string> }
      : {}),
  };
}
