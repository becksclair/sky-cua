import { randomUUID } from "node:crypto";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { KERNEL_SOURCE } from "../kernel/kernel.ts";
import { analyzeCell, type PersistentBinding } from "./cell-analysis.ts";
import {
  NativePipeBroker,
  type NativePipeRequest,
} from "./native-pipe-broker.ts";
import {
  KERNEL_PROTOCOL_VERSION,
  parseExecResult,
  parseKernelMessage,
  type KernelExecResult,
  type KernelMessage,
} from "./kernel-protocol.ts";

export interface KernelChildOptions {
  nodePath: string;
  cwd: string;
  env: NodeJS.ProcessEnv;
  onStderr?: (text: string) => void;
  onSuspension?: (kind: "suspend" | "resume", execId: string) => void;
  onAuthenticatedFetch?: (
    input: string,
    init: Record<string, unknown> | undefined,
    signal: AbortSignal,
  ) => Promise<FetchBridgeResult>;
  onElicitation?: (
    request: Record<string, unknown>,
    signal: AbortSignal,
  ) => Promise<unknown>;
  onConfig?: (
    operation: string,
    payload: Record<string, unknown>,
    signal: AbortSignal,
  ) => Promise<unknown>;
}

export interface FetchBridgeResult {
  status: number;
  statusText: string;
  headers: Record<string, string>;
  body_base64: string;
}

interface PendingExecution {
  resolve: (result: KernelExecResult) => void;
  reject: (error: Error) => void;
}

const KERNEL_ERROR_SUFFIX =
  "; kernel reset. Catch or handle async errors (including Promise rejections and EventEmitter 'error' events) to avoid kernel termination.";
const SIGTERM_EXIT_DEADLINE_MS = 250;
const SIGKILL_EXIT_DEADLINE_MS = 250;

export class KernelChild {
  private readonly options: KernelChildOptions;
  private child: ChildProcessWithoutNullStreams | null = null;
  private ready: Promise<void> | null = null;
  private readyResolve: (() => void) | null = null;
  private readyReject: ((error: Error) => void) | null = null;
  private readonly pending = new Map<string, PendingExecution>();
  private readonly moduleDirPending = new Map<
    string,
    { resolve: (added: boolean) => void; reject: (error: Error) => void }
  >();
  private readonly nativePipeBroker: NativePipeBroker;
  private stderr = "";
  private closed = false;
  private bridgeToken: string | null = null;
  private activeExecId: string | null = null;
  private stopping: Promise<void> | null = null;
  private privilegedAbortController: AbortController | null = null;

  public constructor(options: KernelChildOptions) {
    this.options = options;
    this.nativePipeBroker = new NativePipeBroker({
      env: options.env,
      onEvent: (event) => {
        if (this.child === null || this.closed) return;
        this.write({
          version: KERNEL_PROTOCOL_VERSION,
          type: event.type,
          exec_id: event.execId,
          connection_id: event.connectionId,
          ...(event.type === "native_pipe_data"
            ? { data_base64: event.dataBase64 }
            : { error: event.error }),
        });
      },
    });
  }

  public get pid(): number | undefined {
    return this.child?.pid;
  }

  public get stderrText(): string {
    return this.stderr;
  }

  public async start(): Promise<void> {
    if (this.child !== null) return this.ready ?? Promise.resolve();
    if (this.stopping !== null) {
      await this.stopping;
      this.stopping = null;
    }
    this.closed = false;
    this.stderr = "";
    this.activeExecId = null;
    this.privilegedAbortController = new AbortController();
    this.ready = new Promise<void>((resolve, reject) => {
      this.readyResolve = resolve;
      this.readyReject = reject;
    });
    const child = spawn(
      this.options.nodePath,
      [
        "--experimental-vm-modules",
        "--input-type=module",
        "--eval",
        KERNEL_SOURCE,
      ],
      {
        cwd: this.options.cwd,
        env: this.options.env,
        stdio: ["pipe", "pipe", "pipe", "ipc"],
      },
    ) as ChildProcessWithoutNullStreams;
    this.child = child;
    child.on("message", (message: unknown) => this.handleMessage(message));
    // fd 1 is deliberately not a control channel. Drain it independently so
    // a package writing directly to stdout cannot corrupt MCP or backpressure
    // the private IPC channel.
    child.stdout.on("data", () => undefined);
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      this.stderr += chunk;
      this.options.onStderr?.(chunk);
    });
    child.on("error", (error) => this.handleExit(error));
    child.on("exit", (code, signal) => {
      const suffix =
        signal === null ? `exit ${code ?? "unknown"}` : `signal ${signal}`;
      this.handleExit(
        new Error(
          `node_repl kernel ${suffix}: ${this.stderr.trim() || "child exited"}${KERNEL_ERROR_SUFFIX}`,
        ),
      );
    });
    await this.ready;
  }

  public async execute(
    id: string,
    code: string,
    requestMeta: Record<string, unknown> | null,
  ): Promise<KernelExecResult> {
    await this.start();
    let bindings: PersistentBinding[];
    try {
      bindings = analyzeCell(code);
    } catch (error) {
      return {
        ok: false,
        output: "",
        error: error instanceof Error ? error.message : String(error),
        images: [],
        responseMeta: null,
      };
    }
    if (this.closed || this.child === null)
      throw new Error("node_repl kernel is unavailable");
    this.activeExecId = id;
    this.nativePipeBroker.setActiveExecution(id);
    return new Promise<KernelExecResult>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.write({
        version: KERNEL_PROTOCOL_VERSION,
        type: "exec",
        id,
        code,
        bindings,
        request_meta: requestMeta,
      });
    });
  }

  public cancel(execId: string): void {
    if (this.child === null || this.closed) return;
    this.write({
      version: KERNEL_PROTOCOL_VERSION,
      type: "cancel",
      exec_id: execId,
    });
  }

  public async addNodeModuleDir(path: string): Promise<boolean> {
    await this.start();
    const id = `module-dir-${randomUUID()}`;
    return new Promise<boolean>((resolvePromise, rejectPromise) => {
      this.moduleDirPending.set(id, {
        resolve: resolvePromise,
        reject: rejectPromise,
      });
      this.write({
        version: KERNEL_PROTOCOL_VERSION,
        type: "add_node_module_dir",
        id,
        path,
      });
    });
  }

  public async terminate(reason: string): Promise<void> {
    if (this.child === null) {
      await this.stopping;
      return;
    }
    this.closed = true;
    this.abortPrivilegedCallbacks();
    this.closeNativePipes();
    const child = this.child;
    this.child = null;
    this.readyReject?.(new Error(reason));
    this.ready = null;
    this.readyResolve = null;
    this.readyReject = null;
    for (const pending of this.pending.values())
      pending.reject(new Error(reason));
    this.pending.clear();
    for (const pending of this.moduleDirPending.values())
      pending.reject(new Error(reason));
    this.moduleDirPending.clear();
    this.activeExecId = null;
    this.nativePipeBroker.setActiveExecution(null);
    const stopping = this.stopProcess(child);
    this.stopping = stopping;
    await stopping;
    if (this.stopping === stopping) this.stopping = null;
  }

  public close(): Promise<void> {
    return this.terminate("node_repl kernel closed");
  }

  private write(message: Record<string, unknown>): void {
    if (this.child === null || this.closed || this.child.stdin.destroyed) {
      throw new Error("node_repl kernel control channel is closed");
    }
    try {
      this.child.send(message, (error) => {
        if (error !== null && !this.closed) this.handleExit(error, true);
      });
    } catch (error) {
      this.handleExit(
        error instanceof Error ? error : new Error(String(error)),
        true,
      );
      throw error;
    }
  }

  private handleMessage(value: unknown): void {
    let message: KernelMessage;
    try {
      message = parseKernelMessage(value);
    } catch (error) {
      this.handleExit(
        error instanceof Error ? error : new Error(String(error)),
        true,
      );
      return;
    }
    if (message.type === "privileged_bridge_handshake") {
      if (typeof message.token !== "string" || message.token.length === 0) {
        this.handleExit(
          new Error("kernel bridge handshake token is missing"),
          true,
        );
        return;
      }
      this.bridgeToken = message.token;
      this.nativePipeBroker.setGeneration(message.token);
      this.readyResolve?.();
      this.readyResolve = null;
      this.readyReject = null;
      return;
    }
    if (message.type === "exec_result" && typeof message.id === "string") {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      this.activeExecId = null;
      this.nativePipeBroker.setActiveExecution(null);
      try {
        pending.resolve(parseExecResult(message));
      } catch (error) {
        pending.reject(
          error instanceof Error ? error : new Error(String(error)),
        );
        this.handleExit(
          error instanceof Error ? error : new Error(String(error)),
          true,
        );
      }
      return;
    }
    if (
      message.type === "module_dir_result" &&
      typeof message.id === "string"
    ) {
      const pending = this.moduleDirPending.get(message.id);
      if (pending === undefined) return;
      this.moduleDirPending.delete(message.id);
      pending.resolve(message.added === true);
      return;
    }
    if (
      (message.type === "suspend_timeout" ||
        message.type === "resume_timeout") &&
      typeof message.exec_id === "string"
    ) {
      this.options.onSuspension?.(
        message.type === "suspend_timeout" ? "suspend" : "resume",
        message.exec_id,
      );
      return;
    }
    if (message.type === "privileged_request") {
      void this.handlePrivilegedRequest(message);
      return;
    }
    if (message.type === "protocol_error") {
      this.handleExit(
        new Error(message.error ?? "kernel protocol error"),
        true,
      );
    }
  }

  private async handlePrivilegedRequest(message: KernelMessage): Promise<void> {
    const id = message.id;
    if (typeof id !== "string") return;
    try {
      if (this.bridgeToken === null || message.token !== this.bridgeToken) {
        throw new Error("kernel privileged request token is invalid");
      }
      if (message.generation !== this.bridgeToken) {
        throw new Error("native pipe generation is stale");
      }
      if (message.op === "native_pipe") {
        const result = await this.nativePipeBroker.handle({
          id,
          token: message.token,
          generation: message.generation,
          execId: message.exec_id ?? "",
          operation: nativeOperation(message.native_op),
          ...(message.connection_id === undefined
            ? {}
            : { connectionId: message.connection_id }),
          ...(message.path === undefined ? {} : { path: message.path }),
          ...(message.data_base64 === undefined
            ? {}
            : { dataBase64: message.data_base64 }),
        });
        this.respondBridge(id, true, result);
        return;
      }
      if (this.activeExecId === null || message.exec_id !== this.activeExecId) {
        throw new Error("node_repl exec context not found");
      }
      if (message.op === "emit_image") {
        if (
          typeof message.image_url !== "string" ||
          !message.image_url.startsWith("data:")
        ) {
          throw new Error("invalid image data URL");
        }
        this.respondBridge(id, true, undefined);
        return;
      }
      if (message.op === "authenticated_fetch") {
        if (
          this.options.onAuthenticatedFetch === undefined ||
          typeof message.input !== "string"
        ) {
          throw new Error("authenticated fetch is unavailable");
        }
        const result = await racePrivilegedCallback(
          this.privilegedSignal(),
          (signal) =>
            this.options.onAuthenticatedFetch!(message.input!, message.init, signal),
        );
        this.respondBridge(id, true, result);
        return;
      }
      if (message.op === "elicitation") {
        if (
          this.options.onElicitation === undefined ||
          message.request === undefined
        ) {
          throw new Error("form elicitation is unavailable");
        }
        this.respondBridge(
          id,
          true,
          await racePrivilegedCallback(
            this.privilegedSignal(),
            (signal) => this.options.onElicitation!(message.request!, signal),
          ),
        );
        return;
      }
      if (message.op === "config") {
        if (this.options.onConfig === undefined)
          throw new Error("config bridge is unavailable");
        this.respondBridge(
          id,
          true,
          await racePrivilegedCallback(
            this.privilegedSignal(),
            (signal) =>
              this.options.onConfig!(message.config_op ?? "", message, signal),
          ),
        );
        return;
      }
      if (message.op === "launch_service") {
        throw new Error("launch services are unavailable on Linux");
      }
      throw new Error(
        `unsupported privileged operation: ${message.op ?? "unknown"}`,
      );
    } catch (error) {
      this.respondBridge(
        id,
        false,
        undefined,
        error instanceof Error ? error.message : String(error),
      );
    }
  }

  private respondBridge(
    id: string,
    ok: boolean,
    result?: unknown,
    error?: string,
  ): void {
    if (this.child === null || this.closed) return;
    this.write({
      version: KERNEL_PROTOCOL_VERSION,
      type: "bridge_response",
      id,
      ok,
      ...(result === undefined ? {} : { result }),
      ...(error === undefined ? {} : { error }),
    });
  }

  private closeNativePipes(): void {
    this.nativePipeBroker.closeAll();
  }

  private privilegedSignal(): AbortSignal {
    return this.privilegedAbortController?.signal ?? AbortSignal.abort();
  }

  private abortPrivilegedCallbacks(): void {
    this.privilegedAbortController?.abort();
    this.privilegedAbortController = null;
  }

  private handleExit(error: Error, terminateProcess = false): void {
    if (this.closed && this.child === null) return;
    const child = this.child;
    this.closed = true;
    this.abortPrivilegedCallbacks();
    this.closeNativePipes();
    this.child = null;
    this.activeExecId = null;
    this.nativePipeBroker.setActiveExecution(null);
    this.readyReject?.(error);
    this.readyReject = null;
    this.readyResolve = null;
    this.ready = null;
    for (const pending of this.pending.values()) pending.reject(error);
    this.pending.clear();
    for (const pending of this.moduleDirPending.values()) pending.reject(error);
    this.moduleDirPending.clear();
    if (terminateProcess && child !== null) {
      const stopping = this.stopProcess(child);
      this.stopping = stopping;
      void stopping.catch(() => undefined);
    }
  }

  private async stopProcess(
    child: ChildProcessWithoutNullStreams,
  ): Promise<void> {
    if (child.exitCode !== null || child.signalCode !== null) return;
    child.kill("SIGTERM");
    const exitedAfterTerm = await waitForExit(child, SIGTERM_EXIT_DEADLINE_MS);
    if (exitedAfterTerm) return;
    child.kill("SIGKILL");
    const exitedAfterKill = await waitForExit(child, SIGKILL_EXIT_DEADLINE_MS);
    if (!exitedAfterKill)
      throw new Error(
        `node_repl kernel did not exit within ${SIGKILL_EXIT_DEADLINE_MS}ms after SIGKILL`,
      );
  }
}

async function racePrivilegedCallback<T>(
  signal: AbortSignal,
  callback: (signal: AbortSignal) => Promise<T>,
): Promise<T> {
  if (signal.aborted) throw abortReason(signal);
  return new Promise<T>((resolve, reject) => {
    const onAbort = (): void => reject(abortReason(signal));
    signal.addEventListener("abort", onAbort, { once: true });
    void Promise.resolve()
      .then(() => callback(signal))
      .then(resolve, reject)
      .finally(() => signal.removeEventListener("abort", onAbort));
  });
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error
    ? signal.reason
    : new Error("kernel generation terminated");
}

async function waitForExit(
  child: ChildProcessWithoutNullStreams,
  timeoutMs?: number,
): Promise<boolean> {
  if (child.exitCode !== null || child.signalCode !== null) return true;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let onExit: (() => void) | null = null;
  const exited = new Promise<boolean>((resolve) => {
    onExit = () => resolve(true);
    child.once("exit", onExit);
  });
  if (timeoutMs === undefined) return exited;
  const timeout = new Promise<boolean>((resolve) => {
    timer = setTimeout(() => resolve(false), timeoutMs);
  });
  const result = await Promise.race([exited, timeout]);
  if (timer !== null) clearTimeout(timer);
  if (!result && onExit !== null) child.off("exit", onExit);
  return result;
}

function nativeOperation(
  value: string | undefined,
): NativePipeRequest["operation"] {
  if (
    value === "connect" ||
    value === "write" ||
    value === "close" ||
    value === "list_directory"
  )
    return value;
  throw new Error(`unsupported native pipe operation: ${value ?? "unknown"}`);
}

export function createKernelId(): string {
  return `exec-${randomUUID()}`;
}
