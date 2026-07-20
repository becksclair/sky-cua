import { strict as assert } from "node:assert";
import { readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { test } from "bun:test";
import { executeBrowserCommand } from "../src/wire-runtime.ts";

type Params = Record<string, unknown>;
type Listener = (params: unknown) => void;

class CorrectnessBackend {
  readonly calls: Array<{ method: string; params: Params }> = [];
  private readonly listeners = new Map<string, Set<Listener>>();

  onNotification(method: string, listener: Listener): () => void {
    const listeners = this.listeners.get(method) ?? new Set<Listener>();
    listeners.add(listener);
    this.listeners.set(method, listeners);
    return () => listeners.delete(listener);
  }

  emit(method: string, params: unknown): void {
    for (const listener of this.listeners.get(method) ?? []) listener(params);
  }

  async raw(method: string, params: Params = {}): Promise<unknown> {
    this.calls.push({ method, params });
    if (method !== "executeCdp") return {};
    if (params.method !== "Runtime.evaluate") return {};
    const expression = String((params.commandParams as Params | undefined)?.expression ?? "");
    let value: unknown = {};
    if (expression === "location.href") value = "https://example.test/ready";
    else if (expression === "document.readyState") value = "complete";
    else if (expression.includes("readyState:document.readyState")) {
      value = { readyState: "complete", pending: 0 };
    } else if (expression.includes("performance.getEntriesByType('resource')")) {
      value = {
        pageUrl: "https://example.test/",
        entries: [
          { url: "https://example.test/a/app.js", initiator: "script" },
          { url: "https://example.test/b/app.js", initiator: "script" },
        ],
        inlineSvgs: [],
      };
    } else if (expression.includes("const response=await fetch")) {
      value = {
        base64: Buffer.from(expression.includes("/a/app.js") ? "asset-a" : "asset-b").toString("base64"),
        contentType: "text/javascript",
      };
    }
    return { result: { value } };
  }
}

async function waitForBehavior(backend: CorrectnessBackend, count: number): Promise<Params[]> {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const calls = backend.calls.filter((call) => call.params.method === "Browser.setDownloadBehavior");
    if (calls.length >= count) return calls.map((call) => call.params.commandParams as Params);
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error("timed out waiting for download behavior");
}

test("waitUntil, screenshot clips, and keyboard chords honor the documented options", async () => {
  const backend = new CorrectnessBackend();
  const started = Date.now();
  await executeBrowserCommand(backend, {
    type: "playwright_wait_for_url",
    browser_id: "fixture",
    tab_id: "tab-1",
    url: "https://example.test/ready",
    waitUntil: "networkidle",
    timeoutMs: 750,
  });
  assert.equal(backend.calls.some((call) => call.params.method === "Network.enable"), true);
  assert.equal(Date.now() - started >= 500, true);

  await executeBrowserCommand(backend, {
    type: "tab_screenshot",
    browser_id: "fixture",
    tab_id: "tab-1",
    options: { clip: { x: 1, y: 2, width: 3, height: 4 } },
  });
  const screenshot = backend.calls.find((call) => call.params.method === "Page.captureScreenshot");
  assert.deepEqual((screenshot?.params.commandParams as Params).clip, {
    x: 1, y: 2, width: 3, height: 4, scale: 1,
  });

  await executeBrowserCommand(backend, {
    type: "cua_keypress",
    browser_id: "fixture",
    tab_id: "tab-1",
    keys: ["Control", "Shift", "A"],
  });
  const keys = backend.calls
    .filter((call) => call.params.method === "Input.dispatchKeyEvent")
    .map((call) => call.params.commandParams);
  assert.deepEqual(keys, [
    { type: "keyDown", key: "Control", modifiers: 2 },
    { type: "keyDown", key: "Shift", modifiers: 10 },
    { type: "keyDown", key: "A", modifiers: 10 },
    { type: "keyUp", key: "A", modifiers: 10 },
    { type: "keyUp", key: "Shift", modifiers: 2 },
    { type: "keyUp", key: "Control", modifiers: 0 },
  ]);
});

test("asset bundles use collision-proof deterministic filenames", async () => {
  const backend = new CorrectnessBackend();
  const inventory = await executeBrowserCommand(backend, {
    type: "tab_page_assets_list",
    browser_id: "fixture",
    tab_id: "tab-1",
  }) as Params;
  const bundle = await executeBrowserCommand(backend, {
    type: "tab_page_assets_bundle",
    browser_id: "fixture",
    tab_id: "tab-1",
    inventoryId: inventory.id,
  }) as Params;
  const assets = bundle.assets as Params[];
  try {
    assert.equal(assets.length, 2);
    assert.notEqual(assets[0]?.path, assets[1]?.path);
    assert.equal(await readFile(String(assets[0]?.path), "utf8"), "asset-a");
    assert.equal(await readFile(String(assets[1]?.path), "utf8"), "asset-b");
  } finally {
    await rm(String(bundle.directoryPath), { recursive: true, force: true });
  }
});

test("download handles and paths remain owned by their originating tab", async () => {
  const backend = new CorrectnessBackend();
  const tabOne = executeBrowserCommand(backend, {
    type: "playwright_wait_for_download",
    browser_id: "fixture",
    tab_id: "tab-1",
    timeoutMs: 50,
  });
  const tabTwo = executeBrowserCommand(backend, {
    type: "playwright_wait_for_download",
    browser_id: "fixture",
    tab_id: "tab-2",
    timeoutMs: 1_000,
  });
  const behaviors = await waitForBehavior(backend, 2);
  assert.equal(behaviors[0]?.downloadPath, behaviors[1]?.downloadPath);
  const tabTwoRoot = String(behaviors[1]?.downloadPath);
  const rawPath = join(tabTwoRoot, "download-2");
  await writeFile(rawPath, "owned by tab two");
  backend.emit("onCDPEvent", {
    source: { tabId: "tab-2" },
    method: "Browser.downloadWillBegin",
    params: { guid: "download-2", suggestedFilename: "result.txt" },
  });
  backend.emit("onDownloadChange", {
    id: "download-2",
    status: "complete",
    path: rawPath,
  });
  try {
    assert.equal(await tabTwo, "download-2");
    await assert.rejects(tabOne, /download timed out/u);
    assert.equal(await executeBrowserCommand(backend, {
      type: "playwright_download_path",
      browser_id: "fixture",
      tab_id: "tab-1",
      download: "download-2",
    }), null);
    assert.equal(await executeBrowserCommand(backend, {
      type: "playwright_download_path",
      browser_id: "fixture",
      tab_id: "tab-2",
      download: "download-2",
    }), join(tabTwoRoot, "result.txt"));
  } finally {
    await rm(dirname(rawPath), { recursive: true, force: true });
    const tabOneRoot = String(behaviors[0]?.downloadPath);
    await rm(tabOneRoot, { recursive: true, force: true });
  }
});
