import { createServer, type Server } from "node:net";
import { unlinkSync, writeFileSync } from "node:fs";
import { Buffer } from "node:buffer";

import { describe, expect, test } from "bun:test";

import { sky } from "../src/index";
import { resolveSkyConfig } from "../src/config";
import { requestContext, readRequestMetadata, withSuspendedTimeout } from "../src/context";
import { SkyCuaError } from "../src/errors";
import {
  HEALTH_CAPABILITIES,
  MAX_FRAME_BYTES,
  type ActionResponse,
  type ServiceRequest
} from "../src/protocol/generated";
import { createLinuxClient } from "../src/targets/linux";
import { createMacPlaceholder, macOwnKeys } from "../src/targets/mac-placeholder";
import { NdjsonConnection, SkyCuaTransport } from "../src/transport/ndjson-client";
import { FakeDaemon } from "./fake-daemon/fake-daemon";

const ROOT_SOCKET = `/tmp/sky-cua-js-contract-${Date.now()}.sock`;
const TEST_CONFIG = `/tmp/sky-cua-js-config-${Date.now()}.json`;

function setEnvironment(values: Record<string, string | undefined>): () => void {
  const previous: Record<string, string | undefined> = {};
  for (const [key, value] of Object.entries(values)) {
    previous[key] = process.env[key];
    if (value === undefined) {
      delete process.env[key];
    } else {
      process.env[key] = value;
    }
  }
  return () => {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  };
}

function expectSkyCode(error: unknown, code: string): void {
  expect(error instanceof SkyCuaError).toBe(true);
  expect((error as SkyCuaError).code).toBe(code);
}

async function expectReject(operation: Promise<unknown>, code: string): Promise<SkyCuaError> {
  try {
    await operation;
    throw new Error(`Expected ${code} rejection.`);
  } catch (error) {
    expectSkyCode(error, code);
    return error as SkyCuaError;
  }
}

function actionResponse(
  request: Exclude<ServiceRequest, { type: "health" | "get_screenshot" | "cancel_turn" }>
): ActionResponse {
  return {
    type: request.type,
    ok: true,
    session_id: request.context.session_id,
    turn_id: request.context.turn_id
  };
}

describe("@heliasar/sky-cua public contract", () => {
  test("imports without service I/O and exposes exact Linux own keys", () => {
    const restore = setEnvironment({
      OAI_SKY_CONFIG_PATH: undefined,
      SKY_CUA_JS_CONFIG_PATH: undefined,
      SKY_CUA_SERVICE_SOCKET_PATH: ROOT_SOCKET
    });
    try {
      expect(Object.keys(sky)).toEqual([
        "click",
        "drag",
        "get_screenshot",
        "move",
        "press_key",
        "scroll",
        "type_text"
      ]);
      expect(typeof sky.click).toBe("function");
    } finally {
      restore();
    }
  });

  test("freezes the resolved config and client together on first enumeration", async () => {
    const daemon = new FakeDaemon(ROOT_SOCKET, {
      onRequest: ({ request }) => {
        if (request.type === "click") {
          return actionResponse(request);
        }
        return undefined;
      }
    });
    await daemon.start();
    writeFileSync(TEST_CONFIG, JSON.stringify({ target: "mac" }));
    const restore = setEnvironment({
      OAI_SKY_CONFIG_PATH: TEST_CONFIG,
      SKY_CUA_SERVICE_SOCKET_PATH: `${ROOT_SOCKET}.changed`
    });
    globalThis.nodeRepl = { requestMeta: { session_id: "frozen-s", turn_id: "frozen-t" } };
    try {
      expect(Object.keys(sky).includes("click")).toBe(true);
      await sky.click({ x: 1, y: 2 });
      expect(daemon.requests.map((request) => request.type)).toEqual(["health", "click"]);
    } finally {
      globalThis.nodeRepl = undefined;
      restore();
      unlinkSync(TEST_CONFIG);
      await daemon.close();
    }
  });

  test("configuration precedence is OAI, then first-party alias, then platform", () => {
    writeFileSync(TEST_CONFIG, JSON.stringify({ target: "mac", post_action_sleep_ms: 0 }));
    const restore = setEnvironment({
      OAI_SKY_CONFIG_PATH: TEST_CONFIG,
      SKY_CUA_JS_CONFIG_PATH: "/does/not/exist",
      SKY_CUA_SERVICE_SOCKET_PATH: ROOT_SOCKET
    });
    try {
      expect(resolveSkyConfig().target).toBe("mac");
    } finally {
      restore();
      unlinkSync(TEST_CONFIG);
    }
  });

  test("rejects unknown configuration keys instead of ignoring typos", () => {
    writeFileSync(TEST_CONFIG, JSON.stringify({ target: "linux", post_action_sleep_mz: 0 }));
    const restore = setEnvironment({ OAI_SKY_CONFIG_PATH: TEST_CONFIG });
    try {
      let error: unknown;
      try {
        resolveSkyConfig();
      } catch (cause) {
        error = cause;
      }
      expect(error instanceof Error ? error.message : "").toBe(
        "Sky configuration contains unsupported field post_action_sleep_mz."
      );
    } finally {
      restore();
      unlinkSync(TEST_CONFIG);
    }
  });

  test("uses frozen option defaults and Rust-equivalent endpoint precedence", () => {
    const restore = setEnvironment({
      OAI_SKY_CONFIG_PATH: undefined,
      SKY_CUA_JS_CONFIG_PATH: undefined,
      SKY_CUA_SERVICE_SOCKET_PATH: "/tmp/explicit-service.sock",
      XDG_RUNTIME_DIR: "/tmp/runtime",
      XDG_CACHE_HOME: "/tmp/cache",
      HOME: "/tmp/home"
    });
    try {
      const config = resolveSkyConfig();
      expect(config.post_action_sleep_ms).toBe(100);
      expect(config.mouse_size_px).toBe(12);
      expect(config.target).toBe("linux");
    } finally {
      restore();
    }
  });

  test("Darwin placeholder owns the frozen keys and never needs a socket", async () => {
    const client = createMacPlaceholder();
    expect(Object.keys(client)).toEqual([...macOwnKeys()]);
    await expectReject(client.click({}), "SKY_CUA_TARGET_UNAVAILABLE");
  });
});

describe("nodeRepl compatibility seam", () => {
  test("deep-freezes request metadata and preserves session/turn fields", async () => {
    let suspendCount = 0;
    let responseMeta: Record<string, unknown> | undefined;
    globalThis.nodeRepl = {
      requestMeta: {
        "x-codex-turn-metadata": {
          session_id: "session-1",
          turn_id: "turn-1",
          deadline_ms: 12_345
        },
        nested: { value: true }
      },
      withSuspendedTimeout: async <T>(operation: () => Promise<T>): Promise<T> => {
        suspendCount += 1;
        return operation();
      },
      setResponseMeta(meta) {
        responseMeta = meta;
      },
      emitImage() {
        throw new Error("emitImage must not be called by sky-cua");
      }
    };
    const metadata = readRequestMetadata();
    expect(Object.isFrozen(metadata)).toBe(true);
    expect(Object.isFrozen(metadata.nested)).toBe(true);
    expect(requestContext()).toEqual({
      session_id: "session-1",
      turn_id: "turn-1",
      deadline_ms: 12_345
    });
    let mutationFailed = false;
    try {
      (metadata as { session_id: string }).session_id = "changed";
    } catch {
      mutationFailed = true;
    }
    expect(mutationFailed).toBe(true);
    await withSuspendedTimeout(async () => undefined);
    expect(suspendCount).toBe(1);
    globalThis.nodeRepl.setResponseMeta?.({ "codex/toolSurface": { kind: "computerUse" } });
    responseMeta = globalThis.nodeRepl === undefined ? undefined : responseMeta;
    expect(responseMeta?.["codex/toolSurface"]).toEqual({ kind: "computerUse" });
    globalThis.nodeRepl = undefined;
  });

  test("falls back to NODE_REPL_REQUEST_META when requestMeta is null", () => {
    const restore = setEnvironment({
      NODE_REPL_REQUEST_META: JSON.stringify({ session_id: "env-session", turn_id: "env-turn" })
    });
    globalThis.nodeRepl = { requestMeta: null };
    try {
      expect(requestContext()).toEqual({ session_id: "env-session", turn_id: "env-turn" });
    } finally {
      globalThis.nodeRepl = undefined;
      restore();
    }
  });

  test("preserves only the documented deadline_ms metadata field and validates its range", () => {
    globalThis.nodeRepl = {
      requestMeta: { session_id: "s", turn_id: "t", deadline_ms: 1, deadline: 7 }
    };
    expect(requestContext()).toEqual({ session_id: "s", turn_id: "t", deadline_ms: 1 });
    globalThis.nodeRepl = {
      requestMeta: { session_id: "s", turn_id: "t", deadline_ms: 30_000 }
    };
    expect(requestContext()).toEqual({ session_id: "s", turn_id: "t", deadline_ms: 30_000 });
    for (const deadline_ms of [0, 30_001, 1.5, "20"]) {
      globalThis.nodeRepl = { requestMeta: { session_id: "s", turn_id: "t", deadline_ms } };
      try {
        requestContext();
        throw new Error("Expected invalid deadline_ms metadata.");
      } catch (error) {
        expectSkyCode(error, "SKY_CUA_INVALID_CONTEXT");
      }
    }
    globalThis.nodeRepl = undefined;
  });
});

describe("fake daemon transport", () => {
  test("cancels a public mutation at the requestMeta deadline", async () => {
    const path = `${ROOT_SOCKET}.public-deadline`;
    let cancelSeen = false;
    const daemon = new FakeDaemon(path, {
      onRequest: async ({ request }) => {
        if (request.type === "click") {
          await new Promise((resolve) => setTimeout(resolve, 200));
          return actionResponse(request);
        }
        if (request.type === "cancel_turn") {
          cancelSeen = true;
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    globalThis.nodeRepl = {
      requestMeta: { session_id: "deadline-s", turn_id: "deadline-t", deadline_ms: 20 }
    };
    try {
      const client = createLinuxClient({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      await expectReject(client.click({ x: 1, y: 2 }), "SKY_CUA_DEADLINE_EXCEEDED");
      const click = daemon.requests.find((request) => request.type === "click");
      expect(click?.type === "click" ? click.context.deadline_ms : undefined).toBe(20);
      expect(cancelSeen).toBe(true);
    } finally {
      globalThis.nodeRepl = undefined;
      restore();
      await daemon.close();
    }
  });

  test("leaves post-action pacing entirely to the service", async () => {
    const path = `${ROOT_SOCKET}.service-pacing`;
    let responseReadyAt = 0;
    let requestedPacing: number | undefined;
    const daemon = new FakeDaemon(path, {
      onRequest: async ({ request }) => {
        if (request.type === "click") {
          requestedPacing = request.post_action_sleep_ms;
          await new Promise((resolve) => setTimeout(resolve, requestedPacing));
          responseReadyAt = Date.now();
          return actionResponse(request);
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    globalThis.nodeRepl = { requestMeta: { session_id: "pace-s", turn_id: "pace-t" } };
    try {
      const client = createLinuxClient({ target: "linux", post_action_sleep_ms: 100, mouse_size_px: 0 });
      await client.click({ x: 1, y: 2 });
      expect(requestedPacing).toBe(100);
      expect(Date.now() - responseReadyAt < 75).toBe(true);
    } finally {
      globalThis.nodeRepl = undefined;
      restore();
      await daemon.close();
    }
  });

  test("fragments responses, serializes action requests, and preserves WebP arrays", async () => {
    const path = `${ROOT_SOCKET}.fragmented`;
    let active = 0;
    let maximum = 0;
    const daemon = new FakeDaemon(path, {
      fragmentResponses: true,
      onRequest: async ({ request }) => {
        if (request.type === "click" || request.type === "move") {
          active += 1;
          maximum = Math.max(maximum, active);
          await new Promise((resolve) => setTimeout(resolve, request.type === "click" ? 20 : 0));
          active -= 1;
          return actionResponse(request);
        }
        if (request.type === "get_screenshot") {
          const bytes = Buffer.from("RIFFWEBP", "utf8").toString("base64");
          return {
            type: "get_screenshot",
            ok: true,
            screenshots: [{
              filepath: "/tmp/fake.webp",
              bytes_base64: bytes,
              mime_type: "image/webp",
              width: 20,
              height: 10
            }]
          };
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    let emittedImages = 0;
    let responseMeta: Record<string, unknown> | undefined;
    globalThis.nodeRepl = {
      requestMeta: { session_id: "s", turn_id: "t" },
      emitImage() {
        emittedImages += 1;
      },
      setResponseMeta(meta) {
        responseMeta = meta;
      }
    };
    try {
      const client = createLinuxClient({
        target: "linux",
        post_action_sleep_ms: 0,
        mouse_size_px: 0
      });
      await Promise.all([
        client.click({ x: 1, y: 2, mouse_button: "middle", click_count: 2, key: "Shift" }),
        client.move({ x: 3, y: 4 })
      ]);
      const screenshots = await client.get_screenshot();
      expect(screenshots[0]?.bytes).toEqual(Uint8Array.from(Buffer.from("RIFFWEBP", "utf8")));
      expect(screenshots[0]?.data_url).toBe(`data:image/webp;base64,${Buffer.from("RIFFWEBP", "utf8").toString("base64")}`);
      expect(maximum).toBe(1);
      expect(emittedImages).toBe(0);
      expect(responseMeta?.["codex/toolSurface"]).toEqual({ app: null, kind: "computerUse" });
      expect(daemon.requests.map((request) => request.type)).toEqual([
        "health",
        "click",
        "move",
        "get_screenshot"
      ]);
      expect(daemon.connectionIds.length).toBe(1);
      const click = daemon.requests[1];
      expect(click?.type).toBe("click");
      if (click?.type === "click") {
        expect(click.context).toEqual({ session_id: "s", turn_id: "t" });
        expect(click.mouse_button).toBe("middle");
        expect(click.click_count).toBe(2);
      }
    } finally {
      globalThis.nodeRepl = undefined;
      restore();
      await daemon.close();
    }
  });

  test("requires metadata before opening a socket", async () => {
    const path = `${ROOT_SOCKET}.metadata`;
    const daemon = new FakeDaemon(path);
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    globalThis.nodeRepl = { requestMeta: {} };
    try {
      const client = createLinuxClient({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      await expectReject(client.click({ x: 1, y: 1 }), "SKY_CUA_INVALID_CONTEXT");
      expect(daemon.requests).toEqual([]);
    } finally {
      globalThis.nodeRepl = undefined;
      restore();
      await daemon.close();
    }
  });

  test("times out through separate CancelTurn control connection", async () => {
    const path = `${ROOT_SOCKET}.cancel`;
    let cancelSeen = false;
    const daemon = new FakeDaemon(path, {
      onRequest: async ({ request }) => {
        if (request.type === "click") {
          await new Promise((resolve) => setTimeout(resolve, 200));
          return actionResponse(request);
        }
        if (request.type === "cancel_turn") {
          cancelSeen = true;
          return {
            type: "cancel_turn",
            ok: true,
            session_id: request.session_id,
            turn_id: request.turn_id,
            status: "cancel_requested"
          };
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const request: ServiceRequest = {
        type: "click",
        context: { session_id: "s", turn_id: "t", deadline_ms: 20 },
        x: 1,
        y: 1
      };
      await expectReject(
        transport.request(request, { context: request.context, requiredCapabilities: [...HEALTH_CAPABILITIES] }),
        "SKY_CUA_DEADLINE_EXCEEDED"
      );
      expect(cancelSeen).toBe(true);
      expect(daemon.requests.map((item) => item.type)).toEqual(["health", "click", "cancel_turn"]);
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("cancels through the control connection without retrying the action", async () => {
    const path = `${ROOT_SOCKET}.abort`;
    const daemon = new FakeDaemon(path, {
      onRequest: async ({ request }) => {
        if (request.type === "click") {
          await new Promise((resolve) => setTimeout(resolve, 200));
          return actionResponse(request);
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const controller = new AbortController();
      const request: ServiceRequest = {
        type: "click",
        context: { session_id: "s", turn_id: "t", deadline_ms: 500 },
        x: 1,
        y: 1
      };
      const pending = transport.request(request, {
        context: request.context,
        requiredCapabilities: ["linux.click"],
        signal: controller.signal
      });
      setTimeout(() => controller.abort(), 20);
      await expectReject(pending, "SKY_CUA_TURN_CANCELLED");
      expect(daemon.requests.filter((item) => item.type === "click").length).toBe(1);
      expect(daemon.requests.filter((item) => item.type === "cancel_turn").length).toBe(1);
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("starts a queued deadline only after the request owns the action lane", async () => {
    const path = `${ROOT_SOCKET}.queued-deadline`;
    const daemon = new FakeDaemon(path, {
      onRequest: async ({ request }) => {
        if (request.type === "click") {
          await new Promise((resolve) => setTimeout(resolve, 80));
          return actionResponse(request);
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const first: ServiceRequest = {
        type: "click",
        context: { session_id: "s", turn_id: "first", deadline_ms: 500 },
        x: 1,
        y: 1
      };
      const second: ServiceRequest = {
        type: "move",
        context: { session_id: "s", turn_id: "second", deadline_ms: 20 },
        x: 2,
        y: 2
      };
      await Promise.all([
        transport.request(first, { context: first.context, requiredCapabilities: ["linux.click"] }),
        transport.request(second, { context: second.context, requiredCapabilities: ["linux.move"] })
      ]);
      expect(daemon.requests.map((item) => item.type)).toEqual(["health", "click", "move"]);
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("never retries a mutation after the response becomes ambiguous", async () => {
    const path = `${ROOT_SOCKET}.unknown`;
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request, socket }) => {
        if (request.type === "click") {
          socket.destroy();
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const request: ServiceRequest = {
        type: "click",
        context: { session_id: "s", turn_id: "t" },
        x: 1,
        y: 1
      };
      await expectReject(
        transport.request(request, { context: request.context, requiredCapabilities: ["linux.click"] }),
        "SKY_CUA_ACTION_OUTCOME_UNKNOWN"
      );
      expect(daemon.requests.filter((item) => item.type === "click").length).toBe(1);
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("does not retry an idempotent move after a proven post-write disconnect", async () => {
    const path = `${ROOT_SOCKET}.reconnect`;
    let moveAttempts = 0;
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request, socket }) => {
        if (request.type === "move") {
          moveAttempts += 1;
          if (moveAttempts === 1) {
            socket.destroy();
          }
          return actionResponse(request);
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const request: ServiceRequest = {
        type: "move",
        context: { session_id: "s", turn_id: "t" },
        x: 1,
        y: 1
      };
      const error = await expectReject(
        transport.request(request, {
          context: request.context,
          requiredCapabilities: ["linux.move"]
        }),
        "SKY_CUA_SERVICE_DISCONNECTED"
      );
      expect(error.retry).toBe("never");
      expect(daemon.requests.map((item) => item.type)).toEqual(["health", "move"]);
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("retries once after a proven pre-write failure", async () => {
    const path = `${ROOT_SOCKET}.pre-write`;
    let completedActions = 0;
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request, socket }) => {
        if (request.type === "move") {
          completedActions += 1;
          setTimeout(() => socket.destroy(), 5);
          return actionResponse(request);
        }
        if (request.type === "click") {
          completedActions += 1;
          return actionResponse(request);
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const first: ServiceRequest = {
        type: "move",
        context: { session_id: "s", turn_id: "first" },
        x: 1,
        y: 1
      };
      await transport.request(first, { context: first.context, requiredCapabilities: ["linux.move"] });
      await new Promise((resolve) => setTimeout(resolve, 20));
      const second: ServiceRequest = {
        type: "click",
        context: { session_id: "s", turn_id: "second" },
        x: 2,
        y: 2
      };
      await transport.request(second, { context: second.context, requiredCapabilities: ["linux.click"] });
      expect(completedActions).toBe(2);
      expect(daemon.requests.map((item) => item.type)).toEqual([
        "health",
        "move",
        "health",
        "click"
      ]);
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("retries once only when the daemon explicitly marks reconnect safe", async () => {
    const path = `${ROOT_SOCKET}.safe-retry`;
    let attempts = 0;
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request }) => {
        if (request.type !== "move") {
          return undefined;
        }
        attempts += 1;
        if (attempts === 1) {
          return {
            type: "error",
            ok: false,
            code: "SKY_CUA_SERVICE_DISCONNECTED",
            message: "Reconnect and retry.",
            retry: "safe_after_reconnect"
          };
        }
        return actionResponse(request);
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const request: ServiceRequest = {
        type: "move",
        context: { session_id: "s", turn_id: "t" },
        x: 1,
        y: 1
      };
      await transport.request(request, { context: request.context, requiredCapabilities: ["linux.move"] });
      expect(attempts).toBe(2);
      expect(daemon.requests.map((item) => item.type)).toEqual([
        "health",
        "move",
        "health",
        "move"
      ]);
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("does not leak an unhandled rejection when CancelTurn fails", async () => {
    const path = `${ROOT_SOCKET}.cancel-failure`;
    const unhandled: unknown[] = [];
    const onUnhandled = (error: unknown): void => {
      unhandled.push(error);
    };
    process.on("unhandledRejection", onUnhandled);
    const daemon = new FakeDaemon(path, {
      onRequest: async ({ request, socket }) => {
        if (request.type === "click") {
          await new Promise((resolve) => setTimeout(resolve, 200));
          return actionResponse(request);
        }
        if (request.type === "cancel_turn") {
          socket.destroy();
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const request: ServiceRequest = {
        type: "click",
        context: { session_id: "s", turn_id: "t", deadline_ms: 20 },
        x: 1,
        y: 1
      };
      await expectReject(
        transport.request(request, { context: request.context, requiredCapabilities: ["linux.click"] }),
        "SKY_CUA_SERVICE_DISCONNECTED"
      );
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(unhandled).toEqual([]);
      expect(daemon.requests.filter((item) => item.type === "click").length).toBe(1);
      transport.close();
    } finally {
      process.off("unhandledRejection", onUnhandled);
      restore();
      await daemon.close();
    }
  });

  test("rejects a mismatched action response", async () => {
    const path = `${ROOT_SOCKET}.mismatched-response`;
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request }) => request.type === "click"
        ? { ...actionResponse(request), turn_id: "wrong-turn" }
        : undefined
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const request: ServiceRequest = {
        type: "click",
        context: { session_id: "s", turn_id: "t" },
        x: 1,
        y: 1
      };
      await expectReject(
        transport.request(request, { context: request.context, requiredCapabilities: ["linux.click"] }),
        "SKY_CUA_INVALID_REQUEST"
      );
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("maps a terminal health connection close to service restart required", async () => {
    const path = `${ROOT_SOCKET}.health-close`;
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request, socket }) => {
        if (request.type === "health") {
          socket.destroy();
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    try {
      const transport = new SkyCuaTransport({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      const request: ServiceRequest = {
        type: "move",
        context: { session_id: "s", turn_id: "t" },
        x: 1,
        y: 1
      };
      await expectReject(
        transport.request(request, { context: request.context, requiredCapabilities: ["linux.move"] }),
        "SKY_CUA_SERVICE_RESTART_REQUIRED"
      );
      expect(daemon.requests.map((item) => item.type)).toEqual(["health"]);
      transport.close();
    } finally {
      restore();
      await daemon.close();
    }
  });

  test("requires turn.cancel capability for every mutation", async () => {
    const path = `${ROOT_SOCKET}.cancel-capability`;
    const daemon = new FakeDaemon(path, {
      defaultHealth: {
        capabilities: HEALTH_CAPABILITIES.filter((capability) => capability !== "turn.cancel")
      }
    });
    await daemon.start();
    const restore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: path });
    globalThis.nodeRepl = { requestMeta: { session_id: "s", turn_id: "t" } };
    try {
      const client = createLinuxClient({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      await expectReject(client.move({ x: 1, y: 1 }), "SKY_CUA_CAPABILITY_MISSING");
      expect(daemon.requests.map((item) => item.type)).toEqual(["health"]);
    } finally {
      globalThis.nodeRepl = undefined;
      restore();
      await daemon.close();
    }
  });

  test("maps stopped service, unsupported protocol, and missing capabilities stably", async () => {
    const stoppedPath = `${ROOT_SOCKET}.stopped`;
    const stoppedRestore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: stoppedPath });
    globalThis.nodeRepl = { requestMeta: { session_id: "s", turn_id: "t" } };
    try {
      const stopped = createLinuxClient({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      await expectReject(stopped.click({ x: 1, y: 1 }), "SKY_CUA_SERVICE_RESTART_REQUIRED");
    } finally {
      stoppedRestore();
    }

    const protocolPath = `${ROOT_SOCKET}.protocol`;
    const protocolDaemon = new FakeDaemon(protocolPath, {
      defaultHealth: { protocol_version: 2 as 1 }
    });
    await protocolDaemon.start();
    const protocolRestore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: protocolPath });
    try {
      const unsupported = createLinuxClient({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      await expectReject(unsupported.click({ x: 1, y: 1 }), "SKY_CUA_PROTOCOL_UNSUPPORTED");
    } finally {
      protocolRestore();
      await protocolDaemon.close();
    }

    const capabilityPath = `${ROOT_SOCKET}.capability`;
    const capabilityDaemon = new FakeDaemon(capabilityPath, {
      defaultHealth: { capabilities: ["linux.click"] }
    });
    await capabilityDaemon.start();
    const capabilityRestore = setEnvironment({ SKY_CUA_SERVICE_SOCKET_PATH: capabilityPath });
    try {
      const missing = createLinuxClient({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
      await expectReject(missing.click({ x: 1, y: 1 }), "SKY_CUA_CAPABILITY_MISSING");
    } finally {
      capabilityRestore();
      globalThis.nodeRepl = undefined;
      await capabilityDaemon.close();
    }
  });

  test("rejects an NDJSON request whose newline-inclusive frame exceeds 64 MiB", async () => {
    const path = `${ROOT_SOCKET}.limit`;
    let server: Server | undefined;
    try {
      unlinkSync(path);
    } catch {
      // Nothing to remove.
    }
    server = createServer(() => undefined);
    await new Promise<void>((resolve, reject) => {
      server?.once("error", reject);
      server?.listen(path, resolve);
    });
    const connection = await NdjsonConnection.connect(path);
    try {
      const oversized: ServiceRequest = {
        type: "type_text",
        context: { session_id: "s", turn_id: "t" },
        text: "x".repeat(MAX_FRAME_BYTES)
      };
      await expectReject(connection.request(oversized), "SKY_CUA_FRAME_TOO_LARGE");
    } finally {
      connection.close();
      await new Promise<void>((resolve) => server?.close(() => resolve()));
      try {
        unlinkSync(path);
      } catch {
        // Node removes a Unix socket path when the server closes.
      }
    }
  });

  test("rejects an oversized fragmented NDJSON response incrementally", async () => {
    const path = `${ROOT_SOCKET}.response-limit`;
    let server: Server | undefined;
    try {
      unlinkSync(path);
    } catch {
      // Nothing to remove.
    }
    const segment = Buffer.alloc(MAX_FRAME_BYTES / 4);
    segment.fill(0x20);
    server = createServer((socket) => {
      let sent = false;
      socket.on("error", () => undefined);
      socket.on("data", () => {
        if (sent) {
          return;
        }
        sent = true;
        for (let index = 0; index < 4; index += 1) {
          socket.write(segment);
        }
      });
    });
    await new Promise<void>((resolve, reject) => {
      server?.once("error", reject);
      server?.listen(path, resolve);
    });
    const connection = await NdjsonConnection.connect(path);
    try {
      await expectReject(connection.request({ type: "health" }), "SKY_CUA_FRAME_TOO_LARGE");
    } finally {
      connection.close();
      await new Promise<void>((resolve) => server?.close(() => resolve()));
      try {
        unlinkSync(path);
      } catch {
        // Node removes a Unix socket path when the server closes.
      }
    }
  });
});

describe("Node 24 release surface", () => {
  test("built package keeps only the root export and selects Darwin lazily", () => {
    const source = `import { sky } from ${JSON.stringify(`${process.cwd()}/dist/index.js`)}; console.log(JSON.stringify(Object.keys(sky)));`;
    const result = Bun.spawnSync(["node", "--input-type=module", "-e", source], {
      env: { OAI_SKY_CONFIG_PATH: TEST_CONFIG }
    });
    writeFileSync(TEST_CONFIG, JSON.stringify({ target: "darwin" }));
    const rerun = Bun.spawnSync(["node", "--input-type=module", "-e", source], {
      env: { OAI_SKY_CONFIG_PATH: TEST_CONFIG }
    });
    unlinkSync(TEST_CONFIG);
    expect(result.exitCode).toBe(1);
    expect(rerun.exitCode).toBe(0);
    expect(new TextDecoder().decode(rerun.stdout)).toEqual(
      "[\"click\",\"drag\",\"get_app_state\",\"list_apps\",\"perform_secondary_action\",\"press_key\",\"scroll\",\"select_text\",\"set_value\",\"type_text\"]\n"
    );
  });
});
