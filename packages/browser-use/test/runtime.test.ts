import { strict as assert } from "node:assert";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, test } from "bun:test";
import { API_MANIFEST, API_SURFACE, COMMANDS, setupBrowserRuntime } from "../src/index.ts";
import type { BrowserGlobals, NativePipeConnection } from "../src/globals.ts";

type ListenerMap = {
  data?: (chunk: Uint8Array) => void;
  error?: (error: unknown) => void;
  close?: () => void;
};

class FakeConnection implements NativePipeConnection {
  ended = false;
  readonly requests: Array<Record<string, unknown>> = [];
  readonly responses: Array<Record<string, unknown>> = [];
  private listeners: ListenerMap = {};
  constructor(
    private readonly browserInfo: Record<string, unknown> = {},
    private readonly responseChunkBytes?: number,
  ) {}
  end(): void { this.ended = true; }
  on(event: "data" | "error" | "close", listener: never): void {
    this.listeners[event] = listener;
  }
  write(frame: Uint8Array): void {
    const size = new DataView(frame.buffer, frame.byteOffset, 4).getUint32(0, true);
    const request = JSON.parse(new TextDecoder().decode(frame.slice(4, size + 4))) as Record<string, unknown>;
    if (typeof request.method !== "string") {
      this.responses.push(request);
      return;
    }
    this.requests.push(request);
    const params = request.params as Record<string, unknown>;
    const type = params.type;
    let result: unknown = {};
    const allowedMethods = new Set([
      "claimUserTab",
      "createTab",
      "executeCdp",
      "finalizeTabs",
      "getInfo",
      "getTabs",
      "getUserHistory",
      "getUserTabs",
      "markTab",
      "nameSession",
      "reportBotDetection",
      "browserAuthHandoff",
    ]);
    if (!allowedMethods.has(String(request.method))) {
      this.respond(request, undefined, { code: -32601, message: `rejecting fixture: unknown method ${String(request.method)}` });
      return;
    }
    if (request.method === "getInfo") result = {
      id: "iab:shared-actor",
      type: "iab",
      name: "Codex In-app Browser",
      capabilities: {
        browser: [{ id: "visibility", description: "Host visibility" }],
        tab: [{ id: "cdp", description: "CDP diagnostics" }],
      },
      metadata: {
        provider: "iab",
        browserInstanceId: "codex:iab-1",
        codexSessionId: "session-1",
      },
      ...this.browserInfo,
    };
    else if (request.method === "createTab") result = { id: "tab-1", title: "Fixture", url: "about:blank" };
    else if (request.method === "claimUserTab") result = { id: "tab-claimed", title: "Claimed", url: "https://example.test" };
    else if (request.method === "getUserTabs") result = [{ id: "user-tab-1", title: "User tab" }];
    else if (request.method === "getUserHistory") result = [{ url: "https://example.test", dateVisited: "2026-07-20T00:00:00Z" }];
    else if (request.method === "getTabs") result = [{ id: "tab-1", active: true, title: "Fixture", url: "about:blank" }];
    else if (request.method === "executeCdp" && params.method === "Page.captureScreenshot") result = { data: "iVBORw0KGgo=" };
    else if (request.method === "executeCdp" && params.method === "Page.getFrameTree") {
      result = { frameTree: { frame: { id: "main-frame", url: "https://example.test/ready" } } };
    }
    else if (request.method === "executeCdp" && params.method === "Runtime.evaluate") {
      const expression = String((params.commandParams as Record<string, unknown>)?.expression ?? "");
      const value = expression.includes("return nodes.length") ? 1
        : expression.includes("return !!nodes[0]&&visible") ? true
        : expression.includes("return nodes.map((_,index)") ? [{ kind: "nth", args: [0] }]
        : expression.includes("return {attached:nodes.length") ? { attached: 0, visible: 0 }
        : expression.includes("performance.getEntriesByType('resource')") ? { readyState: "complete", pending: 0 }
        : expression.includes("document.readyState") ? "complete"
        : expression.includes("location.href") ? "https://example.test/ready"
        : expression.includes("location.hostname") ? "example.test"
        : expression.includes("getBoundingClientRect") ? { x: 4, y: 5 }
        : {};
      result = { result: { value } };
    }
    else if (request.method === "executeCdp" && params.method === "Target.getTargets") {
      result = { targetInfos: [{ tabId: 1, targetId: "target-1" }] };
    }
    void type;
    this.respond(request, result);
  }
  emitNotification(method: string, params: Record<string, unknown>): void {
    const payload = new TextEncoder().encode(JSON.stringify({ jsonrpc: "2.0", method, params }));
    const encoded = new Uint8Array(payload.byteLength + 4);
    new DataView(encoded.buffer).setUint32(0, payload.byteLength, true);
    encoded.set(payload, 4);
    this.listeners.data?.(encoded);
  }
  emitRequest(id: number, method: string): void {
    const payload = new TextEncoder().encode(JSON.stringify({ jsonrpc: "2.0", id, method }));
    const encoded = new Uint8Array(payload.byteLength + 4);
    new DataView(encoded.buffer).setUint32(0, payload.byteLength, true);
    encoded.set(payload, 4);
    this.listeners.data?.(encoded);
  }
  private respond(
    request: Record<string, unknown>,
    result: unknown,
    error?: { code: number; message: string },
  ): void {
    const response = new TextEncoder().encode(JSON.stringify({
      jsonrpc: "2.0",
      id: request.id,
      ...(error === undefined ? { result } : { error }),
    }));
    const encoded = new Uint8Array(response.byteLength + 4);
    new DataView(encoded.buffer).setUint32(0, response.byteLength, true);
    encoded.set(response, 4);
    if (this.responseChunkBytes === undefined) {
      this.listeners.data?.(encoded);
      return;
    }
    for (let offset = 0; offset < encoded.byteLength; offset += this.responseChunkBytes) {
      this.listeners.data?.(encoded.slice(offset, offset + this.responseChunkBytes));
    }
  }
}

class SilentConnection implements NativePipeConnection {
  ended = false;
  end(): void { this.ended = true; }
  on(event: "data", listener: (chunk: Uint8Array) => void): void;
  on(event: "error", listener: (error: unknown) => void): void;
  on(event: "close", listener: () => void): void;
  on(): void {}
  write(_data: Uint8Array): void {}
}

function fixture(browserInfo: Record<string, unknown> = {}, responseChunkBytes?: number) {
  const connection = new FakeConnection(browserInfo, responseChunkBytes);
  const browserType = browserInfo.type ?? "iab";
  const hostProvidedIab = browserType === "iab";
  const iabSocketName = "11111111-1111-4111-8111-111111111111.sock";
  const iabSocketPath = `/tmp/codex-browser-use/${iabSocketName}`;
  const extensionSocketPath = "/run/user/1000/sky-cua/browser.sock";
  let connectionCount = 0;
  const responseMeta: Array<Record<string, unknown>> = [];
  const globals = {
    console,
    nodeRepl: {
      env: {
        ...(hostProvidedIab ? {} : { SKY_CUA_CODEX_BROWSER_SOCKET_PATH: extensionSocketPath }),
        SKY_CUA_MCP_CALLER_PROVENANCE: "codex_desktop",
      },
      requestMeta: {
        session_id: "session-1",
        thread_id: "thread-1",
        turn_id: "turn-1",
        "x-codex-turn-metadata": {
          session_id: "session-1",
          thread_id: "thread-1",
          turn_id: "turn-1",
        },
      },
      nativePipe: {
        createConnection: async (path: string) => {
          connectionCount += 1;
          assert.equal(path, hostProvidedIab ? iabSocketPath : extensionSocketPath);
          return connection;
        },
        listDirectory: async () => hostProvidedIab ? [iabSocketName] : [],
      },
      setResponseMeta: (meta: Record<string, unknown>) => responseMeta.push(meta),
      write: () => {},
    },
  } as unknown as BrowserGlobals;
  return { globals, connection, connectionCount: () => connectionCount, responseMeta };
}

function routingFixture(options: {
  entries: string[];
  explicit?: string;
  peers: Map<string, NativePipeConnection | Error>;
}) {
  const attempted: string[] = [];
  const globals = {
    console,
    nodeRepl: {
      env: {
        ...(options.explicit === undefined
          ? {}
          : { SKY_CUA_CODEX_BROWSER_SOCKET_PATH: options.explicit }),
        SKY_CUA_MCP_CALLER_PROVENANCE: "codex_desktop",
      },
      requestMeta: {
        session_id: "session-1",
        thread_id: "thread-1",
        turn_id: "turn-1",
      },
      nativePipe: {
        listDirectory: async (path: string) => {
          assert.equal(path, "/tmp/codex-browser-use");
          return options.entries;
        },
        createConnection: async (path: string) => {
          attempted.push(path);
          const peer = options.peers.get(path);
          if (peer === undefined) throw new Error(`unexpected Browser socket: ${path}`);
          if (peer instanceof Error) throw peer;
          return peer;
        },
      },
    },
  } as unknown as BrowserGlobals;
  return { attempted, globals };
}

describe("canonical Browser runtime", () => {
  test("setup is lazy and preserves exact provider identity plus caller metadata", async () => {
    const state = fixture();
    await setupBrowserRuntime({ globals: state.globals });
    assert.equal(state.connectionCount(), 0);
    const agent = state.globals.agent as { browsers: { list(): Promise<Array<Record<string, unknown>>> } };
    const list = await agent.browsers.list();
    assert.equal(state.connectionCount(), 1);
    assert.deepEqual(list[0], {
      id: "iab:shared-actor",
      type: "iab",
      transport: "host_provided_iab",
      name: "Codex In-app Browser",
      capabilities: {
        browser: [{ id: "visibility", description: "Host visibility" }],
        tab: [{ id: "cdp", description: "CDP diagnostics" }],
      },
      metadata: {
        provider: "iab",
        browserInstanceId: "codex:iab-1",
        codexSessionId: "session-1",
        skyCuaBridgeTransport: "host_provided_iab",
      },
    });
    const getInfo = state.connection.requests[0] as Record<string, unknown>;
    assert.equal(getInfo.method, "getInfo");
    const params = getInfo.params as Record<string, unknown>;
    assert.equal(params.session_id, "session-1");
    assert.equal(params.thread_id, "thread-1");
    assert.equal(params.turn_id, "turn-1");
    assert.equal(params.caller_provenance, "codex_desktop");
  });

  test("flattens nested Codex turn metadata for Browser RPC authorization", async () => {
    const state = fixture();
    state.globals.nodeRepl!.requestMeta = {
      "x-codex-turn-metadata": {
        session_id: "session-1",
        thread_id: "thread-1",
        turn_id: "turn-1",
      },
    };
    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as {
      browsers: { list(): Promise<Array<Record<string, unknown>>> };
    };
    await agent.browsers.list();
    const params = state.connection.requests[0]?.params as Record<string, unknown>;
    assert.equal(params.session_id, "session-1");
    assert.equal(params.thread_id, "thread-1");
    assert.equal(params.turn_id, "turn-1");
    assert.deepEqual(params["x-codex-turn-metadata"], {
      session_id: "session-1",
      thread_id: "thread-1",
      turn_id: "turn-1",
    });
  });

  test("does not expose the trusted native pipe through the public Agent graph", async () => {
    const state = fixture();
    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as {
      browsers: Record<PropertyKey, unknown>;
    };
    assert.equal(agent.browsers.registry, undefined);
    assert.equal(agent.browsers.globals, undefined);
    assert.deepEqual(Reflect.ownKeys(agent.browsers), []);
  });

  test("discovers the session-matched host IAB, ignores stale peers, and keeps extension identity truthful", async () => {
    const staleName = "00000000-0000-4000-8000-000000000000.sock";
    const iabName = "11111111-1111-4111-8111-111111111111.sock";
    const foreignName = "22222222-2222-4222-8222-222222222222.sock";
    const directoryExtensionName = "33333333-3333-4333-8333-333333333333.sock";
    const iabPath = `/tmp/codex-browser-use/${iabName}`;
    const foreignPath = `/tmp/codex-browser-use/${foreignName}`;
    const directoryExtensionPath = `/tmp/codex-browser-use/${directoryExtensionName}`;
    const extensionPath = "/run/user/1000/sky-cua/codex-browser.sock";
    const iab = new FakeConnection({
      id: "iab:session-1",
      type: "iab",
      name: "Codex In-app Browser",
      metadata: { codexSessionId: "session-1" },
    });
    const foreign = new FakeConnection({
      id: "iab:foreign",
      type: "iab",
      metadata: { codexSessionId: "another-session" },
    });
    const extension = new FakeConnection({
      id: "iab:legacy-extension",
      type: "iab",
      name: "Chrome",
      metadata: {
        codexSessionId: "session-1",
        skyCuaBridgeType: "extension",
        skyCuaBridgeTransport: "extension_native_host",
      },
    });
    const directoryExtension = new FakeConnection({
      id: "iab:directory-extension",
      type: "iab",
      metadata: {
        codexSessionId: "session-1",
        skyCuaBridgeTransport: "extension_native_host",
      },
    });
    const state = routingFixture({
      entries: [
        foreignName,
        "extension-1234-deadbeef.sock",
        iabName,
        directoryExtensionName,
        staleName,
        iabName,
        "../escape.sock",
      ],
      explicit: extensionPath,
      peers: new Map<string, FakeConnection | Error>([
        [`/tmp/codex-browser-use/${staleName}`, new Error("stale socket")],
        [iabPath, iab],
        [foreignPath, foreign],
        [directoryExtensionPath, directoryExtension],
        [extensionPath, extension],
      ]),
    });

    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as any;
    const available = await agent.browsers.list();
    assert.deepEqual(available.map((info: Record<string, unknown>) => ({
      id: info.id,
      type: info.type,
      transport: info.transport,
    })), [
      { id: "iab:session-1", type: "iab", transport: "host_provided_iab" },
      { id: "iab:legacy-extension", type: "extension", transport: "extension_native_host" },
    ]);
    assert.equal((await agent.browsers.get("iab")).info.id, "iab:session-1");
    assert.equal((await agent.browsers.get("extension")).info.id, "iab:legacy-extension");
    assert.equal((await agent.browsers.get("iab:legacy-extension")).info.type, "extension");
    assert.equal(foreign.ended, true);
    assert.equal(directoryExtension.ended, true);
    assert.deepEqual(state.attempted, [
      `/tmp/codex-browser-use/${staleName}`,
      iabPath,
      foreignPath,
      directoryExtensionPath,
      extensionPath,
    ]);
  });

  test("responds to server-side ping requests", async () => {
    const state = fixture();
    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as { browsers: { list(): Promise<unknown> } };
    await agent.browsers.list();
    state.connection.emitRequest(41, "ping");
    state.connection.emitRequest(42, "unsupportedServerRequest");
    assert.deepEqual(state.connection.responses, [
      { jsonrpc: "2.0", id: 41, result: "pong" },
      {
        jsonrpc: "2.0",
        id: 42,
        error: {
          code: -32601,
          message: "No handler registered for method: unsupportedServerRequest",
        },
      },
    ]);
  });

  test("an extension-native compatibility label never satisfies get iab", async () => {
    const extensionPath = "/run/user/1000/sky-cua/codex-browser.sock";
    const extension = new FakeConnection({
      id: "iab",
      type: "iab",
      name: "Chrome",
      metadata: {
        skyCuaBridgeType: "extension",
        skyCuaBridgeTransport: "extension_native_host",
      },
    });
    const state = routingFixture({
      entries: [],
      explicit: extensionPath,
      peers: new Map<string, FakeConnection | Error>([[extensionPath, extension]]),
    });

    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as any;
    await assert.rejects(() => agent.browsers.get("iab"), /Browser is not available: iab/u);
    assert.deepEqual((await agent.browsers.list()).map((info: Record<string, unknown>) => info.id), [
      "iab",
    ]);
  });

  test("returns the current-session IAB without waiting for an unrelated silent task socket", async () => {
    const liveName = "11111111-1111-4111-8111-111111111111.sock";
    const silentName = "22222222-2222-4222-8222-222222222222.sock";
    const live = new FakeConnection({
      id: "iab:session-1",
      type: "iab",
      metadata: { codexSessionId: "session-1" },
    });
    const silent = new SilentConnection();
    const state = routingFixture({
      entries: [silentName, liveName],
      peers: new Map<string, NativePipeConnection | Error>([
        [`/tmp/codex-browser-use/${silentName}`, silent],
        [`/tmp/codex-browser-use/${liveName}`, live],
      ]),
    });

    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as any;
    const browser = await Promise.race([
      agent.browsers.get("iab"),
      new Promise<never>((_resolve, reject) =>
        setTimeout(() => reject(new Error("IAB discovery did not complete promptly")), 250)),
    ]);
    assert.equal(browser.info.id, "iab:session-1");
    assert.equal(silent.ended, true);
  });

  test("assembles a fragmented large Browser response without per-chunk buffer replacement", async () => {
    const largeValue = "x".repeat(256 * 1024);
    const state = fixture({ metadata: {
      codexSessionId: "session-1",
      largeValue,
    } }, 64);
    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as any;
    const browsers = await agent.browsers.list();
    assert.equal(browsers[0].metadata.largeValue, largeValue);
  });

  test("full object graph implements the documented API and routes canonical commands", async () => {
    const state = fixture({
      id: "extension:actor",
      type: "extension",
      name: "Chrome Extension",
      apiSupportOverrides: { "Tabs.content": true },
    });
    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as any;
    const browser = await agent.browsers.get("extension");
    const tab = await browser.tabs.new();
    const locator = tab.playwright.getByRole("button", { name: "Save" });
    const frame = tab.playwright.frameLocator("iframe");

    const objects: Record<string, object> = {
      Agent: agent,
      Browsers: agent.browsers,
      Browser: browser,
      BrowserUser: browser.user,
      Tabs: browser.tabs,
      Tab: tab,
      CUAAPI: tab.cua,
      DomCUAAPI: tab.dom_cua,
      PlaywrightAPI: tab.playwright,
      PlaywrightFrameLocator: frame,
      PlaywrightLocator: locator,
      Documentation: agent.documentation,
    };
    for (const [interfaceName, object] of Object.entries(objects)) {
      for (const member of API_SURFACE[interfaceName] ?? []) {
        assert.notEqual((object as Record<string, unknown>)[member], undefined, `${interfaceName}.${member}`);
      }
    }

    assert.equal((await tab.screenshot()).byteLength, 8);
    assert.equal(await tab.title(), "Fixture");
    assert.equal(await locator.count(), 1);
    assert.equal(await locator.isVisible(), true);
    assert.equal((await locator.all()).length, 1);
    const visibility = await browser.capabilities.get("visibility");
    assert.match(await visibility.documentation(), /Host visibility/u);
    assert.throws(() => visibility.set(true), /no executable sky-cua v1 raw-protocol mapping/u);
    await tab.cua.click({ x: 1, y: 2 });
    await tab.dom_cua.get_visible_dom();
    await locator.click({});
    await browser.nameSession("session name");
    await browser.tabs.finalize({ keep: [{ tab, status: "deliverable" }] });
    const methods = state.connection.requests.map((request) => request.method);
    assert.equal(methods.includes("executeAgentCommand"), false);
    assert.equal(methods.includes("createTab"), true);
    assert.equal(methods.includes("getTabs"), true);
    assert.equal(methods.includes("executeCdp"), true);
    assert.equal(methods.includes("executeUnhandledCommand"), false);
    const finalize = state.connection.requests.find((request) => request.method === "finalizeTabs")!;
    assert.deepEqual((finalize.params as Record<string, unknown>).keep, [
      { tabId: "tab-1", status: "deliverable" },
    ]);
    const executeParams = state.connection.requests.find((request) => request.method === "executeCdp")?.params as Record<string, unknown>;
    assert.deepEqual(executeParams._meta, executeParams.request_meta);
    assert.equal((executeParams._meta as Record<string, unknown>).turn_id, "turn-1");
  });

  test("per-tab retention marks never finalize peer tabs", async () => {
    const state = fixture({ id: "extension:marks", type: "extension", name: "Chrome Extension" });
    await setupBrowserRuntime({ globals: state.globals });
    const browser = await (state.globals.agent as any).browsers.get("extension");
    const deliverable = await browser.tabs.get("17");
    const handoff = await browser.tabs.get("peer-tab");

    await deliverable.markDeliverable();
    await handoff.markHandoff();

    const marks = state.connection.requests.filter((request) => request.method === "markTab");
    assert.deepEqual(marks.map((request) => {
      const params = request.params as Record<string, unknown>;
      return { tabId: params.tabId, status: params.status };
    }), [
      { tabId: 17, status: "deliverable" },
      { tabId: "peer-tab", status: "handoff" },
    ]);
    assert.equal(state.connection.requests.some((request) => request.method === "finalizeTabs"), false);
  });

  test("daemon-local tab capabilities are reachable without extension capability mappings", async () => {
    const state = fixture({
      id: "extension:daemon-capabilities",
      type: "extension",
      name: "Chrome Extension",
      capabilities: {
        tab: [
          { id: "cdp", description: "Extension CDP" },
          { id: "botDetection", description: "Daemon bot detection" },
          { id: "browserAuth", description: "Daemon browser auth" },
        ],
      },
    });
    await setupBrowserRuntime({ globals: state.globals });
    const browser = await (state.globals.agent as any).browsers.get("extension");
    const tab = await browser.tabs.get("17");

    await (await tab.capabilities.get("botDetection")).report({ reason: "captcha" });
    await (await tab.capabilities.get("browserAuth")).request({
      origin: "https://example.test",
      expires_at: "2026-07-20T12:00:00Z",
      fields: [{ id: "username", type: "text", required: true }],
    });

    const methods = state.connection.requests.map((request) => request.method);
    assert.equal(methods.includes("reportBotDetection"), true);
    assert.equal(methods.includes("browserAuthHandoff"), true);
  });

  test("expectNavigation ignores stale events, arms before action, and rejects non-navigation", async () => {
    const state = fixture({ id: "extension:navigation", type: "extension", name: "Chrome Extension" });
    await setupBrowserRuntime({ globals: state.globals });
    const browser = await (state.globals.agent as any).browsers.get("extension");
    const tab = await browser.tabs.get("17");
    state.connection.emitNotification("onCDPEvent", {
      source: { tabId: 17 },
      method: "Page.frameNavigated",
      params: { frame: { id: "stale-frame", url: "https://example.test/stale" } },
    });

    await assert.rejects(
      () => tab.playwright.expectNavigation(async () => {
        assert.equal(state.connection.requests.some((request) =>
          request.method === "executeCdp"
          && (request.params as Record<string, unknown>).method === "Page.enable"), true);
        return "no navigation";
      }, { timeoutMs: 10 }),
      /expectNavigation timed out/u,
    );

    const navigated = await tab.playwright.expectNavigation(async () => {
      state.connection.emitNotification("onCDPEvent", {
        source: { tabId: 17 },
        method: "Page.navigatedWithinDocument",
        params: { frameId: "child-frame", url: "https://example.test/iframe#changed" },
      });
      setTimeout(() => state.connection.emitNotification("onCDPEvent", {
        source: { tabId: 17 },
        method: "Page.frameNavigated",
        params: { frame: { id: "main-frame", url: "https://example.test/ready" } },
      }), 10);
      return "navigated";
    }, { timeoutMs: 100, url: "https://example.test/ready" });
    assert.equal(navigated, "navigated");
  });

  test("manifest contains all current interfaces, supporting types, and declarations", () => {
    assert.equal(Object.keys(API_MANIFEST.interfaces).length, 22);
    assert.equal(Object.keys(API_MANIFEST.types).length, 58);
    assert.equal(Object.values(API_MANIFEST.interfaces).flatMap((members) => Object.values(members)).length, 135);
    assert.equal(COMMANDS.length, 72);
  });

  test("every command family decomposes onto the existing low-level wire", async () => {
    const state = fixture({ id: "extension:actor", type: "extension", name: "Chrome Extension" });
    state.globals.nodeRepl!.requestMeta = {
      session_id: "session-family",
      thread_id: "thread-family",
      turn_id: "turn-family",
      client_info: { name: "Codex Desktop", version: "test" },
      identity_synthetic: false,
    };
    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as any;
    const browser = await agent.browsers.get("extension");
    await browser.user.openTabs();
    await browser.user.history({ limit: 1 });
    const claimed = await browser.user.claimTab("user-tab-1");
    const tab = await browser.tabs.new();
    await browser.tabs.list();
    await browser.tabs.selected();
    await tab.goto("https://example.test");
    await tab.reload();
    await tab.back();
    await tab.forward();
    await tab.cua.move({ x: 1, y: 2 });
    await tab.cua.drag({ path: [{ x: 1, y: 2 }, { x: 3, y: 4 }] });
    await tab.cua.scroll({ x: 1, y: 2, scrollX: 0, scrollY: 20 });
    await tab.cua.keypress({ keys: ["Enter"] });
    await tab.cua.type({ text: "hello" });
    await tab.dom_cua.get_visible_dom();
    await tab.dom_cua.click({ node_id: "sky-1" });
    await tab.playwright.domSnapshot();
    await tab.playwright.locator("button").count();
    await tab.playwright.locator("button").click({});
    await browser.nameSession("family");
    await browser.tabs.finalize({ keep: [{ tab, status: "handoff" }] });
    await claimed.close();

    const methods = new Set(state.connection.requests.map((request) => String(request.method)));
    assert.deepEqual([...methods].sort(), [
      "claimUserTab",
      "createTab",
      "executeCdp",
      "finalizeTabs",
      "getInfo",
      "getTabs",
      "getUserHistory",
      "getUserTabs",
      "nameSession",
    ]);
    for (const request of state.connection.requests) {
      const params = request.params as Record<string, unknown>;
      assert.equal(params.session_id, "session-family");
      assert.equal(params.thread_id, "thread-family");
      assert.equal(params.turn_id, "turn-family");
      assert.equal(params.caller_provenance, "codex_desktop");
      assert.deepEqual(params.client_info, { name: "Codex Desktop", version: "test" });
      assert.equal(params.identity_synthetic, false);
      assert.deepEqual(params._meta, params.request_meta);
    }
  });

  test("manifest defaults and apiSupportOverrides produce truthful iab, extension, and cdp views", async () => {
    const variants = [
      { type: "iab", expectedHistory: false, expectedFinalize: true },
      { type: "extension", expectedHistory: true, expectedFinalize: true },
      { type: "cdp", expectedHistory: false, expectedFinalize: false },
    ] as const;
    for (const variant of variants) {
      const state = fixture({ id: `${variant.type}:actor`, type: variant.type, name: variant.type });
      await setupBrowserRuntime({ globals: state.globals });
      const browser = await (state.globals.agent as any).browsers.get(variant.type);
      const tab = await browser.tabs.new();
      assert.equal(typeof browser.user.history === "function", variant.expectedHistory);
      assert.equal(typeof browser.tabs.finalize === "function", variant.expectedFinalize);
      assert.notEqual(tab.clipboard, undefined);
      assert.notEqual(tab.content, undefined);
      assert.notEqual(tab.dev, undefined);
      assert.equal(typeof tab.getJsDialog, "function");
      assert.equal(typeof tab.playwright.elementScreenshot, "function");
      assert.equal(typeof tab.playwright.waitForEvent, "function");
      assert.equal(typeof tab.playwright.locator("a").downloadMedia, "function");
      if (variant.type === "iab") {
        assert.equal(typeof tab.markDeliverable, "function");
        assert.equal(typeof tab.markHandoff, "function");
      }
    }

    const overridden = fixture({
      id: "iab:overridden",
      type: "iab",
      apiSupportOverrides: {
        "BrowserUser.history": true,
        "Tabs.finalize": true,
        "Tab.screenshot": false,
        "PlaywrightAPI.elementScreenshot": false,
      },
    });
    await setupBrowserRuntime({ globals: overridden.globals });
    const browser = await (overridden.globals.agent as any).browsers.get("iab");
    const tab = await browser.tabs.new();
    assert.equal(typeof browser.user.history, "function");
    assert.equal(typeof browser.tabs.finalize, "function");
    assert.equal(tab.screenshot, undefined);
    assert.equal(tab.playwright.elementScreenshot, undefined);
    const docs = await browser.documentation();
    assert.match(docs, /history\(options:/u);
    assert.doesNotMatch(docs, /elementScreenshot/u);
  });

  test("locator descriptors preserve top-level scope, relative has/hasNot filters, and wait states", async () => {
    const state = fixture({ id: "extension:dom", type: "extension" });
    await setupBrowserRuntime({ globals: state.globals });
    const browser = await (state.globals.agent as any).browsers.get("extension");
    const tab = await browser.tabs.new();
    const has = tab.playwright.getByText("Save", { exact: true });
    const hasNot = tab.playwright.getByRole("alert");
    const locator = tab.playwright.locator("section", { has, hasNot, hasText: "Editor" })
      .locator("button", { hasNotText: "Cancel" });
    await locator.count();
    await locator.waitFor({ state: "hidden", timeoutMs: 20 });
    await locator.waitFor({ state: "detached", timeoutMs: 20 });
    await tab.playwright.waitForLoadState({ state: "domcontentloaded", timeoutMs: 20 });
    await tab.playwright.waitForLoadState({ state: "load", timeoutMs: 20 });
    await tab.playwright.waitForURL("https://example.test/ready", { timeoutMs: 20 });
    await tab.playwright.waitForURL("https://*.test/*", { timeoutMs: 20 });
    await assert.rejects(
      () => tab.playwright.waitForURL("example.test", { timeoutMs: 1 }),
      /timed out/u,
    );
    await tab.playwright.waitForLoadState({ state: "networkidle", timeoutMs: 750 });

    const expressions = state.connection.requests
      .filter((request) => request.method === "executeCdp")
      .map((request) => String(((request.params as any).commandParams as any)?.expression ?? ""));
    const locatorExpression = expressions.find((expression) =>
      expression.includes('"kind":"locator"') && expression.includes('"section"'))!;
    assert.match(locatorExpression, /"kind":"getByText"/u);
    assert.match(locatorExpression, /"kind":"getByRole"/u);
    assert.match(locatorExpression, /options\.hasNot/u);
    assert.doesNotMatch(locatorExpression, /"kind":"locator","args":\["\*"/u);
    assert.equal(expressions.filter((expression) => expression.includes("document.readyState")).length >= 3, true);
  });

  test("generated locator programs execute against a real Chrome DOM", async () => {
    const playwrightPath = resolve(
      import.meta.dir,
      "../../../out/components/cua-node-linux-x64-glibc/lib/node_modules/playwright/index.js",
    );
    const chromePath = "/usr/bin/google-chrome-stable";
    assert.equal(await Bun.file(playwrightPath).exists(), true, "bundled Playwright is required for real-DOM acceptance");
    assert.equal(await Bun.file(chromePath).exists(), true, "system Chrome is required for real-DOM acceptance");
    const { chromium } = await import(pathToFileURL(playwrightPath).href) as any;
    const chrome = await chromium.launch({ executablePath: chromePath, headless: true, args: ["--no-sandbox"] });
    try {
      const page = await chrome.newPage();
      await page.setContent(`
        <section data-testid="editor">Editor <span>Save</span><button>Save</button><button>Cancel</button></section>
        <section><div role="alert">Blocked</div><span>Save</span><button>Wrong</button></section>
        <label>Email <input aria-label="Email"></label>
      `);
      const state = fixture({ id: "extension:real-dom", type: "extension" });
      await setupBrowserRuntime({ globals: state.globals });
      const browser = await (state.globals.agent as any).browsers.get("extension");
      const tab = await browser.tabs.new();
      const matching = tab.playwright
        .locator("section", {
          has: tab.playwright.getByText("Save", { exact: true }),
          hasNot: tab.playwright.getByRole("alert"),
          hasText: "Editor",
        })
        .locator("button", { hasNotText: "Cancel" });
      await matching.count();
      const countExpression = String(((state.connection.requests.at(-1)?.params as any).commandParams as any).expression);
      assert.equal(await page.evaluate((source: string) => globalThis.eval(source), countExpression), 1);

      await tab.playwright.getByLabel("Email", { exact: true }).fill("real-dom@example.test");
      const fillExpression = String(((state.connection.requests.at(-1)?.params as any).commandParams as any).expression);
      await page.evaluate((source: string) => globalThis.eval(source), fillExpression);
      assert.equal(await page.locator("input").inputValue(), "real-dom@example.test");
    } finally {
      await chrome.close();
    }
  }, 30_000);

  test("relative explicit socket fails before trusted discovery or connection", async () => {
    const state = fixture();
    state.globals.nodeRepl!.env!.SKY_CUA_CODEX_BROWSER_SOCKET_PATH = "relative.sock";
    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as any;
    await assert.rejects(() => agent.browsers.list(), /SKY_CUA_CODEX_BROWSER_SOCKET_PATH/u);
    assert.equal(state.connectionCount(), 0);
  });

  test("missing explicit socket requires a task-scoped discovery candidate", async () => {
    const state = fixture({ id: "extension:actor", type: "extension" });
    delete state.globals.nodeRepl!.env!.SKY_CUA_CODEX_BROWSER_SOCKET_PATH;
    state.globals.nodeRepl!.nativePipe!.listDirectory = async () => [];
    await setupBrowserRuntime({ globals: state.globals });
    const agent = state.globals.agent as any;
    await assert.rejects(() => agent.browsers.list(), /No task-scoped Browser socket/u);
    assert.equal(state.connectionCount(), 0);
  });
});
