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
const codexRoot = process.env.CODEX_DESKTOP_ROOT ?? "/home/bex/projects/codex-desktop";

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

test("canonical fixtures match the preserved current Codex declaration and command surfaces", async () => {
  const upstreamApi = JSON.parse(await readFile(resolve(
    codexRoot,
    "resources/plugins/openai-bundled/plugins/browser-use/docs/api.json",
  ), "utf8"));
  assert.deepEqual(API_MANIFEST, upstreamApi);
  const upstreamClient = await readFile(resolve(
    codexRoot,
    "resources/plugins/openai-bundled/plugins/browser-use/scripts/browser-client.mjs",
  ), "utf8");
  const commandPattern = /["'`](browser_user_[a-z0-9_]+|close_tab|create_tab|finalize_tabs|list_tabs|name_session|selected_tab|navigate_tab_[a-z0-9_]+|cua_[a-z0-9_]+|dom_cua_[a-z0-9_]+|playwright_[a-z0-9_]+|tab_[a-z0-9_]+)["'`]/gu;
  const commands = [...new Set([...upstreamClient.matchAll(commandPattern)].map((match) => match[1]))]
    .filter((value): value is string => value !== undefined)
    .sort();
  assert.deepEqual([...COMMANDS].sort(), commands);
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

test("trusted hash gate accepts canonical bytes and rejects wrong hash before connect", async () => {
  const root = await mkdtemp(join(tmpdir(), "heliasar-browser-trust-"));
  let connections = 0;
  try {
    const bytes = await buildAt(join(root, "build"));
    const path = join(root, "build", "browser-client.mjs");
    const actual = createHash("sha256").update(bytes).digest("hex");
    const load = async (trusted: string) => {
      const openedBytes = await readFile(path);
      const openedHash = createHash("sha256").update(openedBytes).digest("hex");
      if (openedHash !== trusted) throw new Error("TRUSTED_BROWSER_HASH_REJECTED");
      const client = await import(`${pathToFileURL(path).href}?trusted=${trusted}`);
      const globals = {
        console,
        nodeRepl: {
          env: { SKY_CUA_CODEX_BROWSER_SOCKET_PATH: "/run/user/1000/sky-cua/browser.sock" },
          nativePipe: { createConnection: async () => { connections += 1; throw new Error("probe"); } },
        },
      } as any;
      await client.setupBrowserRuntime({ globals });
      await globals.agent.browsers.list().catch(() => {});
    };
    await assert.rejects(() => load("0".repeat(64)), /TRUSTED_BROWSER_HASH_REJECTED/u);
    assert.equal(connections, 0);
    await load(actual);
    assert.equal(connections, 1);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
