import { Buffer } from "node:buffer";
import { randomUUID } from "node:crypto";
import { lstat, readdir, realpath, stat } from "node:fs/promises";
import type { Stats } from "node:fs";
import { basename, dirname, isAbsolute } from "node:path";
import { createConnection, type Socket } from "node:net";

export const DEFAULT_NATIVE_PIPE_CONNECT_TIMEOUT_MS = 1_000;
export const MAX_NATIVE_PIPE_PATH_BYTES = 107;
export const MAX_NATIVE_PIPE_REQUEST_HISTORY = 1_024;
export const MAX_NATIVE_PIPE_PREAUTH_BYTES = 8 * 1024 * 1024 + 4;

export interface NativePipeRequest {
  id: string;
  token: string;
  generation: string;
  execId: string;
  operation: "connect" | "write" | "close" | "list_directory";
  connectionId?: string;
  path?: string;
  dataBase64?: string;
}

export type NativePipeEvent =
  | {
      type: "native_pipe_data";
      connectionId: string;
      dataBase64: string;
      execId: string;
    }
  | {
      type: "native_pipe_closed";
      connectionId: string;
      error: string | null;
      execId: string;
    };

export interface NativePipeBrokerOptions {
  env?: NodeJS.ProcessEnv;
  onEvent?: (event: NativePipeEvent) => void;
}

interface NativePipeConnection {
  socket: Socket;
  execId: string;
  connected: boolean;
  authorized: boolean;
  closed: boolean;
  terminalError: string | null;
  pendingData: Buffer[];
  pendingDataBytes: number;
}

interface UnixSocketIdentity {
  dev: number;
  ino: number;
  uid: number | null;
  mode: number;
}

interface ValidatedUnixSocket {
  realPath: string;
  identity: UnixSocketIdentity;
}

const NATIVE_PIPE_OPERATIONS = new Set<NativePipeRequest["operation"]>([
  "connect",
  "write",
  "close",
  "list_directory",
]);

export class NativePipeBroker {
  private readonly env: NodeJS.ProcessEnv;
  private readonly onEvent: ((event: NativePipeEvent) => void) | undefined;
  private readonly connections = new Map<string, NativePipeConnection>();
  private readonly requestIds = new Map<string, true>();
  private token: string = randomUUID();
  private activeExecId: string | null = null;

  public constructor(options: NativePipeBrokerOptions = {}) {
    this.env = options.env ?? process.env;
    this.onEvent = options.onEvent;
  }

  public get generationToken(): string {
    return this.token;
  }

  public setGeneration(token: string = randomUUID()): void {
    this.closeAll();
    this.token = token;
    this.activeExecId = null;
    this.requestIds.clear();
  }

  public setActiveExecution(execId: string | null): void {
    this.activeExecId = execId;
  }

  public async handle(
    request: NativePipeRequest,
  ): Promise<Record<string, unknown>> {
    this.validateRequest(request);
    if (this.requestIds.has(request.id))
      throw new Error("native pipe request id has already been used");
    this.requestIds.set(request.id, true);
    while (this.requestIds.size > MAX_NATIVE_PIPE_REQUEST_HISTORY) {
      const oldest = this.requestIds.keys().next().value;
      if (typeof oldest !== "string") break;
      this.requestIds.delete(oldest);
    }
    if (request.operation === "connect") {
      this.requireActiveExecution(request.execId);
      return this.connect(request);
    }
    if (request.operation === "list_directory") {
      this.requireActiveExecution(request.execId);
      if (typeof request.path !== "string")
        throw new Error("native pipe directory path must be absolute");
      return { entries: await listUnixSocketDirectory(request.path) };
    }
    const connectionId = request.connectionId;
    if (connectionId === undefined)
      throw new Error("native pipe connection id is required");
    const connection = this.connections.get(connectionId);
    if (connection === undefined || connection.closed) {
      if (request.operation === "close") return {};
      throw new Error("native pipe connection is closed");
    }
    const isActiveExecution =
      this.activeExecId !== null && request.execId === this.activeExecId;
    if (!isActiveExecution && request.execId !== connection.execId) {
      throw new Error("node_repl exec context not found");
    }
    if (isActiveExecution) connection.execId = request.execId;
    if (request.operation === "write") {
      if (request.dataBase64 === undefined || !isBase64(request.dataBase64)) {
        throw new Error("native pipe write requires base64 data");
      }
      const bytes = Buffer.from(request.dataBase64, "base64");
      connection.socket.write(bytes);
      return {};
    }
    connection.socket.end();
    this.finish(connectionId, null);
    return {};
  }

  public closeAll(): void {
    for (const connection of this.connections.values()) {
      connection.closed = true;
      connection.authorized = false;
      connection.socket.destroy();
    }
    this.connections.clear();
  }

  private validateRequest(request: NativePipeRequest): void {
    if (typeof request.id !== "string" || request.id.length === 0) {
      throw new Error("native pipe request id is required");
    }
    if (request.token !== this.token)
      throw new Error("kernel privileged request token is invalid");
    if (request.generation !== this.token)
      throw new Error("native pipe generation is stale");
    if (!NATIVE_PIPE_OPERATIONS.has(request.operation)) {
      throw new Error(
        `unsupported native pipe operation: ${request.operation}`,
      );
    }
  }

  private requireActiveExecution(execId: string): void {
    if (this.activeExecId === null || execId !== this.activeExecId) {
      throw new Error("node_repl exec context not found");
    }
  }

  private async connect(
    request: NativePipeRequest,
  ): Promise<Record<string, unknown>> {
    if (typeof request.path !== "string")
      throw new Error("native pipe path must be absolute");
    const validated = await validateUnixSocketPathDetails(request.path);
    const connectionId = request.connectionId;
    if (connectionId === undefined || connectionId.length === 0) {
      throw new Error("native pipe connection id is required");
    }
    if (this.connections.has(connectionId))
      throw new Error("native pipe connection id is already active");
    const socket = createConnection(validated.realPath);
    const connection: NativePipeConnection = {
      socket,
      execId: request.execId,
      connected: false,
      authorized: false,
      closed: false,
      terminalError: null,
      pendingData: [],
      pendingDataBytes: 0,
    };
    this.connections.set(connectionId, connection);
    const emitData = (chunk: Buffer): void => {
      this.onEvent?.({
        type: "native_pipe_data",
        connectionId,
        dataBase64: chunk.toString("base64"),
        execId: this.activeExecId ?? connection.execId,
      });
    };
    socket.on("data", (chunk: Buffer) => {
      if (connection.closed) return;
      if (!connection.authorized) {
        connection.pendingData.push(Buffer.from(chunk));
        connection.pendingDataBytes += chunk.byteLength;
        if (connection.pendingDataBytes > MAX_NATIVE_PIPE_PREAUTH_BYTES) {
          socket.destroy(new Error("native pipe initial data is too large"));
        }
        return;
      }
      emitData(chunk);
    });
    socket.on("error", (error: Error) => {
      connection.terminalError = error.message;
      if (!connection.authorized) return;
      this.finish(connectionId, error.message);
    });
    socket.on("close", () => {
      if (connection.authorized && !connection.closed)
        this.finish(connectionId, connection.terminalError);
    });
    socket.pause();

    const timeoutMs = nativePipeConnectTimeoutMs(this.env);
    await new Promise<void>((resolvePromise, rejectPromise) => {
      let settled = false;
      const rejectBeforeAuthorization = (error: Error): void => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        connection.closed = true;
        connection.authorized = false;
        this.connections.delete(connectionId);
        socket.destroy(error);
        rejectPromise(error);
      };
      const timer = setTimeout(() => {
        rejectBeforeAuthorization(
          new Error("native pipe initial connect timed out"),
        );
      }, timeoutMs);
      const onConnect = async (): Promise<void> => {
        if (settled) return;
        connection.connected = true;
        try {
          await verifyConnectedUnixSocket(socket, validated);
        } catch (error) {
          rejectBeforeAuthorization(toError(error));
          return;
        }
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        connection.authorized = true;
        for (const chunk of connection.pendingData) emitData(chunk);
        connection.pendingData = [];
        connection.pendingDataBytes = 0;
        socket.resume();
        resolvePromise();
      };
      const onError = (error: Error) => {
        rejectBeforeAuthorization(error);
      };
      const onCloseBeforeConnect = () => {
        rejectBeforeAuthorization(
          new Error("native pipe initial connect closed"),
        );
      };
      socket.once("connect", () => {
        void onConnect();
      });
      socket.once("error", onError);
      socket.once("close", onCloseBeforeConnect);
    });
    return { connection_id: connectionId };
  }

  private finish(connectionId: string, error: string | null): void {
    const connection = this.connections.get(connectionId);
    if (connection === undefined || connection.closed) return;
    connection.closed = true;
    connection.authorized = false;
    connection.terminalError = error;
    this.connections.delete(connectionId);
    this.onEvent?.({
      type: "native_pipe_closed",
      connectionId,
      error,
      execId: this.activeExecId ?? connection.execId,
    });
  }
}

export async function validateUnixSocketPath(path: string): Promise<string> {
  return (await validateUnixSocketPathDetails(path)).realPath;
}

export async function listUnixSocketDirectory(path: string): Promise<string[]> {
  if (!isAbsolute(path))
    throw new Error("native pipe directory path must be absolute");
  if (path.endsWith("/"))
    throw new Error("native pipe directory path must not end with a separator");
  if (Buffer.byteLength(path) > MAX_NATIVE_PIPE_PATH_BYTES) {
    throw new Error("native pipe directory path is too long");
  }
  try {
    const linkStats = await lstat(path);
    if (linkStats.isSymbolicLink())
      throw new Error("native pipe directory path must not be a symlink");
    const realPath = await realpath(path);
    const directoryStats = await stat(realPath);
    if (!directoryStats.isDirectory())
      throw new Error("native pipe directory path is not a directory");
    assertTrustedSocketOwner(directoryStats);
    return (await readdir(realPath, { withFileTypes: true }))
      .filter((entry) => entry.isSocket())
      .map((entry) => entry.name)
      .sort((left, right) => left.localeCompare(right));
  } catch (error) {
    if (
      error instanceof Error &&
      (error.message === "native pipe directory path is not a directory" ||
        error.message === "native pipe directory path must not be a symlink" ||
        error.message === "native pipe socket owner is not the current user")
    )
      throw error;
    throw new Error("native pipe directory path is not a directory");
  }
}

async function validateUnixSocketPathDetails(
  path: string,
): Promise<ValidatedUnixSocket> {
  if (!isAbsolute(path)) throw new Error("native pipe path must be absolute");
  const parent = dirname(path);
  if (parent === path)
    throw new Error("native pipe path has no parent directory");
  if (path.endsWith("/")) throw new Error("native pipe path has no file name");
  const filename = basename(path);
  if (filename.length === 0)
    throw new Error("native pipe path has no file name");
  if (Buffer.byteLength(path) > MAX_NATIVE_PIPE_PATH_BYTES) {
    throw new Error("native pipe file name is too long");
  }
  try {
    const parentStats = await stat(parent);
    if (!parentStats.isDirectory())
      throw new Error("native pipe path has no parent directory");
  } catch (error) {
    if (
      error instanceof Error &&
      error.message === "native pipe path has no parent directory"
    )
      throw error;
    throw new Error("native pipe path has no parent directory");
  }
  try {
    const linkStats = await lstat(path);
    if (linkStats.isSymbolicLink())
      throw new Error("native pipe path must not be a symlink");
    const realPath = await realpath(path);
    const socketStats = await stat(realPath);
    if (!socketStats.isSocket())
      throw new Error("native pipe path is not a socket");
    assertTrustedSocketOwner(socketStats);
    return { realPath, identity: socketIdentity(socketStats) };
  } catch (error) {
    if (
      error instanceof Error &&
      (error.message === "native pipe path is not a socket" ||
        error.message === "native pipe path must not be a symlink" ||
        error.message === "native pipe socket owner is not the current user")
    )
      throw error;
    throw new Error("native pipe path is not a socket");
  }
}

async function verifyConnectedUnixSocket(
  socket: Socket,
  validated: ValidatedUnixSocket,
): Promise<void> {
  const current = await validateUnixSocketPathDetails(validated.realPath);
  if (!sameSocketIdentity(current.identity, validated.identity)) {
    throw new Error("native pipe socket changed during connect");
  }
  const peer = connectedPeerCredentials(socket);
  const currentUid = process.getuid?.();
  if (peer.available && currentUid !== undefined && peer.uid !== currentUid) {
    throw new Error("native pipe peer is not the current user");
  }
}

function socketIdentity(stats: Stats): UnixSocketIdentity {
  return {
    dev: stats.dev,
    ino: stats.ino,
    uid: typeof stats.uid === "number" ? stats.uid : null,
    mode: stats.mode & 0o777,
  };
}

function sameSocketIdentity(
  left: UnixSocketIdentity,
  right: UnixSocketIdentity,
): boolean {
  return (
    left.dev === right.dev &&
    left.ino === right.ino &&
    left.uid === right.uid &&
    left.mode === right.mode
  );
}

function assertTrustedSocketOwner(stats: Stats): void {
  const currentUid = process.getuid?.();
  if (currentUid !== undefined && stats.uid !== currentUid) {
    throw new Error("native pipe socket owner is not the current user");
  }
}

function connectedPeerCredentials(socket: Socket): {
  available: boolean;
  uid: number | null;
} {
  const handle = (
    socket as Socket & { _handle?: { getPeerCredentials?: () => unknown } }
  )._handle;
  if (handle === undefined || typeof handle.getPeerCredentials !== "function") {
    return { available: false, uid: null };
  }
  const credentials = handle.getPeerCredentials();
  if (typeof credentials === "number")
    return { available: true, uid: credentials };
  if (
    typeof credentials === "object" &&
    credentials !== null &&
    "uid" in credentials
  ) {
    const uid = credentials.uid;
    if (typeof uid === "number") return { available: true, uid };
  }
  throw new Error("native pipe peer credentials are unavailable");
}

export function nativePipeConnectTimeoutMs(
  env: NodeJS.ProcessEnv = process.env,
): number {
  const raw = env.NODE_REPL_NATIVE_PIPE_CONNECT_TIMEOUT_MS?.trim();
  if (raw === undefined || raw.length === 0)
    return DEFAULT_NATIVE_PIPE_CONNECT_TIMEOUT_MS;
  const value = Number(raw);
  if (!Number.isInteger(value) || value < 1)
    return DEFAULT_NATIVE_PIPE_CONNECT_TIMEOUT_MS;
  return value;
}

function isBase64(value: string): boolean {
  if (value.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/u.test(value))
    return false;
  return Buffer.from(value, "base64").toString("base64") === value;
}

function toError(error: unknown): Error {
  if (error instanceof Error) return error;
  if (typeof error === "string") return new Error(error);
  return new Error("native pipe connection failed");
}
