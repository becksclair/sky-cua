import { strict as assert } from "node:assert";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { chmod, copyFile, cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath, pathToFileURL } from "node:url";
import { test } from "bun:test";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const assembledRuntime = join(repo, "out/components/cua-node-linux-x64-glibc");
const fallbackSource = join(repo, "resources/codex-compat/openai-bundled/plugins/browser");

type Reply = {
  id?: number;
  error?: unknown;
  result?: { isError?: boolean; content?: Array<{ text?: string }> };
};

async function startPeer(path: string) {
  let connections = 0;
  const sockets = new Set<Socket>();
  const methods: unknown[] = [];
  const server = createServer((socket) => {
    connections++;
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
    let buffered = Buffer.alloc(0);
    socket.on("data", (chunk) => {
      buffered = Buffer.concat([buffered, Buffer.from(chunk)]);
      while (buffered.length >= 4) {
        const size = buffered.readUInt32LE(0);
        if (buffered.length < size + 4) return;
        const request = JSON.parse(buffered.subarray(4, size + 4).toString("utf8"));
        buffered = buffered.subarray(size + 4);
        methods.push(request.method);
        const payload = Buffer.from(JSON.stringify({
          jsonrpc: "2.0", id: request.id,
          result: request.method === "getInfo"
            ? { id: "iab:fallback", type: "iab", name: "Fallback fixture", capabilities: {}, metadata: { provider: "extension" } }
            : {},
        }));
        const frame = Buffer.alloc(payload.length + 4);
        frame.writeUInt32LE(payload.length, 0);
        payload.copy(frame, 4);
        socket.write(frame);
      }
    });
  });
  await new Promise<void>((done, reject) => {
    server.once("error", reject);
    server.listen(path, done);
  });
  await chmod(path, 0o600);
  return {
    connections: () => connections,
    methods,
    close: async () => {
      for (const socket of sockets) socket.destroy();
      await new Promise<void>((done) => server.close(() => done()));
    },
  };
}

async function dispatch(command: string, adapter: string, socketPath: string, root: string, cwd: string) {
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (key.startsWith("NODE_REPL_") || key === "CODEX_NODE_REPL_PATH" || key === "PLAYWRIGHT_BROWSERS_PATH") delete env[key];
  }
  env.XDG_DATA_HOME = join(root, "absent-standalone");
  env.SKY_CUA_CODEX_BROWSER_SOCKET_PATH = socketPath;
  env.SKY_CUA_MCP_CALLER_PROVENANCE = "codex_desktop";
  const child = spawn(command, [], { cwd, env, stdio: "pipe" });
  let stderr = "";
  child.stderr.on("data", (data) => { stderr += String(data); });
  const reader = createInterface({ input: child.stdout });
  const pending = new Map<number, (value: Reply) => void>();
  reader.on("line", (line) => {
    const response = JSON.parse(line) as Reply;
    if (response.id !== undefined) pending.get(response.id)?.(response);
  });
  let nextId = 0;
  const request = (method: string, params: unknown): Promise<Reply> => {
    const id = ++nextId;
    return new Promise((done, reject) => {
      const timer = setTimeout(() => reject(new Error(`MCP request timed out: ${stderr}`)), 15000);
      pending.set(id, (value) => { clearTimeout(timer); pending.delete(id); done(value); });
      child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  };
  try {
    const initialized = await request("initialize", {
      protocolVersion: "2025-11-25", capabilities: {},
      clientInfo: { name: "codex", version: "fixture" },
    });
    assert.equal(initialized.error, undefined, JSON.stringify(initialized));
    return await request("tools/call", {
      name: "js",
      arguments: {
        code: `var client = await import(${JSON.stringify(pathToFileURL(adapter).href)}); await client.setupBrowserRuntime({ globals: globalThis }); nodeRepl.write(JSON.stringify(await agent.browsers.list()));`,
      },
      _meta: { session_id: "fallback", turn_id: "fallback", "x-codex-turn-metadata": { session_id: "fallback", turn_id: "fallback" } },
    });
  } finally {
    const exit = new Promise<void>((done) => child.once("exit", () => done()));
    child.kill("SIGTERM");
    await exit;
    reader.close();
  }
}

// The integration gate consumes the same full component the plugin producer uses.
// Run scripts/assemble_cua_node.py --allow-development-dirty before this test.
test.skipIf(!existsSync(join(assembledRuntime, "bin/node_repl")))("development and standalone fallback dispatch through their actual launcher and trusted Browser package", async () => {
  const root = await mkdtemp(join(tmpdir(), "browser-fallback-"));
  const browser = join(root, "cache/plugins/browser");
  const socketPath = join(root, "browser.sock");
  const peer = await startPeer(socketPath);
  try {
    // Stage then move the development plugin exactly as a host cache projection does.
    const stagedBundle = join(root, "staged-bundle");
    const staging = join(stagedBundle, "resources/plugins/openai-bundled/plugins/browser");
    const stage = Bun.spawn([
      "uv", "run", "python", "-c",
      "import sys; from pathlib import Path; sys.path.insert(0,'scripts'); import build_plugin; build_plugin.build_development_browser_runtime = lambda: Path(sys.argv[2]); build_plugin.bundled_resource_root = lambda: Path(sys.argv[1]) / 'absent-legacy'; build_plugin.stage_openai_bundled_plugins(Path(sys.argv[1]))",
      stagedBundle, assembledRuntime,
    ], { cwd: repo, stdout: "pipe", stderr: "pipe" });
    const [status, stageError] = await Promise.all([stage.exited, new Response(stage.stderr).text()]);
    assert.equal(status, 0, stageError);
    await mkdir(dirname(browser), { recursive: true });
    await cp(staging, browser, { recursive: true });
    await rm(stagedBundle, { recursive: true });
    const config = JSON.parse(await readFile(join(browser, ".mcp.json"), "utf8"));
    const command = config.mcpServers.node_repl.command;
    const cwd = resolve(browser, config.mcpServers.node_repl.cwd);
    assert.equal(command, "./cua-node/bin/node_repl");
    assert.equal(cwd, browser);
    const adapter = join(browser, "scripts/browser-client.mjs");
    assert.equal(existsSync(join(root, "absent-standalone/sky-cua/RELEASE.json")), false);
    const dev = await dispatch(command, adapter, socketPath, root, cwd);
    assert.equal(dev.result?.isError, false, JSON.stringify(dev));
    assert.match(dev.result?.content?.[0]?.text ?? "", /extension:browser/u);
    assert.equal(peer.connections(), 1);

    // The installed source adapter requires RELEASE metadata but imports the
    // runtime's canonical package, not the semantic copy beside browser assets.
    const installed = join(browser, "cua-node");
    await copyFile(join(fallbackSource, "scripts/browser-client.mjs"), adapter);
    const missingRelease = await dispatch(command, adapter, socketPath, root, cwd);
    assert.equal(missingRelease.result?.isError, true, JSON.stringify(missingRelease));
    assert.match(missingRelease.result?.content?.[0]?.text ?? "", /RELEASE\.json/u);
    assert.equal(peer.connections(), 1);
    await writeFile(join(installed, "RELEASE.json"), JSON.stringify({ paths: { browser_client: "browser/browser-client.mjs" } }));
    await mkdir(join(installed, "browser"));
    const canonical = join(installed, "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs");
    await copyFile(canonical, join(installed, "browser/browser-client.mjs"));
    const standalone = await dispatch(command, adapter, socketPath, root, cwd);
    assert.equal(standalone.result?.isError, false, JSON.stringify(standalone));
    assert.match(standalone.result?.content?.[0]?.text ?? "", /extension:browser/u);
    assert.equal(peer.connections(), 2);

    const wrong = await dispatch(command, join(installed, "browser/browser-client.mjs"), socketPath, root, cwd);
    assert.equal(wrong.result?.isError, true, JSON.stringify(wrong));
    assert.equal(peer.connections(), 2);
    assert.ok(peer.methods.includes("getInfo"));
  } finally {
    await peer.close();
    await rm(root, { recursive: true, force: true });
  }
}, 60000);
