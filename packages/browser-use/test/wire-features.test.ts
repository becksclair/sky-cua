import { strict as assert } from "node:assert";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "bun:test";
import { executeBrowserCommand } from "../src/wire-runtime.ts";

type Params = Record<string, unknown>;
type NotificationListener = (params: unknown) => void;

const WEBP_BASE64 = "UklGRkoAAABXRUJQVlA4ID4AAADQAQCdASoBAAEAAUAmJaQAA3AA/v3AgAA=";

class FeatureBackend {
  readonly calls: Array<{ method: string; params: Params }> = [];
  private readonly listeners = new Map<string, Set<NotificationListener>>();

  onNotification(method: string, listener: NotificationListener): () => void {
    const listeners = this.listeners.get(method) ?? new Set<NotificationListener>();
    listeners.add(listener);
    this.listeners.set(method, listeners);
    return () => listeners.delete(listener);
  }

  emit(method: string, params: unknown): void {
    for (const listener of this.listeners.get(method) ?? []) listener(params);
  }

  async raw(method: string, params: Params = {}): Promise<unknown> {
    this.calls.push({ method, params });
    if (method === "getTabs") {
      return { tabs: [{ id: "tab-1", active: true, title: "Fixture Page", url: "https://example.test/page" }] };
    }
    if (method !== "executeCdp") return {};
    const cdpMethod = String(params.method ?? "");
    const commandParams = params.commandParams as Params | undefined;
    if (cdpMethod === "Page.captureScreenshot") return { data: WEBP_BASE64 };
    if (cdpMethod !== "Runtime.evaluate") return {};
    const expression = String(commandParams?.expression ?? "");
    let value: unknown = {};
    if (expression.includes("html:'<!doctype html>")) {
      value = {
        html: "<!doctype html>\n<html><body><main>Fixture export</main></body></html>",
        title: "Fixture Page",
        url: "https://example.test/page",
      };
    } else if (expression.includes("performance.getEntriesByType('resource')")) {
      value = {
        pageUrl: "https://example.test/page",
        entries: [
          { url: "https://example.test/logo.webp", initiator: "img" },
          { url: "https://example.test/app.js", initiator: "script" },
        ],
        inlineSvgs: [{ id: "svg-1", name: "mark", markup: "<svg viewBox=\"0 0 1 1\"></svg>" }],
      };
    } else if (expression.includes("const response=await fetch")) {
      value = expression.includes("logo.webp")
        ? { base64: WEBP_BASE64, contentType: "image/webp" }
        : { base64: Buffer.from("console.log('fixture')\n").toString("base64"), contentType: "text/javascript" };
    } else if (expression.includes("sky-cua-highlight-")) {
      value = "fixture-overlay";
    } else if (expression === "location.origin") {
      value = "https://example.test";
    } else if (expression === "navigator.clipboard.readText()") {
      value = "clipboard fixture";
    }
    return { result: { value } };
  }
}

async function waitForCall(backend: FeatureBackend, method: string, cdpMethod?: string): Promise<Params> {
  for (let index = 0; index < 200; index += 1) {
    const call = backend.calls.find((candidate) =>
      candidate.method === method && (cdpMethod === undefined || candidate.params.method === cdpMethod));
    if (call !== undefined) return call.params;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  throw new Error(`Timed out waiting for ${method}${cdpMethod === undefined ? "" : `:${cdpMethod}`}`);
}

test("annotated screenshots stay WebP and content/assets produce verified local files", async () => {
  const backend = new FeatureBackend();
  const roots: string[] = [];
  try {
    const screenshot = await executeBrowserCommand(backend, {
      type: "playwright_element_screenshot",
      browser_id: "fixture",
      tab_id: "tab-1",
      x: 12,
      y: 18,
      includeNonInteractable: true,
    }) as { data: string };
    const image = Buffer.from(screenshot.data, "base64");
    assert.equal(image.subarray(0, 4).toString(), "RIFF");
    assert.equal(image.subarray(8, 12).toString(), "WEBP");
    const capture = backend.calls.find((call) => call.params.method === "Page.captureScreenshot");
    assert.equal((capture?.params.commandParams as Params).format, "webp");
    assert.equal(backend.calls.some((call) =>
      String((call.params.commandParams as Params | undefined)?.expression ?? "").includes("fixture-overlay")
    ), true, "annotation overlay is removed after capture");

    const contentPath = String(await executeBrowserCommand(backend, {
      type: "tab_content_export",
      browser_id: "fixture",
      tab_id: "tab-1",
    }));
    roots.push(dirname(contentPath));
    assert.match(await readFile(contentPath, "utf8"), /Fixture export/u);
    assert.equal((await stat(contentPath)).isFile(), true);

    const inventory = await executeBrowserCommand(backend, {
      type: "tab_page_assets_list",
      browser_id: "fixture",
      tab_id: "tab-1",
    }) as Params;
    assert.equal((inventory.summary as Params).totalCount, 2);
    const bundle = await executeBrowserCommand(backend, {
      type: "tab_page_assets_bundle",
      browser_id: "fixture",
      tab_id: "tab-1",
      inventoryId: inventory.id,
    }) as Params;
    roots.push(String(bundle.directoryPath));
    assert.equal((bundle.summary as Params).downloadedCount, 2);
    assert.equal((bundle.summary as Params).failedCount, 0);
    assert.equal((await stat(String(bundle.manifestPath))).isFile(), true);
    for (const asset of bundle.assets as Params[]) assert.equal((await stat(String(asset.path))).isFile(), true);
  } finally {
    for (const root of roots) await rm(root, { recursive: true, force: true });
  }
});

test("CDP notifications drive events, logs, dialogs, file choosers, and downloads", async () => {
  const backend = new FeatureBackend();
  const uploadRoot = await mkdtemp(join(tmpdir(), "browser-use-upload-"));
  let downloadRoot: string | undefined;
  try {
    await executeBrowserCommand(backend, { type: "tab_id", browser_id: "fixture", tab_id: "tab-1" });
    backend.emit("onCDPEvent", {
      source: { tabId: "tab-1", sessionId: "session-1", targetId: "target-1" },
      method: "Runtime.consoleAPICalled",
      params: { type: "warning", timestamp: Date.now(), args: [{ value: "fixture" }, { value: "warning" }] },
    });
    backend.emit("onCDPEvent", {
      source: { tabId: "tab-1", sessionId: "session-1", targetId: "target-1" },
      method: "Page.javascriptDialogOpening",
      params: { type: "prompt" },
    });
    const events = await executeBrowserCommand(backend, {
      type: "tab_cdp_events",
      browser_id: "fixture",
      tab_id: "tab-1",
      after_sequence: 0,
      methods: ["Runtime.consoleAPICalled"],
    }) as Params;
    assert.equal((events.events as unknown[]).length, 1);
    assert.equal(events.cursor, 1);
    const logs = await executeBrowserCommand(backend, {
      type: "tab_dev_logs",
      browser_id: "fixture",
      tab_id: "tab-1",
      options: { levels: ["warn"] },
    }) as Params[];
    assert.equal(logs[0]?.message, "fixture warning");
    const dialog = await executeBrowserCommand(backend, {
      type: "tab_get_js_dialog",
      browser_id: "fixture",
      tab_id: "tab-1",
    }) as Params;
    assert.equal(dialog.type, "prompt");
    await executeBrowserCommand(backend, {
      type: "tab_handle_js_dialog",
      browser_id: "fixture",
      tab_id: "tab-1",
      dialog_id: dialog.id,
      action: "accept",
      prompt_text: "accepted",
    });
    const dialogCall = backend.calls.find((call) => call.params.method === "Page.handleJavaScriptDialog");
    assert.deepEqual(dialogCall?.params.target, { tabId: "tab-1", sessionId: "session-1", targetId: "target-1" });
    assert.deepEqual(dialogCall?.params.commandParams, { accept: true, promptText: "accepted" });

    const chooserPromise = executeBrowserCommand(backend, {
      type: "playwright_wait_for_file_chooser",
      browser_id: "fixture",
      tab_id: "tab-1",
      timeoutMs: 1_000,
    }) as Promise<Params>;
    await waitForCall(backend, "executeCdp", "Page.setInterceptFileChooserDialog");
    backend.emit("onCDPEvent", {
      source: { tabId: "tab-1" },
      method: "Page.fileChooserOpened",
      params: { backendNodeId: 44, mode: "selectMultiple" },
    });
    const chooser = await chooserPromise;
    assert.equal(chooser.multiple, true);
    const uploadPath = join(uploadRoot, "fixture.txt");
    await writeFile(uploadPath, "upload fixture\n");
    await executeBrowserCommand(backend, {
      type: "playwright_file_chooser_set_files",
      browser_id: "fixture",
      tab_id: "tab-1",
      chooser: chooser.handle,
      files: [uploadPath],
    });
    const setFiles = backend.calls.find((call) => call.params.method === "DOM.setFileInputFiles");
    assert.deepEqual(setFiles?.params.commandParams, { backendNodeId: 44, files: [uploadPath] });

    const downloadPromise = executeBrowserCommand(backend, {
      type: "playwright_wait_for_download",
      browser_id: "fixture",
      tab_id: "tab-1",
      timeoutMs: 1_000,
    });
    const behavior = await waitForCall(backend, "executeCdp", "Browser.setDownloadBehavior");
    downloadRoot = String((behavior.commandParams as Params).downloadPath);
    const rawDownloadPath = join(downloadRoot, "download-1");
    await writeFile(rawDownloadPath, "download fixture\n");
    backend.emit("onCDPEvent", {
      source: { tabId: "tab-1" },
      method: "Browser.downloadWillBegin",
      params: { guid: "download-1", suggestedFilename: "result.txt", url: "https://example.test/result.txt" },
    });
    backend.emit("onDownloadChange", {
      id: "download-1",
      status: "complete",
      filename: "result.txt",
      path: rawDownloadPath,
    });
    const downloadId = await downloadPromise;
    assert.equal(downloadId, "download-1");
    const finalPath = String(await executeBrowserCommand(backend, {
      type: "playwright_download_path",
      browser_id: "fixture",
      tab_id: "tab-1",
      download: downloadId,
    }));
    assert.equal(finalPath, join(downloadRoot, "result.txt"));
    assert.equal(await readFile(finalPath, "utf8"), "download fixture\n");
  } finally {
    await rm(uploadRoot, { recursive: true, force: true });
    if (downloadRoot !== undefined) await rm(downloadRoot, { recursive: true, force: true });
  }
});

test("clipboard helpers route through origin-scoped CDP primitives", async () => {
  const backend = new FeatureBackend();
  const value = await executeBrowserCommand(backend, {
    type: "tab_clipboard_read_text",
    browser_id: "fixture",
    tab_id: "tab-1",
  });
  assert.equal(value, "clipboard fixture");
  const grant = backend.calls.find((call) => call.params.method === "Browser.grantPermissions");
  assert.deepEqual(grant?.params.commandParams, {
    origin: "https://example.test",
    permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"],
  });
});
