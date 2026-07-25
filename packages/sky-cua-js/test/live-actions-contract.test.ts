import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, test } from "bun:test";

import { createLinuxClient } from "../src/targets/linux";
import { SkyCuaError } from "../src/errors";
import { HEALTH_CAPABILITIES } from "../src/protocol/generated";
import type { TransportRequest } from "../src/window-action";
import { FakeDaemon } from "./fake-daemon/fake-daemon";

const temporaryDirectories: string[] = [];

function fixturePath(name: string): string {
  const directory = mkdtempSync(join(tmpdir(), `sky-cua-actions-${name}-`));
  temporaryDirectories.push(directory);
  return join(directory, "service.sock");
}

function setSocket(path: string): () => void {
  const previous = process.env.SKY_CUA_SERVICE_SOCKET_PATH;
  process.env.SKY_CUA_SERVICE_SOCKET_PATH = path;
  return () => {
    if (previous === undefined) {
      delete process.env.SKY_CUA_SERVICE_SOCKET_PATH;
    } else {
      process.env.SKY_CUA_SERVICE_SOCKET_PATH = previous;
    }
  };
}

function client() {
  return createLinuxClient({ target: "linux", post_action_sleep_ms: 0, mouse_size_px: 0 });
}

function cleanup(): void {
  globalThis.nodeRepl = undefined;
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
}

async function expectSkyReject(operation: Promise<unknown>, code: string): Promise<void> {
  let error: unknown;
  try {
    await operation;
  } catch (cause) {
    error = cause;
  }
  expect(error instanceof SkyCuaError).toBe(true);
  expect((error as SkyCuaError).code).toBe(code);
}

describe("ordinary public facade live-action contract", () => {
  test("covers screenshot, pointer, keyboard, text, and safe window activation through one service", async () => {
    const path = fixturePath("all");
    const daemon = new FakeDaemon(path);
    await daemon.start();
    const restore = setSocket(path);
    const responseMetadata: Record<string, unknown>[] = [];
    globalThis.nodeRepl = {
      requestMeta: { session_id: "acceptance-session", turn_id: "acceptance-turn" },
      setResponseMeta(meta) {
        responseMetadata.push(meta);
      }
    };
    try {
      const sky = client();
      await sky.get_screenshot();
      await sky.move({ x: 10, y: 20 });
      await sky.click({ x: 10, y: 20 });
      await sky.drag({ from_x: 10, from_y: 20, to_x: 30, to_y: 40 });
      await sky.scroll({ direction: "down", pixels: 80, x: 30, y: 40 });
      await sky.press_key({ key: "Shift" });
      await sky.type_text({ text: "sky-cua acceptance" });
      const outcome = await sky.activate_window({ window_id: "fixture-window" });

      expect(outcome).toEqual({
        success: true,
        message: "window activated",
        code: "Activated",
        diagnostics: []
      });
      expect(daemon.requests.map((request) => request.type)).toEqual([
        "health",
        "get_screenshot",
        "move",
        "click",
        "drag",
        "scroll",
        "press_key",
        "type_text",
        "activate_window"
      ]);
      const mutations = daemon.requests.filter((request) =>
        !["health", "get_screenshot"].includes(request.type)
      );
      expect(mutations.every((request) =>
        "context" in request &&
        request.context?.session_id === "acceptance-session" &&
        request.context?.turn_id === "acceptance-turn"
      )).toBe(true);
      expect(daemon.requests.at(-1)).toEqual({
        type: "activate_window",
        target: { window_id: "fixture-window" },
        context: { session_id: "acceptance-session", turn_id: "acceptance-turn" }
      });
      expect(responseMetadata.length).toBe(7);
      expect(responseMetadata.every((meta) =>
        JSON.stringify(meta) === JSON.stringify({ "codex/toolSurface": { app: null, kind: "computerUse" } })
      )).toBe(true);
    } finally {
      restore();
      await daemon.close();
      cleanup();
    }
  });

  test("normalizes every established WindowTarget field and preserves structured failure", async () => {
    const path = fixturePath("target");
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request }) => request.type === "activate_window"
        ? {
            type: "activate_window",
            outcome: {
              success: false,
              message: "window was not found",
              code: "WindowNotFound",
              diagnostics: [{ code: "WindowNotFound", message: "no match", details: "fixture" }]
            }
          }
        : undefined
    });
    await daemon.start();
    const restore = setSocket(path);
    globalThis.nodeRepl = { requestMeta: { session_id: "s", turn_id: "t" } };
    try {
      const outcome = await client().activate_window({
        window_id: "  id  ",
        pid: 42,
        tty: " tty1 ",
        terminal_pid: 43,
        terminal_command: " bash ",
        terminal_cwd: " /tmp ",
        app_id: " app ",
        wm_class: " class ",
        title: " title "
      });
      expect(daemon.requests.at(-1)).toEqual({
        type: "activate_window",
        target: {
          window_id: "id",
          pid: 42,
          tty: "tty1",
          terminal_pid: 43,
          terminal_command: "bash",
        terminal_cwd: "/tmp",
        app_id: "app",
        wm_class: "class",
        title: "title"
        },
        context: { session_id: "s", turn_id: "t" }
      });
      expect(outcome).toEqual({
        success: false,
        message: "window was not found",
        code: "WindowNotFound",
        diagnostics: [{ code: "WindowNotFound", message: "no match", details: "fixture" }]
      });
    } finally {
      restore();
      await daemon.close();
      cleanup();
    }
  });

  test("propagates the same bounded request identity to activate_window and CUA actions", async () => {
    const path = fixturePath("identity-gap");
    const daemon = new FakeDaemon(path);
    await daemon.start();
    const restore = setSocket(path);
    globalThis.nodeRepl = {
      requestMeta: {
        session_id: "identity-session",
        turn_id: "identity-turn",
        deadline_ms: 1234,
        caller_provenance: "openclaw",
        identity_synthetic: true,
        client_info: { name: "OpenClaw", version: "fixture" }
      }
    };
    try {
      const sky = client();
      await sky.move({ x: 1, y: 2 });
      await sky.activate_window({ title: "fixture" });

      const move = daemon.requests.find((request) => request.type === "move");
      expect(move?.type).toBe("move");
      if (move?.type === "move") {
        expect(move.context).toEqual({
          session_id: "identity-session",
          turn_id: "identity-turn",
          deadline_ms: 1234
        });
        expect("caller_provenance" in move.context).toBe(false);
        expect("identity_synthetic" in move.context).toBe(false);
        expect("client_info" in move.context).toBe(false);
      }

      const activation = daemon.requests.find((request) => request.type === "activate_window");
      expect(activation?.type).toBe("activate_window");
      if (activation?.type === "activate_window") {
        expect(activation.context).toEqual({
          session_id: "identity-session",
          turn_id: "identity-turn",
          deadline_ms: 1234
        });
      }
    } finally {
      restore();
      await daemon.close();
      cleanup();
    }
  });

  test("rejects missing metadata and invalid selectors before socket I/O", async () => {
    const path = fixturePath("validation");
    const daemon = new FakeDaemon(path);
    await daemon.start();
    const restore = setSocket(path);
    try {
      const sky = client();
      globalThis.nodeRepl = { requestMeta: {} };
      await expectSkyReject(sky.activate_window({ window_id: "one" }), "SKY_CUA_INVALID_CONTEXT");
      globalThis.nodeRepl = { requestMeta: { session_id: "s", turn_id: "t" } };
      for (const input of [
        {},
        { window_id: " " },
        { pid: 0 },
        { terminal_pid: 1.5 },
        { title: "ok", unexpected: true }
      ]) {
        await expectSkyReject(sky.activate_window(input), "SKY_CUA_INVALID_ARGUMENT");
      }
      expect(daemon.requests).toEqual([]);
    } finally {
      restore();
      await daemon.close();
      cleanup();
    }
  });

  test("requires the activate_window capability before sending the request", async () => {
    const path = fixturePath("capability");
    const daemon = new FakeDaemon(path, {
      defaultHealth: {
        capabilities: HEALTH_CAPABILITIES.filter(
          (capability) => capability !== "linux.activate_window"
        )
      }
    });
    await daemon.start();
    const restore = setSocket(path);
    globalThis.nodeRepl = { requestMeta: { session_id: "s", turn_id: "t" } };
    try {
      await expectSkyReject(
        client().activate_window({ title: "fixture" }),
        "SKY_CUA_CAPABILITY_MISSING"
      );
      expect(daemon.requests.map((request) => request.type)).toEqual(["health"]);
    } finally {
      restore();
      await daemon.close();
      cleanup();
    }
  });

  test("exposes the dedicated AppShot producer request without an MCP capture shim", async () => {
    const path = fixturePath("appshot");
    const daemon = new FakeDaemon(path);
    await daemon.start();
    const restore = setSocket(path);
    try {
      const result = await client().appshot_capture({
        request_id: "composer-1",
        frontmost: true,
        flags: { include_ax_text: true }
      });
      expect(result.capture_scope).toBe("window");
      expect(result.application.window_id).toBe("fixture-window");
      expect(daemon.requests).toEqual([
        { type: "health" },
        {
          type: "appshot_capture",
          request_id: "composer-1",
          frontmost: true,
          flags: { include_ax_text: true }
        }
      ]);
    } finally {
      restore();
      await daemon.close();
      cleanup();
    }
  });

  test("preserves native AppShot failure codes when the legacy error omits ok", async () => {
    const path = fixturePath("appshot-error");
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request }) => request.type === "appshot_capture"
        ? {
            type: "error",
            code: "PortalApprovalPending",
            message: "The screenshot portal request is still pending."
          }
        : undefined
    });
    await daemon.start();
    const restore = setSocket(path);
    try {
      await expectSkyReject(
        client().appshot_capture({ request_id: "composer-pending", frontmost: true }),
        "PortalApprovalPending"
      );
    } finally {
      restore();
      await daemon.close();
      cleanup();
    }
  });

  test("uses request metadata deadline and the control connection for activation cancellation", async () => {
    const path = fixturePath("deadline");
    let activationCount = 0;
    const daemon = new FakeDaemon(path, {
      onRequest: ({ request }) => {
        if (request.type === "activate_window") {
          activationCount += 1;
          return new Promise(() => {});
        }
        return undefined;
      }
    });
    await daemon.start();
    const restore = setSocket(path);
    globalThis.nodeRepl = {
      requestMeta: { session_id: "deadline-session", turn_id: "deadline-turn", deadline_ms: 20 }
    };
    try {
      await expectSkyReject(
        client().activate_window({ title: "fixture" }),
        "SKY_CUA_DEADLINE_EXCEEDED"
      );
      expect(activationCount).toBe(1);
      expect(daemon.requests.map((request) => request.type)).toEqual([
        "health",
        "activate_window",
        "cancel_turn"
      ]);
      const activation = daemon.requests[1];
      expect(activation?.type === "activate_window" ? activation.context : undefined).toEqual({
        session_id: "deadline-session",
        turn_id: "deadline-turn",
        deadline_ms: 20
      });
    } finally {
      restore();
      await daemon.close();
      cleanup();
    }
  });

  test("rejects malformed outcomes and never retries a post-write activation disconnect", async () => {
    const malformedPath = fixturePath("malformed");
    const malformed = new FakeDaemon(malformedPath, {
      onRequest: ({ request }) => request.type === "activate_window"
        ? ({ type: "activate_window", outcome: { success: true } } as never)
        : undefined
    });
    await malformed.start();
    let restore = setSocket(malformedPath);
    globalThis.nodeRepl = { requestMeta: { session_id: "s", turn_id: "t" } };
    try {
      await expectSkyReject(
        client().activate_window({ title: "fixture" }),
        "SKY_CUA_INVALID_ARGUMENT"
      );
    } finally {
      restore();
      await malformed.close();
    }

    const disconnectPath = fixturePath("disconnect");
    let activationCount = 0;
    const disconnect = new FakeDaemon(disconnectPath, {
      onRequest: ({ request, socket }) => {
        if (request.type === "activate_window") {
          activationCount += 1;
          socket.destroy();
        }
        return undefined;
      }
    });
    await disconnect.start();
    restore = setSocket(disconnectPath);
    try {
      let error: unknown;
      try {
        await client().activate_window({ title: "fixture" });
      } catch (cause) {
        error = cause;
      }
      expect(error instanceof SkyCuaError).toBe(true);
      expect((error as SkyCuaError).code).toBe("SKY_CUA_SERVICE_DISCONNECTED");
      expect((error as SkyCuaError).retry).toBe("never");
      expect(activationCount).toBe(1);
    } finally {
      restore();
      await disconnect.close();
      cleanup();
    }
  });
});

void ({} as TransportRequest);
