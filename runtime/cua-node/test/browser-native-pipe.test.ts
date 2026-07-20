import { createHash } from "node:crypto";
import { unlinkSync } from "node:fs";
import { createServer, type Server, type Socket } from "node:net";
import {
  chmod,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { strict as assert } from "node:assert";
import { test } from "bun:test";
import browserFixture from "./fixtures/browser-native-pipe.json";
import { McpServer } from "../src/host/mcp-server.ts";
import {
  MAX_NATIVE_PIPE_REQUEST_HISTORY,
  NativePipeBroker,
  validateUnixSocketPath,
} from "../src/host/native-pipe-broker.ts";
import {
  parseTrustedBrowserClientSha256s,
  TRUSTED_CODE_PATHS_ENV,
  TrustedModulePolicy,
} from "../src/host/trusted-module-policy.ts";
import { RuntimeManager } from "../src/host/runtime-manager.ts";
import { KERNEL_SOURCE } from "../src/kernel/kernel.ts";
import { TEST_NODE_PATH } from "./test-node-path.ts";

const PACKAGE_ROOT = resolve(
  import.meta.dir,
  "fixtures",
  "browser-native-pipe-package",
);
const PACKAGE_ENTRYPOINT = resolve(
  PACKAGE_ROOT,
  "node_modules",
  "browser-native-pipe-fixture",
  "index.mjs",
);
const INSTALLED_BROWSER_CLIENT = resolve(
  process.env.CUA_NODE_BROWSER_CLIENT_PATH ??
    resolve(import.meta.dir, "../../../packages/browser-use/build/browser-client.mjs"),
);

type Peer = {
  server: Server;
  socketPath: string;
  connectionCount: () => number;
  requests: Array<Record<string, unknown>>;
  mode:
    | "roundtrip"
    | "persistent"
    | "callback"
    | "early-close"
    | "oversize"
    | "replace-after-connect"
    | "browser-client";
  sockets: Set<Socket>;
};

type Response = {
  result?: {
    content?: Array<Record<string, unknown>>;
    isError?: boolean;
  };
};

function result(response: Response): Response["result"] {
  assert.ok(response.result !== undefined);
  return response.result;
}

function js(
  id: number,
  code: string,
): {
  jsonrpc: "2.0";
  id: number;
  method: "tools/call";
  params: Record<string, unknown>;
} {
  return {
    jsonrpc: "2.0",
    id,
    method: "tools/call",
    params: { name: "js", arguments: { code } },
  };
}

function browserJs(id: number, code: string, turnId: string) {
  const request = js(id, code);
  request.params._meta = {
    session_id: "session-browser-client",
    turn_id: turnId,
    "x-codex-turn-metadata": {
      session_id: "session-browser-client",
      turn_id: turnId,
    },
  };
  return request;
}

test("trusted browser policy parses fail-closed and hashes the installed exact client bytes", async () => {
  assert.deepEqual([...parseTrustedBrowserClientSha256s(undefined)], []);
  assert.deepEqual([...parseTrustedBrowserClientSha256s("bad," + "a".repeat(64))], []);
  assert.deepEqual(
    [...parseTrustedBrowserClientSha256s(" " + "A".repeat(64) + " ")],
    ["a".repeat(64)],
  );

  const bytes = await readFile(INSTALLED_BROWSER_CLIENT);
  const actualHash = createHash("sha256").update(bytes).digest("hex");
  assert.equal(actualHash, browserFixture.client_sha256);
  const policy = new TrustedModulePolicy({
    env: { NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: actualHash },
  });
  const loaded = policy.readEntrypoint(INSTALLED_BROWSER_CLIENT);
  assert.equal(loaded.sha256, actualHash);
  assert.deepEqual(Buffer.from(loaded.bytes), bytes);
  assert.equal(loaded.trusted, true);
  assert.equal(KERNEL_SOURCE.includes("@oai/sky"), false);
  assert.equal(KERNEL_SOURCE.includes("__codexNativePipe"), false);

  const modified = Buffer.from(bytes);
  modified[0] = (modified[0] ?? 0) ^ 1;
  const modifiedLoad = policy.evaluate(INSTALLED_BROWSER_CLIENT, modified, true, false);
  assert.equal(modifiedLoad.trusted, false);
  assert.notEqual(modifiedLoad.sha256, actualHash);
});

test("trusted browser code cannot recover the host context through builtins", async () => {
  const root = await mkdtemp(join(tmpdir(), "cua-node-trusted-builtins-"));
  const entrypoint = join(root, "browser-client.mjs");
  const source = Buffer.from(String.raw`
export async function contextBreakingBuiltinsUnavailable() {
  let importDenied = false;
  try { await import("node:vm"); } catch { importDenied = true; }
  return importDenied && process.getBuiltinModule("vm") === undefined;
}
`);
  await writeFile(entrypoint, source);
  const digest = createHash("sha256").update(source).digest("hex");
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
    env: {
      NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
    },
  });
  const server = new McpServer({ manager });
  try {
    const response = result(
      (await server.dispatch(
        js(
          1,
          `const browser = await import(${JSON.stringify(pathToFileURL(entrypoint).href)}); nodeRepl.write(String(await browser.contextBreakingBuiltinsUnavailable()))`,
        ),
      )) as Response,
    );
    assert.equal(response?.isError, false);
    assert.equal(response?.content?.[0]?.text, "true");
  } finally {
    await server.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("trusted directory authorization uses exact opened bytes and rejects symlink escapes and swaps", async () => {
  const root = await mkdtemp(join(tmpdir(), "cua-node-trusted-root-"));
  const outside = await mkdtemp(join(tmpdir(), "cua-node-trusted-outside-"));
  const trustedFile = join(root, "trusted.mjs");
  const outsideFile = join(outside, "outside.mjs");
  const goodBytes = Buffer.from("export const trusted = true;\n", "utf8");
  const outsideBytes = Buffer.from("export const trusted = false;\n", "utf8");
  try {
    await writeFile(trustedFile, goodBytes);
    await writeFile(outsideFile, outsideBytes);
    const policy = new TrustedModulePolicy({
      env: { [TRUSTED_CODE_PATHS_ENV]: root },
    });

    assert.equal(
      policy.evaluate(trustedFile, outsideBytes, false, false).trusted,
      false,
    );
    assert.equal(policy.isTrustedDirectoryPath(trustedFile), true);
    assert.equal(policy.evaluate(trustedFile, goodBytes, false, false).trusted, true);

    assert.equal(policy.isTrustedDirectoryPath(trustedFile), true);
    await writeFile(trustedFile, outsideBytes);
    await writeFile(trustedFile, goodBytes);
    assert.equal(
      policy.evaluate(trustedFile, outsideBytes, false, false).trusted,
      false,
    );

    await rm(trustedFile, { force: true });
    await symlink(outsideFile, trustedFile);
    assert.equal(policy.isTrustedDirectoryPath(trustedFile), false);
    assert.throws(
      () => policy.readEntrypoint(trustedFile),
      /ELOOP|too many symbolic links/u,
    );

    const linkedDirectory = join(root, "linked-directory");
    await symlink(outside, linkedDirectory);
    assert.equal(
      policy.isTrustedDirectoryPath(join(linkedDirectory, "outside.mjs")),
      false,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
});

test("the exact installed Browser client bytes compile through the trusted Node REPL loader", async () => {
  const root = await mkdtemp(join(tmpdir(), "cua-node-installed-browser-client-"));
  const packageRoot = join(root, "node_modules", "browser-use-installed");
  await mkdir(packageRoot, { recursive: true });
  await cp(INSTALLED_BROWSER_CLIENT, join(packageRoot, "browser-client.mjs"));
  await writeFile(
    join(packageRoot, "package.json"),
    JSON.stringify({
      name: "browser-use-installed",
      type: "module",
      exports: { ".": "./browser-client.mjs" },
    }),
    "utf8",
  );
  const digest = createHash("sha256")
    .update(await readFile(INSTALLED_BROWSER_CLIENT))
    .digest("hex");
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
    env: {
      NODE_REPL_NODE_MODULE_DIRS: root,
      NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
    },
  });
  const server = new McpServer({ manager });
  try {
    const response = await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: 'const browserClient = await import("browser-use-installed"); nodeRepl.write(typeof browserClient.setupBrowserRuntime);',
        },
      },
    });
    const content =
      response?.result &&
      typeof response.result === "object" &&
      "content" in response.result
        ? (response.result.content as Array<{ text?: string }>)[0]?.text
        : undefined;
    assert.equal(content, "function");
    assert.equal(
      response?.result &&
        typeof response.result === "object" &&
        "isError" in response.result
        ? response.result.isError
        : undefined,
      false,
    );
  } finally {
    await server.close();
    await rm(root, { recursive: true, force: true });
  }
});

test("the exact installed Browser client decodes its public screenshot and emits it through nodeRepl", async () => {
  const root = await mkdtemp(join(tmpdir(), "cua-node-browser-screenshot-"));
  const packageRoot = join(root, "node_modules", "browser-use-installed");
  const packageScripts = join(packageRoot, "scripts");
  const pipeRoot = join(tmpdir(), "codex-browser-use");
  await mkdir(packageScripts, { recursive: true });
  await mkdir(pipeRoot, { recursive: true });
  const socketPath = join(
    pipeRoot,
    `wp04-${process.pid}-${root.split("-").at(-1) ?? "fixture"}.sock`,
  );
  const peer = await startPeer(socketPath, "browser-client");
  await cp(INSTALLED_BROWSER_CLIENT, join(packageScripts, "browser-client.mjs"));
  await writeFile(
    join(packageRoot, "package.json"),
    JSON.stringify({
      name: "browser-use-installed",
      type: "module",
      exports: { ".": "./scripts/browser-client.mjs" },
    }),
    "utf8",
  );
  const digest = createHash("sha256")
    .update(await readFile(INSTALLED_BROWSER_CLIENT))
    .digest("hex");
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
    env: {
      NODE_REPL_NODE_MODULE_DIRS: root,
      NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
      SKY_CUA_CODEX_BROWSER_SOCKET_PATH: socketPath,
      SKY_CUA_MCP_CALLER_PROVENANCE: "codex_desktop",
    },
  });
  const server = new McpServer({ manager });
  try {
    const setup = result(
      (await server.dispatch(
        browserJs(
          1,
          'globalThis.exactBrowserClient = await import("browser-use-installed"); await exactBrowserClient.setupBrowserRuntime({ globals: globalThis }); globalThis.exactBrowser = await agent.browsers.get("iab"); nodeRepl.write("exact-client-ready");',
          "turn-browser-client-setup",
        ),
      )) as Response,
    );
    assert.deepEqual(setup?.content, [{ type: "text", text: "exact-client-ready" }]);

    const screenshot = result(
      (await server.dispatch(
        browserJs(
          2,
          "globalThis.exactTab = await exactBrowser.tabs.new(); globalThis.exactScreenshot = await exactTab.screenshot(); await nodeRepl.emitImage(exactScreenshot); nodeRepl.write(JSON.stringify({ byteLength: exactScreenshot.byteLength, constructor: exactScreenshot.constructor.name }));",
          "turn-browser-client-screenshot",
        ),
      )) as Response,
    );
    assert.deepEqual(screenshot?.content, [
      {
        type: "text",
        text: JSON.stringify({ byteLength: 8, constructor: "Uint8Array" }),
      },
      {
        type: "image",
        data: "iVBORw0KGgo=",
        mimeType: "image/png",
        _meta: { "codex/imageDetail": "original" },
      },
    ]);
    assert.equal(peer.connectionCount(), 1);
    assert.equal(peer.requests[0]?.method, "getInfo");
    assert.equal(peer.requests[1]?.method, "createTab");
    assert.ok(
      peer.requests.some(
        (request) =>
          request.method === "executeCdp" &&
          JSON.stringify(request.params).includes("Page.captureScreenshot"),
      ),
      "the public screenshot surface must request Page.captureScreenshot",
    );
  } finally {
    await server.close();
    await closePeer(peer);
    await rm(root, { recursive: true, force: true });
  }
});

test("NativePipeBroker rejects stale token, generation, and execution requests before path access", async () => {
  const broker = new NativePipeBroker();
  broker.setGeneration("generation-a");
  broker.setActiveExecution("exec-a");
  const request = {
    id: "native-pipe-0",
    token: "generation-a",
    generation: "generation-a",
    execId: "exec-a",
    operation: "connect" as const,
    connectionId: "connection-a",
    path: "/does/not/exist",
  };
  await assert.rejects(
    broker.handle({ ...request, token: "old-token" }),
    /token is invalid/u,
  );
  await assert.rejects(
    broker.handle({ ...request, generation: "old-generation" }),
    /generation is stale/u,
  );
  await assert.rejects(
    broker.handle({ ...request, execId: "old-exec" }),
    /exec context not found/u,
  );
});

test("NativePipeBroker rejects non-Unix and malformed socket paths", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-path-")).then(resolve);
  const regularFile = join(directory, "regular-file");
  await writeFile(regularFile, "not a socket", "utf8");
  try {
    await assert.rejects(validateUnixSocketPath("relative.sock"), /must be absolute/u);
    await assert.rejects(
      validateUnixSocketPath(join(directory, "missing-parent", "socket")),
      /no parent directory/u,
    );
    await assert.rejects(validateUnixSocketPath(regularFile), /not a socket/u);
    await assert.rejects(
      validateUnixSocketPath(join(directory, "x".repeat(108))),
      /file name is too long/u,
    );
    await assert.rejects(validateUnixSocketPath(`${directory}/`), /no file name/u);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("NativePipeBroker accepts a permissive same-user socket", async () => {
  const socketPath = join(
    await mkdtemp(join(tmpdir(), "cua-node-owner-")),
    "browser.sock",
  );
  const peer = await startPeer(socketPath, "roundtrip");
  await chmod(socketPath, 0o666);
  const broker = new NativePipeBroker();
  broker.setGeneration("generation-owner");
  broker.setActiveExecution("exec-owner");
  try {
    await broker.handle({
      id: "native-pipe-owner-0",
      token: "generation-owner",
      generation: "generation-owner",
      execId: "exec-owner",
      operation: "connect",
      connectionId: "connection-owner",
      path: socketPath,
    });
    assert.equal(peer.connectionCount(), 1);
  } finally {
    await closePeer(peer);
  }
});

test("NativePipeBroker keeps a trusted connection alive between executions", async () => {
  const socketPath = join(
    await mkdtemp(join(tmpdir(), "cua-node-persistent-owner-")),
    "browser.sock",
  );
  const peer = await startPeer(socketPath, "roundtrip");
  const broker = new NativePipeBroker();
  const common = {
    token: "generation-persistent",
    generation: "generation-persistent",
    connectionId: "connection-persistent",
  };
  broker.setGeneration(common.token);
  broker.setActiveExecution("exec-a");
  try {
    await broker.handle({
      ...common,
      id: "persistent-connect",
      execId: "exec-a",
      operation: "connect",
      path: socketPath,
    });
    broker.setActiveExecution(null);
    await broker.handle({
      ...common,
      id: "persistent-background-a",
      execId: "exec-a",
      operation: "write",
      dataBase64: encodeFrame({ phase: "background-a" }).toString("base64"),
    });
    broker.setActiveExecution("exec-b");
    await broker.handle({
      ...common,
      id: "persistent-active-b",
      execId: "exec-b",
      operation: "write",
      dataBase64: encodeFrame({ phase: "active-b" }).toString("base64"),
    });
    broker.setActiveExecution(null);
    await broker.handle({
      ...common,
      id: "persistent-background-b",
      execId: "exec-b",
      operation: "write",
      dataBase64: encodeFrame({ phase: "background-b" }).toString("base64"),
    });
    await assert.rejects(
      broker.handle({
        ...common,
        id: "persistent-stale-a",
        execId: "exec-a",
        operation: "write",
        dataBase64: encodeFrame({ phase: "stale-a" }).toString("base64"),
      }),
      /exec context not found/u,
    );
    for (let attempt = 0; attempt < 50 && peer.requests.length < 3; attempt += 1)
      await delay(10);
    assert.deepEqual(
      peer.requests.map((request) => request.phase),
      ["background-a", "active-b", "background-b"],
    );
  } finally {
    broker.closeAll();
    await closePeer(peer);
  }
});

test("NativePipeBroker rejects a socket path replaced during connect", async () => {
  const socketPath = join(
    await mkdtemp(join(tmpdir(), "cua-node-race-")),
    "browser.sock",
  );
  const peer = await startPeer(socketPath, "replace-after-connect");
  const broker = new NativePipeBroker();
  broker.setGeneration("generation-race");
  broker.setActiveExecution("exec-race");
  try {
    await assert.rejects(
      broker.handle({
        id: "native-pipe-race-0",
        token: "generation-race",
        generation: "generation-race",
        execId: "exec-race",
        operation: "connect",
        connectionId: "connection-race",
        path: socketPath,
      }),
      /(?:socket changed during connect|path is not a socket)/u,
    );
    assert.equal(peer.requests.length, 0);
  } finally {
    await closePeer(peer);
  }
});

test("first-party Node REPL trusted Browser fixture connects, frames little-endian JSON, and preserves metadata", async () => {
  const socketPath = join(
    await mkdtemp(join(tmpdir(), "cua-node-peer-")),
    "browser.sock",
  );
  const peer = await startPeer(socketPath, "roundtrip");
  const digest = createHash("sha256")
    .update(await readFile(PACKAGE_ENTRYPOINT))
    .digest("hex");
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
    cwd: process.cwd(),
    env: {
      NODE_REPL_NODE_MODULE_DIRS: PACKAGE_ROOT,
      NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
    },
  });
  const server = new McpServer({ manager });
  try {
    const response = await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: `const browser = await import("browser-native-pipe-fixture"); const result = await browser.browserRoundTrip(${JSON.stringify(socketPath)}, { jsonrpc: "2.0", id: 7, method: "browser.inspect" }); nodeRepl.write(JSON.stringify({ result, surface: browser.browserNativePipeSurface() }));`,
        },
        _meta: {
          session_id: "session-browser-fixture",
          turn_id: "turn-browser-fixture",
          "x-codex-turn-metadata": {
            session_id: "session-browser-fixture",
            turn_id: "turn-browser-fixture",
          },
        },
      },
    });
    const text =
      response?.result &&
      typeof response.result === "object" &&
      "content" in response.result
        ? (response.result.content as Array<{ text?: string }>)[0]?.text
        : undefined;
    assert.equal(
      response?.result &&
        typeof response.result === "object" &&
        "isError" in response.result
        ? response.result.isError
        : undefined,
      false,
    );
    const output = JSON.parse(text ?? "null") as {
      result: { request_meta: Record<string, unknown> };
      surface: {
        inheritedNativePipe: boolean;
        hasLegacyImportMetaBridge: boolean;
      };
    };
    assert.equal(output.surface.inheritedNativePipe, true);
    assert.equal(output.surface.hasLegacyImportMetaBridge, false);
    assert.deepEqual(output.result.request_meta, {
      session_id: "session-browser-fixture",
      turn_id: "turn-browser-fixture",
      "x-codex-turn-metadata": {
        session_id: "session-browser-fixture",
        turn_id: "turn-browser-fixture",
      },
    });
    assert.equal(peer.requests.length, 1);
    assert.equal(peer.requests[0]?.method, "browser.inspect");
  } finally {
    await server.close();
    await closePeer(peer);
  }
});

test("native-pipe callbacks retain the JavaScript execution context", async () => {
  const socketPath = join(
    await mkdtemp(join(tmpdir(), "cua-node-callback-")),
    "browser.sock",
  );
  const peer = await startPeer(socketPath, "callback");
  const digest = createHash("sha256")
    .update(await readFile(PACKAGE_ENTRYPOINT))
    .digest("hex");
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
    cwd: process.cwd(),
    env: {
      NODE_REPL_NODE_MODULE_DIRS: PACKAGE_ROOT,
      NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
    },
  });
  const server = new McpServer({ manager });
  try {
    const response = await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: `const browser = await import("browser-native-pipe-fixture"); const value = await browser.browserCallbackContext(${JSON.stringify(socketPath)}); nodeRepl.write(value);`,
        },
      },
    });
    const content =
      response?.result &&
      typeof response.result === "object" &&
      "content" in response.result
        ? (response.result.content as Array<{ text?: string }>)[0]?.text
        : undefined;
    assert.equal(content, "native-callback-context|callback-complete");
  } finally {
    await server.close();
    await closePeer(peer);
  }
});

test("persistent Browser callbacks can reply while idle and during a later cell", async () => {
  const socketPath = join(
    await mkdtemp(join(tmpdir(), "cua-node-persistent-callback-")),
    "browser.sock",
  );
  const peer = await startPeer(socketPath, "persistent");
  const digest = createHash("sha256")
    .update(await readFile(PACKAGE_ENTRYPOINT))
    .digest("hex");
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
    cwd: process.cwd(),
    env: {
      NODE_REPL_NODE_MODULE_DIRS: PACKAGE_ROOT,
      NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
    },
  });
  const server = new McpServer({ manager });
  const sendPeerEvent = (phase: string): void => {
    for (const socket of peer.sockets) socket.write(encodeFrame({ phase }));
  };
  const waitForRequests = async (count: number): Promise<void> => {
    for (let attempt = 0; attempt < 100 && peer.requests.length < count; attempt += 1)
      await delay(10);
    assert.equal(peer.requests.length, count);
  };
  try {
    const connected = result(
      (await server.dispatch(
        js(
          1,
          `globalThis.browserFixture = await import("browser-native-pipe-fixture"); nodeRepl.write(await browserFixture.browserPersistentConnect(${JSON.stringify(socketPath)}));`,
        ),
      )) as Response,
    );
    assert.deepEqual(connected?.content, [
      { type: "text", text: "persistent-connected" },
    ]);

    sendPeerEvent("idle-a");
    await waitForRequests(1);
    assert.deepEqual(peer.requests[0], { acknowledged: "idle-a" });

    const laterCell = server.dispatch(
      js(
        2,
        "await new Promise((resolve) => setTimeout(resolve, 100)); nodeRepl.write('cell-b')",
      ),
    );
    await delay(20);
    sendPeerEvent("active-b");
    await waitForRequests(2);
    const later = result((await laterCell) as Response);
    assert.deepEqual(later?.content, [{ type: "text", text: "cell-b" }]);
    assert.deepEqual(peer.requests[1], { acknowledged: "active-b" });

    await server.dispatch(js(3, "browserFixture.browserPersistentClose()"));
    assert.equal(peer.connectionCount(), 1);
  } finally {
    await server.close();
    await closePeer(peer);
  }
});

test("wrong, missing, malformed, modified, and stale Browser hashes produce zero peer connections", async () => {
  const cases: Array<{
    name: string;
    hash: string | undefined;
    mutation: "none" | "before-manager" | "after-manager";
  }> = [
    { name: "wrong", hash: "0".repeat(64), mutation: "none" },
    { name: "missing", hash: undefined, mutation: "none" },
    {
      name: "malformed",
      hash: "0".repeat(64) + ",not-a-sha",
      mutation: "none",
    },
    {
      name: "modified",
      hash: createHash("sha256")
        .update(await readFile(PACKAGE_ENTRYPOINT))
        .digest("hex"),
      mutation: "before-manager",
    },
    {
      name: "stale",
      hash: createHash("sha256")
        .update(await readFile(PACKAGE_ENTRYPOINT))
        .digest("hex"),
      mutation: "after-manager",
    },
  ];
  for (const testCase of cases) {
    const root = await mkdtemp(join(tmpdir(), `cua-node-hash-${testCase.name}-`));
    const packageRoot = join(root, "node_modules", "browser-native-pipe-fixture");
    await cp(
      resolve(PACKAGE_ROOT, "node_modules", "browser-native-pipe-fixture"),
      packageRoot,
      { recursive: true },
    );
    const entrypoint = join(packageRoot, "index.mjs");
    if (testCase.mutation === "before-manager") {
      await writeFile(
        entrypoint,
        `${await readFile(entrypoint, "utf8")}\n// one byte changed\n`,
        "utf8",
      );
    }
    const socketPath = join(root, "browser.sock");
    const peer = await startPeer(socketPath, "roundtrip");
    const manager = new RuntimeManager({
      allowHostNode: true,
      nodePath: TEST_NODE_PATH,
      runtimeMetadata: null,
      env: {
        NODE_REPL_NODE_MODULE_DIRS: root,
        ...(testCase.hash === undefined
          ? {}
          : { NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: testCase.hash }),
      },
    });
    const server = new McpServer({ manager });
    try {
      if (testCase.mutation === "after-manager")
        await writeFile(
          entrypoint,
          `${await readFile(entrypoint, "utf8")}\n// trusted hash became stale\n`,
          "utf8",
        );
      const response = await server.dispatch({
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: {
          name: "js",
          arguments: {
            code: `const browser = await import("browser-native-pipe-fixture"); await browser.browserRoundTrip(${JSON.stringify(socketPath)}, { method: "should-not-connect" });`,
          },
        },
      });
      assert.equal(
        response?.result &&
          typeof response.result === "object" &&
          "isError" in response.result
          ? response.result.isError
          : undefined,
        true,
        testCase.name,
      );
      assert.equal(peer.connectionCount(), 0, testCase.name);
    } finally {
      await server.close();
      await closePeer(peer);
      await rm(root, { recursive: true, force: true });
    }
  }
});

test("a symlinked trusted Browser entrypoint fails before any peer connection", async () => {
  const root = await mkdtemp(join(tmpdir(), "cua-node-hash-symlink-"));
  const socketPath = join(root, "browser.sock");
  const peer = await startPeer(socketPath, "roundtrip");
  const linkedEntrypoint = join(root, "browser-client.mjs");
  const digest = createHash("sha256")
    .update(await readFile(PACKAGE_ENTRYPOINT))
    .digest("hex");
  await symlink(PACKAGE_ENTRYPOINT, linkedEntrypoint);
  const policy = new TrustedModulePolicy({
    env: { NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest },
  });
  try {
    assert.throws(
      () => policy.readEntrypoint(linkedEntrypoint),
      /ELOOP|too many symbolic links/u,
    );
    assert.equal(peer.connectionCount(), 0);
  } finally {
    await closePeer(peer);
    await rm(root, { recursive: true, force: true });
  }
});

test("Browser framing rejects an oversized response and replays an early close to pending work", async () => {
  for (const mode of ["oversize", "early-close"] as const) {
    const root = await mkdtemp(join(tmpdir(), `cua-node-frame-${mode}-`));
    const socketPath = join(root, "browser.sock");
    const peer = await startPeer(socketPath, mode);
    const digest = createHash("sha256")
      .update(await readFile(PACKAGE_ENTRYPOINT))
      .digest("hex");
    const manager = new RuntimeManager({
      allowHostNode: true,
      nodePath: TEST_NODE_PATH,
      runtimeMetadata: null,
      env: {
        NODE_REPL_NODE_MODULE_DIRS: PACKAGE_ROOT,
        NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
      },
    });
    const server = new McpServer({ manager });
    try {
      const response = await server.dispatch({
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: {
          name: "js",
          arguments: {
            code: `const browser = await import("browser-native-pipe-fixture"); await browser.browserRoundTrip(${JSON.stringify(socketPath)}, { method: "${mode}" });`,
          },
        },
      });
      const content =
        response?.result &&
        typeof response.result === "object" &&
        "content" in response.result
          ? (response.result.content as Array<{ text?: string }>)[0]?.text
          : "";
      assert.equal(
        response?.result &&
          typeof response.result === "object" &&
          "isError" in response.result
          ? response.result.isError
          : undefined,
        true,
      );
      assert.match(
        content ?? "",
        mode === "oversize"
          ? /NATIVE_PIPE_FRAME_TOO_LARGE/u
          : /NATIVE_PIPE_EARLY_CLOSE/u,
      );
    } finally {
      await server.close();
      await closePeer(peer);
      await rm(root, { recursive: true, force: true });
    }
  }
});

test("NativePipeBroker bounds request history and removes every closed connection", async () => {
  const socketPath = join(
    await mkdtemp(join(tmpdir(), "cua-node-cycles-")),
    "browser.sock",
  );
  const peer = await startPeer(socketPath, "roundtrip");
  const broker = new NativePipeBroker();
  broker.setGeneration("generation-cycles");
  broker.setActiveExecution("exec-cycles");
  try {
    for (let index = 0; index < MAX_NATIVE_PIPE_REQUEST_HISTORY + 32; index += 1) {
      const suffix = String(index);
      await broker.handle({
        id: `native-pipe-connect-${suffix}`,
        token: "generation-cycles",
        generation: "generation-cycles",
        execId: "exec-cycles",
        operation: "connect",
        connectionId: `connection-${suffix}`,
        path: socketPath,
      });
      await broker.handle({
        id: `native-pipe-close-${suffix}`,
        token: "generation-cycles",
        generation: "generation-cycles",
        execId: "exec-cycles",
        operation: "close",
        connectionId: `connection-${suffix}`,
      });
    }
    const state = broker as unknown as {
      connections: Map<string, unknown>;
      requestIds: Map<string, true>;
    };
    assert.equal(state.connections.size, 0);
    assert.ok(state.requestIds.size <= MAX_NATIVE_PIPE_REQUEST_HISTORY);
  } finally {
    await closePeer(peer);
  }
});

async function startPeer(socketPath: string, mode: Peer["mode"]): Promise<Peer> {
  const requests: Array<Record<string, unknown>> = [];
  let connections = 0;
  const sockets = new Set<Socket>();
  const server = createServer((socket) => {
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
    connections += 1;
    if (mode === "replace-after-connect") {
      unlinkSync(socketPath);
      return;
    }
    if (mode === "early-close") {
      setTimeout(() => socket.destroy(), 10);
      return;
    }
    if (mode === "oversize") {
      socket.write(Buffer.from([1, 0, 128, 0]));
      return;
    }
    if (mode === "callback") {
      socket.write(encodeFrame({ method: "browser.callback" }));
      return;
    }
    let buffered = Buffer.alloc(0);
    socket.on("data", (chunk: Buffer) => {
      buffered = Buffer.concat([buffered, chunk]);
      while (buffered.length >= 4) {
        const length = buffered.readUInt32LE(0);
        if (buffered.length < length + 4) return;
        const payload = JSON.parse(
          buffered.subarray(4, length + 4).toString("utf8"),
        ) as Record<string, unknown>;
        requests.push(payload);
        buffered = buffered.subarray(length + 4);
        if (mode === "browser-client") {
          const method = payload.method;
          const params = payload.params;
          const commandType =
            params !== null && typeof params === "object"
              ? (params as Record<string, unknown>).type
              : undefined;
          let result: unknown;
          if (method === "getInfo")
            result = {
              type: "iab",
              name: "WP-04 deterministic fake browser",
              capabilities: {},
              metadata: { codexSessionId: "session-browser-client" },
            };
          else if (method === "createTab")
            result = {
              id: 41,
              active: true,
              title: "WP-04",
              url: "about:blank",
            };
          else if (method === "getTabs")
            result = [
              {
                id: 41,
                active: true,
                title: "WP-04",
                url: "about:blank",
              },
            ];
          else if (method === "executeCdp") {
            const serializedParams = JSON.stringify(params);
            if (serializedParams.includes("Page.getLayoutMetrics"))
              result = {
                cssVisualViewport: {
                  pageX: 0,
                  pageY: 0,
                  clientWidth: 1,
                  clientHeight: 1,
                },
              };
            else if (serializedParams.includes("Page.captureScreenshot"))
              result = { data: "iVBORw0KGgo=" };
            else if (serializedParams.includes("Runtime.evaluate"))
              result = { result: { value: 1 } };
            else result = {};
          } else if (
            method === "executeUnhandledCommand" &&
            commandType === "tab_screenshot"
          )
            result = { data: "iVBORw0KGgo=" };
          else result = {};
          socket.write(encodeFrame({ jsonrpc: "2.0", id: payload.id, result }));
          continue;
        }
        if (mode === "persistent") continue;
        const reply = encodeFrame({
          ok: true,
          request_meta: payload.request_meta,
        });
        const extra = encodeFrame({ event: "coalesced" });
        socket.write(reply.subarray(0, 2));
        setTimeout(() => socket.write(Buffer.concat([reply.subarray(2), extra])), 0);
      }
    });
  });
  await new Promise<void>((resolvePromise, rejectPromise) => {
    server.once("error", rejectPromise);
    server.listen(socketPath, () => resolvePromise());
  });
  await chmod(socketPath, 0o600);
  return {
    server,
    socketPath,
    connectionCount: () => connections,
    requests,
    mode,
    sockets,
  };
}

function encodeFrame(value: Record<string, unknown>): Buffer {
  const payload = Buffer.from(JSON.stringify(value), "utf8");
  const frame = Buffer.allocUnsafe(4 + payload.length);
  frame.writeUInt32LE(payload.length, 0);
  payload.copy(frame, 4);
  return frame;
}

async function closePeer(peer: Peer): Promise<void> {
  for (const socket of peer.sockets) socket.destroy();
  await new Promise<void>((resolvePromise) => {
    peer.server.close(() => resolvePromise());
  });
  await rm(peer.socketPath, { force: true });
}
