import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { EventEmitter } from "node:events";
import {
  chmodSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import { test } from "bun:test";
import {
  buildAcceptanceCode,
  buildMcpRequests,
  closeNodeRepl,
  combinePrimaryAndCleanupErrors,
  parseBrowserAcceptanceArgs,
  parseToolResult,
  validateInstalledSelection,
  validateShutdownResponse,
  verifyBraveOriginNativeHost,
  type BrowserAcceptanceOptions,
} from "./browser-live-acceptance.ts";

class FakeNodeRepl extends EventEmitter {
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;
  pid = 4242;
  alive = true;
  stdinClosed = false;
  signals: NodeJS.Signals[] = [];
  readonly stdin = {
    end: (): void => {
      this.stdinClosed = true;
      if (this.exitOnStdin) queueMicrotask(() => this.exit(0, null));
    },
  };

  constructor(
    private readonly exitOnSignal: NodeJS.Signals | null,
    private readonly exitOnStdin = false,
  ) {
    super();
  }

  kill(signal: NodeJS.Signals): boolean {
    this.signals.push(signal);
    if (signal === this.exitOnSignal) queueMicrotask(() => this.exit(null, signal));
    return true;
  }

  private exit(code: number | null, signal: NodeJS.Signals | null): void {
    this.exitCode = code;
    this.signalCode = signal;
    this.alive = false;
    this.emit("exit", code, signal);
  }
}

function pngCrc32(bytes: Buffer): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
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

function imageItem(bytes: Buffer, mimeType = "image/png"): Record<string, unknown> {
  return {
    type: "image",
    data: bytes.toString("base64"),
    mimeType,
    _meta: { "codex/imageDetail": "original" },
  };
}

function solidWebp(marker: number): Buffer {
  return Buffer.from(
    marker === 0
      ? "UklGRjwAAABXRUJQVlA4IDAAAADQAQCdASoDAAMAAgA0JaACdLoB+AADsAD+8MQL/yC5YXXI1/8gP+QH/ID/+PIAAAA="
      : "UklGRjgAAABXRUJQVlA4ICwAAACQAQCdASoDAAMAAgA0JaACdLoAA5gA/vmTb/+QH/+QH/+QH/8gP+IXeyAwAA==",
    "base64",
  );
}

function fixture(): { options: BrowserAcceptanceOptions; clientHash: string } {
  const root = mkdtempSync(join(tmpdir(), "browser-live-acceptance-"));
  mkdirSync(join(root, "bin"), { recursive: true });
  mkdirSync(join(root, "plugins/browser-use/scripts"), { recursive: true });
  const nodeRepl = join(root, "bin/node_repl");
  const browserClient = join(root, "plugins/browser-use/scripts/browser-client.mjs");
  writeFileSync(nodeRepl, "#!/bin/sh\nexit 0\n", "utf8");
  chmodSync(nodeRepl, 0o755);
  writeFileSync(
    browserClient,
    "export async function setupBrowserRuntime() {}\n",
    "utf8",
  );
  const digest = (path: string): string =>
    createHash("sha256").update(readFileSync(path)).digest("hex");
  const nodeReplHash = digest(nodeRepl);
  const clientHash = digest(browserClient);
  writeFileSync(
    join(root, "manifest.json"),
    JSON.stringify({
      runtime_name: "cua_node",
      node_repl_path: "bin/node_repl",
      node_repl_sha256: nodeReplHash,
      trusted_browser_client_sha256s: [clientHash],
    }),
    "utf8",
  );
  return {
    options: parseBrowserAcceptanceArgs(
      [
        `--runtime-root=${root}`,
        `--browser-client=${browserClient}`,
        "--scenario=iab",
        "--url=https://example.test/acceptance",
        "--session-id=session-1",
        "--turn-id=turn-1",
      ],
      {},
    ),
    clientHash,
  };
}

test("keeps IAB and Brave Origin parameterized while rejecting every Skynet selection", () => {
  const common = [
    "--url=https://example.test/acceptance",
    "--session-id=session-1",
    "--turn-id=turn-1",
  ];
  assert.equal(
    parseBrowserAcceptanceArgs(["--scenario=iab", ...common], {}).scenario,
    "iab",
  );
  assert.equal(
    parseBrowserAcceptanceArgs(["--scenario=brave-origin-extension", ...common], {})
      .scenario,
    "brave-origin-extension",
  );
  assert.throws(
    () =>
      parseBrowserAcceptanceArgs(["--scenario=iab", ...common], {
        CODEX_BROWSER_PROVIDER: "SkYneT",
      }),
    /forbidden/u,
  );
  for (const scenario of ["iab", "brave-origin-extension"] as const) {
    const parsed = parseBrowserAcceptanceArgs([`--scenario=${scenario}`, ...common], {
      CODEX_BROWSER_PROVIDER: "",
    });
    assert.equal(parsed.scenario, scenario);
  }
});

test("validates the selected installed node_repl and exact manifest browser-client SHA", () => {
  const { options, clientHash } = fixture();
  const selection = validateInstalledSelection(options);
  assert.equal(selection.browserClientSha256, clientHash);
  assert.deepEqual(selection.trustedBrowserClientSha256s, [clientHash]);
  writeFileSync(options.browserClient, "wrong bytes", "utf8");
  assert.throws(
    () => validateInstalledSelection(options),
    /is not trusted by runtime manifest/u,
  );
});

test("rejects Skynet browser-client paths before command construction", () => {
  const { options } = fixture();
  options.browserClient = join(
    options.runtimeRoot,
    "plugins/skynet-browser-use/scripts/browser-client.mjs",
  );
  assert.throws(
    () => validateInstalledSelection(options),
    /must not be a Skynet client path/u,
  );
});

test("wrong browser-client hash emits fail-closed evidence before connection", () => {
  const { options } = fixture();
  writeFileSync(options.browserClient, "wrong bytes", "utf8");
  const result = spawnSync(
    process.execPath,
    [
      join(__dirname, "browser-live-acceptance.ts"),
      `--runtime-root=${options.runtimeRoot}`,
      `--browser-client=${options.browserClient}`,
      "--scenario=iab",
      `--url=${options.url}`,
      `--session-id=${options.sessionId}`,
      `--turn-id=${options.turnId}`,
      "--json",
    ],
    {
      encoding: "utf8",
      env: { ...process.env, CODEX_BROWSER_PROVIDER: "" },
    },
  );
  assert.equal(result.status, 1);
  const report = JSON.parse(result.stdout) as {
    connection_attempted: boolean;
    evidence: { failure_phase: string; wrong_hash_fail_closed: boolean };
  };
  assert.equal(report.connection_attempted, false);
  assert.equal(report.evidence.failure_phase, "preflight");
  assert.equal(report.evidence.wrong_hash_fail_closed, true);
});

test("rejects a node_repl shutdown error instead of accepting its response", () => {
  assert.throws(
    () =>
      validateShutdownResponse({
        jsonrpc: "2.0",
        id: 4,
        result: null,
        error: { code: -32603, message: "shutdown failed" },
      }),
    /must be result:null with no error/u,
  );
});

test("acknowledged shutdown that ignores SIGTERM escalates to SIGKILL", async () => {
  const child = new FakeNodeRepl("SIGKILL");
  validateShutdownResponse({ jsonrpc: "2.0", id: 4, result: null });
  const exit = await closeNodeRepl(child, 5);
  assert.equal(child.stdinClosed, true);
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
  assert.deepEqual(exit, { code: null, signal: "SIGKILL" });
  assert.equal(child.alive, false);
});

test("fails when SIGKILL escalation cannot confirm node_repl exit", async () => {
  const child = new FakeNodeRepl(null);
  await assert.rejects(
    closeNodeRepl(child, 5),
    /child PID 4242 did not exit after SIGKILL/u,
  );
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
  assert.equal(child.alive, true);
});

test("preserves shutdown protocol failure when forced cleanup also fails", async () => {
  const primary = new Error(
    "node_repl shutdown response must be result:null with no error",
  );
  const child = new FakeNodeRepl(null);
  let cleanupFailure: unknown;
  try {
    await closeNodeRepl(child, 5);
  } catch (error) {
    cleanupFailure = error;
  }
  const combined = combinePrimaryAndCleanupErrors(primary, cleanupFailure);
  assert.equal(combined.cause, primary);
  assert.equal(combined.errors[0], primary);
  assert.match(combined.message, /shutdown response must be result:null/u);
  assert.match(combined.message, /cleanup also failed/u);
  assert.match(combined.message, /did not exit after SIGKILL/u);
});

test("confirms node_repl is no longer live before shutdown succeeds", async () => {
  const child = new FakeNodeRepl(null, true);
  const exit = await closeNodeRepl(child, 5);
  assert.deepEqual(exit, { code: 0, signal: null });
  assert.equal(child.alive, false);
  assert.deepEqual(child.signals, []);
});

test("constructs structured IAB command with metadata, keyboard, click, readback, and tool surface trigger", () => {
  const { options } = fixture();
  const selection = validateInstalledSelection(options);
  const code = buildAcceptanceCode(options, selection);
  assert.match(code, /agent\.browsers\.get\("iab"\)/u);
  assert.match(code, /\.type\("CUA NODE BROWSER ACCEPTANCE"/u);
  assert.match(code, /input\.press\("End"/u);
  assert.match(code, /input\.evaluate\(\(element\) => element\.value/u);
  assert.match(code, /getByRole\("button"/u);
  assert.match(code, /await okButton\.click/u);
  assert.match(code, /await acceptanceTab\.screenshot\(\)/u);
  assert.match(code, /await nodeRepl\.emitImage\(beforeScreenshot\)/u);
  assert.match(code, /await nodeRepl\.emitImage\(afterScreenshot\)/u);
  assert.match(code, /finally \{\s+await acceptanceTab\.close\(\)/u);
  assert.match(code, /browser-live-readback/u);
  const requests = buildMcpRequests(options, selection);
  const call = requests[1] as { params: { _meta: Record<string, unknown> } };
  assert.deepEqual(call.params._meta, {
    session_id: "session-1",
    turn_id: "turn-1",
    "x-codex-turn-metadata": { session_id: "session-1", turn_id: "turn-1" },
  });
});

test("parses and verifies complete structured live evidence", () => {
  const { options } = fixture();
  const beforePng = solidPng(16);
  const afterPng = solidPng(240);
  const requestMeta = {
    session_id: options.sessionId,
    turn_id: options.turnId,
    "x-codex-turn-metadata": {
      session_id: options.sessionId,
      turn_id: options.turnId,
    },
  };
  const browserEvidence = {
    browser: { id: "iab", name: "Codex", type: "iab" },
    navigation: { requested_url: options.url, final_url: options.url },
    keyboard: {
      method: "PlaywrightLocator.type+press",
      key: "End",
      text: options.typedText,
      value: options.typedText,
    },
    click: { actual: true, button_name: options.buttonName },
    readback: options.typedText,
    screenshot: {
      method: "Tab.screenshot",
      emitted: true,
      expected_width: 2,
      expected_height: 2,
      before_byte_length: beforePng.length,
      after_byte_length: afterPng.length,
    },
    request_meta: requestMeta,
  };
  const parsed = parseToolResult(
    {
      jsonrpc: "2.0",
      id: 2,
      result: {
        content: [
          imageItem(beforePng),
          imageItem(afterPng),
          { type: "text", text: JSON.stringify(browserEvidence) },
        ],
        isError: false,
        _meta: {
          "codex/toolSurface": {
            kind: "browserUse",
            backend: "iab",
            browserId: "iab",
          },
        },
      },
    },
    options,
  );
  assert.deepEqual(parsed.navigation, browserEvidence.navigation);
  assert.deepEqual(parsed.tool_surface, {
    kind: "browserUse",
    backend: "iab",
    browserId: "iab",
  });
  assert.deepEqual(parsed.emitted_images, {
    before: {
      mime_type: "image/png",
      byte_length: beforePng.length,
      width: 2,
      height: 2,
      detail: "original",
    },
    after: {
      mime_type: "image/png",
      byte_length: afterPng.length,
      width: 2,
      height: 2,
      detail: "original",
    },
  });
});

test("accepts WebP-default screenshots at the display scale", () => {
  const { options } = fixture();
  const beforeWebp = solidWebp(0);
  const afterWebp = solidWebp(1);
  const requestMeta = {
    session_id: options.sessionId,
    turn_id: options.turnId,
    "x-codex-turn-metadata": {
      session_id: options.sessionId,
      turn_id: options.turnId,
    },
  };
  const evidence = {
    navigation: { requested_url: options.url, final_url: options.url },
    keyboard: {
      method: "PlaywrightLocator.type+press",
      key: "End",
      text: options.typedText,
      value: options.typedText,
    },
    click: { actual: true, button_name: options.buttonName },
    readback: options.typedText,
    screenshot: {
      method: "Tab.screenshot",
      emitted: true,
      expected_width: 2,
      expected_height: 2,
      before_byte_length: beforeWebp.length,
      after_byte_length: afterWebp.length,
    },
    request_meta: requestMeta,
  };
  const parsed = parseToolResult(
    {
      result: {
        content: [
          imageItem(beforeWebp, "image/webp"),
          imageItem(afterWebp, "image/webp"),
          { type: "text", text: JSON.stringify(evidence) },
        ],
        isError: false,
        _meta: {
          "codex/toolSurface": {
            kind: "browserUse",
            backend: "iab",
            browserId: "iab",
          },
        },
      },
    },
    options,
  );
  assert.deepEqual(parsed.emitted_images, {
    before: {
      mime_type: "image/webp",
      byte_length: beforeWebp.length,
      width: 3,
      height: 3,
      detail: "original",
    },
    after: {
      mime_type: "image/webp",
      byte_length: afterWebp.length,
      width: 3,
      height: 3,
      detail: "original",
    },
  });
});

test("rejects missing or malformed screenshot image evidence", () => {
  const { options } = fixture();
  const evidence = {
    navigation: { requested_url: options.url, final_url: options.url },
    keyboard: {
      method: "PlaywrightLocator.type+press",
      key: "End",
      text: options.typedText,
      value: options.typedText,
    },
    click: { actual: true, button_name: options.buttonName },
    readback: options.typedText,
    screenshot: {
      method: "Tab.screenshot",
      emitted: true,
      expected_width: 2,
      expected_height: 2,
      before_byte_length: 8,
      after_byte_length: 8,
    },
    request_meta: {
      session_id: options.sessionId,
      turn_id: options.turnId,
      "x-codex-turn-metadata": {
        session_id: options.sessionId,
        turn_id: options.turnId,
      },
    },
  };
  const response = (content: Array<Record<string, unknown>>) => ({
    result: {
      content,
      isError: false,
      _meta: {
        "codex/toolSurface": {
          kind: "browserUse",
          backend: "iab",
          browserId: "iab",
        },
      },
    },
  });
  assert.throws(
    () =>
      parseToolResult(
        response([{ type: "text", text: JSON.stringify(evidence) }]),
        options,
      ),
    /screenshots were not emitted/u,
  );
  assert.throws(
    () =>
      parseToolResult(
        response([
          { type: "image", data: "", mimeType: "image/png" },
          { type: "image", data: "", mimeType: "image/png" },
          { type: "text", text: JSON.stringify(evidence) },
        ]),
        options,
      ),
    /screenshots were not emitted/u,
  );
  assert.throws(
    () =>
      parseToolResult(
        response([
          {
            type: "image",
            data: "iVBORw0KGgo=",
            mimeType: "image/png",
            _meta: { "codex/imageDetail": "original" },
          },
          {
            type: "image",
            data: "iVBORw0KGgo=",
            mimeType: "image/png",
            _meta: { "codex/imageDetail": "original" },
          },
          {
            type: "text",
            text: JSON.stringify({
              ...evidence,
              screenshot: { ...evidence.screenshot, after_byte_length: 7 },
            }),
          },
        ]),
        options,
      ),
    /byte length does not match/u,
  );
});

test("rejects signature-only PNG and header-only WebP screenshots", () => {
  const { options } = fixture();
  const validPng = solidPng(32);
  const requestMeta = {
    session_id: options.sessionId,
    turn_id: options.turnId,
    "x-codex-turn-metadata": {
      session_id: options.sessionId,
      turn_id: options.turnId,
    },
  };
  const response = (before: Buffer, after: Buffer, mimeType = "image/png") => ({
    result: {
      content: [
        imageItem(before, mimeType),
        imageItem(after, mimeType),
        {
          type: "text",
          text: JSON.stringify({
            navigation: { requested_url: options.url, final_url: options.url },
            keyboard: {
              method: "PlaywrightLocator.type+press",
              key: "End",
              text: options.typedText,
              value: options.typedText,
            },
            click: { actual: true, button_name: options.buttonName },
            readback: options.typedText,
            screenshot: {
              method: "Tab.screenshot",
              emitted: true,
              expected_width: 2,
              expected_height: 2,
              before_byte_length: before.length,
              after_byte_length: after.length,
            },
            request_meta: requestMeta,
          }),
        },
      ],
      isError: false,
      _meta: {
        "codex/toolSurface": {
          kind: "browserUse",
          backend: "iab",
          browserId: "iab",
        },
      },
    },
  });
  assert.throws(
    () =>
      parseToolResult(
        response(Buffer.from("89504e470d0a1a0a", "hex"), validPng),
        options,
      ),
    /truncated/u,
  );
  assert.throws(
    () => parseToolResult(response(validPng.subarray(0, -4), validPng), options),
    /truncated/u,
  );
  const headerOnly = Buffer.alloc(30);
  headerOnly.write("RIFF", 0, "ascii");
  headerOnly.writeUInt32LE(22, 4);
  headerOnly.write("WEBPVP8X", 8, "ascii");
  headerOnly.writeUInt32LE(10, 16);
  assert.throws(
    () => parseToolResult(response(headerOnly, solidWebp(0), "image/webp"), options),
    /no complete WebP image payload/u,
  );
});

test("rejects identical decoded before and after screenshots", () => {
  const { options } = fixture();
  const png = solidPng(64);
  const requestMeta = {
    session_id: options.sessionId,
    turn_id: options.turnId,
    "x-codex-turn-metadata": {
      session_id: options.sessionId,
      turn_id: options.turnId,
    },
  };
  assert.throws(
    () =>
      parseToolResult(
        {
          result: {
            content: [
              imageItem(png),
              imageItem(png),
              {
                type: "text",
                text: JSON.stringify({
                  navigation: {
                    requested_url: options.url,
                    final_url: options.url,
                  },
                  keyboard: {
                    method: "PlaywrightLocator.type+press",
                    key: "End",
                    text: options.typedText,
                    value: options.typedText,
                  },
                  click: { actual: true, button_name: options.buttonName },
                  readback: options.typedText,
                  screenshot: {
                    method: "Tab.screenshot",
                    emitted: true,
                    expected_width: 2,
                    expected_height: 2,
                    before_byte_length: png.length,
                    after_byte_length: png.length,
                  },
                  request_meta: requestMeta,
                }),
              },
            ],
            isError: false,
            _meta: {
              "codex/toolSurface": {
                kind: "browserUse",
                backend: "iab",
                browserId: "iab",
              },
            },
          },
        },
        options,
      ),
    /identical decoded pixels/u,
  );
});

test("constructs Brave-primary Origin extension scenario with extension selection", () => {
  const { options } = fixture();
  options.scenario = "brave-origin-extension";
  const code = buildAcceptanceCode(options, validateInstalledSelection(options));
  assert.match(code, /agent\.browsers\.get\("extension"\)/u);
  assert.match(code, /entry\.type === "extension"/u);
  // The upstream protocol reports the logical extension backend as Chrome even
  // when the native host belongs to Brave Origin. Browser provenance is a live
  // acceptance precondition, not an API display-name assertion.
  assert.doesNotMatch(code, /includes\("brave"\)/u);
  assert.doesNotMatch(code, /agent\.browsers\.get\("iab"\)/u);
  assert.match(code, /await acceptanceTab\.screenshot\(\)/u);
  assert.match(code, /await nodeRepl\.emitImage\(beforeScreenshot\)/u);
  assert.match(code, /await nodeRepl\.emitImage\(afterScreenshot\)/u);
  assert.doesNotMatch(code, /skynet/iu);
});

test("proves the extension native host is parented by Brave Origin", () => {
  const { options } = fixture();
  const procRoot = mkdtempSync(join(tmpdir(), "browser-live-proc-"));
  const braveExecutable = join(procRoot, "brave-origin");
  const hostExecutable = join(procRoot, "sky-cua-chrome-host");
  writeFileSync(braveExecutable, "brave", "utf8");
  writeFileSync(hostExecutable, "host", "utf8");
  chmodSync(braveExecutable, 0o755);
  chmodSync(hostExecutable, 0o755);
  options.braveExecutable = braveExecutable;
  mkdirSync(join(procRoot, "100"));
  mkdirSync(join(procRoot, "200"));
  symlinkSync(braveExecutable, join(procRoot, "100", "exe"));
  symlinkSync(hostExecutable, join(procRoot, "200", "exe"));
  writeFileSync(join(procRoot, "200", "status"), "Name:\thost\nPPid:\t100\n");
  writeFileSync(
    join(procRoot, "200", "cmdline"),
    "sky-cua-chrome-host\0chrome-extension://hehggadaopoacecdllhhajmbjkdcmajg/\0",
  );

  assert.deepEqual(verifyBraveOriginNativeHost(options, procRoot), {
    executable: braveExecutable,
    browser_pid: 100,
    native_host_pid: 200,
    native_host: hostExecutable,
    extension_origin: "chrome-extension://hehggadaopoacecdllhhajmbjkdcmajg/",
  });
});

test("rejects ambiguous simultaneous extension native hosts", () => {
  const { options } = fixture();
  const procRoot = mkdtempSync(join(tmpdir(), "browser-live-proc-ambiguous-"));
  const braveExecutable = join(procRoot, "brave-origin");
  const otherBrowserExecutable = join(procRoot, "other-browser");
  const hostExecutable = join(procRoot, "sky-cua-chrome-host");
  for (const executable of [braveExecutable, otherBrowserExecutable, hostExecutable]) {
    writeFileSync(executable, "executable", "utf8");
    chmodSync(executable, 0o755);
  }
  options.braveExecutable = braveExecutable;
  for (const pid of ["100", "200", "300", "400"]) mkdirSync(join(procRoot, pid));
  symlinkSync(braveExecutable, join(procRoot, "100", "exe"));
  symlinkSync(hostExecutable, join(procRoot, "200", "exe"));
  symlinkSync(otherBrowserExecutable, join(procRoot, "300", "exe"));
  symlinkSync(hostExecutable, join(procRoot, "400", "exe"));
  const origin = "chrome-extension://hehggadaopoacecdllhhajmbjkdcmajg/";
  writeFileSync(join(procRoot, "200", "status"), "Name:\thost\nPPid:\t100\n");
  writeFileSync(join(procRoot, "200", "cmdline"), `host\0${origin}\0`);
  writeFileSync(join(procRoot, "400", "status"), "Name:\thost\nPPid:\t300\n");
  writeFileSync(join(procRoot, "400", "cmdline"), `host\0${origin}\0`);

  assert.throws(
    () => verifyBraveOriginNativeHost(options, procRoot),
    /ambiguous .* native hosts: 2 candidates/u,
  );
});
