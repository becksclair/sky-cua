import { strict as assert } from "node:assert";
import { test } from "bun:test";
import {
  COMMANDS,
  RAW_PROTOCOL_SUPPORTED_COMMANDS,
  RAW_PROTOCOL_UNSUPPORTED_COMMANDS,
} from "../src/commands.ts";
import { executeBrowserCommand } from "../src/wire-runtime.ts";

test("all 72 canonical commands are supported and the final two use their explicit raw ingress seams", async () => {
  assert.equal(RAW_PROTOCOL_UNSUPPORTED_COMMANDS.length, 0);
  assert.deepEqual(RAW_PROTOCOL_SUPPORTED_COMMANDS, COMMANDS);

  const rawCalls: Array<{ method: string; params?: Record<string, unknown> }> = [];
  const backend = {
    raw: async (method: string, params?: Record<string, unknown>) => {
      rawCalls.push({ method, ...(params === undefined ? {} : { params }) });
      if (method === "executeCdp") return { result: { value: "example.test" } };
      throw new Error(`rejecting raw fixture reached ${method}`);
    },
  };
  await assert.rejects(
    () => executeBrowserCommand(backend, {
      type: "tab_bot_detection_report",
      browser_id: "fixture",
      tab_id: "tab-1",
      reason: "challenge_loop",
    }),
    /rejecting raw fixture reached reportBotDetection/u,
  );
  await assert.rejects(
    () => executeBrowserCommand(backend, {
      type: "tab_browser_auth_handoff",
      browser_id: "fixture",
      tab_id: "tab-1",
      origin: "https://example.test",
      reason: "Sign in",
      expires_at: "2026-07-20T12:00:00Z",
      fields: [],
    }),
    /rejecting raw fixture reached browserAuthHandoff/u,
  );
  assert.deepEqual(rawCalls, [
    {
      method: "executeCdp",
      params: {
        target: { tabId: "tab-1" },
        method: "Runtime.evaluate",
        commandParams: {
          expression: "location.hostname",
          awaitPromise: true,
          returnByValue: true,
        },
      },
    },
    {
      method: "reportBotDetection",
      params: { tabId: "tab-1", reason: "challenge_loop", hostname: "example.test" },
    },
    {
      method: "browserAuthHandoff",
      params: {
        tabId: "tab-1",
        origin: "https://example.test",
        reason: "Sign in",
        expires_at: "2026-07-20T12:00:00Z",
        fields: [],
      },
    },
  ]);
});

test("source contains no catch-all raw RPC escape hatch", async () => {
  const source = await Bun.file(new URL("../src/wire-runtime.ts", import.meta.url)).text();
  assert.doesNotMatch(source, /executeUnhandledCommand/u);
});
