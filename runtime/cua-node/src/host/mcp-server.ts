import { createInterface, type Interface } from "node:readline";
import { randomUUID } from "node:crypto";
import { stdin, stdout } from "node:process";
import toolsFixture from "../../test/fixtures/upstream-5307/tools-list.json";
import {
  RuntimeManager,
  makeRequestMeta,
  makeResponseMeta,
  type RuntimeRequestId,
  validateTimeout,
} from "./runtime-manager.ts";

type RequestId = string | number;
type JsonObject = Record<string, unknown>;

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id?: RequestId;
  method: string;
  params?: unknown;
}

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: RequestId | null;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

export interface McpServerOptions {
  manager?: RuntimeManager;
  input?: NodeJS.ReadableStream;
  output?: NodeJS.WritableStream;
  callerProvenance?: string;
}

export const MCP_CALLER_PROVENANCE_VALUES = [
  "codex_desktop",
  "openclaw",
  "opencode",
  "direct_mcp",
] as const;

export type McpCallerProvenance =
  (typeof MCP_CALLER_PROVENANCE_VALUES)[number];

const MCP_CALLER_PROVENANCE_SET = new Set<string>(MCP_CALLER_PROVENANCE_VALUES);

const SUPPORTED_PROTOCOLS = [
  "2024-11-05",
  "2025-03-26",
  "2025-06-18",
  "2025-11-25",
] as const;
const SUPPORTED_PROTOCOL_SET = new Set<string>(SUPPORTED_PROTOCOLS);
const SERVER_INSTRUCTIONS = [
  "Use `js` to run JavaScript in the persistent Node-backed kernel.",
  "When a skill or prompt says to use `node_repl`, call this server's `js` execution tool.",
  "Calls default to a 30000 ms (30 seconds) timeout when `timeout_ms` is omitted.",
  "The runtime exposes `nodeRepl.cwd`, `nodeRepl.homeDir`, `nodeRepl.tmpDir`, `nodeRepl.requestMeta`, `nodeRepl.setResponseMeta(...)`, `nodeRepl.write(value)`, and `await nodeRepl.emitImage(...)`.",
  "Use `nodeRepl.write(value)` to surface text output without a trailing newline, or `console.log(...)` for debugging with newlines; the value of the last expression in your code is not returned — only explicit `nodeRepl.write(...)`, `console.log(...)`, and `nodeRepl.emitImage(...)` calls produce content in the tool result.",
  "Top-level bindings persist across `js` calls until `js_reset`; do not redeclare existing `const` or `let` names.",
  "Reuse existing bindings, use top-level `var` for reusable state that may be assigned again, or choose a fresh descriptive name.",
  "Use `js_add_node_module_dir` before `js` when a skill provides an extra package directory, and use dynamic imports like `await import(\"playwright\")` rather than filesystem paths under `./node_modules`.",
].join(" ");
const TOOLS = toolsFixture.tools;

export const MCP_SERVER_INFO = Object.freeze({ name: "node_repl", version: "0.1.0" });

export class McpServer {
  public readonly manager: RuntimeManager;
  private readonly input: NodeJS.ReadableStream;
  private readonly output: NodeJS.WritableStream;
  private reader: Interface | null = null;
  private closed = false;
  private nextRequestId = 1;
  private nextSyntheticTurn = 1;
  private formElicitationSupported = false;
  private readonly syntheticSessionId = `node-repl-${randomUUID()}`;
  private readonly declaredCallerProvenance: string | undefined;
  private initializeClientInfo: Record<string, unknown> | null = null;
  private readonly pendingHandlers = new Set<Promise<void>>();
  private readonly clientRequests = new Map<
    RequestId,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
      signal?: AbortSignal;
      onAbort?: () => void;
    }
  >();

  public constructor(options: McpServerOptions = {}) {
    this.input = options.input ?? stdin;
    this.output = options.output ?? stdout;
    this.manager =
      options.manager ??
      new RuntimeManager({
        ...(process.env.NODE_REPL_NODE_PATH === undefined
          ? {}
          : { nodePath: process.env.NODE_REPL_NODE_PATH }),
        cwd: process.cwd(),
        env: process.env,
        allowHostNode: process.env.NODE_REPL_ALLOW_HOST_NODE === "1",
        onElicitation: (request, signal) =>
          this.requestClient("elicitation/create", request, signal),
      });
    this.declaredCallerProvenance =
      options.callerProvenance ?? process.env.SKY_CUA_MCP_CALLER_PROVENANCE;
  }

  public async start(): Promise<void> {
    if (this.reader !== null) return;
    this.reader = createInterface({ input: this.input, crlfDelay: Infinity });
    this.reader.on("line", (line) => {
      const handler = this.handleLine(line);
      this.pendingHandlers.add(handler);
      void handler.finally(() => this.pendingHandlers.delete(handler));
    });
    await new Promise<void>((resolvePromise) =>
      this.reader?.once("close", resolvePromise),
    );
    await this.close();
    await Promise.all([...this.pendingHandlers]);
  }

  public async handleLine(line: string): Promise<void> {
    if (line.trim().length === 0 || this.closed) return;
    let request: unknown;
    try {
      request = JSON.parse(line);
    } catch {
      this.write(makeError(null, -32700, "Invalid JSON"));
      return;
    }
    if (
      isObject(request) &&
      ("result" in request || "error" in request) &&
      "id" in request
    ) {
      this.resolveClientRequest(request);
      return;
    }
    if (!isRequest(request)) {
      this.write(makeError(null, -32600, "Invalid Request"));
      return;
    }
    const response = await this.dispatch(request);
    if (response !== null) this.write(response);
    if (request.method === "shutdown") await this.close();
  }

  public async dispatch(request: JsonRpcRequest): Promise<JsonRpcResponse | null> {
    if (request.method === "notifications/initialized") return null;
    if (request.method === "notifications/cancelled") {
      const requestId = cancellationRequestId(request.params);
      if (requestId !== undefined) this.manager.cancel(requestId);
      return null;
    }
    if (request.id === undefined) return null;
    if (request.method === "initialize")
      return this.handleInitialize(request.id, request.params);
    if (request.method === "tools/list")
      return {
        jsonrpc: "2.0",
        id: request.id,
        result: { tools: structuredClone(TOOLS) },
      };
    if (request.method === "tools/call")
      return this.handleToolsCall(request.id, request.params);
    if (request.method === "shutdown") {
      return { jsonrpc: "2.0", id: request.id, result: null };
    }
    return makeError(request.id, -32601, `Method not found: ${request.method}`);
  }

  public async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.reader?.close();
    this.reader = null;
    for (const pending of this.clientRequests.values()) {
      if (pending.signal !== undefined && pending.onAbort !== undefined)
        pending.signal.removeEventListener("abort", pending.onAbort);
      pending.reject(new Error("MCP host closed"));
    }
    this.clientRequests.clear();
    await this.manager.close();
  }

  private handleInitialize(id: RequestId, params: unknown): JsonRpcResponse {
    const requested =
      isObject(params) && typeof params.protocolVersion === "string"
        ? params.protocolVersion
        : undefined;
    this.formElicitationSupported =
      isObject(params) &&
      isObject(params.capabilities) &&
      isObject(params.capabilities.elicitation) &&
      isObject(params.capabilities.elicitation.form);
    this.initializeClientInfo = initializeClientInfo(params);
    const protocolVersion =
      requested !== undefined && SUPPORTED_PROTOCOL_SET.has(requested)
        ? requested
        : requested === undefined
          ? SUPPORTED_PROTOCOLS[0]
          : SUPPORTED_PROTOCOLS.find((version) => version === requested);
    if (protocolVersion === undefined)
      return makeError(id, -32602, "Unsupported protocol version");
    return {
      jsonrpc: "2.0",
      id,
      result: {
        protocolVersion,
        capabilities: { tools: { listChanged: true } },
        serverInfo: MCP_SERVER_INFO,
        instructions: SERVER_INSTRUCTIONS,
      },
    };
  }

  private async handleToolsCall(
    id: RequestId,
    params: unknown,
  ): Promise<JsonRpcResponse> {
    if (!isObject(params) || typeof params.name !== "string")
      return toolError(id, "tools/call requires a tool name");
    const args = params.arguments;
    try {
      const requestMeta = this.requestMetaForToolCall(params._meta);
      if (params.name === "js") return await this.callJs(id, args, requestMeta);
      if (params.name === "js_reset") {
        validateEmptyObject(args, "js_reset");
        const value = await this.manager.reset();
        return toolSuccess(id, [{ type: "text", text: String(value) }]);
      }
      if (params.name === "js_add_node_module_dir") {
        const path = validateModuleDirArguments(args);
        const value = await this.manager.addNodeModuleDir(path);
        return toolSuccess(id, [{ type: "text", text: String(value) }]);
      }
      return toolError(id, `Unknown tool: ${params.name}`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      return toolError(id, message);
    }
  }

  private async callJs(
    id: RequestId,
    args: unknown,
    requestMeta: Record<string, unknown>,
  ): Promise<JsonRpcResponse> {
    const parsed = validateJsArguments(args);
    try {
      const result = await this.manager.execute(
        parsed.code,
        parsed.timeoutMs === undefined
          ? { requestId: id, requestMeta }
          : { timeoutMs: parsed.timeoutMs, requestId: id, requestMeta },
      );
      if (!result.ok) throw new Error(result.error ?? "JavaScript execution failed");
      const content: Array<Record<string, unknown>> = [];
      if (result.output.length > 0) content.push({ type: "text", text: result.output });
      for (const imageUrl of result.images) content.push(imageContent(imageUrl));
      const response: JsonRpcResponse = {
        jsonrpc: "2.0",
        id,
        result: { content, isError: false },
      };
      const responseMeta = makeResponseMeta(result.responseMeta);
      if (responseMeta !== null) (response.result as JsonObject)._meta = responseMeta;
      return response;
    } catch (error) {
      return toolError(id, error instanceof Error ? error.message : String(error));
    }
  }

  private requestMetaForToolCall(meta: unknown): Record<string, unknown> {
    const supplied = makeRequestMeta(meta);
    if (supplied !== null && hasUsableCallerIdentity(supplied)) return supplied;
    const turnId = `${this.syntheticSessionId}-turn-${this.nextSyntheticTurn++}`;
    const suppliedTurnMetadata =
      supplied !== null && isObject(supplied["x-codex-turn-metadata"])
        ? supplied["x-codex-turn-metadata"]
        : {};
    return {
      ...(supplied ?? {}),
      session_id: this.syntheticSessionId,
      turn_id: turnId,
      caller_provenance: resolveMcpCallerProvenance(
        this.declaredCallerProvenance,
        this.initializeClientInfo,
      ),
      client_info:
        this.initializeClientInfo === null
          ? null
          : structuredClone(this.initializeClientInfo),
      identity_synthetic: true,
      "x-codex-turn-metadata": {
        ...suppliedTurnMetadata,
        session_id: this.syntheticSessionId,
        turn_id: turnId,
      },
    };
  }

  private write(response: JsonRpcResponse): void {
    if (this.closed) return;
    this.output.write(`${JSON.stringify(response)}\n`);
  }

  public requestClient(
    method: string,
    params: Record<string, unknown>,
    signal?: AbortSignal,
  ): Promise<unknown> {
    if (!this.formElicitationSupported)
      return Promise.reject(
        new Error("form elicitation is not supported by the MCP client"),
      );
    if (this.closed) return Promise.reject(new Error("MCP host closed"));
    const id = `node-repl-client-${this.nextRequestId++}`;
    if (signal?.aborted === true)
      return Promise.reject(clientAbortReason(signal));
    this.output.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    return new Promise<unknown>((resolvePromise, rejectPromise) => {
      const onAbort =
        signal === undefined
          ? undefined
          : () => {
              const pending = this.clientRequests.get(id);
              if (pending === undefined) return;
              this.clientRequests.delete(id);
              rejectPromise(clientAbortReason(signal));
            };
      this.clientRequests.set(id, {
        resolve: resolvePromise,
        reject: rejectPromise,
        ...(signal === undefined ? {} : { signal }),
        ...(onAbort === undefined ? {} : { onAbort }),
      });
      if (onAbort !== undefined)
        signal?.addEventListener("abort", onAbort, { once: true });
      if (signal?.aborted === true) onAbort?.();
    });
  }

  private resolveClientRequest(response: JsonObject): void {
    const id = response.id;
    if (typeof id !== "string" && typeof id !== "number") return;
    const pending = this.clientRequests.get(id);
    if (pending === undefined) return;
    this.clientRequests.delete(id);
    if (pending.signal !== undefined && pending.onAbort !== undefined)
      pending.signal.removeEventListener("abort", pending.onAbort);
    if ("error" in response) {
      const error = response.error;
      const code = isObject(error) && typeof error.code === "number" ? error.code : null;
      const message =
        isObject(error) && typeof error.message === "string"
          ? error.message
          : "unknown MCP client error";
      pending.reject(
        new Error(
          `MCP client request failed${code === null ? "" : ` (${code})`}: ${message}`,
        ),
      );
    } else pending.resolve(response.result);
  }
}

function nonEmptyString(value: unknown): boolean {
  return typeof value === "string" && value.trim().length > 0;
}

function hasUsableCallerIdentity(meta: Record<string, unknown>): boolean {
  if (nonEmptyString(meta.session_id) && nonEmptyString(meta.turn_id)) return true;
  const nested = meta["x-codex-turn-metadata"];
  return (
    isObject(nested) &&
    nonEmptyString(nested.session_id) &&
    nonEmptyString(nested.turn_id)
  );
}

function clientAbortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error
    ? signal.reason
    : new Error("kernel generation terminated");
}

export function validateJsArguments(value: unknown): {
  code: string;
  timeoutMs?: number;
} {
  if (!isObject(value)) throw new Error("js arguments must be an object");
  const keys = new Set(["code", "timeout_ms", "title"]);
  for (const key of Object.keys(value))
    if (!keys.has(key))
      throw new Error(`js arguments contain unknown property: ${key}`);
  if (typeof value.code !== "string" || value.code.length === 0)
    throw new Error("js.code must be a non-empty string");
  if (
    value.title !== undefined &&
    (typeof value.title !== "string" ||
      value.title.length < 1 ||
      value.title.length > 80)
  )
    throw new Error("js.title must contain 1 to 80 characters");
  if (value.timeout_ms !== undefined) {
    if (typeof value.timeout_ms !== "number")
      throw new Error("timeout_ms must be a positive integer");
    validateTimeout(value.timeout_ms);
    return { code: value.code, timeoutMs: value.timeout_ms };
  }
  return { code: value.code };
}

function validateModuleDirArguments(value: unknown): string {
  if (!isObject(value) || typeof value.path !== "string" || value.path.length === 0)
    throw new Error("js_add_node_module_dir.path must be a non-empty absolute path");
  if (!value.path.startsWith("/"))
    throw new Error("js_add_node_module_dir.path must be absolute");
  for (const key of Object.keys(value))
    if (key !== "path")
      throw new Error(
        `js_add_node_module_dir arguments contain unknown property: ${key}`,
      );
  return value.path;
}

function validateEmptyObject(value: unknown, tool: string): void {
  if (!isObject(value) || Object.keys(value).length !== 0)
    throw new Error(`${tool} arguments must be an empty object`);
}

function imageContent(imageUrl: string): Record<string, unknown> {
  const comma = imageUrl.indexOf(",");
  if (!imageUrl.startsWith("data:") || comma < 0)
    throw new Error("invalid image data URL");
  const header = imageUrl.slice(5, comma);
  const [mimeType = "", encoding] = header.split(";");
  if (encoding !== "base64" || mimeType.length === 0)
    throw new Error("image data URL must be base64 encoded");
  return {
    type: "image",
    data: imageUrl.slice(comma + 1),
    mimeType,
    _meta: { "codex/imageDetail": "original" },
  };
}

function toolSuccess(
  id: RequestId,
  content: Array<Record<string, unknown>>,
): JsonRpcResponse {
  return { jsonrpc: "2.0", id, result: { content, isError: false } };
}

function toolError(id: RequestId | null, message: string): JsonRpcResponse {
  return {
    jsonrpc: "2.0",
    id,
    result: { content: [{ type: "text", text: message }], isError: true },
  };
}

function makeError(
  id: RequestId | null,
  code: number,
  message: string,
): JsonRpcResponse {
  return { jsonrpc: "2.0", id, error: { code, message } };
}

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isRequest(value: unknown): value is JsonRpcRequest {
  return isObject(value) && value.jsonrpc === "2.0" && typeof value.method === "string";
}

function cancellationRequestId(value: unknown): RuntimeRequestId | undefined {
  if (!isObject(value)) return undefined;
  return typeof value.requestId === "string" || typeof value.requestId === "number"
    ? value.requestId
    : undefined;
}

function initializeClientInfo(params: unknown): Record<string, unknown> | null {
  if (!isObject(params) || params.clientInfo === undefined) return null;
  if (!isObject(params.clientInfo)) return null;
  return structuredClone(params.clientInfo);
}

export function resolveMcpCallerProvenance(
  declared: string | undefined,
  clientInfo: Record<string, unknown> | null,
): McpCallerProvenance {
  if (declared !== undefined && declared.trim().length > 0) {
    const normalized = declared.trim().toLowerCase();
    if (!MCP_CALLER_PROVENANCE_SET.has(normalized))
      throw new Error(
        `SKY_CUA_MCP_CALLER_PROVENANCE must be one of ${MCP_CALLER_PROVENANCE_VALUES.join(", ")}`,
      );
    return normalized as McpCallerProvenance;
  }
  const clientName =
    typeof clientInfo?.name === "string"
      ? clientInfo.name.trim().toLowerCase()
      : "";
  if (clientName.includes("openclaw")) return "openclaw";
  if (clientName.includes("opencode")) return "opencode";
  if (clientName.includes("codex") || clientName.includes("chatgpt"))
    return "codex_desktop";
  return "direct_mcp";
}

export function createStdioServer(): McpServer {
  return new McpServer({ input: stdin, output: stdout });
}
