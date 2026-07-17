#!/usr/bin/env node

import { Buffer } from "node:buffer";
import { mkdtemp, rm, stat } from "node:fs/promises";
import { createServer, createConnection } from "node:net";
import { tmpdir } from "node:os";
import { extname, isAbsolute, join } from "node:path";
import { performance } from "node:perf_hooks";
import { pathToFileURL } from "node:url";

const DEFAULT_ITERATIONS = 100;
const DEFAULT_MOUSE_SIZE_PX = 12;
const MAX_FRAME_BYTES = 64 * 1024 * 1024;
const MAX_P95_OVERHEAD_PERCENT = 10;
const MAX_PAYLOAD_INFLATION_PERCENT = 5;
const DATA_URL_PREFIX = "data:image/webp;base64,";
const REQUIRED_CAPABILITIES = new Set([
  "linux.get_screenshot",
  "screen.cursor_size",
  "screenshot.webp"
]);

function fail(message) {
  throw new Error(message);
}

export function percentile(samples, quantile) {
  if (samples.length === 0 || quantile <= 0 || quantile > 1) {
    fail("percentile requires samples and a quantile in (0, 1].");
  }
  const sorted = [...samples].sort((left, right) => left - right);
  return sorted[Math.ceil(sorted.length * quantile) - 1];
}

function summarize(samples) {
  return {
    p50_ms: percentile(samples, 0.5),
    p95_ms: percentile(samples, 0.95)
  };
}

function overheadPercent(baseline, adapter) {
  if (baseline <= 0) {
    fail("raw p95 must be greater than zero.");
  }
  return ((adapter - baseline) / baseline) * 100;
}

function defaultSocketPath() {
  const explicit = process.env.SKY_CUA_SERVICE_SOCKET_PATH;
  if (explicit) return explicit;
  if (process.env.XDG_RUNTIME_DIR) return join(process.env.XDG_RUNTIME_DIR, "sky-cua", "service.sock");
  if (process.env.XDG_CACHE_HOME) return join(process.env.XDG_CACHE_HOME, "sky-cua", "service.sock");
  if (process.env.HOME) return join(process.env.HOME, ".cache", "sky-cua", "service.sock");
  const uid = typeof process.getuid === "function" ? process.getuid() : "unknown";
  return join(tmpdir(), `sky-cua-uid-${uid}`, "service.sock");
}

function parseArguments(argv) {
  const options = {
    socketPath: defaultSocketPath(),
    iterations: DEFAULT_ITERATIONS,
    selfTest: false
  };
  const positional = [];
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--self-test") {
      options.selfTest = true;
    } else if (argument === "--socket") {
      options.socketPath = argv[++index];
    } else if (argument?.startsWith("--socket=")) {
      options.socketPath = argument.slice("--socket=".length);
    } else if (argument === "--iterations") {
      options.iterations = Number(argv[++index]);
    } else if (argument?.startsWith("--iterations=")) {
      options.iterations = Number(argument.slice("--iterations=".length));
    } else if (argument === "--help" || argument === "-h") {
      console.log("Usage: node --expose-gc scripts/benchmark-screenshot.mjs [--socket PATH] [--iterations N]");
      console.log("       node --expose-gc scripts/benchmark-screenshot.mjs [SOCKET_PATH] [ITERATIONS]");
      process.exit(0);
    } else if (argument?.startsWith("-")) {
      fail(`unknown argument: ${argument}`);
    } else if (argument !== undefined) {
      positional.push(argument);
    }
  }
  if (positional[0] !== undefined) options.socketPath = positional[0];
  if (positional[1] !== undefined) options.iterations = Number(positional[1]);
  if (positional.length > 2) fail("expected at most socket path and iterations as positional arguments.");
  if (typeof options.socketPath !== "string" || options.socketPath.length === 0) {
    fail("socket path must be a non-empty string.");
  }
  if (!Number.isInteger(options.iterations) || options.iterations < 2) {
    fail("iterations must be an integer of at least 2.");
  }
  return options;
}

class PersistentNdjsonClient {
  constructor(socket) {
    this.socket = socket;
    this.buffer = Buffer.alloc(0);
    this.lines = [];
    this.waiters = [];
    this.failure = undefined;
    socket.setNoDelay(true);
    socket.on("data", (chunk) => this.accept(chunk));
    socket.on("error", (error) => this.reject(error));
    socket.on("close", () => this.reject(new Error("NDJSON socket closed.")));
  }

  static async connect(path) {
    const socket = createConnection({ path });
    await new Promise((resolve, reject) => {
      socket.once("connect", resolve);
      socket.once("error", reject);
    });
    return new PersistentNdjsonClient(socket);
  }

  accept(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    if (this.buffer.length > MAX_FRAME_BYTES) fail("NDJSON response exceeds 64 MiB.");
    while (true) {
      const newline = this.buffer.indexOf(0x0a);
      if (newline < 0) return;
      const line = this.buffer.subarray(0, newline);
      this.buffer = this.buffer.subarray(newline + 1);
      const item = { value: JSON.parse(line.toString("utf8")), frameBytes: newline + 1 };
      const waiter = this.waiters.shift();
      if (waiter === undefined) this.lines.push(item);
      else waiter.resolve(item);
    }
  }

  reject(error) {
    if (this.failure !== undefined) return;
    this.failure = error;
    for (const waiter of this.waiters.splice(0)) waiter.reject(error);
  }

  async requestSequential(value) {
    if (this.failure !== undefined) throw this.failure;
    this.socket.write(`${JSON.stringify(value)}\n`);
    const buffered = this.lines.shift();
    return buffered ?? await new Promise((resolve, reject) => this.waiters.push({ resolve, reject }));
  }

  close() {
    this.socket.destroy();
  }
}

function validateHealth(response) {
  if (response?.type !== "health" || response.ok !== true || response.protocol_version !== 1) {
    fail("raw connection received an invalid health response.");
  }
  const capabilities = new Set(response.capabilities);
  for (const capability of REQUIRED_CAPABILITIES) {
    if (!capabilities.has(capability)) fail(`raw health is missing capability ${capability}.`);
  }
}

function validateScreenshotPath(filepath, lane, index) {
  if (!isAbsolute(filepath) || extname(filepath).toLowerCase() !== ".webp") {
    fail(`${lane} screenshot ${index} path is not an absolute WebP path.`);
  }
}

function decodeCanonicalBase64(value, lane, index) {
  const bytes = Buffer.from(value, "base64");
  if (bytes.length === 0 || bytes.toString("base64") !== value) {
    fail(`${lane} screenshot ${index} bytes are not canonical base64.`);
  }
  return bytes;
}

function validateRawResponse(response) {
  if (response?.type !== "get_screenshot" || response.ok !== true || !Array.isArray(response.screenshots)) {
    fail("raw connection received an invalid get_screenshot response.");
  }
  if (response.screenshots.length === 0) fail("raw connection returned no screenshots.");
  return response.screenshots.map((screenshot, index) => {
    if (
      typeof screenshot?.filepath !== "string" ||
      typeof screenshot.bytes_base64 !== "string" ||
      screenshot.mime_type !== "image/webp" ||
      !Number.isInteger(screenshot.width) || screenshot.width <= 0 ||
      !Number.isInteger(screenshot.height) || screenshot.height <= 0
    ) {
      fail("raw connection received invalid screenshot metadata.");
    }
    validateScreenshotPath(screenshot.filepath, "raw", index);
    const bytes = decodeCanonicalBase64(screenshot.bytes_base64, "raw", index);
    const dimensions = webpDimensions(bytes);
    if (dimensions.width !== screenshot.width || dimensions.height !== screenshot.height) {
      fail(`raw WebP dimensions ${dimensions.width}x${dimensions.height} disagree with wire dimensions ${screenshot.width}x${screenshot.height}.`);
    }
    const expectedDataUrl = `${DATA_URL_PREFIX}${screenshot.bytes_base64}`;
    if (screenshot.data_url !== undefined && screenshot.data_url !== expectedDataUrl) {
      fail(`raw screenshot ${index} data URL disagrees with its bytes.`);
    }
    return { ...screenshot, bytes };
  });
}

function validateFacadeResponse(response) {
  if (!Array.isArray(response) || response.length === 0) {
    fail("facade returned no screenshots.");
  }
  return response.map((screenshot, index) => {
    if (typeof screenshot?.filepath !== "string" || !(screenshot.bytes instanceof Uint8Array)) {
      fail(`facade screenshot ${index} has invalid metadata.`);
    }
    validateScreenshotPath(screenshot.filepath, "facade", index);
    const bytes = Buffer.from(screenshot.bytes);
    const dimensions = webpDimensions(bytes);
    const expectedDataUrl = `${DATA_URL_PREFIX}${bytes.toString("base64")}`;
    if (screenshot.data_url !== expectedDataUrl) {
      fail(`facade screenshot ${index} data URL disagrees with its bytes.`);
    }
    return { ...screenshot, bytes, dimensions };
  });
}

async function validatePathsExist(screenshots, lane) {
  for (const [index, screenshot] of screenshots.entries()) {
    let metadata;
    try {
      metadata = await stat(screenshot.filepath);
    } catch {
      fail(`${lane} screenshot ${index} path does not exist.`);
    }
    if (!metadata.isFile()) fail(`${lane} screenshot ${index} path is not a file.`);
  }
}

export function webpDimensions(bytes) {
  if (bytes.length < 30 || bytes.toString("ascii", 0, 4) !== "RIFF" || bytes.toString("ascii", 8, 12) !== "WEBP") {
    fail("screenshot bytes are not a WebP RIFF payload.");
  }
  let offset = 12;
  while (offset + 8 <= bytes.length) {
    const chunk = bytes.toString("ascii", offset, offset + 4);
    const length = bytes.readUInt32LE(offset + 4);
    const data = offset + 8;
    if (data + length > bytes.length) fail("WebP contains a truncated chunk.");
    if (chunk === "VP8X" && length >= 10) {
      return {
        width: 1 + bytes.readUIntLE(data + 4, 3),
        height: 1 + bytes.readUIntLE(data + 7, 3)
      };
    }
    if (chunk === "VP8 " && length >= 10 && bytes[data + 3] === 0x9d && bytes[data + 4] === 0x01 && bytes[data + 5] === 0x2a) {
      return {
        width: bytes.readUInt16LE(data + 6) & 0x3fff,
        height: bytes.readUInt16LE(data + 8) & 0x3fff
      };
    }
    if (chunk === "VP8L" && length >= 5 && bytes[data] === 0x2f) {
      const packed = bytes.readUInt32LE(data + 1);
      return { width: 1 + (packed & 0x3fff), height: 1 + ((packed >>> 14) & 0x3fff) };
    }
    offset = data + length + (length % 2);
  }
  fail("WebP does not contain a supported dimensions chunk.");
}

function compareParity(raw, facade) {
  if (!Array.isArray(facade) || facade.length !== raw.length) {
    fail(`screenshot count differs: raw=${raw.length}, facade=${Array.isArray(facade) ? facade.length : "invalid"}.`);
  }
  let rawBytes = 0;
  let facadeBytes = 0;
  let facadeDataUrlCharacters = 0;
  for (let index = 0; index < raw.length; index += 1) {
    const expected = raw[index];
    const actual = facade[index];
    if (actual?.filepath !== expected.filepath) fail(`screenshot ${index} path differs.`);
    const bytes = Buffer.from(actual?.bytes ?? []);
    if (!bytes.equals(expected.bytes)) fail(`screenshot ${index} binary bytes differ.`);
    const dimensions = webpDimensions(bytes);
    if (dimensions.width !== expected.width || dimensions.height !== expected.height) {
      fail(`screenshot ${index} facade dimensions differ from raw wire dimensions.`);
    }
    const expectedDataUrl = `${DATA_URL_PREFIX}${expected.bytes_base64}`;
    if (actual.data_url !== expectedDataUrl) fail(`screenshot ${index} WebP data URL differs.`);
    rawBytes += expected.bytes.length;
    facadeBytes += bytes.length;
    facadeDataUrlCharacters += actual.data_url.length;
  }
  return { rawBytes, facadeBytes, facadeDataUrlCharacters };
}

async function gcHeapUsed() {
  if (typeof globalThis.gc !== "function") {
    fail("explicit GC is unavailable; run Node with --expose-gc.");
  }
  globalThis.gc();
  await new Promise((resolve) => setImmediate(resolve));
  globalThis.gc();
  return process.memoryUsage().heapUsed;
}

function heapGrowth(samples, iterations) {
  const first = samples[0];
  const last = samples.at(-1);
  const growthBytes = last - first;
  const slopeBytesPerIteration = growthBytes / iterations;
  let rises = 0;
  for (let index = 1; index < samples.length; index += 1) {
    if (samples[index] > samples[index - 1]) rises += 1;
  }
  const sustainedRiseRatio = rises / Math.max(1, samples.length - 1);
  const allowanceBytes = Math.max(1024 * 1024, first * 0.05);
  const unbounded = growthBytes > allowanceBytes && slopeBytesPerIteration > 8 * 1024 && sustainedRiseRatio >= 0.7;
  return { first, last, growthBytes, slopeBytesPerIteration, sustainedRiseRatio, allowanceBytes, unbounded };
}

async function importFacade(socketPath) {
  process.env.SKY_CUA_SERVICE_SOCKET_PATH = socketPath;
  delete process.env.OAI_SKY_CONFIG_PATH;
  delete process.env.SKY_CUA_JS_CONFIG_PATH;
  const entrypoint = pathToFileURL(join(import.meta.dirname, "..", "dist", "index.js"));
  entrypoint.searchParams.set("benchmark", `${process.pid}-${performance.now()}`);
  const module = await import(entrypoint.href);
  return module.sky;
}

export async function runBenchmark({
  socketPath,
  iterations,
  enforceGates = true,
  exactCaptureParity = false,
  requirePathsExist = true
}) {
  if (Number(process.versions.node.split(".")[0]) < 24) fail("the screenshot benchmark requires Node 24 or newer.");
  const rawClient = await PersistentNdjsonClient.connect(socketPath);
  try {
    const health = await rawClient.requestSequential({ type: "health" });
    validateHealth(health.value);
    const sky = await importFacade(socketPath);

    const rawWarmup = validateRawResponse((await rawClient.requestSequential({ type: "get_screenshot", mouse_size_px: DEFAULT_MOUSE_SIZE_PX })).value);
    const facadeWarmup = validateFacadeResponse(await sky.get_screenshot());
    if (rawWarmup.length !== facadeWarmup.length) {
      fail(`warmup screenshot count differs: raw=${rawWarmup.length}, facade=${facadeWarmup.length}.`);
    }
    if (requirePathsExist) {
      await validatePathsExist(rawWarmup, "raw warmup");
      await validatePathsExist(facadeWarmup, "facade warmup");
    }
    if (exactCaptureParity) compareParity(rawWarmup, facadeWarmup);

    const rawDurations = [];
    const facadeDurations = [];
    const heapSamples = [await gcHeapUsed()];
    const heapInterval = Math.max(1, Math.ceil(iterations / 10));
    let rawPayloadBytes = 0;
    let facadePayloadBytes = 0;
    let rawFrameBytes = 0;
    let facadeDataUrlCharacters = 0;

    for (let index = 0; index < iterations; index += 1) {
      let rawCapture;
      let facadeCapture;
      const measureRaw = async () => {
        const started = performance.now();
        const response = await rawClient.requestSequential({ type: "get_screenshot", mouse_size_px: DEFAULT_MOUSE_SIZE_PX });
        rawDurations.push(performance.now() - started);
        rawFrameBytes += response.frameBytes;
        rawCapture = validateRawResponse(response.value);
        if (requirePathsExist) await validatePathsExist(rawCapture, "raw");
      };
      const measureFacade = async () => {
        const started = performance.now();
        const response = await sky.get_screenshot();
        facadeDurations.push(performance.now() - started);
        facadeCapture = validateFacadeResponse(response);
        if (requirePathsExist) await validatePathsExist(facadeCapture, "facade");
      };
      if (index % 2 === 0) {
        await measureRaw();
        await measureFacade();
      } else {
        await measureFacade();
        await measureRaw();
      }
      if (rawCapture.length !== facadeCapture.length) {
        fail(`screenshot count differs: raw=${rawCapture.length}, facade=${facadeCapture.length}.`);
      }
      if (exactCaptureParity) compareParity(rawCapture, facadeCapture);
      rawPayloadBytes += rawCapture.reduce((total, screenshot) => total + screenshot.bytes.length, 0);
      facadePayloadBytes += facadeCapture.reduce((total, screenshot) => total + screenshot.bytes.length, 0);
      facadeDataUrlCharacters += facadeCapture.reduce((total, screenshot) => total + screenshot.data_url.length, 0);
      if ((index + 1) % heapInterval === 0 || index + 1 === iterations) heapSamples.push(await gcHeapUsed());
    }

    const raw = summarize(rawDurations);
    const facade = summarize(facadeDurations);
    const p95OverheadPercent = overheadPercent(raw.p95_ms, facade.p95_ms);
    const payloadInflationPercent = rawPayloadBytes === 0
      ? 0
      : ((facadePayloadBytes - rawPayloadBytes) / rawPayloadBytes) * 100;
    const heap = heapGrowth(heapSamples, iterations);
    const failures = [];
    if (p95OverheadPercent > MAX_P95_OVERHEAD_PERCENT) {
      failures.push(`adapter p95 overhead ${p95OverheadPercent.toFixed(2)}% exceeds ${MAX_P95_OVERHEAD_PERCENT}%.`);
    }
    if (payloadInflationPercent > MAX_PAYLOAD_INFLATION_PERCENT) {
      failures.push(`canonical payload inflation ${payloadInflationPercent.toFixed(2)}% exceeds ${MAX_PAYLOAD_INFLATION_PERCENT}%.`);
    }
    if (heap.unbounded) failures.push("post-GC heap samples show sustained unbounded growth.");
    const result = {
      socket_path: socketPath,
      iterations,
      warmup_iterations_per_lane: 1,
      raw,
      facade,
      adapter_p95_overhead_percent: p95OverheadPercent,
      payload: {
        raw_binary_bytes: rawPayloadBytes,
        facade_binary_bytes: facadePayloadBytes,
        canonical_inflation_percent: payloadInflationPercent,
        raw_ndjson_frame_bytes: rawFrameBytes,
        facade_data_url_characters: facadeDataUrlCharacters,
        validation: exactCaptureParity
          ? "exact fake-daemon adapter transformation parity"
          : "independent WebP/path/dimensions/data-URL/count validation"
      },
      heap,
      failures
    };
    if (enforceGates && failures.length > 0) fail(failures.join(" "));
    return result;
  } finally {
    rawClient.close();
  }
}

function renderResult(result) {
  console.log(`screenshot benchmark: ${result.iterations} iterations, socket ${result.socket_path}`);
  console.log(`raw     p50=${result.raw.p50_ms.toFixed(3)}ms p95=${result.raw.p95_ms.toFixed(3)}ms`);
  console.log(`facade  p50=${result.facade.p50_ms.toFixed(3)}ms p95=${result.facade.p95_ms.toFixed(3)}ms`);
  console.log(`adapter p95 overhead=${result.adapter_p95_overhead_percent.toFixed(2)}% (limit ${MAX_P95_OVERHEAD_PERCENT}%)`);
  console.log(`payload inflation=${result.payload.canonical_inflation_percent.toFixed(2)}% (limit ${MAX_PAYLOAD_INFLATION_PERCENT}%); ${result.payload.validation}`);
  console.log(`heap growth=${result.heap.growthBytes} bytes, slope=${result.heap.slopeBytesPerIteration.toFixed(1)} bytes/iteration, sustained-rise=${(result.heap.sustainedRiseRatio * 100).toFixed(1)}%`);
}

function fakeWebp(width, height) {
  const bytes = Buffer.alloc(30);
  bytes.write("RIFF", 0, "ascii");
  bytes.writeUInt32LE(22, 4);
  bytes.write("WEBPVP8X", 8, "ascii");
  bytes.writeUInt32LE(10, 16);
  bytes.writeUIntLE(width - 1, 24, 3);
  bytes.writeUIntLE(height - 1, 27, 3);
  return bytes;
}

async function runSelfTest() {
  const directory = await mkdtemp(join(tmpdir(), "sky-cua-screenshot-benchmark-"));
  const socketPath = join(directory, "daemon.sock");
  const connections = [];
  const sockets = [];
  const requests = [];
  const bytes = fakeWebp(20, 10);
  const server = createServer((socket) => {
    sockets.push(socket);
    const connection = { requests: [] };
    connections.push(connection);
    let buffer = "";
    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8");
      while (buffer.includes("\n")) {
        const newline = buffer.indexOf("\n");
        const request = JSON.parse(buffer.slice(0, newline));
        buffer = buffer.slice(newline + 1);
        requests.push(request);
        connection.requests.push(request);
        const response = request.type === "health"
          ? {
              type: "health", ok: true, protocol_version: 1, service_version: "0.1.0",
              capabilities: [...REQUIRED_CAPABILITIES], service_socket: socketPath
            }
          : {
              type: "get_screenshot", ok: true,
              screenshots: [{
                filepath: "/tmp/fake.webp", bytes_base64: bytes.toString("base64"),
                mime_type: "image/webp", width: 20, height: 10
              }]
            };
        setTimeout(() => socket.write(`${JSON.stringify(response)}\n`), 4);
      }
    });
  });
  try {
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(socketPath, resolve);
    });
    const result = await runBenchmark({
      socketPath,
      iterations: 6,
      enforceGates: false,
      exactCaptureParity: true,
      requirePathsExist: false
    });
    if (percentile([5, 1, 3, 2, 4], 0.5) !== 3 || percentile([5, 1, 3, 2, 4], 0.95) !== 5) {
      fail("self-test percentile calculation failed.");
    }
    if (overheadPercent(10, 11) !== 10) fail("self-test overhead calculation failed.");
    const boundedHeap = heapGrowth([10_000_000, 10_500_000, 10_750_000], 100);
    const growingHeap = heapGrowth([10_000_000, 11_000_000, 12_000_000, 13_000_000], 100);
    if (boundedHeap.unbounded || !growingHeap.unbounded) fail("self-test heap-growth calculation failed.");
    if (connections.length !== 2) fail(`self-test expected 2 persistent connections, got ${connections.length}.`);
    for (const [index, connection] of connections.entries()) {
      const healthCount = connection.requests.filter((request) => request.type === "health").length;
      const screenshotCount = connection.requests.filter((request) => request.type === "get_screenshot").length;
      if (healthCount !== 1) fail(`self-test connection ${index} sent ${healthCount} health requests.`);
      if (screenshotCount !== 7) fail(`self-test connection ${index} sent ${screenshotCount} screenshots.`);
    }
    if (requests.some((request) => request.type !== "health" && request.type !== "get_screenshot")) {
      fail("self-test observed a mutation or lifecycle request.");
    }
    if (result.payload.canonical_inflation_percent !== 0 || result.failures.some((failure) => failure.includes("payload"))) {
      fail("self-test payload parity calculation failed.");
    }
    console.log("screenshot benchmark self-test: PASS (percentiles, two persistent connections, one health each, exact WebP parity)");
  } finally {
    for (const socket of sockets) socket.destroy();
    await new Promise((resolve) => server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  let exitCode = 0;
  try {
    const options = parseArguments(process.argv.slice(2));
    if (options.selfTest) await runSelfTest();
    else renderResult(await runBenchmark(options));
  } catch (error) {
    console.error(`screenshot benchmark: FAIL: ${error instanceof Error ? error.message : String(error)}`);
    exitCode = 1;
  }
  process.exit(exitCode);
}
