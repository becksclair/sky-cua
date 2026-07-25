import { mkdir, mkdtemp, rm, symlink } from "node:fs/promises";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "bun:test";
import { strict as assert } from "node:assert";
import toolsFixture from "../../test/fixtures/upstream-5307/tools-list.json";
import transcriptFixture from "../../test/fixtures/upstream-5307/mcp-transcripts.json";
import { TEST_NODE_PATH } from "../test-node-path.ts";

interface JsonRpcMessage {
  jsonrpc: "2.0";
  id?: string | number;
  result?: Record<string, unknown>;
  error?: Record<string, unknown>;
}

test("built host reproduces the golden initialize, tools, and js transcript", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-golden-"));
  try {
    const build = await Bun.build({
      entrypoints: [join(import.meta.dir, "../../src/cli.ts")],
      outdir: directory,
      target: "node",
      naming: "node-repl.js",
    });
    assert.equal(build.success, true, build.logs.map((log) => log.message).join("\n"));
    const isolatedExactNodePath = join(directory, "bin", "node");
    await mkdir(join(directory, "bin"));
    await symlink(TEST_NODE_PATH, isolatedExactNodePath);
    const hostExecutable = isolatedExactNodePath;
    const child = spawn(hostExecutable, [join(directory, "node-repl.js")], {
      cwd: directory,
      env: {
        ...process.env,
        NODE_REPL_ALLOW_HOST_NODE: "1",
        NODE_REPL_NODE_PATH: isolatedExactNodePath,
      },
      stdio: ["pipe", "pipe", "pipe"],
    });
    const reader = createInterface({ input: child.stdout, crlfDelay: Infinity });
    const pending = new Map<string | number, (message: JsonRpcMessage) => void>();
    const stdoutLines: string[] = [];
    reader.on("line", (line) => {
      stdoutLines.push(line);
      const message = JSON.parse(line) as JsonRpcMessage;
      if (message.id !== undefined) pending.get(message.id)?.(message);
    });
    let stderr = "";
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
    });
    const request = (
      id: number,
      method: string,
      params: Record<string, unknown>,
    ): Promise<JsonRpcMessage> =>
      new Promise((resolve) => {
        pending.set(id, resolve);
        child.stdin.write(
          `${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`,
        );
      });
    const initialize = await request(1, "initialize", {
      protocolVersion: "2025-11-25",
      clientInfo: { name: "OpenCode", version: "1.0" },
    });
    const canonicalInitialize = transcriptFixture.modern_canonical.initialize.response;
    const { instructions_fixture: _fixtureMarker, ...expectedInitializeResult } =
      canonicalInitialize.result;
    const { instructions: _instructions, ...actualInitializeResult } =
      initialize.result ?? {};
    assert.deepEqual(actualInitializeResult, expectedInitializeResult);
    const instructions = String(
      (initialize.result as Record<string, unknown> | undefined)?.instructions ?? "",
    );
    assert.ok(instructions.length > 0, "instructions must be present");
    assert.ok(
      instructions.startsWith(
        "Use `js` to run JavaScript in the persistent Node-backed kernel.",
      ),
    );
    assert.ok(
      instructions.includes(
        "the value of the last expression in your code is not returned",
      ),
      "instructions must state no implicit return",
    );
    assert.ok(
      instructions.includes("nodeRepl.write(value)"),
      "instructions must mention nodeRepl.write(value)",
    );
    const tools = await request(2, "tools/list", {});
    assert.deepEqual(tools.result, { tools: toolsFixture.tools });
    const first = await request(3, "tools/call", {
      name: "js",
      arguments: {
        code: "console.log('hostile-console'); (await import('node:fs')).writeSync(1, Buffer.from('hostile-fd1')); nodeRepl.write('hello')",
      },
    });
    assert.deepEqual(first.result, {
      content: [{ type: "text", text: "hostile-console\nhello" }],
      isError: false,
    });
    const second = await request(4, "tools/call", {
      name: "js",
      arguments: { code: "var goldenCounter = 41; nodeRepl.write(goldenCounter)" },
    });
    assert.deepEqual(second.result, {
      content: [{ type: "text", text: "41" }],
      isError: false,
    });
    const third = await request(5, "tools/call", {
      name: "js",
      arguments: { code: "nodeRepl.write(goldenCounter + 1)" },
    });
    assert.deepEqual(third.result, {
      content: [{ type: "text", text: "42" }],
      isError: false,
    });
    const metadata = await request(6, "tools/call", {
      name: "js",
      arguments: {
        code: "await Promise.resolve(); nodeRepl.write(JSON.stringify(nodeRepl.requestMeta))",
      },
    });
    const metadataText = (
      metadata.result?.content as Array<{ text?: unknown }> | undefined
    )?.[0]?.text;
    assert.equal(typeof metadataText, "string");
    const synthetic = JSON.parse(metadataText as string) as Record<string, unknown>;
    assert.equal(synthetic.caller_provenance, "opencode");
    assert.equal(synthetic.identity_synthetic, true);
    assert.deepEqual(synthetic.client_info, { name: "OpenCode", version: "1.0" });
    assert.deepEqual(synthetic["x-codex-turn-metadata"], {
      session_id: synthetic.session_id,
      turn_id: synthetic.turn_id,
    });
    const reset = await request(7, "tools/call", {
      name: "js_reset",
      arguments: {},
    });
    assert.deepEqual(reset.result, {
      content: [{ type: "text", text: "true" }],
      isError: false,
    });
    const afterReset = await request(8, "tools/call", {
      name: "js",
      arguments: { code: "nodeRepl.write(typeof goldenCounter)" },
    });
    assert.deepEqual(afterReset.result, {
      content: [{ type: "text", text: "undefined" }],
      isError: false,
    });
    const shutdown = await request(9, "shutdown", {});
    assert.deepEqual(shutdown.result, null);
    child.stdin.end();
    const exit = await new Promise<{
      code: number | null;
      signal: NodeJS.Signals | null;
    }>((resolve) => child.once("exit", (code, signal) => resolve({ code, signal })));
    assert.deepEqual(exit, { code: 0, signal: null }, stderr);
    assert.equal(
      stdoutLines.every((line) => line.startsWith("{")),
      true,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
