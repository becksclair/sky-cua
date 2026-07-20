#!/usr/bin/env node

import { createHash, randomUUID } from "node:crypto";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

import { sky } from "../dist/index.js";

const HEALTH_CAPABILITIES = [
  "action.held_key",
  "action.post_action_sleep_ms",
  "linux.activate_window",
  "linux.click",
  "linux.click.button",
  "linux.click.count",
  "linux.drag",
  "linux.get_screenshot",
  "linux.move",
  "linux.press_key",
  "linux.scroll",
  "linux.scroll.direction",
  "linux.scroll.origin",
  "linux.scroll.pixels",
  "linux.type_text",
  "screen.cursor_size",
  "screenshot.webp",
  "transport.max_frame_64_mib",
  "transport.ndjson",
  "turn.cancel"
];

function usage() {
  return [
    "Usage:",
    "  sky-cua-acceptance --dry-run [--output FILE]",
    "  sky-cua-acceptance --live --window-id ID --x N --y N --drag-to-x N --drag-to-y N --text TEXT [options]",
    "",
    "Live options:",
    "  --socket PATH       Override SKY_CUA_SERVICE_SOCKET_PATH.",
    "  --key KEY           Safe keyboard probe (default: Shift).",
    "  --scroll-pixels N   Scroll distance (default: 40).",
    "  --session-id ID     Request metadata session id.",
    "  --turn-id ID        Request metadata turn id.",
    "  --output FILE       Also write the structured JSON evidence to FILE."
  ].join("\n");
}

function parseNumber(value, name) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    throw new Error(`${name} must be a finite number.`);
  }
  return parsed;
}

function parseArguments(argv) {
  const options = { mode: "dry-run", key: "Shift", scrollPixels: 40 };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") {
      options.help = true;
      continue;
    }
    if (argument === "--dry-run") {
      options.mode = "dry-run";
      continue;
    }
    if (argument === "--live") {
      options.mode = "live";
      continue;
    }
    const value = argv[index + 1];
    if (value === undefined) {
      throw new Error(`${argument} requires a value.`);
    }
    index += 1;
    switch (argument) {
      case "--socket": options.socket = value; break;
      case "--window-id": options.windowId = value; break;
      case "--x": options.x = parseNumber(value, "--x"); break;
      case "--y": options.y = parseNumber(value, "--y"); break;
      case "--drag-to-x": options.dragToX = parseNumber(value, "--drag-to-x"); break;
      case "--drag-to-y": options.dragToY = parseNumber(value, "--drag-to-y"); break;
      case "--text": options.text = value; break;
      case "--key": options.key = value; break;
      case "--scroll-pixels": options.scrollPixels = parseNumber(value, "--scroll-pixels"); break;
      case "--session-id": options.sessionId = value; break;
      case "--turn-id": options.turnId = value; break;
      case "--output": options.output = resolve(value); break;
      default: throw new Error(`Unknown argument ${argument}.`);
    }
  }
  if (options.mode === "live") {
    for (const [name, value] of [
      ["--window-id", options.windowId],
      ["--x", options.x],
      ["--y", options.y],
      ["--drag-to-x", options.dragToX],
      ["--drag-to-y", options.dragToY],
      ["--text", options.text]
    ]) {
      if (value === undefined || value === "") {
        throw new Error(`Live acceptance requires ${name}.`);
      }
    }
  }
  if (!Number.isInteger(options.scrollPixels) || options.scrollPixels < 1 || options.scrollPixels > 10_000) {
    throw new Error("--scroll-pixels must be an integer between 1 and 10000.");
  }
  if (typeof options.key !== "string" || options.key.length === 0) {
    throw new Error("--key must be non-empty.");
  }
  return options;
}

async function startDryFixture() {
  const directory = await mkdtemp(join(tmpdir(), "sky-cua-actions-fixture-"));
  const socketPath = join(directory, "service.sock");
  const requests = [];
  const sockets = new Set();
  const screenshotBytes = Buffer.from("RIFF\u0004\u0000\u0000\u0000WEBP", "binary");
  const server = createServer((socket) => {
    sockets.add(socket);
    socket.once("close", () => sockets.delete(socket));
    let buffer = "";
    socket.on("data", (chunk) => {
      buffer += Buffer.from(chunk).toString("utf8");
      while (buffer.includes("\n")) {
        const newline = buffer.indexOf("\n");
        const request = JSON.parse(buffer.slice(0, newline));
        buffer = buffer.slice(newline + 1);
        requests.push(request);
        let response;
        if (request.type === "health") {
          response = {
            type: "health",
            ok: true,
            protocol_version: 1,
            service_version: "0.1.0",
            capabilities: HEALTH_CAPABILITIES,
            service_socket: socketPath
          };
        } else if (request.type === "get_screenshot") {
          response = {
            type: "get_screenshot",
            ok: true,
            screenshots: [{
              filepath: "/tmp/sky-cua-actions-fixture.webp",
              bytes_base64: screenshotBytes.toString("base64"),
              mime_type: "image/webp",
              width: 1,
              height: 1
            }]
          };
        } else if (request.type === "activate_window") {
          response = {
            type: "activate_window",
            outcome: { success: true, message: "fixture window activated", code: "Activated", diagnostics: [] }
          };
        } else {
          response = {
            type: request.type,
            ok: true,
            session_id: request.context.session_id,
            turn_id: request.context.turn_id
          };
        }
        socket.write(`${JSON.stringify(response)}\n`);
      }
    });
  });
  await new Promise((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(socketPath, resolvePromise);
  });
  return {
    socketPath,
    requests,
    async close() {
      for (const socket of sockets) socket.destroy();
      await new Promise((resolvePromise) => server.close(resolvePromise));
      await rm(directory, { recursive: true, force: true });
    }
  };
}

function errorEvidence(error) {
  return {
    name: error instanceof Error ? error.name : "Error",
    message: error instanceof Error ? error.message : String(error),
    ...(error && typeof error === "object" && "code" in error ? { code: error.code } : {}),
    ...(error && typeof error === "object" && "retry" in error ? { retry: error.retry } : {})
  };
}

async function runStep(steps, name, operation) {
  const started = performance.now();
  try {
    const details = await operation();
    steps.push({ name, ok: true, duration_ms: Number((performance.now() - started).toFixed(3)), ...(details === undefined ? {} : { details }) });
    return details;
  } catch (error) {
    steps.push({ name, ok: false, duration_ms: Number((performance.now() - started).toFixed(3)), error: errorEvidence(error) });
    throw error;
  }
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options.help) {
    console.log(usage());
    return;
  }
  if (Number(process.versions.node.split(".")[0]) < 24) {
    throw new Error("sky-cua action acceptance requires Node 24 or newer.");
  }

  const fixture = options.mode === "dry-run" ? await startDryFixture() : undefined;
  const originalSocket = process.env.SKY_CUA_SERVICE_SOCKET_PATH;
  const socketPath = fixture?.socketPath ?? options.socket ?? originalSocket;
  if (socketPath === undefined || socketPath.length === 0) {
    throw new Error("Live acceptance requires --socket or SKY_CUA_SERVICE_SOCKET_PATH.");
  }
  process.env.SKY_CUA_SERVICE_SOCKET_PATH = socketPath;

  const responseMetadata = [];
  const emittedImages = [];
  const suppliedBridge = globalThis.nodeRepl;
  if (suppliedBridge === undefined) {
    const runId = randomUUID();
    globalThis.nodeRepl = {
      requestMeta: {
        session_id: options.sessionId ?? `sky-cua-acceptance-${runId}`,
        turn_id: options.turnId ?? `actions-${runId}`
      },
      setResponseMeta(meta) {
        responseMetadata.push(meta);
      },
      emitImage(dataUrl) {
        emittedImages.push({ mime_type: dataUrl.slice(5, dataUrl.indexOf(";")), characters: dataUrl.length });
      }
    };
  }

  const point = options.mode === "dry-run" ? { x: 10, y: 20, dragToX: 30, dragToY: 40 } : options;
  const text = options.mode === "dry-run" ? "sky-cua fixture acceptance" : options.text;
  const windowId = options.mode === "dry-run" ? "fixture-window" : options.windowId;
  const steps = [];
  const startedAt = new Date().toISOString();
  let failure;
  let captureForEmission;
  try {
    await runStep(steps, "activate_window", async () => {
      const outcome = await sky.activate_window({ window_id: windowId });
      if (!outcome.success) throw new Error(`${outcome.code}: ${outcome.message}`);
      return outcome;
    });
    const screenshots = await runStep(steps, "get_screenshot", async () => {
      const captures = await sky.get_screenshot();
      if (captures.length === 0) throw new Error("get_screenshot returned no captures.");
      captureForEmission = captures[0];
      return captures.map((capture) => ({
        filepath: capture.filepath,
        bytes: capture.bytes.length,
        sha256: createHash("sha256").update(capture.bytes).digest("hex"),
        mime_type: "image/webp"
      }));
    });
    if (captureForEmission && typeof globalThis.nodeRepl?.emitImage === "function") {
      await runStep(steps, "emit_screenshot", async () => {
        await globalThis.nodeRepl.emitImage(captureForEmission.data_url);
        return { emitted: true, source: screenshots[0]?.filepath };
      });
    }
    await runStep(steps, "move", () => sky.move({ x: point.x, y: point.y }));
    await runStep(steps, "click", () => sky.click({ x: point.x, y: point.y }));
    await runStep(steps, "drag", () => sky.drag({ from_x: point.x, from_y: point.y, to_x: point.dragToX, to_y: point.dragToY }));
    await runStep(steps, "scroll", () => sky.scroll({ direction: "down", pixels: options.scrollPixels, x: point.dragToX, y: point.dragToY }));
    await runStep(steps, "press_key", () => sky.press_key({ key: options.key }));
    await runStep(steps, "type_text", () => sky.type_text({ text }));
  } catch (error) {
    failure = errorEvidence(error);
  } finally {
    if (suppliedBridge === undefined) globalThis.nodeRepl = undefined;
    if (originalSocket === undefined) delete process.env.SKY_CUA_SERVICE_SOCKET_PATH;
    else process.env.SKY_CUA_SERVICE_SOCKET_PATH = originalSocket;
    await fixture?.close();
  }

  const evidence = {
    schema_version: 1,
    package: "@heliasar/sky-cua",
    node_version: process.versions.node,
    mode: options.mode,
    started_at: startedAt,
    completed_at: new Date().toISOString(),
    socket_path: socketPath,
    service_lifecycle_control: false,
    webp_default: true,
    steps,
    response_metadata: responseMetadata,
    emitted_images: emittedImages,
    ...(fixture === undefined ? {} : { fixture_request_types: fixture.requests.map((request) => request.type) }),
    ok: failure === undefined,
    ...(failure === undefined ? {} : { failure })
  };
  const encoded = `${JSON.stringify(evidence, null, 2)}\n`;
  if (options.output !== undefined) {
    await mkdir(dirname(options.output), { recursive: true });
    await writeFile(options.output, encoded, { encoding: "utf8", mode: 0o600 });
  }
  process.stdout.write(encoded);
  if (failure !== undefined) process.exitCode = 1;
}

main().catch((error) => {
  process.stderr.write(`${JSON.stringify({ ok: false, failure: errorEvidence(error) }, null, 2)}\n`);
  process.exitCode = 1;
});
