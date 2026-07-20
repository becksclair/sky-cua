import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "bun:test";
import { strict as assert } from "node:assert";

const fixtureRoot = import.meta.dir;
const nodePath = join(fixtureRoot, "bin", "node");
const nodeReplPath = join(fixtureRoot, "bin", "node_repl");
const nodeModulesPath = join(fixtureRoot, "lib", "node_modules");
const playwrightPath = join(fixtureRoot, "share", "playwright");
const trustedHashes = JSON.parse(
  readFileSync(join(fixtureRoot, "manifest.json"), "utf8"),
).trusted_browser_client_sha256s.join(",");
const env = {
  ...process.env,
  CODEX_NODE_REPL_PATH: nodeReplPath,
  NODE_REPL_NODE_PATH: nodePath,
  NODE_REPL_NODE_MODULE_DIRS: nodeModulesPath,
  NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: trustedHashes,
  PLAYWRIGHT_BROWSERS_PATH: playwrightPath,
};

function run(args: string[], input?: string) {
  const result = spawnSync(nodeReplPath, args, {
    cwd: fixtureRoot,
    env,
    encoding: "utf8",
    input,
  });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}

test("fake runtime exposes executable Node and node_repl identities", () => {
  const node = spawnSync(nodePath, ["--version"], { env, encoding: "utf8" });
  assert.equal(node.status, 0, node.stderr);
  assert.equal(node.stdout.trim(), "v24.14.0");

  assert.equal(run(["--version"]), "node_repl-fake/1.0.0");
});

test("fake runtime prints the five hydrated environment values and imports sky", () => {
  assert.deepEqual(JSON.parse(run(["--print-env"])), {
    CODEX_NODE_REPL_PATH: nodeReplPath,
    NODE_REPL_NODE_PATH: nodePath,
    NODE_REPL_NODE_MODULE_DIRS: nodeModulesPath,
    NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: trustedHashes,
    PLAYWRIGHT_BROWSERS_PATH: playwrightPath,
  });

  assert.deepEqual(JSON.parse(run(["--smoke"])), {
    runtime: "fake-cua-node",
    target: "linux-x64-glibc",
    node_version: "24.14.0",
    sky_export: "sky",
    sky_is_lazy: true,
  });
});

test("fake node_repl responds with the frozen minimum MCP tool surface", () => {
  const lines = run(
    [],
    [
      JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} }),
      JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} }),
      JSON.stringify({
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: { name: "js", arguments: { code: "1 + 1" } },
      }),
      JSON.stringify({ jsonrpc: "2.0", id: 4, method: "shutdown", params: {} }),
      "",
    ].join("\n"),
  );
  const responses = lines.split("\n").map((line) => JSON.parse(line));

  assert.equal(responses.length, 4);
  assert.equal(responses[0].result.serverInfo.name, "node_repl-fake");
  assert.deepEqual(
    responses[1].result.tools.map((tool: { name: string }) => tool.name),
    ["js", "js_reset", "js_add_node_module_dir"],
  );
  assert.equal(responses[2].result.content[0].text, "fake-js-result");
  assert.equal(responses[3].result, null);
});
