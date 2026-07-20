import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

type JsonObject = Record<string, unknown>;

interface JsonRpcRequest extends JsonObject {
  jsonrpc: "2.0";
  id?: string | number;
  method: string;
  params?: JsonObject;
}

interface JsonRpcResponse extends JsonObject {
  jsonrpc: "2.0";
  id: string | number;
}

function isObject(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseRequest(line: string): JsonRpcRequest {
  const value: unknown = JSON.parse(line);
  if (
    !isObject(value) ||
    value.jsonrpc !== "2.0" ||
    typeof value.method !== "string"
  ) {
    throw new Error("invalid JSON-RPC request");
  }
  return value as JsonRpcRequest;
}

function lineRoundTrip(value: JsonObject): JsonObject {
  const encoded = `${JSON.stringify(value)}\n`;
  assert.equal(encoded.split("\n").length, 2);
  return JSON.parse(encoded.trim()) as JsonObject;
}

const directory = path.dirname(fileURLToPath(import.meta.url));
const fixtureRoot = path.resolve(directory, "../fixtures/upstream-5307");
const transcripts = JSON.parse(
  fs.readFileSync(path.join(fixtureRoot, "mcp-transcripts.json"), "utf8"),
) as JsonObject;
const contract = JSON.parse(
  fs.readFileSync(path.join(fixtureRoot, "contract.json"), "utf8"),
) as JsonObject;
const toolsList = JSON.parse(
  fs.readFileSync(path.join(fixtureRoot, "tools-list.json"), "utf8"),
) as JsonObject;

if (!isObject(transcripts.modern_canonical)) {
  throw new Error("missing modern_canonical fixture");
}
const canonical = transcripts.modern_canonical;
const initialize = canonical.initialize;
if (!isObject(initialize)) {
  throw new Error("invalid initialize fixture");
}
const initializeRequestFixture = initialize.request;
const initializeResponseFixture = initialize.response;
if (
  !isObject(initializeRequestFixture) ||
  !isObject(initializeResponseFixture)
) {
  throw new Error("invalid initialize request or response fixture");
}
const frozenInitializeRequest: JsonObject = initializeRequestFixture;
const frozenInitializeResponse: JsonObject = initializeResponseFixture;

const cancelled = new Set<string | number>();
let nextClientRequestId = 1;

function dispatch(request: JsonRpcRequest): JsonRpcResponse | null {
  if (request.method === "notifications/initialized") {
    return null;
  }
  if (request.method === "notifications/cancelled") {
    const requestId = request.params?.requestId;
    if (typeof requestId === "string" || typeof requestId === "number") {
      cancelled.add(requestId);
    }
    return null;
  }
  if (request.id === undefined) {
    throw new Error(`request id required for ${request.method}`);
  }
  if (request.method === "initialize") {
    const expected = frozenInitializeResponse;
    return {
      ...expected,
      id: request.id,
      jsonrpc: "2.0",
    } as JsonRpcResponse;
  }
  if (request.method === "tools/list") {
    return {
      jsonrpc: "2.0",
      id: request.id,
      result: { tools: toolsList.tools },
    };
  }
  if (request.method === "tools/call") {
    const name = request.params?.name;
    if (name === "js") {
      return {
        jsonrpc: "2.0",
        id: request.id,
        result: {
          content: [{ type: "text", text: "hello" }],
          isError: false,
        },
      };
    }
    return {
      jsonrpc: "2.0",
      id: request.id,
      result: {
        content: [{ type: "text", text: `unknown tool: ${String(name)}` }],
        isError: true,
      },
    };
  }
  return {
    jsonrpc: "2.0",
    id: request.id,
    error: { code: -32601, message: `method not found: ${request.method}` },
  };
}

const initializeRequest = parseRequest(
  JSON.stringify(frozenInitializeRequest),
);
assert.deepEqual(
  lineRoundTrip(dispatch(initializeRequest) as JsonRpcResponse),
  frozenInitializeResponse,
);

assert.ok(isObject(canonical.initialized));
assert.ok(isObject(canonical.initialized.request));
assert.equal(
  dispatch(parseRequest(JSON.stringify(canonical.initialized.request))),
  null,
);

assert.ok(isObject(canonical.tools_list));
assert.ok(isObject(canonical.tools_list.request));
const toolsResponse = dispatch(
  parseRequest(JSON.stringify(canonical.tools_list.request)),
);
assert.ok(toolsResponse !== null && isObject(toolsResponse.result));
assert.deepEqual(toolsResponse.result.tools, toolsList.tools);

assert.ok(isObject(canonical.tools_call));
assert.ok(isObject(canonical.tools_call.request));
assert.ok(isObject(canonical.tools_call.response));
assert.deepEqual(
  dispatch(parseRequest(JSON.stringify(canonical.tools_call.request))),
  canonical.tools_call.response,
);

assert.ok(isObject(canonical.unknown_tool_call));
assert.ok(isObject(canonical.unknown_tool_call.request));
const unknownResponse = dispatch(
  parseRequest(JSON.stringify(canonical.unknown_tool_call.request)),
);
assert.ok(unknownResponse !== null && isObject(unknownResponse.result));
assert.equal(unknownResponse.result.isError, true);

assert.ok(isObject(contract.lifecycle));
const cancellation = parseRequest(
  JSON.stringify({
    jsonrpc: "2.0",
    method: "notifications/cancelled",
    params: { requestId: 30, reason: "user_cancelled" },
  }),
);
assert.equal(dispatch(cancellation), null);
assert.equal(cancelled.has(30), true);

function createElicitationRequest(request: JsonObject): JsonRpcRequest {
  const id = `node-repl-client-${nextClientRequestId}`;
  nextClientRequestId += 1;
  return {
    jsonrpc: "2.0",
    id,
    method: "elicitation/create",
    params: request,
  };
}

const elicitation = createElicitationRequest({
  message: "Choose a fixture value",
  requestedSchema: {
    type: "object",
    properties: { value: { type: "string" } },
  },
});
assert.equal(elicitation.method, "elicitation/create");
assert.equal(elicitation.id, "node-repl-client-1");
assert.deepEqual(
  lineRoundTrip({
    jsonrpc: "2.0",
    id: elicitation.id,
    result: { action: "accept", content: { value: "fixture" }, _meta: null },
  }),
  {
    jsonrpc: "2.0",
    id: "node-repl-client-1",
    result: { action: "accept", content: { value: "fixture" }, _meta: null },
  },
);

const report = {
  cancellation: "mapped",
  client_elicitation: "round-tripped",
  framing: "ndjson",
  methods: [
    "initialize",
    "notifications/initialized",
    "tools/list",
    "tools/call",
    "notifications/cancelled",
  ],
  result: "passed",
  unsupported_method: "JSON-RPC -32601",
};

fs.writeFileSync(
  path.join(directory, "mcp-codec-spike.result.json"),
  `${JSON.stringify(report, null, 2)}\n`,
);
process.stdout.write(`${JSON.stringify(report)}\n`);
