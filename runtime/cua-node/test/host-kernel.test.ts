import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "bun:test";
import { strict as assert } from "node:assert";
import outputMetadata from "./fixtures/upstream-5307/output-metadata.json";
import toolsFixture from "./fixtures/upstream-5307/tools-list.json";
import capabilityContract from "../contracts/runtime-capabilities.contract.json";
import {
  McpServer,
  resolveMcpCallerProvenance,
} from "../src/host/mcp-server.ts";
import {
  MAX_CANCELLATION_TOMBSTONES,
  RuntimeManager,
} from "../src/host/runtime-manager.ts";
import type { RuntimeAssetMetadata } from "../src/host/runtime-asset-discovery.ts";
import { TEST_NODE_PATH } from "./test-node-path.ts";

type Response = {
  result?: {
    content?: Array<Record<string, unknown>>;
    isError?: boolean;
    _meta?: Record<string, unknown>;
  };
};

function result(response: Response): Response["result"] {
  assert.ok(response.result !== undefined);
  return response.result;
}

async function withServer(run: (server: McpServer) => Promise<void>): Promise<void> {
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  const server = new McpServer({ manager });
  try {
    await run(server);
  } finally {
    await server.close();
  }
}

function js(
  id: number,
  code: string,
  timeoutMs?: number,
  meta?: Record<string, unknown>,
): {
  jsonrpc: "2.0";
  id: number;
  method: "tools/call";
  params: Record<string, unknown>;
} {
  const argumentsValue =
    timeoutMs === undefined ? { code } : { code, timeout_ms: timeoutMs };
  return {
    jsonrpc: "2.0",
    id,
    method: "tools/call",
    params: {
      name: "js",
      arguments: argumentsValue,
      ...(meta === undefined ? {} : { _meta: meta }),
    },
  };
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

test("MCP initialize and tools/list match the frozen public surface", async () => {
  await withServer(async (server) => {
    const initialize = await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: { protocolVersion: "2025-11-25" },
    });
    assert.deepEqual(
      initialize?.result && { ...initialize.result, instructions: undefined },
      {
        protocolVersion: "2025-11-25",
        capabilities: { tools: { listChanged: true } },
        serverInfo: { name: "node_repl", version: "0.1.0" },
        instructions: undefined,
      },
    );
    const tools = await server.dispatch({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/list",
      params: {},
    });
    assert.deepEqual(tools?.result, { tools: toolsFixture.tools });
  });
});

test("metadata-free callers receive stable synthetic process identity and fresh turns", async () => {
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  const server = new McpServer({ manager, callerProvenance: "openclaw" });
  try {
    await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        clientInfo: {
          name: "OpenClaw",
          version: "2026.7.1",
          extension: { channel: "local" },
        },
      },
    });
    const readMeta = async (id: number): Promise<Record<string, unknown>> => {
      const response = result(
        (await server.dispatch(
          js(id, "nodeRepl.write(JSON.stringify(nodeRepl.requestMeta))"),
        )) as Response,
      );
      const text = response?.content?.[0]?.text;
      assert.equal(typeof text, "string");
      return JSON.parse(text as string) as Record<string, unknown>;
    };
    const first = await readMeta(2);
    const second = await readMeta(3);
    assert.equal(first.session_id, second.session_id);
    assert.notEqual(first.turn_id, second.turn_id);
    assert.equal(first.caller_provenance, "openclaw");
    assert.equal(first.identity_synthetic, true);
    assert.deepEqual(first.client_info, {
      name: "OpenClaw",
      version: "2026.7.1",
      extension: { channel: "local" },
    });
    assert.deepEqual(first["x-codex-turn-metadata"], {
      session_id: first.session_id,
      turn_id: first.turn_id,
    });
    assert.deepEqual(second["x-codex-turn-metadata"], {
      session_id: second.session_id,
      turn_id: second.turn_id,
    });
  } finally {
    await server.close();
  }
});

test("supplied Codex metadata is preserved exactly without synthetic augmentation", async () => {
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  const server = new McpServer({ manager, callerProvenance: "codex_desktop" });
  const supplied = {
    session_id: "codex-session",
    turn_id: "codex-turn",
    "x-codex-turn-metadata": {
      session_id: "codex-session",
      thread_id: "codex-thread",
      turn_id: "codex-turn",
    },
    opaque: { retained: [1, true, "yes"] },
  };
  try {
    await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        clientInfo: { name: "Codex Desktop", version: "1" },
      },
    });
    const response = result(
      (await server.dispatch(
        js(
          2,
          "nodeRepl.write(JSON.stringify(nodeRepl.requestMeta))",
          undefined,
          supplied,
        ),
      )) as Response,
    );
    const text = response?.content?.[0]?.text;
    assert.equal(typeof text, "string");
    assert.deepEqual(JSON.parse(text as string), supplied);
  } finally {
    await server.close();
  }
});

test("transport-only progress metadata is preserved while generic identity is synthesized", async () => {
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  const server = new McpServer({ manager, callerProvenance: "opencode" });
  try {
    await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        clientInfo: { name: "OpenCode", version: "1.0" },
      },
    });
    const readMeta = async (id: number, progressToken: number) => {
      const response = result(
        (await server.dispatch(
          js(
            id,
            "nodeRepl.write(JSON.stringify(nodeRepl.requestMeta))",
            undefined,
            { progressToken, opaque: { preserved: true } },
          ),
        )) as Response,
      );
      return JSON.parse(response?.content?.[0]?.text as string) as Record<string, unknown>;
    };
    const first = await readMeta(2, 11);
    const second = await readMeta(3, 12);
    assert.equal(first.progressToken, 11);
    assert.deepEqual(first.opaque, { preserved: true });
    assert.equal(first.session_id, second.session_id);
    assert.notEqual(first.turn_id, second.turn_id);
    assert.equal(first.caller_provenance, "opencode");
    assert.equal(first.identity_synthetic, true);
    assert.deepEqual(first.client_info, { name: "OpenCode", version: "1.0" });
  } finally {
    await server.close();
  }
});

test("caller provenance accepts only the explicit v1 enum and infers generic clients", () => {
  assert.equal(resolveMcpCallerProvenance("codex_desktop", null), "codex_desktop");
  assert.equal(resolveMcpCallerProvenance(" openclaw ", null), "openclaw");
  assert.equal(resolveMcpCallerProvenance("OpenCode", null), "opencode");
  assert.equal(resolveMcpCallerProvenance("direct_mcp", null), "direct_mcp");
  assert.equal(
    resolveMcpCallerProvenance(undefined, { name: "OpenCode", version: "1" }),
    "opencode",
  );
  assert.equal(
    resolveMcpCallerProvenance(undefined, { name: "unknown", version: "1" }),
    "direct_mcp",
  );
  assert.throws(
    () => resolveMcpCallerProvenance("codex_cli", null),
    /must be one of/u,
  );
});

test("persistent cells, binding errors, metadata, and process isolation work", async () => {
  await withServer(async (server) => {
    const first = result(
      (await server.dispatch(
        js(1, "var counter = 41; nodeRepl.write(counter)"),
      )) as Response,
    );
    assert.deepEqual(first?.content, [{ type: "text", text: "41" }]);
    const second = result(
      (await server.dispatch(js(2, "nodeRepl.write(counter + 1)"))) as Response,
    );
    assert.deepEqual(second?.content, [{ type: "text", text: "42" }]);
    const redeclare = result(
      (await server.dispatch(js(3, "const fixed = 1"))) as Response,
    );
    assert.equal(redeclare?.isError, false);
    const conflict = result(
      (await server.dispatch(js(4, "const fixed = 2"))) as Response,
    );
    assert.equal(conflict?.isError, true);
    assert.match(String(conflict?.content?.[0]?.text), /already been declared/u);
    const laterError = result(
      (await server.dispatch(
        js(
          5,
          "const beforeThrowFixture = 7; var writtenBeforeThrowFixture = 8; throw new Error('later')",
        ),
      )) as Response,
    );
    assert.equal(laterError?.isError, true);
    const carriedAfterError = result(
      (await server.dispatch(
        js(
          6,
          "nodeRepl.write(beforeThrowFixture); nodeRepl.write(writtenBeforeThrowFixture)",
        ),
      )) as Response,
    );
    assert.deepEqual(carriedAfterError?.content, [{ type: "text", text: "78" }]);
    const processImport = result(
      (await server.dispatch(js(7, "await import('node:process')"))) as Response,
    );
    assert.equal(processImport?.isError, true);
    const metadata = result(
      (await server.dispatch(
        js(
          8,
          "nodeRepl.setResponseMeta({one: 1}); nodeRepl.setResponseMeta({two: 2}); nodeRepl.write(nodeRepl.requestMeta.session_id)",
          undefined,
          { session_id: "session-fixture", turn_id: "turn-fixture" },
        ),
      )) as Response,
    );
    assert.deepEqual(metadata?.content, [{ type: "text", text: "session-fixture" }]);
    assert.deepEqual(metadata?._meta, { one: 1, two: 2 });
    const unsafeMetadata = result(
      (await server.dispatch(
        js(9, "nodeRepl.setResponseMeta({bad: BigInt(1)})"),
      )) as Response,
    );
    assert.equal(unsafeMetadata?.isError, true);
    const browserGlobals = result(
      (await server.dispatch(
        js(
          10,
          "nodeRepl.write(typeof performance + '|' + typeof performance.now + '|' + typeof crypto + '|' + typeof crypto.randomUUID + '|' + typeof crypto.subtle)",
        ),
      )) as Response,
    );
    assert.deepEqual(browserGlobals?.content, [
      { type: "text", text: "object|function|object|function|object" },
    ]);
    const globalAssignment = result(
      (await server.dispatch(
        js(11, "globalThis.browserFixture = { value: 'persistent' }"),
      )) as Response,
    );
    assert.equal(globalAssignment?.isError, false);
    const persistedGlobal = result(
      (await server.dispatch(
        js(12, "nodeRepl.write(globalThis.browserFixture.value)"),
      )) as Response,
    );
    assert.deepEqual(persistedGlobal?.content, [{ type: "text", text: "persistent" }]);
  });
});

test("REPL contexts expose Node 24 web globals required by bundled packages", async () => {
  await withServer(async (server) => {
    const requiredGlobals = Object.values(
      capabilityContract.node_global_descriptors,
    ).flat();
    const response = (await server.dispatch(
      js(
        1,
        `var requiredGlobalNames = ${JSON.stringify(requiredGlobals)};
var absentGlobalNames = ${JSON.stringify(capabilityContract.deliberately_absent)};
nodeRepl.write(JSON.stringify({
  missing: requiredGlobalNames.filter((name) => typeof globalThis[name] === "undefined"),
  falseBrowserGlobals: absentGlobalNames.filter((name) => typeof globalThis[name] !== "undefined"),
  globalAlias: global === globalThis,
  atobRoundtrip: atob(btoa("cua-node")),
  urlPattern: new URLPattern({ pathname: "/files/:name" }).exec("https://localhost/files/example.png")?.pathname.groups.name,
  navigator: navigator.constructor.name,
  crypto: CryptoKey.name,
  performance: PerformanceObserver.name,
}))`,
      ),
    )) as Response;
    assert.equal(response.result?.isError, false);
    const text = response.result?.content?.[0]?.text;
    if (typeof text !== "string") throw new Error("missing REPL web-global output");
    assert.deepEqual(JSON.parse(text), {
      missing: [],
      falseBrowserGlobals: [],
      globalAlias: true,
      atobRoundtrip: "cua-node",
      urlPattern: "example.png",
      navigator: "Navigator",
      crypto: "CryptoKey",
      performance: "PerformanceObserver",
    });
  });
});

test("nodeRepl exposes deeply frozen runtime metadata and fixed loaders", async () => {
  const runtimeMetadata: RuntimeAssetMetadata = {
    version: 1,
    root: "/fixture/cua_node",
    node: { version: "24.14.0", execPath: "/fixture/cua_node/bin/node" },
    modules: { root: "/fixture/cua_node/lib/node_modules" },
    browser: {
      playwrightRoot: "/fixture/cua_node/share/playwright",
      executablePath: "/fixture/brave-origin",
      executableKind: "brave-origin",
    },
    pdfjs: {
      root: "/fixture/cua_node/share/pdfjs",
      cMapUrl: "/fixture/cua_node/share/pdfjs/cmaps/",
      standardFontDataUrl: "/fixture/cua_node/share/pdfjs/standard_fonts/",
      wasmUrl: null,
      workerSrc:
        "file:///fixture/cua_node/lib/node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs",
    },
    tesseract: {
      tessdataRoot: "/fixture/cua_node/share/tessdata",
      languages: ["eng", "osd"],
    },
    licenses: { root: "/fixture/cua_node/licenses" },
    sbomPath: "/fixture/cua_node/sbom.cdx.json",
  };
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata,
  });
  const server = new McpServer({ manager });
  try {
    const response = (await server.dispatch(
      js(
        1,
        `nodeRepl.write(JSON.stringify({
          version: nodeRepl.runtime.version,
          node: nodeRepl.runtime.node,
          browser: nodeRepl.runtime.browser,
          languages: nodeRepl.runtime.tesseract.languages,
          frozen: Object.isFrozen(nodeRepl.runtime) && Object.isFrozen(nodeRepl.runtime.pdfjs) && Object.isFrozen(nodeRepl.runtime.tesseract.languages),
          mutated: Reflect.set(nodeRepl.runtime.node, "version", "changed"),
          loaders: Object.keys(nodeRepl.loaders),
          genericLoader: typeof nodeRepl.loaders.load,
        }))`,
      ),
    )) as Response;
    const text = response.result?.content?.[0]?.text;
    assert.equal(typeof text, "string");
    assert.deepEqual(JSON.parse(text as string), {
      version: 1,
      node: { version: "24.14.0", execPath: "/fixture/cua_node/bin/node" },
      browser: {
        playwrightRoot: "/fixture/cua_node/share/playwright",
        executablePath: "/fixture/brave-origin",
        executableKind: "brave-origin",
      },
      languages: ["eng", "osd"],
      frozen: true,
      mutated: false,
      loaders: ["canvas", "pdfjs", "pixelmatch", "playwright", "sharp", "tesseract"],
      genericLoader: "undefined",
    });
  } finally {
    await server.close();
  }
});

test("persistent binding analysis ignores Browser bootstrap locals and nested declarations", async () => {
  await withServer(async (server) => {
    const bootstrap = result(
      (await server.dispatch(
        js(
          1,
          `globalThis.agent = { browsers: {} };
if (globalThis.agent?.browsers == null) {
  const { setupBrowserRuntime } = await import("node:module");
  await setupBrowserRuntime({ globals: globalThis });
}
{
  const nestedBlock = 1;
}
function persistentFactory() {
  const nestedFunction = 2;
  return nestedFunction;
}
try {
  throw new Error("fixture");
} catch (nestedCatch) {
  const nestedCatchValue = nestedCatch.message;
}
nodeRepl.write("bootstrapped");`,
        ),
      )) as Response,
    );
    assert.equal(bootstrap?.isError, false);
    assert.deepEqual(bootstrap?.content, [{ type: "text", text: "bootstrapped" }]);

    const persisted = result(
      (await server.dispatch(
        js(
          2,
          "nodeRepl.write([persistentFactory(), typeof setupBrowserRuntime, typeof nestedBlock, typeof nestedFunction, typeof nestedCatch, typeof nestedCatchValue].join('|'))",
        ),
      )) as Response,
    );
    assert.deepEqual(persisted?.content, [
      {
        type: "text",
        text: "2|undefined|undefined|undefined|undefined|undefined",
      },
    ]);
  });
});

test("top-level nested destructuring bindings persist across cells", async () => {
  await withServer(async (server) => {
    const declaration = result(
      (await server.dispatch(
        js(
          1,
          `const { branch: { leaf = 4 }, list: [first, ...remaining], ...objectRest } = { branch: {}, list: [1, 2, 3], extra: 5 };
let [head, , { value: renamed = 7 }, ...arrayRest] = [6, 0, {}, 8, 9];
var { reusable = 10 } = {};`,
        ),
      )) as Response,
    );
    assert.equal(declaration?.isError, false);

    const persisted = result(
      (await server.dispatch(
        js(
          2,
          "nodeRepl.write(JSON.stringify({ leaf, first, remaining, objectRest, head, renamed, arrayRest, reusable }))",
        ),
      )) as Response,
    );
    assert.deepEqual(persisted?.content, [
      {
        type: "text",
        text: '{"leaf":4,"first":1,"remaining":[2,3],"objectRest":{"extra":5},"head":6,"renamed":7,"arrayRest":[8,9],"reusable":10}',
      },
    ]);

    const varRedeclaration = result(
      (await server.dispatch(js(3, "var { reusable = 11 } = {}"))) as Response,
    );
    assert.equal(varRedeclaration?.isError, false);
  });
});

test("module-scoped var declarations in top-level control flow persist", async () => {
  await withServer(async (server) => {
    const declaration = result(
      (await server.dispatch(
        js(
          1,
          `if (true) { var blockVar = 1; }
for (var loopVar of [2]) { var loopBodyVar = loopVar + 1; }
try { var tryVar = 4; } finally { var finallyVar = 5; }
function localScope() { var functionLocal = 6; return functionLocal; }
nodeRepl.write(localScope());`,
        ),
      )) as Response,
    );
    assert.deepEqual(declaration?.content, [{ type: "text", text: "6" }]);

    const persisted = result(
      (await server.dispatch(
        js(
          2,
          "nodeRepl.write([blockVar, loopVar, loopBodyVar, tryVar, finallyVar, typeof functionLocal].join('|'))",
        ),
      )) as Response,
    );
    assert.deepEqual(persisted?.content, [
      { type: "text", text: "1|2|3|4|5|undefined" },
    ]);
  });
});

test("static module syntax remains a cell error without resetting bindings", async () => {
  await withServer(async (server) => {
    const declaration = result(
      (await server.dispatch(js(1, "const survivesStaticSyntax = 12"))) as Response,
    );
    assert.equal(declaration?.isError, false);

    const staticImport = result(
      (await server.dispatch(js(2, 'import value from "fixture"'))) as Response,
    );
    assert.equal(staticImport?.isError, true);
    assert.match(
      String(staticImport?.content?.[0]?.text),
      /Top-level static import is not supported in node_repl/u,
    );

    const staticExport = result(
      (await server.dispatch(js(3, "export const value = 1"))) as Response,
    );
    assert.equal(staticExport?.isError, true);
    assert.match(
      String(staticExport?.content?.[0]?.text),
      /Top-level export is not supported in node_repl cells/u,
    );

    const persisted = result(
      (await server.dispatch(
        js(4, "nodeRepl.write(survivesStaticSyntax)"),
      )) as Response,
    );
    assert.deepEqual(persisted?.content, [{ type: "text", text: "12" }]);
  });
});

test("unknown MCP notifications are silent", async () => {
  await withServer(async (server) => {
    assert.equal(
      await server.dispatch({
        jsonrpc: "2.0",
        method: "notifications/unknown",
        params: {},
      }),
      null,
    );
  });
});

test("image content consumes the resolved output-metadata shape", async () => {
  await withServer(async (server) => {
    const response = result(
      (await server.dispatch(
        js(
          1,
          "await nodeRepl.emitImage('data:image/png;base64,iVBORw0KGgo='); await nodeRepl.emitImage(new Uint8Array([0xff, 0xd8, 0xff]))",
        ),
      )) as Response,
    );
    const expectedShape = outputMetadata.mcp_image_mapping.shape;
    assert.deepEqual(
      Object.keys(response?.content?.[0] ?? {}).sort(),
      Object.keys(expectedShape).sort(),
    );
    assert.deepEqual(response?.content?.[0]?._meta, {
      "codex/imageDetail": "original",
    });
    assert.deepEqual(response?.content?.[0], {
      type: expectedShape.type,
      data: "iVBORw0KGgo=",
      mimeType: "image/png",
      _meta: expectedShape._meta,
    });
    assert.deepEqual(response?.content?.[1], {
      type: "image",
      data: "/9j/",
      mimeType: "image/jpeg",
      _meta: { "codex/imageDetail": "original" },
    });
  });
});

test("module roots persist across reset and local files reload between cells", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-test-"));
  const modulePath = join(directory, "fixture.mjs");
  try {
    await writeFile(modulePath, "export const value = 'one';\n", "utf8");
    const manager = new RuntimeManager({
      allowHostNode: true,
      nodePath: TEST_NODE_PATH,
      runtimeMetadata: null,
      env: { NODE_REPL_NODE_MODULE_DIRS: "" },
    });
    const server = new McpServer({ manager });
    try {
      const added = await server.dispatch({
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: {
          name: "js_add_node_module_dir",
          arguments: {
            path: join(
              import.meta.dir,
              "fixtures",
              "fake-runtime",
              "lib",
              "node_modules",
            ),
          },
        },
      });
      assert.deepEqual(result(added as Response)?.content, [
        { type: "text", text: "true" },
      ]);
      const packageImport = result(
        (await server.dispatch(
          js(
            2,
            "var fakeSky = await import('@heliasar/sky-cua'); nodeRepl.write(fakeSky.sky.__fake_lazy)",
          ),
        )) as Response,
      );
      assert.deepEqual(packageImport?.content, [{ type: "text", text: "true" }]);
      const localImport = result(
        (await server.dispatch(
          js(
            3,
            `var localFixture = await import(${JSON.stringify(`file://${modulePath}`)}); nodeRepl.write(localFixture.value)`,
          ),
        )) as Response,
      );
      assert.deepEqual(localImport?.content, [{ type: "text", text: "one" }]);
      await writeFile(modulePath, "export const value = 'two';\n", "utf8");
      const reloaded = result(
        (await server.dispatch(
          js(
            4,
            `var localFixtureAgain = await import(${JSON.stringify(`file://${modulePath}`)}); nodeRepl.write(localFixtureAgain.value)`,
          ),
        )) as Response,
      );
      assert.deepEqual(reloaded?.content, [{ type: "text", text: "two" }]);
      const reset = result(
        (await server.dispatch({
          jsonrpc: "2.0",
          id: 5,
          method: "tools/call",
          params: { name: "js_reset", arguments: {} },
        })) as Response,
      );
      assert.deepEqual(reset?.content, [{ type: "text", text: "true" }]);
      const afterReset = result(
        (await server.dispatch(
          js(
            6,
            "var postResetSky = await import('@heliasar/sky-cua'); nodeRepl.write(postResetSky.sky.__fake_lazy)",
          ),
        )) as Response,
      );
      assert.deepEqual(afterReset?.content, [{ type: "text", text: "true" }]);
    } finally {
      await server.close();
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("trusted package entrypoints receive only the hash-gated privileged surface", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-trusted-"));
  const packageDirectory = join(directory, "node_modules", "trusted-fixture");
  const entrypoint = join(packageDirectory, "index.mjs");
  try {
    await mkdir(packageDirectory, { recursive: true });
    await writeFile(
      join(packageDirectory, "package.json"),
      JSON.stringify({
        name: "trusted-fixture",
        type: "module",
        exports: { ".": "./index.mjs" },
      }),
      "utf8",
    );
    await writeFile(
      entrypoint,
      "import { createRequire } from 'node:module'; const importedRequiredProcess = createRequire(import.meta.url)('node:process'); const builtinRequiredProcess = process.getBuiltinModule('module').createRequire(import.meta.url)('node:process'); export const safeModuleFacades = [importedRequiredProcess, builtinRequiredProcess].every((value) => typeof value.exit === 'undefined' && typeof value.send === 'undefined'); export const own = Object.keys(nodeRepl).join(','); export const inheritedWrite = Object.prototype.hasOwnProperty.call(nodeRepl, 'write'); export const processFrozen = Object.isFrozen(process); export const processExitFacade = typeof process.once + '|' + typeof process.off + '|' + typeof process.nextTick; export const scheduleLateHook = () => setTimeout(() => nodeRepl.addAfterSubmittedCodeHook({ run: () => nodeRepl.write('late-hook'), timeoutMs: 10 }), 20); export const wait = () => nodeRepl.withSuspendedTimeout(async () => { await new Promise((resolve) => setTimeout(resolve, 40)); return 'suspended'; });\n",
      "utf8",
    );
    const digest = createHash("sha256")
      .update(await readFile(entrypoint))
      .digest("hex");
    const manager = new RuntimeManager({
      allowHostNode: true,
      nodePath: TEST_NODE_PATH,
      runtimeMetadata: null,
      env: {
        NODE_REPL_NODE_MODULE_DIRS: join(directory, "node_modules"),
        NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
      },
    });
    const server = new McpServer({ manager });
    try {
      const response = result(
        (await server.dispatch(
          js(
            1,
            "var trustedFixture = await import('trusted-fixture'); nodeRepl.write([trustedFixture.own, trustedFixture.inheritedWrite, trustedFixture.processFrozen, trustedFixture.processExitFacade, trustedFixture.safeModuleFacades].join('|'))",
          ),
        )) as Response,
      );
      assert.deepEqual(response?.content, [
        {
          type: "text",
          text: "addAfterSubmittedCodeHook,gaasBrowserConfig,launchServices,config,env,createElicitation,fetch,nativePipe,withSuspendedTimeout|false|true|function|function|function|true",
        },
      ]);
      const suspended = result(
        (await server.dispatch(
          js(
            2,
            "var trustedFixtureWait = await import('trusted-fixture'); nodeRepl.write(await trustedFixtureWait.wait())",
            25,
          ),
        )) as Response,
      );
      assert.deepEqual(suspended?.content, [{ type: "text", text: "suspended" }]);
      const lateHook = result(
        (await server.dispatch(
          js(3, "trustedFixture.scheduleLateHook(); nodeRepl.write('first')"),
        )) as Response,
      );
      assert.deepEqual(lateHook?.content, [{ type: "text", text: "first" }]);
      await delay(45);
      const afterLateHook = result(
        (await server.dispatch(js(4, "nodeRepl.write('second')"))) as Response,
      );
      assert.deepEqual(afterLateHook?.content, [{ type: "text", text: "second" }]);
    } finally {
      await server.close();
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("an exact Browser client hash trusts a direct file import and a wrong hash does not", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-trusted-file-"));
  const entrypoint = join(directory, "browser-client.mjs");
  try {
    await writeFile(
      entrypoint,
      "export const nativePipeType = typeof nodeRepl.nativePipe?.createConnection;\n",
      "utf8",
    );
    const digest = createHash("sha256")
      .update(await readFile(entrypoint))
      .digest("hex");
    for (const [name, configuredHash, expected] of [
      ["exact", digest, "function"],
      ["wrong", "0".repeat(64), "undefined"],
    ] as const) {
      const manager = new RuntimeManager({
        allowHostNode: true,
        nodePath: TEST_NODE_PATH,
        runtimeMetadata: null,
        env: { NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: configuredHash },
      });
      const server = new McpServer({ manager });
      try {
        const response = result(
          (await server.dispatch(
            js(
              1,
              `const browserClient = await import(${JSON.stringify(`file://${entrypoint}`)}); nodeRepl.write(browserClient.nativePipeType)`,
            ),
          )) as Response,
        );
        assert.deepEqual(response?.content, [{ type: "text", text: expected }], name);
      } finally {
        await server.close();
      }
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("timeout and cancellation reset the child and leave the next call usable", async () => {
  await withServer(async (server) => {
    const timedOut = result(
      (await server.dispatch(js(1, "await new Promise(() => {})", 25))) as Response,
    );
    assert.equal(timedOut?.isError, true);
    assert.match(String(timedOut?.content?.[0]?.text), /js execution timed out/u);
    const afterTimeout = result(
      (await server.dispatch(js(2, "nodeRepl.write('fresh')"))) as Response,
    );
    assert.deepEqual(afterTimeout?.content, [{ type: "text", text: "fresh" }]);
    const cancellation = server.dispatch(js(3, "await new Promise(() => {})", 5_000));
    await new Promise((resolve) => setTimeout(resolve, 10));
    await server.dispatch({
      jsonrpc: "2.0",
      method: "notifications/cancelled",
      params: { requestId: 3 },
    });
    const cancelled = result((await cancellation) as Response);
    assert.equal(cancelled?.isError, true);
    assert.match(String(cancelled?.content?.[0]?.text), /cancelled/u);
    const afterCancel = result(
      (await server.dispatch(js(4, "nodeRepl.write('usable')"))) as Response,
    );
    assert.deepEqual(afterCancel?.content, [{ type: "text", text: "usable" }]);
  });
});

test("admission is atomic, cancellation uses the exact MCP id, and cold-start tombstones are consumed", async () => {
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  manager.cancel("cold-start");
  await assert.rejects(
    manager.execute("nodeRepl.write('should not start')", {
      requestId: "cold-start",
      requestMeta: null,
    }),
    /cancelled/u,
  );
  const first = manager.execute("await new Promise(() => {})", {
    requestId: 7,
    timeoutMs: 25,
    requestMeta: null,
  });
  await assert.rejects(
    manager.execute("nodeRepl.write('second')", {
      requestId: "7",
      requestMeta: null,
    }),
    /another js execution is already active/u,
  );
  manager.cancel("7");
  await assert.rejects(first, /timed out/u);
  manager.cancel(7);
  await manager.close();
});

test("idle cancellation tombstones are bounded and retain the newest ids", async () => {
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  try {
    for (let index = 0; index <= MAX_CANCELLATION_TOMBSTONES; index += 1)
      manager.cancel(index);
    const state = manager as unknown as {
      cancellationTombstones: Set<string>;
    };
    assert.equal(state.cancellationTombstones.size, MAX_CANCELLATION_TOMBSTONES);

    const evicted = await manager.execute("nodeRepl.write('evicted-runs')", {
      requestId: 0,
      requestMeta: null,
      timeoutMs: 5_000,
    });
    assert.equal(evicted.output, "evicted-runs");
    await assert.rejects(
      manager.execute("nodeRepl.write('recent-does-not-run')", {
        requestId: MAX_CANCELLATION_TOMBSTONES,
        requestMeta: null,
        timeoutMs: 5_000,
      }),
      /cancelled/u,
    );
  } finally {
    await manager.close();
  }
});

test("a late cancellation does not poison legal request-id reuse", async () => {
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  try {
    const first = await manager.execute("nodeRepl.write('first')", {
      requestId: 7,
      requestMeta: null,
      timeoutMs: 5_000,
    });
    assert.equal(first.output, "first");
    manager.cancel(7);
    const reused = await manager.execute("nodeRepl.write('reused')", {
      requestId: 7,
      requestMeta: null,
      timeoutMs: 5_000,
    });
    assert.equal(reused.output, "reused");
  } finally {
    await manager.close();
  }
});

test("late async callbacks stay in their finished execution context", async () => {
  await withServer(async (server) => {
    const first = result(
      (await server.dispatch(
        js(1, "setTimeout(() => nodeRepl.write('late'), 35); nodeRepl.write('first')"),
      )) as Response,
    );
    assert.deepEqual(first?.content, [{ type: "text", text: "first" }]);
    await delay(55);
    const second = result(
      (await server.dispatch(js(2, "nodeRepl.write('second')"))) as Response,
    );
    assert.deepEqual(second?.content, [{ type: "text", text: "second" }]);
  });
});

test("trusted packages use Node resolution, exact binary fetch bytes, TOML serialization, and protected config checks", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-node-resolution-"));
  const packageDirectory = join(directory, "node_modules", "conditional-fixture");
  const outputPath = join(directory, "output.toml");
  const protectedLink = join(directory, "protected-config.toml");
  const entrypoint = join(packageDirectory, "index.mjs");
  const server = createServer((request, response) => {
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => {
      assert.deepEqual(Buffer.concat(chunks), Buffer.from([0x00, 0xff, 0x17]));
      response.writeHead(200, { "content-type": "application/octet-stream" });
      response.end(Buffer.from([0x04, 0x00, 0xfe, 0x19]));
    });
  });
  try {
    await mkdir(packageDirectory, { recursive: true });
    await writeFile(
      join(packageDirectory, "package.json"),
      JSON.stringify({
        name: "conditional-fixture",
        type: "module",
        exports: {
          ".": { import: "./index.mjs", require: "./index.cjs" },
          "./common": { import: "./common.cjs", require: "./common.cjs" },
        },
      }),
      "utf8",
    );
    await writeFile(
      join(packageDirectory, "index.cjs"),
      "module.exports = { kind: 'require' };\n",
      "utf8",
    );
    await writeFile(
      join(packageDirectory, "common.cjs"),
      "module.exports = { kind: 'commonjs', bytes: 3 };\n",
      "utf8",
    );
    await writeFile(
      entrypoint,
      "export const kind = 'import'; export async function readCommon() { const value = await import('conditional-fixture/common'); return [value.default.kind, value.kind, value.default.bytes].join('|'); } export async function fetchBytes(url) { const response = await nodeRepl.fetch(url, { method: 'POST', body: new Uint8Array([0xaa, 0x00, 0xff, 0x17]).subarray(1) }); return Array.from(new Uint8Array(await response.arrayBuffer())).join(','); } export function writeConfig(path) { return nodeRepl.config.writeToml(path, { title: 'fixture', enabled: true, nested: { count: 2 } }); }\n",
      "utf8",
    );
    const digest = createHash("sha256")
      .update(await readFile(entrypoint))
      .digest("hex");
    await symlink(
      join(process.env.HOME ?? "/tmp", ".codex", "config.toml"),
      protectedLink,
    );
    const addressPromise = new Promise<number>((resolve) =>
      server.listen(0, "127.0.0.1", () => {
        const address = server.address();
        assert.ok(address !== null && typeof address !== "string");
        resolve(address.port);
      }),
    );
    const port = await addressPromise;
    const manager = new RuntimeManager({
      allowHostNode: true,
      nodePath: TEST_NODE_PATH,
      runtimeMetadata: null,
      env: {
        NODE_REPL_NODE_MODULE_DIRS: join(directory, "node_modules"),
        NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: digest,
      },
    });
    const mcp = new McpServer({ manager });
    try {
      const imported = result(
        (await mcp.dispatch(
          js(
            1,
            "var conditionalFixture = await import('conditional-fixture'); nodeRepl.write([conditionalFixture.kind, await conditionalFixture.readCommon()].join('|'))",
          ),
        )) as Response,
      );
      assert.deepEqual(imported?.content, [
        { type: "text", text: "import|commonjs|commonjs|3" },
      ]);
      const fetched = result(
        (await mcp.dispatch(
          js(
            2,
            `nodeRepl.write(await conditionalFixture.fetchBytes(${JSON.stringify(`http://127.0.0.1:${port}/bytes`)}))`,
          ),
        )) as Response,
      );
      assert.deepEqual(fetched?.content, [{ type: "text", text: "4,0,254,25" }]);
      const written = result(
        (await mcp.dispatch(
          js(
            3,
            `await conditionalFixture.writeConfig(${JSON.stringify(outputPath)}); nodeRepl.write('written')`,
          ),
        )) as Response,
      );
      assert.deepEqual(written?.content, [{ type: "text", text: "written" }]);
      assert.equal(
        await readFile(outputPath, "utf8"),
        'title = "fixture"\nenabled = true\n\n[nested]\ncount = 2\n',
      );
      const protectedActual = result(
        (await mcp.dispatch(
          js(
            4,
            `await conditionalFixture.writeConfig(${JSON.stringify(join(process.env.HOME ?? "/tmp", ".codex", "config.toml"))})`,
          ),
        )) as Response,
      );
      assert.equal(protectedActual?.isError, true);
      const protectedSymlink = result(
        (await mcp.dispatch(
          js(
            5,
            `await conditionalFixture.writeConfig(${JSON.stringify(protectedLink)})`,
          ),
        )) as Response,
      );
      assert.equal(protectedSymlink?.isError, true);
    } finally {
      await mcp.close();
    }
  } finally {
    await new Promise<void>((resolve) => server.close(() => resolve()));
    await rm(directory, { recursive: true, force: true });
  }
});
