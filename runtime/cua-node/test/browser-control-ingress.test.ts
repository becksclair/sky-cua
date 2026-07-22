import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  chmod,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { createServer, type Server, type Socket } from "node:net";
import { endianness, tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { test } from "bun:test";

import { McpServer } from "../src/host/mcp-server.ts";
import { RuntimeManager } from "../src/host/runtime-manager.ts";
import { TEST_NODE_PATH } from "./test-node-path.ts";

type BrowserPeer = {
  connectionCount: () => number;
  requests: Array<Record<string, unknown>>;
  server: Server;
  sockets: Set<Socket>;
};

const CLIENT_PATH = resolve(
  process.env.CUA_NODE_BROWSER_CLIENT_PATH ??
    resolve(
      import.meta.dir,
      "../../../packages/browser-use/build/browser-client.mjs",
    ),
);
const CODEX_APP_BUILD_FLAVOR = "sky-cua-compatibility-test";

function browserJs(id: number, code: string) {
  return {
    jsonrpc: "2.0" as const,
    id,
    method: "tools/call",
    params: {
      name: "js",
      arguments: { code },
      _meta: {
        session_id: "session-control-ingress",
        turn_id: "turn-control-ingress",
        "x-codex-turn-metadata": {
          session_id: "session-control-ingress",
          turn_id: "turn-control-ingress",
        },
      },
    },
  };
}

function toolResult(response: unknown): Record<string, unknown> | null {
  if (
    response === null ||
    typeof response !== "object" ||
    !("result" in response)
  ) {
    return null;
  }
  const result = response.result;
  return result !== null && typeof result === "object"
    ? (result as Record<string, unknown>)
    : null;
}

function encodeFrame(value: Record<string, unknown>): Buffer {
  const payload = Buffer.from(JSON.stringify(value), "utf8");
  const frame = Buffer.allocUnsafe(payload.length + 4);
  if (endianness() === "LE") {
    frame.writeUInt32LE(payload.length, 0);
  } else {
    frame.writeUInt32BE(payload.length, 0);
  }
  payload.copy(frame, 4);
  return frame;
}

async function startBrowserPeer(socketPath: string): Promise<BrowserPeer> {
  const requests: Array<Record<string, unknown>> = [];
  const sockets = new Set<Socket>();
  let connections = 0;
  const server = createServer((socket) => {
    connections += 1;
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
    let buffered = Buffer.alloc(0);
    socket.on("data", (chunk: Buffer) => {
      buffered = Buffer.concat([buffered, chunk]);
      while (buffered.length >= 4) {
        const length =
          endianness() === "LE"
            ? buffered.readUInt32LE(0)
            : buffered.readUInt32BE(0);
        if (buffered.length < length + 4) return;
        const request = JSON.parse(
          buffered.subarray(4, length + 4).toString("utf8"),
        ) as Record<string, unknown>;
        requests.push(request);
        buffered = buffered.subarray(length + 4);
        const result =
          request.method === "getInfo"
            ? {
                type: "extension",
                name: "sky-cua compatibility fixture",
                capabilities: {},
                metadata: {
                  codexAppBuildFlavor: CODEX_APP_BUILD_FLAVOR,
                  codexSessionId: "session-control-ingress",
                  skyCuaBridgeTransport: "extension_native_host",
                },
              }
            : {};
        socket.write(encodeFrame({ jsonrpc: "2.0", id: request.id, result }));
      }
    });
  });
  await new Promise<void>((resolvePromise, rejectPromise) => {
    server.once("error", rejectPromise);
    server.listen(socketPath, resolvePromise);
  });
  await chmod(socketPath, 0o600);
  return { connectionCount: () => connections, requests, server, sockets };
}

test("exact Browser client selects the sky-cua socket and stays disconnected after reset", async () => {
  const root = await mkdtemp(
    join(tmpdir(), "cua-node-browser-control-ingress-"),
  );
  const packageRoot = join(root, "node_modules", "browser-use-installed");
  const packageScripts = join(packageRoot, "scripts");
  const socketPath = join(root, "codex-browser-compat.sock");
  const peer = await startBrowserPeer(socketPath);
  await mkdir(packageScripts, { recursive: true });
  await cp(CLIENT_PATH, join(packageScripts, "browser-client.mjs"));
  await writeFile(
    join(packageRoot, "package.json"),
    JSON.stringify({
      name: "browser-use-installed",
      type: "module",
      exports: { ".": "./scripts/browser-client.mjs" },
    }),
    "utf8",
  );
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
    env: {
      NODE_REPL_NODE_MODULE_DIRS: root,
      NODE_REPL_TRUSTED_CODE_PATHS: packageRoot,
      BROWSER_USE_CODEX_APP_BUILD_FLAVOR: CODEX_APP_BUILD_FLAVOR,
      SKY_CUA_CODEX_BROWSER_INGRESS: "sky_cua",
      SKY_CUA_CODEX_BROWSER_SOCKET_PATH: socketPath,
      SKY_CUA_MCP_CALLER_PROVENANCE: "codex_desktop",
    },
  });
  const server = new McpServer({ manager });
  try {
    const response = await server.dispatch(
      browserJs(
        1,
        'const client = await import("browser-use-installed"); await client.setupBrowserRuntime({ globals: globalThis }); await agent.browsers.get("extension"); nodeRepl.write("sky-cua-ingress-ready");',
      ),
    );
    const result = toolResult(response);
    assert.deepEqual(result?.content, [
      { type: "text", text: "sky-cua-ingress-ready" },
    ]);
    assert.equal(result?.isError, false);
    assert.equal(peer.connectionCount(), 1);
    assert.equal(peer.requests.length, 1);
    assert.equal(peer.requests[0]?.method, "getInfo");
    const expectedContext = {
      session_id: "session-control-ingress",
      turn_id: "turn-control-ingress",
      "x-codex-turn-metadata": {
        session_id: "session-control-ingress",
        turn_id: "turn-control-ingress",
      },
      caller_provenance: "codex_desktop",
    };
    assert.deepEqual(peer.requests[0]?.params, {
      ...expectedContext,
      _meta: expectedContext,
      request_meta: expectedContext,
    });
    const reset = await server.dispatch({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name: "js_reset", arguments: {} },
    });
    assert.deepEqual(toolResult(reset)?.content, [
      { type: "text", text: "true" },
    ]);
    for (let attempt = 0; attempt < 50 && peer.sockets.size > 0; attempt += 1) {
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
    }
    assert.equal(
      peer.sockets.size,
      0,
      "kernel reset must close the compatibility socket",
    );
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 100));
    assert.equal(
      peer.connectionCount(),
      1,
      "Browser client must not reconnect autonomously after generation reset",
    );
  } finally {
    await server.close();
    for (const socket of peer.sockets) socket.destroy();
    await new Promise<void>((resolvePromise) =>
      peer.server.close(() => resolvePromise()),
    );
    await rm(root, { force: true, recursive: true });
  }
});
