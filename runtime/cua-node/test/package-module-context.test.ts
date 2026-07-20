import { strict as assert } from "node:assert";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "bun:test";
import { McpServer } from "../src/host/mcp-server.ts";
import { RuntimeManager } from "../src/host/runtime-manager.ts";
import { TEST_NODE_PATH } from "./test-node-path.ts";

type ToolResponse = {
  result?: {
    content?: Array<{ text?: string }>;
    isError?: boolean;
  };
};

function textResult(response: ToolResponse): string | undefined {
  return response.result?.content?.[0]?.text;
}

async function installPackageFixtures(directory: string): Promise<string> {
  const modules = join(directory, "node_modules");
  const skyRoot = join(modules, "@heliasar", "sky-cua");
  const cjsRoot = join(modules, "cjs-package-fixture");
  await Promise.all([
    mkdir(skyRoot, { recursive: true }),
    mkdir(cjsRoot, { recursive: true }),
  ]);
  await Promise.all([
    writeFile(
      join(skyRoot, "package.json"),
      JSON.stringify({
        name: "@heliasar/sky-cua",
        version: "0.1.0-fixture",
        type: "module",
        private: true,
        exports: { ".": "./index.mjs", "./subpath": "./subpath.mjs" },
      }),
    ),
    writeFile(
      join(skyRoot, "index.mjs"),
      String.raw`
import { readFileSync } from "node:fs";
import { createConnection } from "node:net";
import * as importedModuleBuiltin from "node:module";
import processModule from "node:process";

const importedPlatform = process.platform;

function context() {
  const moduleBuiltin = process.getBuiltinModule("module");
  const processViaImport = importedModuleBuiltin.createRequire(import.meta.url)("node:process");
  const processViaBuiltin = moduleBuiltin.createRequire(import.meta.url)("node:process");
  let escapedPackageRoot = false;
  try { moduleBuiltin.createRequire("/tmp/outside-package.cjs"); escapedPackageRoot = true; } catch {}
  return {
    envFrozen: Object.isFrozen(process.env),
    hasExit: typeof process.exit === "function",
    hasProcessEvents:
      typeof process.on === "function" && typeof process.off === "function",
    hasNativePipe: typeof globalThis.nodeRepl?.nativePipe !== "undefined",
    hasSend: typeof process.send === "function",
    hasSuspendedTimeout:
      typeof globalThis.nodeRepl?.withSuspendedTimeout === "function",
    importedProcessMatchesGlobal: processModule === process,
    packageSurfaceOwnKeys: Object.keys(globalThis.nodeRepl),
    platform: importedPlatform,
    runtimeDir: process.env.XDG_RUNTIME_DIR,
    processCompatibility: {
      argvFrozen: Object.isFrozen(process.argv),
      builtinModule: typeof moduleBuiltin?.createRequire,
      createRequireFacades: [processViaImport, processViaBuiltin].every((value) => value !== globalThis && typeof value.exit === "undefined" && typeof value.send === "undefined" && typeof value.cwd === "function"),
      escapedPackageRoot,
      builtinProcessMatchesFacade: process.getBuiltinModule("process") === process,
      cpuUsage: typeof process.cpuUsage,
      execPath: typeof process.execPath,
      hrtimeBigint: typeof process.hrtime?.bigint,
      memoryUsageRss: typeof process.memoryUsage?.rss,
      nextTick: typeof process.nextTick,
      releaseFrozen: Object.isFrozen(process.release),
      resourceUsage: typeof process.resourceUsage,
      stringTag: String(process),
      uptime: typeof process.uptime,
      nodeMajor: process.versions.node.split(".")[0],
      version: typeof process.version,
      versionsFrozen: Object.isFrozen(process.versions),
    },
    privilegedKeys: [
      "config",
      "createElicitation",
      "fetch",
      "launchServices",
      "nativePipe",
    ].filter((key) => key in globalThis.nodeRepl),
    processFrozen: Object.isFrozen(process),
    webGlobals: {
      atob: typeof atob,
      fetch: typeof fetch,
      globalAlias: global === globalThis,
      navigator: typeof navigator,
      performanceObserver: typeof PerformanceObserver,
      urlPattern: typeof URLPattern,
      webSocket: typeof WebSocket,
    },
  };
}

async function connect() {
  const configPath = process.env.OAI_SKY_CONFIG_PATH;
  if (typeof configPath !== "string" || configPath.length === 0) {
    throw new Error("fixture requires OAI_SKY_CONFIG_PATH");
  }
  const config = JSON.parse(readFileSync(configPath, "utf8"));
  const request = {
    type: "fixture_action",
    request_meta: globalThis.nodeRepl?.requestMeta ?? null,
  };
  return await globalThis.nodeRepl.withSuspendedTimeout(
    () =>
      new Promise((resolvePromise, rejectPromise) => {
        const socket = createConnection({ path: config.service_socket_path });
        let response = "";
        socket.setEncoding("utf8");
        socket.once("connect", () => socket.write(JSON.stringify(request) + "\n"));
        socket.on("data", (chunk) => {
          response += chunk;
          const newline = response.indexOf("\n");
          if (newline === -1) return;
          socket.end();
          resolvePromise(JSON.parse(response.slice(0, newline)));
        });
        socket.once("error", rejectPromise);
      }),
  );
}

async function contextBreakingBuiltinsUnavailable() {
  let importDenied = false;
  try { await import("node:vm"); } catch { importDenied = true; }
  return importDenied && process.getBuiltinModule("vm") === undefined;
}

export const sky = Object.freeze({ connect, context, contextBreakingBuiltinsUnavailable });
`,
    ),
    writeFile(
      join(skyRoot, "subpath.mjs"),
      String.raw`
export default Object.freeze({
  fetch: typeof fetch,
  processTag: String(process),
  processType: typeof process,
});
`,
    ),
    writeFile(
      join(cjsRoot, "package.json"),
      JSON.stringify({
        name: "cjs-package-fixture",
        version: "1.0.0",
        private: true,
        main: "index.cjs",
      }),
    ),
    writeFile(
      join(cjsRoot, "index.cjs"),
      String.raw`
const { readFileSync } = require("node:fs");
let vmRequireDenied = false;
try { require("node:vm"); } catch { vmRequireDenied = true; }

let calls = 0;
let processReceiver;
let listenerCalls = 0;
const repeatedListener = () => { listenerCalls += 1; };
const distinctListener = () => { listenerCalls += 100; };

module.exports = {
  increment() {
    calls += 1;
    return calls;
  },
  readConfiguredSocket() {
    const config = JSON.parse(
      readFileSync(process.env.OAI_SKY_CONFIG_PATH, "utf8"),
    );
    return config.service_socket_path;
  },
  armProcessReceiver() {
    let resolveReceiver;
    processReceiver = new Promise((resolve) => { resolveReceiver = resolve; });
    const removed = function () { resolveReceiver({ removed: true }); };
    process.on("SIGCONT", removed);
    process.off("SIGCONT", removed);
    process.on("SIGCONT", repeatedListener);
    process.on("SIGCONT", repeatedListener);
    process.on("SIGCONT", distinctListener);
    process.off("SIGCONT", repeatedListener);
    process.off("SIGCONT", distinctListener);
    process.off("SIGCONT", repeatedListener);
    process.once("SIGCONT", function () {
      listenerCalls += 1;
      resolveReceiver({
        facade: this === process,
        hasExit: typeof this.exit === "function",
        hasRawStdout: typeof this.stdout?.write === "function",
        hasSend: typeof this.send === "function",
        removed: false,
      });
    });
    return true;
  },
  async readProcessReceiver() {
    return await processReceiver;
  },
  listenerCount() { return listenerCalls; },
  contextBreakingBuiltinsUnavailable() {
    return vmRequireDenied && process.getBuiltinModule("vm") === undefined;
  },
  runtime: Object.freeze({
    facadeFrozen: Object.isFrozen(process),
    cwdWritable: Object.getOwnPropertyDescriptor(process, "cwd")?.writable === true,
    hasChdir: typeof process.chdir === "function",
    hasExit: typeof process.exit === "function",
    hasProcessEvents:
      typeof process.on === "function" && typeof process.off === "function",
    stdio: [process.stdin, process.stdout, process.stderr].map((stream) => ({
      fd: stream.fd,
      frozen: Object.isFrozen(stream),
      hasWrite: typeof stream.write === "function",
      isTTY: stream.isTTY,
    })),
    importedProcessMatchesGlobal: require("node:process") === process,
    nodeReplFrozen: Object.isFrozen(globalThis.nodeRepl),
    platform: process.platform,
    runtimeDir: process.env.XDG_RUNTIME_DIR,
    nodeReplKeys: Object.keys(globalThis.nodeRepl),
  }),
};
`,
    ),
  ]);
  return modules;
}

test("@heliasar/sky-cua imports lazily in a package context and connects with request metadata", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-package-context-"));
  const socketDirectory = join(directory, "socket");
  const socketPath = join(socketDirectory, "sky-cua.sock");
  const configPath = join(directory, "sky-config.json");
  await mkdir(socketDirectory, { recursive: true });
  await writeFile(configPath, JSON.stringify({ service_socket_path: socketPath }));
  const modules = await installPackageFixtures(directory);

  const requests: Array<Record<string, unknown>> = [];
  const peer = createServer((socket) => {
    socket.setEncoding("utf8");
    let input = "";
    socket.on("data", (chunk) => {
      input += chunk;
      const newline = input.indexOf("\n");
      if (newline === -1) return;
      const request = JSON.parse(input.slice(0, newline)) as Record<string, unknown>;
      requests.push(request);
      setTimeout(() => {
        socket.end(`${JSON.stringify({ ok: true, received: request.request_meta })}\n`);
      }, 40);
    });
  });
  await new Promise<void>((resolvePromise, rejectPromise) => {
    peer.once("error", rejectPromise);
    peer.listen(socketPath, resolvePromise);
  });

  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
    env: {
      NODE_REPL_NODE_MODULE_DIRS: modules,
      OAI_SKY_CONFIG_PATH: configPath,
      XDG_RUNTIME_DIR: socketDirectory,
    },
  });
  const server = new McpServer({ manager });
  try {
    const imported = (await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: 'const {sky}=await import("@heliasar/sky-cua"); nodeRepl.write(JSON.stringify({keys:Object.keys(sky),context:sky.context()}))',
        },
      },
    })) as ToolResponse;
    assert.equal(imported.result?.isError, false);
    assert.deepEqual(JSON.parse(textResult(imported) ?? "null"), {
      keys: ["connect", "context", "contextBreakingBuiltinsUnavailable"],
      context: {
        envFrozen: true,
        hasExit: false,
        hasProcessEvents: false,
        hasNativePipe: false,
        hasSend: false,
        hasSuspendedTimeout: true,
        importedProcessMatchesGlobal: true,
        packageSurfaceOwnKeys: ["withSuspendedTimeout"],
        platform: process.platform,
        runtimeDir: socketDirectory,
        processCompatibility: {
          argvFrozen: true,
          builtinModule: "function",
          createRequireFacades: true,
          escapedPackageRoot: false,
          builtinProcessMatchesFacade: true,
          cpuUsage: "function",
          execPath: "string",
          hrtimeBigint: "function",
          memoryUsageRss: "function",
          nextTick: "function",
          releaseFrozen: true,
          resourceUsage: "function",
          stringTag: "[object process]",
          uptime: "function",
          nodeMajor: "24",
          version: "string",
          versionsFrozen: true,
        },
        privilegedKeys: [],
        processFrozen: true,
        webGlobals: {
          atob: "function",
          fetch: "function",
          globalAlias: true,
          navigator: "object",
          performanceObserver: "function",
          urlPattern: "function",
          webSocket: "function",
        },
      },
    });
    assert.equal(requests.length, 0);

    const packageVm = (await server.dispatch({
      jsonrpc: "2.0",
      id: 19,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: "nodeRepl.write(String(await sky.contextBreakingBuiltinsUnavailable()))",
        },
      },
    })) as ToolResponse;
    assert.equal(textResult(packageVm), "true");

    const packageSubpath = (await server.dispatch({
      jsonrpc: "2.0",
      id: 20,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: 'var packageSubpath = (await import("@heliasar/sky-cua/subpath")).default; nodeRepl.write(JSON.stringify(packageSubpath))',
        },
      },
    })) as ToolResponse;
    assert.equal(packageSubpath.result?.isError, false);
    assert.deepEqual(JSON.parse(textResult(packageSubpath) ?? "null"), {
      fetch: "function",
      processTag: "[object process]",
      processType: "object",
    });

    const connected = (await server.dispatch({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: "nodeRepl.write(JSON.stringify(await sky.connect()))",
          timeout_ms: 20,
        },
        _meta: { session_id: "session-fixture", turn_id: "turn-fixture" },
      },
    })) as ToolResponse;
    assert.equal(connected.result?.isError, false);
    assert.deepEqual(JSON.parse(textResult(connected) ?? "null"), {
      ok: true,
      received: { session_id: "session-fixture", turn_id: "turn-fixture" },
    });
    assert.deepEqual(requests, [
      {
        type: "fixture_action",
        request_meta: {
          session_id: "session-fixture",
          turn_id: "turn-fixture",
        },
      },
    ]);

    const repeatedPackage = (await server.dispatch({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: 'var skyAgain = (await import("@heliasar/sky-cua")).sky; nodeRepl.write(String(skyAgain === sky))',
        },
      },
    })) as ToolResponse;
    assert.equal(textResult(repeatedPackage), "true");

    const commonJsFirst = (await server.dispatch({
      jsonrpc: "2.0",
      id: 4,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: 'var cjsFirst = await import("cjs-package-fixture"); nodeRepl.write(JSON.stringify({count:cjsFirst.increment(),socket:cjsFirst.readConfiguredSocket(),runtime:cjsFirst.runtime}))',
        },
      },
    })) as ToolResponse;
    assert.equal(
      commonJsFirst.result?.isError,
      false,
      textResult(commonJsFirst) ?? "CommonJS fixture import failed",
    );
    assert.deepEqual(JSON.parse(textResult(commonJsFirst) ?? "null"), {
      count: 1,
      socket: socketPath,
      runtime: {
        facadeFrozen: false,
        cwdWritable: true,
        hasChdir: false,
        hasExit: false,
        hasProcessEvents: true,
        stdio: [0, 1, 2].map((fd) => ({
          fd,
          frozen: true,
          hasWrite: false,
          isTTY: false,
        })),
        importedProcessMatchesGlobal: true,
        nodeReplFrozen: true,
        platform: process.platform,
        runtimeDir: socketDirectory,
        nodeReplKeys: ["withSuspendedTimeout"],
      },
    });
    const commonJsVm = (await server.dispatch({
      jsonrpc: "2.0",
      id: 39,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: "nodeRepl.write(String(cjsFirst.contextBreakingBuiltinsUnavailable()))",
        },
      },
    })) as ToolResponse;
    assert.equal(textResult(commonJsVm), "true");

    const armedReceiver = (await server.dispatch({
      jsonrpc: "2.0",
      id: 40,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: "nodeRepl.write(String(cjsFirst.armProcessReceiver()))",
        },
      },
    })) as ToolResponse;
    assert.equal(textResult(armedReceiver), "true");
    const kernelPid = manager.kernelPid;
    assert.equal(typeof kernelPid, "number");
    process.kill(kernelPid as number, "SIGCONT");
    const processReceiver = (await server.dispatch({
      jsonrpc: "2.0",
      id: 41,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: "nodeRepl.write(JSON.stringify(await cjsFirst.readProcessReceiver()))",
        },
      },
    })) as ToolResponse;
    assert.deepEqual(JSON.parse(textResult(processReceiver) ?? "null"), {
      facade: true,
      hasExit: false,
      hasRawStdout: false,
      hasSend: false,
      removed: false,
    });
    process.kill(kernelPid as number, "SIGCONT");
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    const listenerCount = (await server.dispatch({
      jsonrpc: "2.0",
      id: 42,
      method: "tools/call",
      params: {
        name: "js",
        arguments: { code: "nodeRepl.write(String(cjsFirst.listenerCount()))" },
      },
    })) as ToolResponse;
    assert.equal(textResult(listenerCount), "1");

    const commonJsSecond = (await server.dispatch({
      jsonrpc: "2.0",
      id: 5,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: 'var cjsSecond = await import("cjs-package-fixture"); nodeRepl.write(JSON.stringify({same:cjsFirst.default===cjsSecond.default,count:cjsSecond.increment()}))',
        },
      },
    })) as ToolResponse;
    assert.equal(commonJsSecond.result?.isError, false);
    assert.deepEqual(JSON.parse(textResult(commonJsSecond) ?? "null"), {
      same: true,
      count: 2,
    });

    const modelProcess = (await server.dispatch({
      jsonrpc: "2.0",
      id: 6,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: "nodeRepl.write(typeof process); await import('node:process')",
        },
      },
    })) as ToolResponse;
    assert.equal(modelProcess.result?.isError, true);
    assert.match(
      textResult(modelProcess) ?? "",
      /Cannot import process from untrusted/u,
    );
    const modelVm = (await server.dispatch({
      jsonrpc: "2.0",
      id: 59,
      method: "tools/call",
      params: { name: "js", arguments: { code: "await import('node:vm')" } },
    })) as ToolResponse;
    assert.equal(modelVm.result?.isError, true);
    assert.match(textResult(modelVm) ?? "", /builtin module is unavailable: vm/u);
    for (const code of [
      "var m = await import('node:module'); m.createRequire(import.meta.url)('node:process')",
      "var m2 = await import('node:module'); m2.createRequire(import.meta.url)('process')",
    ]) {
      const escaped = (await server.dispatch({
        jsonrpc: "2.0",
        id: 60,
        method: "tools/call",
        params: { name: "js", arguments: { code } },
      })) as ToolResponse;
      assert.equal(escaped.result?.isError, true);
      assert.match(textResult(escaped) ?? "", /Cannot require process from untrusted/u);
    }
  } finally {
    await server.close();
    await new Promise<void>((resolvePromise) => peer.close(() => resolvePromise()));
    await rm(directory, { recursive: true, force: true });
  }
});
