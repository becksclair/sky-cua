import { createServer, type Socket } from "node:net";
import { chmod, cp, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { strict as assert } from "node:assert";
import { test } from "bun:test";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const skyCuaRoot = process.env.SKY_CUA_ROOT ?? resolve(packageRoot, "../..");

type DispatchResponse = {
  result?: { isError?: boolean; content?: Array<{ text?: string }> };
};

async function startBrowserPeer(socketPath: string) {
  let connectionCount = 0;
  const sockets = new Set<Socket>();
  const server = createServer((socket) => {
    connectionCount += 1;
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
    let buffered = Buffer.alloc(0);
    socket.on("data", (chunk) => {
      const bytes = typeof chunk === "string" ? Buffer.from(chunk) : chunk;
      buffered = Buffer.concat([buffered, bytes]);
      while (buffered.byteLength >= 4) {
        const size = buffered.readUInt32LE(0);
        if (buffered.byteLength < size + 4) return;
        const request = JSON.parse(buffered.subarray(4, size + 4).toString("utf8")) as Record<string, unknown>;
        buffered = buffered.subarray(size + 4);
        const payload = Buffer.from(JSON.stringify({
          jsonrpc: "2.0",
          id: request.id,
          result: request.method === "getInfo"
            ? { id: "iab:trust", type: "iab", name: "Trust fixture", capabilities: {}, metadata: { provider: "extension" } }
            : {},
        }));
        const frame = Buffer.alloc(payload.byteLength + 4);
        frame.writeUInt32LE(payload.byteLength, 0);
        payload.copy(frame, 4);
        socket.write(frame);
      }
    });
  });
  await new Promise<void>((resolvePromise, rejectPromise) => {
    server.once("error", rejectPromise);
    server.listen(socketPath, resolvePromise);
  });
  await chmod(socketPath, 0o600);
  return {
    connectionCount: () => connectionCount,
    close: async () => {
      for (const socket of sockets) socket.destroy();
      await new Promise<void>((resolvePromise) => server.close(() => resolvePromise()));
    },
  };
}

test("the real cua_node loader trusts only the fixed installed Browser package path", async () => {
  const [{ RuntimeManager }, { McpServer }, { TEST_NODE_PATH }] = await Promise.all([
    import(`${skyCuaRoot}/runtime/cua-node/src/host/runtime-manager.ts`),
    import(`${skyCuaRoot}/runtime/cua-node/src/host/mcp-server.ts`),
    import(`${skyCuaRoot}/runtime/cua-node/test/test-node-path.ts`),
  ]);
  const root = await mkdtemp(join(tmpdir(), "browser-use-real-trust-"));
  const runtimeRoot = join(root, "runtime");
  const moduleRoot = join(runtimeRoot, "lib", "node_modules", "@heliasar", "browser-use");
  const wrongRoot = join(root, "wrong-package");
  const socketPath = join(root, "browser.sock");
  const peer = await startBrowserPeer(socketPath);
  try {
    await cp(
      join(skyCuaRoot, "runtime/cua-node/test/fixtures/fake-runtime"),
      runtimeRoot,
      { recursive: true },
    );
    await mkdir(join(runtimeRoot, "share", "pdfjs", "cmaps"), { recursive: true });
    await mkdir(join(runtimeRoot, "share", "pdfjs", "standard_fonts"), { recursive: true });
    const pdfWorker = join(
      runtimeRoot,
      "lib",
      "node_modules",
      "pdfjs-dist",
      "legacy",
      "build",
      "pdf.worker.mjs",
    );
    await mkdir(dirname(pdfWorker), { recursive: true });
    await writeFile(pdfWorker, "export {};\n");
    await writeFile(join(runtimeRoot, "share", "tessdata", "eng.traineddata"), "fixture\n");
    await mkdir(join(moduleRoot, "build"), { recursive: true });
    await mkdir(wrongRoot, { recursive: true });
    const browserClient = join(packageRoot, "build", "browser-client.mjs");
    await cp(browserClient, join(moduleRoot, "build", "browser-client.mjs"));
    await cp(browserClient, join(wrongRoot, "browser-client.mjs"));
    const dispatch = async (entrypoint: string): Promise<DispatchResponse> => {
      const manager = new RuntimeManager({
        allowHostNode: true,
        nodePath: TEST_NODE_PATH,
        runtimeRoot,
        env: {
          SKY_CUA_CODEX_BROWSER_SOCKET_PATH: socketPath,
          SKY_CUA_MCP_CALLER_PROVENANCE: "codex_desktop",
        },
      });
      const server = new McpServer({ manager });
      try {
        return await server.dispatch({
          jsonrpc: "2.0",
          id: 1,
          method: "tools/call",
          params: {
            name: "js",
            arguments: {
              code: `const client = await import(${JSON.stringify(pathToFileURL(entrypoint).href)}); await client.setupBrowserRuntime({ globals: globalThis }); nodeRepl.write(JSON.stringify(await agent.browsers.list()));`,
            },
            _meta: {
              session_id: "session-trust",
              turn_id: "turn-trust",
              "x-codex-turn-metadata": { session_id: "session-trust", turn_id: "turn-trust" },
            },
          },
        }) as DispatchResponse;
      } finally {
        await server.close();
      }
    };

    const wrong = await dispatch(join(wrongRoot, "browser-client.mjs"));
    assert.equal(wrong.result?.isError, true, JSON.stringify(wrong));
    assert.equal(peer.connectionCount(), 0);

    const correct = await dispatch(join(moduleRoot, "build", "browser-client.mjs"));
    assert.equal(correct.result?.isError, false, JSON.stringify(correct));
    assert.match(correct.result?.content?.[0]?.text ?? "", /extension:browser/u);
    assert.equal(peer.connectionCount(), 1);
  } finally {
    await peer.close();
    await rm(root, { recursive: true, force: true });
  }
});
