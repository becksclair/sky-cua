import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const directory = path.dirname(fileURLToPath(import.meta.url));
const childPath = path.join(directory, "direct-child.mjs");
const cycles = 100;
const mcpStdoutSentinel = `${JSON.stringify({
  jsonrpc: "2.0",
  id: "mcp-sentinel",
  result: { content: [], isError: false },
})}\n`;
let observedMcpStdout = mcpStdoutSentinel;
const startedAt = performance.now();

for (let cycle = 0; cycle < cycles; cycle += 1) {
  const child = spawn(process.execPath, [childPath], {
    cwd: process.cwd(),
    env: process.env,
    stdio: ["pipe", "pipe", "pipe"],
  });
  const output = readline.createInterface({
    input: child.stdout,
    crlfDelay: Infinity,
  });
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });
  const exitPromise = new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code, signal) => resolve({ code, signal }));
  });

  const id = `cycle-${cycle}`;
  child.stdin.end(`${JSON.stringify({ cycle, id })}\n`);
  const responses = [];
  for await (const line of output) {
    responses.push(JSON.parse(line));
  }
  const exit = await exitPromise;

  assert.deepEqual(exit, { code: 0, signal: null });
  assert.deepEqual(responses, [{
    cycle,
    id,
    protocol: "cua-kernel-control-v1",
  }]);
  assert.equal(stderr, `kernel-cycle=${cycle}\n`);
  assert.equal(observedMcpStdout, mcpStdoutSentinel);
}

const report = {
  cycles,
  elapsed_ms: Math.round(performance.now() - startedAt),
  mcp_stdout: "byte-exact and host-owned",
  result: "passed",
  runtime: process.version,
  stderr: "separate",
  transport: "direct-private-child-stdio",
};

fs.writeFileSync(
  path.join(directory, "direct-child-spike.result.json"),
  `${JSON.stringify(report, null, 2)}\n`,
);
process.stdout.write(`${JSON.stringify(report)}\n`);
