import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { strict as assert } from "node:assert";
import { test } from "bun:test";
import apiFixture from "../fixtures/api-surface.json";
import commandFixture from "../fixtures/commands.json";
import { API_MANIFEST, API_SURFACE, COMMANDS } from "../src/index.ts";
import { materializeCodexProjections } from "../src/projection.ts";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

async function buildAt(path: string): Promise<Uint8Array> {
  const result = await Bun.$`BROWSER_USE_BUILD_DIR=${path} bun run src/build.ts`.cwd(packageRoot).quiet();
  if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  return readFile(join(path, "browser-client.mjs"));
}

test("generated API and command fixtures are exhaustive and current", () => {
  assert.deepEqual(apiFixture.interfaces, API_SURFACE);
  assert.deepEqual(apiFixture.declarations, API_MANIFEST);
  assert.equal(commandFixture.count, 72);
  assert.deepEqual(commandFixture.commands, COMMANDS);
});

test("canonical declarations expose truthful native transport identities", () => {
  const listDeclaration = API_MANIFEST.interfaces.Browsers?.list?.declarations[0]?.text;
  assert.match(listDeclaration ?? "", /transport: "host_provided_iab" \| "extension_native_host"/u);
  assert.equal((listDeclaration ?? "").includes("skynet"), false);
});

test("Bun builds deterministic canonical bytes and projections are byte-identical", async () => {
  const root = await mkdtemp(join(tmpdir(), "heliasar-browser-build-"));
  try {
    const first = await buildAt(join(root, "first"));
    const second = await buildAt(join(root, "second"));
    assert.deepEqual(first, second);
    const canonical = join(root, "first", "browser-client.mjs");
    const projectionRoot = join(root, "projection");
    const projected = await materializeCodexProjections(canonical, projectionRoot);
    const expectedHash = createHash("sha256").update(first).digest("hex");
    assert.equal(projected.sha256, expectedHash);
    assert.equal(projected.paths.length, 2);
    for (const path of projected.paths) assert.deepEqual(await readFile(path), first);
    assert.equal(projected.paths.some((path) => path.includes("skynet")), false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("the canonical fixed-path Browser module connects without a caller hash", async () => {
  const root = await mkdtemp(join(tmpdir(), "heliasar-browser-fixed-path-"));
  let connections = 0;
  try {
    await buildAt(join(root, "build"));
    const path = join(root, "build", "browser-client.mjs");
    const client = await import(pathToFileURL(path).href);
    const globals = {
      console,
      nodeRepl: {
        env: { SKY_CUA_CODEX_BROWSER_SOCKET_PATH: "/run/user/1000/sky-cua/browser.sock" },
        nativePipe: { createConnection: async () => { connections += 1; throw new Error("probe"); } },
      },
    } as any;
    await client.setupBrowserRuntime({ globals });
    await globals.agent.browsers.list().catch(() => {});
    assert.equal(connections, 1);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("Node 24 ABI 137 initializes the canonical Chrome projection without native addons", async () => {
  const root = await mkdtemp(join(tmpdir(), "heliasar-browser-node24-"));
  try {
    await buildAt(join(root, "build"));
    const clientPath = join(root, "build", "browser-client.mjs");
    const script = [
      'import { pathToFileURL } from "node:url";',
      `const client = await import(pathToFileURL(${JSON.stringify(clientPath)}).href);`,
      "const globals = {};",
      "await client.setupBrowserRuntime({ globals });",
      'if (typeof globals.agent?.browsers?.list !== "function") throw new Error("Browser runtime was not initialized");',
      'process.stdout.write(`${process.versions.node} ${process.versions.modules}`);',
    ].join("\n");
    const process = Bun.spawn(["node", "--input-type=module", "--eval", script], {
      stdout: "pipe",
      stderr: "pipe",
    });
    const [exitCode, stdout, stderr] = await Promise.all([
      process.exited,
      new Response(process.stdout).text(),
      new Response(process.stderr).text(),
    ]);
    assert.equal(exitCode, 0, stderr);
    assert.match(stdout, /^24\.\d+\.\d+ 137$/u);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
