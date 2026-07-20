const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { test } = require("bun:test");

export {};

type JsonObject = Record<string, unknown>;

const fixtureRoot = __dirname;
const repositoryRoot = path.resolve(
  process.env.CUA_NODE_UPSTREAM_EVIDENCE_ROOT ??
    path.resolve(fixtureRoot, "../../../../../"),
);
const upstreamEvidenceAvailable = fs.existsSync(
  path.resolve(
    repositoryRoot,
    "research/upstream-cua-node/26.707.72221-darwin-arm64/cua_node/bin/node_repl",
  ),
);

function readJson(name: string): unknown {
  return JSON.parse(fs.readFileSync(path.join(fixtureRoot, name), "utf8"));
}

function object(value: unknown, label: string): JsonObject {
  assert.equal(typeof value, "object", `${label} must be an object`);
  assert.notEqual(value, null, `${label} must not be null`);
  assert.equal(Array.isArray(value), false, `${label} must not be an array`);
  return value as JsonObject;
}

function requiredEntry<T>(value: T | undefined, label: string): T {
  if (value === undefined) {
    throw new Error(`${label} is missing`);
  }
  return value;
}

const fixtureNames = [
  "contract.json",
  "tools-list.json",
  "provenance.json",
  "mcp-transcripts.json",
  "kernel.json",
  "node-repl-surfaces.json",
  "output-metadata.json",
  "lifecycle.json",
  "trusted-helper.json",
  "native-pipe.json",
];

test("all upstream-5307 contract fixtures are valid JSON", () => {
  for (const name of fixtureNames) {
    const value = readJson(name);
    assert.notEqual(value, undefined, name);
  }
});

test("tools-list freezes modern tool order and schemas", () => {
  const fixture = object(readJson("tools-list.json"), "tools-list");
  const tools = fixture.tools;
  assert.equal(Array.isArray(tools), true);
  const toolObjects = (tools as unknown[]).map((entry, index) =>
    object(entry, `tools[${index}]`),
  );
  assert.deepEqual(
    toolObjects.map((entry) => entry.name),
    ["js", "js_reset", "js_add_node_module_dir"],
  );
  const jsTool = requiredEntry(toolObjects[0], "js tool");
  const resetTool = requiredEntry(toolObjects[1], "reset tool");
  const moduleDirTool = requiredEntry(toolObjects[2], "module-dir tool");

  const jsSchema = object(jsTool.inputSchema, "js inputSchema");
  const jsProperties = object(jsSchema.properties, "js properties");
  assert.deepEqual(jsSchema.required, ["code"]);
  assert.equal(jsSchema.additionalProperties, false);
  assert.equal(object(jsProperties.timeout_ms, "timeout_ms").minimum, 1);
  assert.equal(object(jsProperties.title, "title").maxLength, 80);
  assert.equal(object(jsProperties.title, "title").minLength, 1);

  const resetSchema = object(resetTool.inputSchema, "reset inputSchema");
  assert.deepEqual(resetSchema.properties, {});
  assert.equal(resetSchema.additionalProperties, false);

  const moduleSchema = object(
    moduleDirTool.inputSchema,
    "module-dir inputSchema",
  );
  assert.deepEqual(moduleSchema.required, ["path"]);
  assert.equal(
    object(object(moduleSchema.properties, "module-dir properties").path, "path")
      .minLength,
    1,
  );
  assert.match(String(jsTool.description), /persistent Node-backed kernel/u);
  assert.match(String(moduleDirTool.description), /stays available.*js_reset/u);
});

test.skipIf(!upstreamEvidenceAvailable)(
  "provenance paths resolve inside the checked-out evidence set",
  () => {
  const fixture = object(readJson("provenance.json"), "provenance");
  const sources = fixture.sources;
  assert.equal(Array.isArray(sources), true);
  for (const entry of sources as unknown[]) {
    const source = object(entry, "provenance source");
    const sourcePath = source.path;
    assert.equal(typeof sourcePath, "string");
    assert.equal(
      fs.existsSync(path.resolve(repositoryRoot, sourcePath as string)),
      true,
      sourcePath as string,
    );
    assert.equal(typeof source.selector, "string");
  }
  },
);

test.skipIf(!upstreamEvidenceAvailable)(
  "pinned local evidence still contains the contract markers",
  () => {
  const darwinBinary = fs.readFileSync(
    path.resolve(
      repositoryRoot,
      "research/upstream-cua-node/26.707.72221-darwin-arm64/cua_node/bin/node_repl",
    ),
  ).toString("latin1");
  for (const marker of [
    "struct JsToolArgs with 3 elements",
    "js_reset schema should deserialize",
    "js_add_node_module_dir schema should deserialize",
    "previousModule",
    "privileged_bridge_handshake",
    "native_pipe_request",
    "nodeRepl.emitImage expected non-empty bytes",
  ]) {
    assert.notEqual(darwinBinary.indexOf(marker), -1, marker);
  }

  const browserSource = fs.readFileSync(
    path.resolve(
      repositoryRoot,
      "resources/upstream/plugins/openai-bundled/plugins/browser/scripts/browser-client.mjs",
    ),
    "utf8",
  );
  assert.match(browserSource, /\/tmp\/codex-browser-use/u);
  assert.match(browserSource, /globalThis\.nodeRepl\?\.nativePipe/u);
  const syncSource = fs.readFileSync(
    path.resolve(repositoryRoot, "scripts/sync-browser-use-plugin.ts"),
    "utf8",
  );
  assert.doesNotMatch(syncSource, /import\.meta\.__codexNativePipe/u);
  assert.doesNotMatch(syncSource, /CODEX_BROWSER_PROVIDER/u);
  assert.equal(
    fs.existsSync(path.resolve(repositoryRoot, "resources/node_repl")),
    true,
  );
  },
);

test.skipIf(!upstreamEvidenceAvailable)(
  "modern tool descriptions are recovered verbatim from the Darwin binary",
  () => {
  const fixture = object(readJson("tools-list.json"), "tools-list");
  const tools = fixture.tools as unknown[];
  const darwinStrings = execFileSync(
    "strings",
    [
      "-a",
      "-n",
      "5",
      path.resolve(
        repositoryRoot,
        "research/upstream-cua-node/26.707.72221-darwin-arm64/cua_node/bin/node_repl",
      ),
    ],
    { encoding: "utf8", maxBuffer: 50 * 1024 * 1024 },
  );
  for (const entry of tools) {
    const tool = object(entry, "tool");
    assert.equal(darwinStrings.includes(String(tool.description)), true, String(tool.name));
  }
  },
);

test("kernel surfaces and lifecycle fixtures retain the safety invariants", () => {
  const kernel = object(readJson("kernel.json"), "kernel");
  const processSurface = object(kernel.process_surface, "process surface");
  const untrusted = object(processSurface.untrusted, "untrusted process");
  assert.equal(untrusted.ambient_process, "absent");
  assert.deepEqual(untrusted.dynamic_imports, [
    { specifier: "process", result: "rejected" },
    { specifier: "node:process", result: "rejected" },
  ]);

  const surfaces = object(readJson("node-repl-surfaces.json"), "surfaces");
  const publicSurface = object(surfaces.untrusted, "untrusted nodeRepl");
  assert.equal(publicSurface.object_is_frozen, true);
  assert.deepEqual(publicSurface.own_keys_order, [
    "cwd",
    "env",
    "homeDir",
    "tmpDir",
    "requestMeta",
    "write",
    "setResponseMeta",
    "emitImage",
  ]);
  const trustedSurface = object(surfaces.trusted, "trusted nodeRepl");
  assert.equal(trustedSurface.object_is_frozen, true);
  assert.equal(trustedSurface.inherited_keys, "all untrusted own keys");

  const lifecycle = object(readJson("lifecycle.json"), "lifecycle");
  const timeout = object(lifecycle.timeout, "timeout");
  assert.equal(timeout.default_ms, 30000);
  assert.equal(timeout.minimum_ms, 1);
  assert.equal(timeout.expected_error, "js execution timed out; kernel reset, rerun your request");
  const reset = object(lifecycle.reset, "reset");
  assert.equal(reset.pending_error, "js execution reset");
});

test("native pipe fixture has a valid deterministic little-endian frame", () => {
  const fixture = object(readJson("native-pipe.json"), "native pipe");
  const socket = object(fixture.browser_socket, "browser socket");
  const framing = object(socket.framing, "framing");
  assert.equal(framing.length_prefix_bytes, 4);
  assert.equal(framing.byte_order, "little-endian on Linux");
  assert.equal(framing.maximum_frame_bytes, 8388608);
  const ping = object(socket.ping_example, "ping example");
  const payload = Buffer.from(String(ping.json), "utf8");
  assert.equal(payload.byteLength, ping.payload_bytes);
  assert.equal(
    Buffer.from(String(ping.little_endian_prefix_hex), "hex").readUInt32LE(0),
    payload.byteLength,
  );
  assert.equal(String(ping.full_frame_hex), `${ping.little_endian_prefix_hex}${payload.toString("hex")}`);
});

test("image fixtures encode the recovered MIME sniffing contract", () => {
  const fixture = object(readJson("output-metadata.json"), "output metadata");
  const images = object(fixture.images, "images");
  const cases = images.sniff_cases;
  assert.equal(Array.isArray(cases), true);
  for (const entry of cases as unknown[]) {
    const image = object(entry, "image sniff case");
    const bytes = Buffer.from(String(image.bytes_hex), "hex");
    const dataUrl = `data:${image.mimeType};base64,${bytes.toString("base64")}`;
    assert.equal(dataUrl, image.data_url);
  }
});

test("unknown markers are explicit and explained", () => {
  const contract = object(readJson("contract.json"), "contract");
  const unknowns = contract.unknowns;
  assert.equal(Array.isArray(unknowns), true);
  for (const entry of unknowns as unknown[]) {
    const unknown = object(entry, "unknown marker");
    assert.equal(unknown.status, "unknown");
    assert.equal(typeof unknown.reason, "string");
    assert.notEqual(String(unknown.reason).trim(), "");
  }
});
