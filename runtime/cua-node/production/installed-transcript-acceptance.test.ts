import { strict as assert } from "node:assert";
import {
  chmodSync,
  mkdtempSync,
  mkdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "bun:test";
import contractFixture from "../test/fixtures/upstream-5307/contract.json";
import toolsFixture from "../test/fixtures/upstream-5307/tools-list.json";
import {
  parseArgs,
  runAcceptance,
  validateInstalledRuntime,
} from "./installed-transcript-acceptance";

const script = resolve(__dirname, "installed-transcript-acceptance.ts");

function fixtureServer(options: { unexpectedTool?: boolean } = {}): string {
  const tools = structuredClone(toolsFixture.tools) as Array<Record<string, unknown>>;
  if (options.unexpectedTool === true)
    tools.push({
      name: "unexpected",
      description: "must be rejected",
      inputSchema: { type: "object" },
    });
  return `#!${process.execPath}
import { createHash } from "node:crypto";
import { mkdirSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
const tools = ${JSON.stringify(tools)};
const initializeResult = ${JSON.stringify({
    protocolVersion: "2025-11-25",
    capabilities: contractFixture.mcp.initialize.capabilities,
    serverInfo: contractFixture.mcp.server_info,
    instructions: contractFixture.mcp.initialize.instructions,
  })};
const provenance = process.env.SKY_CUA_MCP_CALLER_PROVENANCE;
const sessionId = "fixture-" + provenance + "-" + process.pid;
let nextTurn = 1;
let clientInfo = null;
let counterExists = false;
let moduleDir = null;
const pending = new Set();
const output = (message) => process.stdout.write(JSON.stringify(message) + "\\n");
const success = (id, text, extra = {}) => output({jsonrpc:"2.0",id,result:{content:[{type:"text",text}],isError:false,...extra}});
const failure = (id, text) => output({jsonrpc:"2.0",id,result:{content:[{type:"text",text}],isError:true}});
const reader = createInterface({input:process.stdin,crlfDelay:Infinity});
reader.on("line", (line) => {
  const request = JSON.parse(line);
  if (request.method === "initialize") {
    clientInfo = request.params.clientInfo;
    output({jsonrpc:"2.0",id:request.id,result:initializeResult});
    return;
  }
  if (request.method === "notifications/initialized") return;
  if (request.method === "tools/list") {
    output({jsonrpc:"2.0",id:request.id,result:{tools}});
    return;
  }
  if (request.method === "notifications/cancelled") {
    const id = request.params.requestId;
    if (pending.delete(id)) failure(id, "js execution cancelled; kernel reset");
    return;
  }
  if (request.method === "shutdown") {
    output({jsonrpc:"2.0",id:request.id,result:null});
    reader.close();
    return;
  }
  if (request.method !== "tools/call") return;
  const id = request.id;
  const name = request.params.name;
  const args = request.params.arguments;
  const title = args?.title ?? "";
  const callTurn = sessionId + "-turn-" + nextTurn++;
  if (name === "js_reset") {
    counterExists = false;
    success(id, "true");
    return;
  }
  if (name === "js_add_node_module_dir") {
    const added = moduleDir !== args.path;
    moduleDir = args.path;
    success(id, String(added));
    return;
  }
  if (title.startsWith("Inspect ") || title === "Preserve Codex metadata") {
    let meta = request.params._meta;
    if (!(meta?.session_id && meta?.turn_id)) meta = {
      ...(meta ?? {}),
      session_id: sessionId,
      turn_id: callTurn,
      caller_provenance: provenance,
      client_info: clientInfo,
      identity_synthetic: true,
      "x-codex-turn-metadata": {session_id: sessionId, turn_id: callTurn},
    };
    success(id, JSON.stringify(meta));
    return;
  }
  if (title === "Create persistent binding") {
    counterExists = true;
    success(id, "41");
    return;
  }
  if (title === "Reuse persistent binding") {
    success(id, counterExists ? "42" : "missing");
    return;
  }
  if (title === "Load local and native modules") {
    success(id, JSON.stringify({
      marker:"installed-local-module",
      file_url:pathToFileURL(join(moduleDir, "installed-transcript-fixture", "index.mjs")).href,
      native_addon:true,
    }));
    return;
  }
  if (title === "Exercise installed file values") {
    const match = /var installedOutputPath = ("(?:[^"\\\\]|\\\\.)*")/.exec(args.code);
    const outputPath = JSON.parse(match[1]);
    const bytes = Buffer.from("installed transcript acceptance\\n", "utf8");
    mkdirSync(dirname(outputPath), {recursive:true});
    writeFileSync(outputPath, bytes);
    const dataUrl = "data:application/octet-stream;base64," + bytes.toString("base64");
    const fileUrl = pathToFileURL(outputPath).href;
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    output({jsonrpc:"2.0",id,result:{
      content:[
        {type:"text",text:JSON.stringify({path_round_trip:outputPath,buffer:true,array_buffer:true,data_url:dataUrl})},
        {type:"image",data:"iVBORw0KGgo=",mimeType:"image/png",_meta:{"codex/imageDetail":"original"}},
      ],
      isError:false,
      _meta:{output_path:outputPath,file_url:fileUrl,byte_length:bytes.length,sha256,data_url:dataUrl},
    }});
    return;
  }
  if (title === "Verify reset and module roots") {
    success(id, JSON.stringify({binding:counterExists ? "number" : "undefined",module:"installed-local-module"}));
    return;
  }
  if (title === "Verify execution timeout") {
    failure(id, "js execution timed out; kernel reset, rerun your request");
    return;
  }
  if (title === "Verify execution cancellation") {
    pending.add(id);
    return;
  }
  if (title === "Recover after timeout") return success(id, "timeout-recovered");
  if (title === "Recover after cancellation") return success(id, "cancel-recovered");
  failure(id, "unexpected fixture call: " + title);
});
reader.on("close", () => process.exit(0));
`;
}

function createInstalledFixture(options: {
  nodeVersion?: string;
  unexpectedTool?: boolean;
} = {}): string {
  const root = mkdtempSync(join(tmpdir(), "cua-installed-transcript-fixture-"));
  mkdirSync(join(root, "bin"), { recursive: true });
  mkdirSync(join(root, "lib/node_modules"), { recursive: true });
  writeFileSync(
    join(root, "manifest.json"),
    JSON.stringify({
      target: "linux-x64-glibc",
      node_version: "24.14.0",
      node_path: "bin/node",
      node_repl_path: "bin/node_repl",
      node_modules: "lib/node_modules",
    }),
  );
  writeFileSync(
    join(root, "bin/node"),
    `#!/bin/sh\nprintf '%s\\n' '${options.nodeVersion ?? "v24.14.0"}'\n`,
  );
  writeFileSync(
    join(root, "bin/node_repl"),
    fixtureServer(
      options.unexpectedTool === undefined
        ? {}
        : { unexpectedTool: options.unexpectedTool },
    ),
  );
  chmodSync(join(root, "bin/node"), 0o755);
  chmodSync(join(root, "bin/node_repl"), 0o755);
  return root;
}

function runCli(arguments_: string[]) {
  return Bun.spawnSync([process.execPath, script, ...arguments_], {
    cwd: resolve(__dirname, ".."),
    env: process.env,
    stdout: "pipe",
    stderr: "pipe",
  });
}

test("argument and installed-runtime preflight reject incomplete or wrong candidates", () => {
  assert.throws(() => parseArgs([]), /--runtime-root/u);
  assert.throws(() => parseArgs(["--runtime-root=/tmp/x", "--wat"]), /unknown argument/u);
  const missing = mkdtempSync(join(tmpdir(), "cua-installed-transcript-missing-"));
  try {
    assert.throws(
      () => validateInstalledRuntime({ runtimeRoot: missing, timeoutMs: 30_000 }),
      /manifest is missing/u,
    );
  } finally {
    rmSync(missing, { recursive: true, force: true });
  }
  const wrongVersion = createInstalledFixture({ nodeVersion: "v23.0.0" });
  try {
    assert.throws(
      () => validateInstalledRuntime({ runtimeRoot: wrongVersion, timeoutMs: 30_000 }),
      /bundled Node version/u,
    );
  } finally {
    rmSync(wrongVersion, { recursive: true, force: true });
  }
});

test("deterministic installed fixture passes the complete transcript matrix", async () => {
  const root = createInstalledFixture();
  try {
    const report = await runAcceptance({ runtimeRoot: root, timeoutMs: 5_000 });
    assert.equal(report.status, "passed");
    assert.equal(report.runtime_root, root);
    assert.deepEqual(report.checks, {
      node_version: "24.14.0",
      tool_count: 3,
      initialize_client_info: true,
      openclaw_synthetic_identity: true,
      opencode_synthetic_identity: true,
      independent_process_sessions: true,
      process_cleanup: true,
      codex_metadata_exact: true,
      persistent_bindings: true,
      top_level_await: true,
      reset_and_module_persistence: true,
      timeout_and_recovery: true,
      cancellation_and_recovery: true,
      local_file_bytes: 32,
      native_addon_loaded: true,
      emitted_images: 1,
    });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("unexpected tools fail the exact installed surface gate", async () => {
  const root = createInstalledFixture({ unexpectedTool: true });
  try {
    await assert.rejects(
      runAcceptance({ runtimeRoot: root, timeoutMs: 5_000 }),
      /Expected values to be strictly deep-equal/u,
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("CLI emits a machine-readable failure report", () => {
  const result = runCli(["--runtime-root=/definitely/missing"]);
  assert.equal(result.exitCode, 1);
  const report = JSON.parse(result.stdout.toString()) as Record<string, unknown>;
  assert.equal(report.schema, "com.heliasar.cua-node.installed-transcript-acceptance");
  assert.equal(report.status, "failed");
  assert.match(String(report.error), /manifest is missing/u);
});
