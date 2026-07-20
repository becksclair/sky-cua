export type NativePipeConnection = {
  end(): void;
  on(event: "data", listener: (chunk: Uint8Array) => void): void;
  on(event: "error", listener: (error: unknown) => void): void;
  on(event: "close", listener: () => void): void;
  write(data: Uint8Array): void;
};

export type NodeReplBridge = {
  env?: Record<string, string | undefined>;
  requestMeta?: Record<string, unknown>;
  nativePipe?: {
    createConnection(path: string): Promise<NativePipeConnection>;
    listDirectory?(path: string): Promise<string[]>;
  };
  emitImage?(image: Uint8Array | string): Promise<void> | void;
  setResponseMeta?(meta: Record<string, unknown>): void;
  write?(value: unknown): void;
};

declare global {
  // Injected lexically by the trusted cua_node module context. It is deliberately
  // richer than globals.nodeRepl supplied by model code.
  const nodeRepl: NodeReplBridge | undefined;
}

export function trustedNodeRepl(fallback: BrowserGlobals): NodeReplBridge | undefined {
  return typeof nodeRepl === "undefined" ? fallback.nodeRepl : nodeRepl;
}

export type BrowserGlobals = typeof globalThis & {
  agent?: unknown;
  display?: (value: unknown) => Promise<void>;
  nodeRepl?: NodeReplBridge;
};

export const ALLOWED_PROVENANCE = [
  "codex_desktop",
  "openclaw",
  "opencode",
  "direct_mcp",
] as const;

export type CallerProvenance = (typeof ALLOWED_PROVENANCE)[number];

export function callerContext(globals: BrowserGlobals): Record<string, unknown> {
  const meta = globals.nodeRepl?.requestMeta ?? {};
  const configured = globals.nodeRepl?.env?.SKY_CUA_MCP_CALLER_PROVENANCE;
  const provenance = ALLOWED_PROVENANCE.includes(configured as CallerProvenance)
    ? configured
    : "direct_mcp";
  const clientInfo = meta.client_info ?? meta.clientInfo;
  const identitySynthetic = meta.identity_synthetic ?? meta.identitySynthetic;
  return {
    ...meta,
    caller_provenance: provenance,
    ...(clientInfo === undefined ? {} : { client_info: clientInfo }),
    ...(identitySynthetic === undefined ? {} : { identity_synthetic: identitySynthetic }),
  };
}
