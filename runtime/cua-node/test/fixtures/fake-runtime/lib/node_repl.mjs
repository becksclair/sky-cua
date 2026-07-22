import { createInterface } from "node:readline";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const runtimeRoot = join(dirname(new URL(import.meta.url).pathname), "..");
const ENVIRONMENT_KEYS = [
  "CODEX_NODE_REPL_PATH",
  "NODE_REPL_NODE_PATH",
  "NODE_REPL_NODE_MODULE_DIRS",
  "PLAYWRIGHT_BROWSERS_PATH",
];

function selectedEnvironment() {
  return Object.fromEntries(
    ENVIRONMENT_KEYS.map((key) => [key, process.env[key] ?? null]),
  );
}

function moduleRoot() {
  const first = (process.env.NODE_REPL_NODE_MODULE_DIRS ?? "")
    .split(":")
    .find((entry) => entry.length > 0);
  if (!first) {
    throw new Error("NODE_REPL_NODE_MODULE_DIRS is required");
  }
  return first;
}

function manifest() {
  const manifestPath = join(runtimeRoot, "manifest.json");
  return JSON.parse(readFileSync(manifestPath, "utf8"));
}

function assertSmokeEnvironment() {
  const current = selectedEnvironment();
  for (const [key, value] of Object.entries(current)) {
    if (typeof value !== "string" || value.length === 0) {
      throw new Error(`${key} is missing`);
    }
  }

  const expected = manifest();
  if (
    current.CODEX_NODE_REPL_PATH !== join(runtimeRoot, expected.node_repl_path)
  ) {
    throw new Error(
      "CODEX_NODE_REPL_PATH does not select the fixture node_repl",
    );
  }
  if (current.NODE_REPL_NODE_PATH !== join(runtimeRoot, expected.node_path)) {
    throw new Error("NODE_REPL_NODE_PATH does not select the fixture node");
  }
  if (
    current.NODE_REPL_NODE_MODULE_DIRS !==
    join(runtimeRoot, expected.node_modules)
  ) {
    throw new Error(
      "NODE_REPL_NODE_MODULE_DIRS does not select the fixture modules",
    );
  }
  if (
    current.PLAYWRIGHT_BROWSERS_PATH !==
    join(runtimeRoot, expected.data.playwright)
  ) {
    throw new Error("PLAYWRIGHT_BROWSERS_PATH does not select fixture data");
  }
}

async function importSky() {
  const entrypoint = join(moduleRoot(), "@heliasar", "sky-cua", "index.mjs");
  if (!existsSync(entrypoint)) {
    throw new Error(`fake sky package is missing: ${entrypoint}`);
  }
  return import(pathToFileURL(entrypoint).href);
}

async function runSmoke() {
  assertSmokeEnvironment();
  const imported = await importSky();
  if (JSON.stringify(Object.keys(imported)) !== '["sky"]') {
    throw new Error("fake sky package must export exactly the named sky value");
  }
  if (typeof imported.sky !== "object" || imported.sky === null) {
    throw new Error("fake sky package did not expose an object sky export");
  }
  return {
    runtime: "fake-cua-node",
    target: manifest().target,
    node_version: manifest().node_version,
    sky_export: "sky",
    sky_is_lazy: imported.sky.__fake_lazy === true,
  };
}

function response(id, result) {
  return JSON.stringify({ jsonrpc: "2.0", id: id ?? null, result });
}

async function handleRequest(request) {
  const id = request.id ?? null;
  if (request.method === "initialize") {
    return response(id, {
      protocolVersion: "2024-11-05",
      serverInfo: { name: "node_repl-fake", version: "1.0.0" },
      capabilities: { tools: {} },
    });
  }
  if (request.method === "notifications/initialized") {
    return null;
  }
  if (request.method === "tools/list") {
    return response(id, {
      tools: [
        { name: "js", inputSchema: { type: "object", required: ["code"] } },
        { name: "js_reset", inputSchema: { type: "object" } },
        {
          name: "js_add_node_module_dir",
          inputSchema: { type: "object", required: ["path"] },
        },
      ],
    });
  }
  if (request.method === "shutdown") {
    return response(id, null);
  }
  if (request.method === "tools/call") {
    const name = request.params?.name;
    if (name === "js") {
      return response(id, {
        content: [{ type: "text", text: "fake-js-result" }],
      });
    }
    if (name === "js_reset") {
      return response(id, {
        content: [{ type: "text", text: "fake-js-reset" }],
      });
    }
    if (name === "js_add_node_module_dir") {
      return response(id, {
        content: [{ type: "text", text: "fake-node-module-dir-added" }],
      });
    }
  }
  return JSON.stringify({
    jsonrpc: "2.0",
    id,
    error: {
      code: -32601,
      message: `Unsupported fake method: ${request.method ?? ""}`,
    },
  });
}

async function main() {
  if (process.argv[2] === "--print-env") {
    process.stdout.write(`${JSON.stringify(selectedEnvironment())}\n`);
    return;
  }
  if (process.argv[2] === "--smoke") {
    process.stdout.write(`${JSON.stringify(await runSmoke())}\n`);
    return;
  }

  const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of input) {
    if (!line.trim()) {
      continue;
    }
    let request;
    try {
      request = JSON.parse(line);
    } catch {
      process.stdout.write(
        `${JSON.stringify({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "Invalid JSON" } })}\n`,
      );
      continue;
    }
    const output = await handleRequest(request);
    if (output !== null) {
      process.stdout.write(`${output}\n`);
    }
    if (request.method === "shutdown") {
      break;
    }
  }
}

void main().catch((error) => {
  const message =
    error instanceof Error ? error.message : "fake node_repl failed";
  process.stderr.write(`${message}\n`);
  process.exitCode = 1;
});
