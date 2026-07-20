import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  accessSync,
  constants,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import contractFixture from "../test/fixtures/upstream-5307/contract.json";
import toolsFixture from "../test/fixtures/upstream-5307/tools-list.json";
import {
  startMcpSession,
  type McpSession,
} from "./web-workbench-acceptance-helper";

type JsonObject = Record<string, unknown>;
type McpResponse = { id?: unknown; result?: unknown; error?: unknown };
type InstalledTranscriptOptions = {
  runtimeRoot: string;
  timeoutMs: number;
};
type InstalledRuntime = InstalledTranscriptOptions & {
  nodePath: string;
  nodeReplPath: string;
  nodeModulesPath: string;
};
type AcceptanceReport = {
  schema: "com.heliasar.cua-node.installed-transcript-acceptance";
  schema_version: 1;
  status: "passed" | "failed";
  runtime_root?: string;
  checks: Record<string, string | number | boolean>;
  error?: string;
};

const SCHEMA = "com.heliasar.cua-node.installed-transcript-acceptance" as const;
const NODE_VERSION = "24.14.0";
const PROTOCOL_VERSION = "2025-11-25";
const DEFAULT_TIMEOUT_MS = 30_000;
const CLEANUP_TIMEOUT_MS = 2_000;
const OUTPUT_BYTES = Buffer.from("installed transcript acceptance\n", "utf8");
const IMAGE_DATA = "iVBORw0KGgo=";

function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function executable(path: string): boolean {
  try {
    accessSync(path, constants.R_OK | constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

export function parseArgs(argv: string[]): InstalledTranscriptOptions {
  let runtimeRoot: string | undefined;
  let timeoutMs = DEFAULT_TIMEOUT_MS;
  for (const argument of argv) {
    if (argument.startsWith("--runtime-root=")) {
      runtimeRoot = resolve(argument.slice("--runtime-root=".length));
    } else if (argument.startsWith("--timeout-ms=")) {
      timeoutMs = Number(argument.slice("--timeout-ms=".length));
      if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 1_000)
        throw new Error("--timeout-ms must be an integer of at least 1000");
    } else if (argument !== "--json") {
      throw new Error(`unknown argument: ${argument}`);
    }
  }
  if (runtimeRoot === undefined)
    throw new Error("--runtime-root=<installed cua_node component> is required");
  return { runtimeRoot, timeoutMs };
}

export function validateInstalledRuntime(
  options: InstalledTranscriptOptions,
): InstalledRuntime {
  const manifestPath = join(options.runtimeRoot, "manifest.json");
  if (!existsSync(manifestPath))
    throw new Error(`installed runtime manifest is missing: ${manifestPath}`);
  let manifest: unknown;
  try {
    manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  } catch (error) {
    throw new Error(`installed runtime manifest is invalid JSON: ${errorText(error)}`);
  }
  if (!isObject(manifest)) throw new Error("installed runtime manifest must be an object");
  assert.equal(manifest.target, "linux-x64-glibc", "installed runtime target");
  assert.equal(manifest.node_version, NODE_VERSION, "manifest Node version");
  assert.equal(manifest.node_path, "bin/node", "manifest Node path");
  assert.equal(manifest.node_repl_path, "bin/node_repl", "manifest node_repl path");
  assert.equal(manifest.node_modules, "lib/node_modules", "manifest module path");

  const nodePath = join(options.runtimeRoot, "bin/node");
  const nodeReplPath = join(options.runtimeRoot, "bin/node_repl");
  const nodeModulesPath = join(options.runtimeRoot, "lib/node_modules");
  if (!executable(nodePath)) throw new Error(`bundled Node is not executable: ${nodePath}`);
  if (!executable(nodeReplPath))
    throw new Error(`installed node_repl is not executable: ${nodeReplPath}`);
  if (!existsSync(nodeModulesPath))
    throw new Error(`installed module directory is missing: ${nodeModulesPath}`);
  const version = spawnSync(nodePath, ["--version"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 5_000,
  });
  if (version.status !== 0)
    throw new Error(`bundled Node version probe failed: ${(version.stderr ?? "").trim()}`);
  assert.equal((version.stdout ?? "").trim(), `v${NODE_VERSION}`, "bundled Node version");
  return { ...options, nodePath, nodeReplPath, nodeModulesPath };
}

function resultObject(response: McpResponse, label: string): JsonObject {
  if (response.error !== undefined)
    throw new Error(`${label} returned an MCP error: ${JSON.stringify(response.error)}`);
  if (!isObject(response.result)) throw new Error(`${label} returned no result object`);
  return response.result;
}

function toolResult(response: McpResponse, label: string): JsonObject {
  const result = resultObject(response, label);
  if (result.isError === true) {
    const content = Array.isArray(result.content) ? result.content : [];
    const text = content.find(
      (entry) => isObject(entry) && entry.type === "text" && typeof entry.text === "string",
    );
    throw new Error(`${label} failed: ${isObject(text) ? String(text.text) : "unknown tool error"}`);
  }
  assert.equal(result.isError, false, `${label} isError`);
  assert.ok(Array.isArray(result.content), `${label} content`);
  return result;
}

function failedToolText(response: McpResponse, label: string): string {
  const result = resultObject(response, label);
  assert.equal(result.isError, true, `${label} must fail at the tool boundary`);
  const content = Array.isArray(result.content) ? result.content : [];
  const text = content.find(
    (entry) => isObject(entry) && entry.type === "text" && typeof entry.text === "string",
  );
  assert.ok(isObject(text), `${label} error text`);
  return String(text.text);
}

function textOf(result: JsonObject, label: string): string {
  const content = Array.isArray(result.content) ? result.content : [];
  const text = content.find(
    (entry) => isObject(entry) && entry.type === "text" && typeof entry.text === "string",
  );
  assert.ok(isObject(text), `${label} text content`);
  return String(text.text);
}

function parseJsonText(result: JsonObject, label: string): JsonObject {
  const parsed: unknown = JSON.parse(textOf(result, label));
  assert.ok(isObject(parsed), `${label} JSON output`);
  return parsed;
}

async function initializeAndList(
  session: McpSession,
  clientInfo: JsonObject,
): Promise<void> {
  const initialize = await session.request("initialize", {
    protocolVersion: PROTOCOL_VERSION,
    capabilities: {},
    clientInfo,
  });
  assert.deepEqual(resultObject(initialize, "initialize"), {
    protocolVersion: PROTOCOL_VERSION,
    capabilities: contractFixture.mcp.initialize.capabilities,
    serverInfo: contractFixture.mcp.server_info,
    instructions: contractFixture.mcp.initialize.instructions,
  });
  session.notify("notifications/initialized", {});
  const listed = resultObject(await session.request("tools/list", {}), "tools/list");
  assert.deepEqual(listed, { tools: toolsFixture.tools });
  assert.deepEqual(
    (listed.tools as Array<{ name: string }>).map((tool) => tool.name),
    ["js", "js_reset", "js_add_node_module_dir"],
  );
}

function sessionEnvironment(runtime: InstalledRuntime, provenance: string) {
  return {
    ...process.env,
    NODE_REPL_NODE_PATH: runtime.nodePath,
    NODE_REPL_NODE_MODULE_DIRS: runtime.nodeModulesPath,
    SKY_CUA_MCP_CALLER_PROVENANCE: provenance,
  };
}

async function withSession<T>(
  runtime: InstalledRuntime,
  tempRoot: string,
  provenance: string,
  run: (session: McpSession) => Promise<T>,
): Promise<T> {
  const session = startMcpSession({
    executable: runtime.nodeReplPath,
    cwd: tempRoot,
    env: sessionEnvironment(runtime, provenance),
    timeoutMs: runtime.timeoutMs,
  });
  let shutdownSucceeded = false;
  try {
    const value = await run(session);
    const shutdown = await session.request("shutdown", {});
    assert.equal(shutdown.error, undefined, "shutdown MCP error");
    assert.equal(shutdown.result, null, "shutdown result");
    shutdownSucceeded = true;
    return value;
  } finally {
    const exit = await session.close(CLEANUP_TIMEOUT_MS);
    if (shutdownSucceeded)
      assert.deepEqual(exit, { code: 0, signal: null }, session.stderr().trim());
  }
}

async function verifySyntheticIdentity(
  runtime: InstalledRuntime,
  tempRoot: string,
  provenance: "openclaw" | "opencode",
  clientInfo: JsonObject,
): Promise<string> {
  return withSession(runtime, tempRoot, provenance, async (session) => {
    await initializeAndList(session, clientInfo);
    const readMeta = async (progressToken: number): Promise<JsonObject> => {
      const result = toolResult(
        await session.request("tools/call", {
          name: "js",
          arguments: {
            title: `Inspect ${provenance} identity`,
            code: "nodeRepl.write(JSON.stringify(nodeRepl.requestMeta))",
          },
          _meta: { progressToken },
        }),
        `${provenance} metadata`,
      );
      return parseJsonText(result, `${provenance} metadata`);
    };
    const first = await readMeta(11);
    const second = await readMeta(12);
    assert.equal(first.caller_provenance, provenance);
    assert.equal(first.identity_synthetic, true);
    assert.deepEqual(first.client_info, clientInfo);
    assert.equal(first.progressToken, 11);
    assert.equal(second.progressToken, 12);
    assert.equal(typeof first.session_id, "string");
    assert.equal(first.session_id, second.session_id, "synthetic process session stability");
    assert.equal(typeof first.turn_id, "string");
    assert.notEqual(first.turn_id, second.turn_id, "one synthetic turn per tools/call");
    assert.deepEqual(first["x-codex-turn-metadata"], {
      session_id: first.session_id,
      turn_id: first.turn_id,
    });
    assert.deepEqual(second["x-codex-turn-metadata"], {
      session_id: second.session_id,
      turn_id: second.turn_id,
    });
    return String(first.session_id);
  });
}

async function verifyCodexTranscript(
  runtime: InstalledRuntime,
  tempRoot: string,
): Promise<Record<string, string | number | boolean>> {
  const moduleRoot = join(tempRoot, "module fixture");
  const nodeModules = join(moduleRoot, "node_modules");
  const fixturePackage = join(nodeModules, "installed-transcript-fixture");
  mkdirSync(fixturePackage, { recursive: true });
  writeFileSync(
    join(fixturePackage, "package.json"),
    JSON.stringify({ name: "installed-transcript-fixture", type: "module", exports: "./index.mjs" }),
  );
  writeFileSync(
    join(fixturePackage, "index.mjs"),
    "export const marker = 'installed-local-module'; export const moduleUrl = import.meta.url;\n",
  );
  const outputPath = join(tempRoot, "outputs", "local result.bin");
  const suppliedMeta = {
    session_id: "codex-installed-session",
    turn_id: "codex-installed-turn",
    "x-codex-turn-metadata": {
      session_id: "codex-installed-session",
      turn_id: "codex-installed-turn",
      thread_id: "codex-installed-thread",
    },
    opaque: { nested: [1, true, "preserved"] },
  };

  return withSession(runtime, tempRoot, "codex_desktop", async (session) => {
    await initializeAndList(session, {
      name: "Codex Desktop",
      version: "installed-acceptance",
    });
    const meta = parseJsonText(
      toolResult(
        await session.request("tools/call", {
          name: "js",
          arguments: {
            title: "Preserve Codex metadata",
            code: "nodeRepl.write(JSON.stringify(nodeRepl.requestMeta))",
          },
          _meta: suppliedMeta,
        }),
        "Codex metadata",
      ),
      "Codex metadata",
    );
    assert.deepEqual(meta, suppliedMeta, "supplied Codex _meta must be exact");

    assert.equal(
      textOf(
        toolResult(
          await session.request("tools/call", {
            name: "js",
            arguments: {
              title: "Create persistent binding",
              code: "var installedTranscriptCounter = 40; await Promise.resolve(); nodeRepl.write(installedTranscriptCounter + 1)",
            },
          }),
          "persistent binding creation",
        ),
        "persistent binding creation",
      ),
      "41",
    );
    assert.equal(
      textOf(
        toolResult(
          await session.request("tools/call", {
            name: "js",
            arguments: {
              title: "Reuse persistent binding",
              code: "await Promise.resolve(); nodeRepl.write(installedTranscriptCounter + 2)",
            },
          }),
          "persistent binding reuse",
        ),
        "persistent binding reuse",
      ),
      "42",
    );

    const addModule = toolResult(
      await session.request("tools/call", {
        name: "js_add_node_module_dir",
        arguments: { path: nodeModules },
      }),
      "add module directory",
    );
    assert.equal(textOf(addModule, "add module directory"), "true");
    const duplicateModule = toolResult(
      await session.request("tools/call", {
        name: "js_add_node_module_dir",
        arguments: { path: nodeModules },
      }),
      "duplicate module directory",
    );
    assert.equal(textOf(duplicateModule, "duplicate module directory"), "false");
    const moduleEvidence = parseJsonText(
      toolResult(
        await session.request("tools/call", {
          name: "js",
          arguments: {
            title: "Load local and native modules",
            code: "var installedLocalModule = await import('installed-transcript-fixture'); var installedCanvasModule = await import('@napi-rs/canvas'); nodeRepl.write(JSON.stringify({marker: installedLocalModule.marker, file_url: installedLocalModule.moduleUrl, native_addon: typeof installedCanvasModule.createCanvas === 'function'}))",
          },
        }),
        "local and native module import",
      ),
      "local and native module import",
    );
    assert.deepEqual(moduleEvidence, {
      marker: "installed-local-module",
      file_url: pathToFileURL(join(fixturePackage, "index.mjs")).href,
      native_addon: true,
    });

    const expectedSha = createHash("sha256").update(OUTPUT_BYTES).digest("hex");
    const localFile = toolResult(
      await session.request("tools/call", {
        name: "js",
        arguments: {
          title: "Exercise installed file values",
          code: `var installedFs = await import('node:fs/promises'); var installedPath = await import('node:path'); var installedUrl = await import('node:url'); var installedCrypto = await import('node:crypto'); var installedOutputPath = ${JSON.stringify(outputPath)}; await installedFs.mkdir(installedPath.dirname(installedOutputPath), {recursive:true}); var installedBuffer = Buffer.from(${JSON.stringify(OUTPUT_BYTES.toString("utf8"))}, 'utf8'); await installedFs.writeFile(installedUrl.pathToFileURL(installedOutputPath), installedBuffer); var installedReadBuffer = await installedFs.readFile(installedOutputPath); var installedArrayBuffer = installedReadBuffer.buffer.slice(installedReadBuffer.byteOffset, installedReadBuffer.byteOffset + installedReadBuffer.byteLength); var installedDataUrl = 'data:application/octet-stream;base64,' + Buffer.from(installedArrayBuffer).toString('base64'); var installedFileUrl = installedUrl.pathToFileURL(installedOutputPath).href; var installedSha = installedCrypto.createHash('sha256').update(installedReadBuffer).digest('hex'); nodeRepl.setResponseMeta({output_path: installedOutputPath, file_url: installedFileUrl, byte_length: installedReadBuffer.byteLength, sha256: installedSha, data_url: installedDataUrl}); await nodeRepl.emitImage('data:image/png;base64,${IMAGE_DATA}'); nodeRepl.write(JSON.stringify({path_round_trip: installedUrl.fileURLToPath(installedFileUrl), buffer: Buffer.isBuffer(installedReadBuffer), array_buffer: installedArrayBuffer instanceof ArrayBuffer, data_url: installedDataUrl}))`,
        },
      }),
      "local file and output metadata",
    );
    assert.deepEqual(parseJsonText(localFile, "local file and output metadata"), {
      path_round_trip: outputPath,
      buffer: true,
      array_buffer: true,
      data_url: `data:application/octet-stream;base64,${OUTPUT_BYTES.toString("base64")}`,
    });
    assert.deepEqual(localFile._meta, {
      output_path: outputPath,
      file_url: pathToFileURL(outputPath).href,
      byte_length: OUTPUT_BYTES.length,
      sha256: expectedSha,
      data_url: `data:application/octet-stream;base64,${OUTPUT_BYTES.toString("base64")}`,
    });
    const image = (localFile.content as unknown[]).find(
      (entry) => isObject(entry) && entry.type === "image",
    );
    assert.deepEqual(image, {
      type: "image",
      data: IMAGE_DATA,
      mimeType: "image/png",
      _meta: { "codex/imageDetail": "original" },
    });
    assert.deepEqual(readFileSync(outputPath), OUTPUT_BYTES);

    const reset = toolResult(
      await session.request("tools/call", { name: "js_reset", arguments: {} }),
      "js_reset",
    );
    assert.equal(textOf(reset, "js_reset"), "true");
    const afterReset = parseJsonText(
      toolResult(
        await session.request("tools/call", {
          name: "js",
          arguments: {
            title: "Verify reset and module roots",
            code: "var installedModuleAfterReset = await import('installed-transcript-fixture'); nodeRepl.write(JSON.stringify({binding: typeof installedTranscriptCounter, module: installedModuleAfterReset.marker}))",
          },
        }),
        "after reset",
      ),
      "after reset",
    );
    assert.deepEqual(afterReset, {
      binding: "undefined",
      module: "installed-local-module",
    });

    const timeout = await session.request("tools/call", {
      name: "js",
      arguments: {
        title: "Verify execution timeout",
        code: "await new Promise(() => {})",
        timeout_ms: 25,
      },
    });
    assert.match(failedToolText(timeout, "timeout"), /timed out.*kernel reset/u);
    assert.equal(
      textOf(
        toolResult(
          await session.request("tools/call", {
            name: "js",
            arguments: { title: "Recover after timeout", code: "nodeRepl.write('timeout-recovered')" },
          }),
          "timeout recovery",
        ),
        "timeout recovery",
      ),
      "timeout-recovered",
    );

    const cancellation = session.requestWithId(
      "tools/call",
      {
        name: "js",
        arguments: {
          title: "Verify execution cancellation",
          code: "await new Promise(() => {})",
          timeout_ms: 10_000,
        },
      },
      runtime.timeoutMs,
    );
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 25));
    session.notify("notifications/cancelled", {
      requestId: cancellation.id,
      reason: "installed transcript acceptance",
    });
    assert.match(
      failedToolText(await cancellation.response, "cancellation"),
      /cancelled.*kernel reset/u,
    );
    assert.equal(
      textOf(
        toolResult(
          await session.request("tools/call", {
            name: "js",
            arguments: { title: "Recover after cancellation", code: "nodeRepl.write('cancel-recovered')" },
          }),
          "cancellation recovery",
        ),
        "cancellation recovery",
      ),
      "cancel-recovered",
    );

    return {
      codex_metadata_exact: true,
      persistent_bindings: true,
      top_level_await: true,
      reset_and_module_persistence: true,
      timeout_and_recovery: true,
      cancellation_and_recovery: true,
      local_file_bytes: OUTPUT_BYTES.length,
      native_addon_loaded: true,
      emitted_images: 1,
    };
  });
}

export async function runAcceptance(
  options: InstalledTranscriptOptions,
): Promise<AcceptanceReport> {
  const runtime = validateInstalledRuntime(options);
  const tempRoot = mkdtempSync(join(tmpdir(), "cua-node-installed-transcript-"));
  try {
    const codexChecks = await verifyCodexTranscript(runtime, tempRoot);
    const openclawSession = await verifySyntheticIdentity(
      runtime,
      tempRoot,
      "openclaw",
      { name: "OpenClaw", version: "installed-acceptance" },
    );
    const opencodeSession = await verifySyntheticIdentity(
      runtime,
      tempRoot,
      "opencode",
      { name: "OpenCode", version: "installed-acceptance" },
    );
    assert.notEqual(openclawSession, opencodeSession, "independent process sessions");
    return {
      schema: SCHEMA,
      schema_version: 1,
      status: "passed",
      runtime_root: runtime.runtimeRoot,
      checks: {
        node_version: NODE_VERSION,
        tool_count: 3,
        initialize_client_info: true,
        openclaw_synthetic_identity: true,
        opencode_synthetic_identity: true,
        independent_process_sessions: true,
        process_cleanup: true,
        ...codexChecks,
      },
    };
  } finally {
    rmSync(tempRoot, { recursive: true, force: true });
    if (existsSync(tempRoot))
      throw new Error(`acceptance temporary root was not removed: ${tempRoot}`);
  }
}

async function main(argv = process.argv.slice(2)): Promise<number> {
  let runtimeRoot: string | undefined;
  try {
    const options = parseArgs(argv);
    runtimeRoot = options.runtimeRoot;
    const report = await runAcceptance(options);
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return 0;
  } catch (error) {
    const report: AcceptanceReport = {
      schema: SCHEMA,
      schema_version: 1,
      status: "failed",
      ...(runtimeRoot === undefined ? {} : { runtime_root: runtimeRoot }),
      checks: {},
      error: errorText(error),
    };
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return 1;
  }
}

if (import.meta.main)
  void main().then((exitCode) => {
    process.exitCode = exitCode;
  });
