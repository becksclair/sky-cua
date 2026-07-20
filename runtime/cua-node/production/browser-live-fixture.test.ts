import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { deflateSync } from "node:zlib";
import { test } from "bun:test";
import {
  parseBrowserAcceptanceArgs,
  runPackagedRootTrustNegatives,
  type BrowserAcceptanceOptions,
} from "./browser-live-acceptance.ts";
import {
  createDisposableRuntime,
  startBrowserAcceptanceFixture,
} from "./browser-live-fixture.ts";

type RuntimeFixture = {
  root: string;
  options: BrowserAcceptanceOptions;
  clientHash: string;
};

function digest(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function runtimeFixture(): RuntimeFixture {
  const root = mkdtempSync(join(tmpdir(), "browser-live-fixture-test-"));
  const nodeRepl = join(root, "bin/node_repl");
  const browserClient = join(
    root,
    "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs",
  );
  mkdirSync(join(root, "bin"), { recursive: true });
  mkdirSync(dirname(browserClient), { recursive: true });
  writeFileSync(nodeRepl, "#!/bin/sh\nexit 0\n", "utf8");
  chmodSync(nodeRepl, 0o755);
  writeFileSync(
    browserClient,
    "export async function setupBrowserRuntime() {}\n",
    "utf8",
  );
  const clientHash = digest(browserClient);
  writeFileSync(
    join(root, "manifest.json"),
    `${JSON.stringify(
      {
        runtime_name: "cua_node",
        node_repl_path: "bin/node_repl",
        node_repl_sha256: digest(nodeRepl),
        trusted_browser_client_sha256s: [clientHash],
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  return {
    root,
    options: parseBrowserAcceptanceArgs(
      [
        `--runtime-root=${root}`,
        `--browser-client=${browserClient}`,
        "--scenario=iab",
        "--session-id=session-1",
        "--turn-id=turn-1",
      ],
      {},
    ),
    clientHash,
  };
}

function pngCrc32(bytes: Buffer): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1)
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngChunk(type: string, data: Buffer): Buffer {
  const typeBytes = Buffer.from(type, "ascii");
  const chunk = Buffer.alloc(12 + data.length);
  chunk.writeUInt32BE(data.length, 0);
  typeBytes.copy(chunk, 4);
  data.copy(chunk, 8);
  chunk.writeUInt32BE(pngCrc32(Buffer.concat([typeBytes, data])), 8 + data.length);
  return chunk;
}

function solidPng(red: number): Buffer {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(2, 0);
  header.writeUInt32BE(2, 4);
  header[8] = 8;
  header[9] = 6;
  const row = Buffer.from([0, red, 0, 0, 255, red, 0, 0, 255]);
  return Buffer.concat([
    Buffer.from("89504e470d0a1a0a", "hex"),
    pngChunk("IHDR", header),
    pngChunk("IDAT", deflateSync(Buffer.concat([row, row]))),
    pngChunk("IEND", Buffer.alloc(0)),
  ]);
}

function installFakeRawNodeRepl(options: BrowserAcceptanceOptions): void {
  const before = solidPng(20).toString("base64");
  const after = solidPng(230).toString("base64");
  const source = `#!/usr/bin/env bun
import { createInterface } from "node:readline";
const before = ${JSON.stringify(before)};
const after = ${JSON.stringify(after)};
if (Object.hasOwn(process.env, "CODEX_BROWSER_PROVIDER")) throw new Error("CODEX_BROWSER_PROVIDER must be unset");
const output = (value) => process.stdout.write(JSON.stringify(value) + "\\n");
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
for await (const line of lines) {
  const request = JSON.parse(line);
  if (request.method === "initialize") {
    output({ jsonrpc: "2.0", id: request.id, result: { protocolVersion: "2025-06-18", capabilities: {}, serverInfo: { name: "fake-raw-node-repl", version: "1" } } });
  } else if (request.method === "tools/call" && request.id === 2) {
    output({ jsonrpc: "2.0", id: request.id, result: { content: [{ type: "text", text: "BROWSER-SETUP-OK" }], isError: false } });
  } else if (request.method === "tools/call" && request.id === 3) {
    const code = request.params.arguments.code;
    const url = code.match(/http:\\/\\/127\\.0\\.0\\.1:\\d+\\/acceptance/u)?.[0];
    if (url == null) throw new Error("owned fixture URL missing from action code");
    const page = await fetch(url);
    const html = await page.text();
    if (!html.includes('data-testid="browser-live-input"') || !html.includes('data-testid="browser-live-readback"') || !html.includes('data-testid="browser-live-download"')) throw new Error("owned fixture DOM incomplete");
    const download = await fetch(new URL("/download/browser-live-acceptance.txt", url));
    if ((await download.text()) !== "sky-cua browser live acceptance\\n") throw new Error("owned fixture download mismatch");
    const meta = request.params._meta;
    const evidence = {
      browser: { id: "iab", name: "Fake raw IAB", type: "iab" },
      navigation: { requested_url: url, final_url: url },
      keyboard: { method: "PlaywrightLocator.type+press", key: "End", text: "CUA NODE BROWSER ACCEPTANCE", value: "CUA NODE BROWSER ACCEPTANCE" },
      click: { actual: true, button_name: "OK" },
      readback: "CUA NODE BROWSER ACCEPTANCE",
      screenshot: { method: "Tab.screenshot", emitted: true, expected_width: 2, expected_height: 2, before_byte_length: Buffer.from(before, "base64").byteLength, after_byte_length: Buffer.from(after, "base64").byteLength },
      request_meta: meta,
    };
    const image = (data) => ({ type: "image", data, mimeType: "image/png", _meta: { "codex/imageDetail": "original" } });
    output({ jsonrpc: "2.0", id: request.id, result: { content: [image(before), image(after), { type: "text", text: JSON.stringify(evidence) }], isError: false, _meta: { "codex/toolSurface": { kind: "browserUse", backend: "iab", browserId: "iab" } } } });
  } else if (request.method === "shutdown") {
    output({ jsonrpc: "2.0", id: request.id, result: null });
  }
}
`;
  writeFileSync(options.nodeRepl, source, "utf8");
  chmodSync(options.nodeRepl, 0o755);
  const manifestPath = join(options.runtimeRoot, "manifest.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Record<
    string,
    unknown
  >;
  manifest.node_repl_sha256 = digest(options.nodeRepl);
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

function cleanEnvironment(): NodeJS.ProcessEnv {
  const environment = { ...process.env };
  delete environment.CODEX_BROWSER_PROVIDER;
  return environment;
}

test("omitted URL selects the owned loopback fixture", () => {
  const parsed = parseBrowserAcceptanceArgs(
    ["--scenario=iab", "--session-id=session-1", "--turn-id=turn-1"],
    {},
  );
  assert.equal(parsed.ownsFixture, true);
  assert.equal(parsed.url, "http://127.0.0.1/acceptance");
  assert.equal(parsed.trustNegative, "none");
});

test("serves deterministic DOM, readback, health, and download routes on loopback only", async () => {
  const server = await startBrowserAcceptanceFixture();
  assert.match(server.origin, /^http:\/\/127\.0\.0\.1:\d+$/u);
  try {
    const page = await fetch(server.url);
    assert.equal(page.status, 200);
    const html = await page.text();
    assert.match(html, /data-testid="browser-live-input"/u);
    assert.match(html, /data-testid="browser-live-readback"/u);
    assert.match(html, /data-testid="browser-live-download"/u);
    assert.match(html, /document\.body\.classList\.add\("accepted"\)/u);
    assert.deepEqual(await (await fetch(`${server.origin}/health`)).json(), {
      status: "ok",
    });
    const download = await fetch(server.downloadUrl);
    assert.equal(
      download.headers.get("content-disposition"),
      'attachment; filename="browser-live-acceptance.txt"',
    );
    assert.equal(await download.text(), "sky-cua browser live acceptance\n");
    assert.equal((await fetch(`${server.origin}/missing`)).status, 404);
  } finally {
    await server.close();
  }
  await server.close();
  await assert.rejects(fetch(server.url));
});

test("runs the omitted-URL path through a deterministic raw node_repl and cleans up", async () => {
  const fixture = runtimeFixture();
  try {
    installFakeRawNodeRepl(fixture.options);
    const result = spawnSync(
      process.execPath,
      [
        join(__dirname, "browser-live-acceptance.ts"),
        `--runtime-root=${fixture.options.runtimeRoot}`,
        `--browser-client=${fixture.options.browserClient}`,
        "--scenario=iab",
        "--session-id=session-1",
        "--turn-id=turn-1",
        "--timeout-ms=5000",
        "--json",
      ],
      { encoding: "utf8", env: cleanEnvironment(), timeout: 15_000 },
    );
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    const report = JSON.parse(result.stdout) as {
      status: string;
      connection_attempted: boolean;
      evidence: {
        navigation: { requested_url: string };
        fixture: { owned: boolean; url: string; cleaned_up: boolean };
      };
    };
    assert.equal(report.status, "passed");
    assert.equal(report.connection_attempted, true);
    assert.equal(report.evidence.fixture.owned, true);
    assert.equal(report.evidence.fixture.cleaned_up, true);
    assert.equal(
      report.evidence.navigation.requested_url,
      report.evidence.fixture.url,
    );
    await assert.rejects(fetch(report.evidence.fixture.url));
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("packaged-root trust negatives reject all Browser mutations without touching source", () => {
  const fixture = runtimeFixture();
  try {
    fixture.options.trustNegative = "all";
    const originalBytes = readFileSync(fixture.options.browserClient);
    const evidence = runPackagedRootTrustNegatives(fixture.options);
    assert.equal(evidence.browser_client_sha256, fixture.clientHash);
    assert.deepEqual(
      evidence.cases.map((item) => [
        item.case,
        item.rejected,
        item.connection_attempted,
        item.hash_verified_before_connection,
      ]),
      [
        ["tampered", true, false, false],
        ["missing", true, false, false],
        ["wrong-manifest-hash", true, false, false],
      ],
    );
    assert.equal(
      readFileSync(fixture.options.browserClient).equals(originalBytes),
      true,
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("disposable trust-negative roots are deleted and never mutate packaged root", () => {
  const fixture = runtimeFixture();
  try {
    const original = readFileSync(fixture.options.browserClient);
    const disposable = createDisposableRuntime(
      fixture.options.runtimeRoot,
      fixture.options.browserClient,
      fixture.options.nodeRepl,
      "tampered",
    );
    const disposableRoot = disposable.runtimeRoot;
    assert.equal(existsSync(disposableRoot), true);
    assert.notEqual(digest(disposable.browserClient), digest(fixture.options.browserClient));
    disposable.cleanup();
    assert.equal(existsSync(disposableRoot), false);
    assert.equal(readFileSync(fixture.options.browserClient).equals(original), true);
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});

test("trust-negative CLI reports every rejection before connection", () => {
  const fixture = runtimeFixture();
  try {
    const result = spawnSync(
      process.execPath,
      [
        join(__dirname, "browser-live-acceptance.ts"),
        `--runtime-root=${fixture.options.runtimeRoot}`,
        `--browser-client=${fixture.options.browserClient}`,
        "--scenario=iab",
        "--session-id=session-1",
        "--turn-id=turn-1",
        "--trust-negative=all",
        "--json",
      ],
      { encoding: "utf8", env: cleanEnvironment() },
    );
    assert.equal(result.status, 0, result.stderr);
    const report = JSON.parse(result.stdout) as {
      status: string;
      connection_attempted: boolean;
      evidence: {
        original_untouched: boolean;
        cases: Array<{ case: string; connection_attempted: boolean }>;
      };
    };
    assert.equal(report.status, "passed");
    assert.equal(report.connection_attempted, false);
    assert.equal(report.evidence.original_untouched, true);
    assert.deepEqual(
      report.evidence.cases.map((item) => [item.case, item.connection_attempted]),
      [
        ["tampered", false],
        ["missing", false],
        ["wrong-manifest-hash", false],
      ],
    );
  } finally {
    rmSync(fixture.root, { recursive: true, force: true });
  }
});
