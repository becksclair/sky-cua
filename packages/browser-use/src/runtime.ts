import { API_MANIFEST, API_SURFACE, DOCUMENT_NAMES, readDocument, renderApiReference } from "./api.ts";
import type { BrowserCommand } from "./commands.ts";
import { trustedNodeRepl, type BrowserGlobals } from "./globals.ts";
import { BrowserBackend, BrowserBackendRegistry, type BrowserInfo } from "./transport.ts";

type Params = Record<string, unknown>;

function asObject(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" ? value as Record<string, unknown> : {};
}

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}

function idOf(value: unknown): string {
  if (typeof value === "string" || typeof value === "number") return String(value);
  const id = asObject(value).id;
  if (typeof id === "string" || typeof id === "number") return String(id);
  throw new Error("Browser command returned no tab id");
}

function rawTabId(value: string): string | number {
  return /^\d+$/u.test(value) && Number.isSafeInteger(Number(value)) ? Number(value) : value;
}

function bytesOf(value: unknown): Uint8Array {
  if (value instanceof Uint8Array) return value;
  const object = asObject(value);
  const encoded = typeof object.data === "string" ? object.data
    : typeof object.base64 === "string" ? object.base64
    : typeof value === "string" ? value
    : undefined;
  if (encoded === undefined) throw new Error("Browser screenshot returned no image bytes");
  return Uint8Array.from(globalThis.atob(encoded), (character) => character.charCodeAt(0));
}

function serializeFunction(value: unknown): unknown {
  return typeof value === "function" ? value.toString() : value;
}

class Documentation {
  async get(name: string): Promise<string> { return readDocument(name); }
}

class BrowserDocumentation {
  constructor(private readonly info: BrowserInfo) {}
  async api(): Promise<string> { return renderApiReference(this.info.type, this.info.apiSupportOverrides); }
  async get(name: string): Promise<string> { return readDocument(name); }
  async guidance(): Promise<unknown> { return DOCUMENT_NAMES; }
  lookupCatalog(): string { return DOCUMENT_NAMES.join("\n"); }
}

class CapabilityCollection {
  constructor(
    private readonly backend: BrowserBackend,
    private readonly scope: "browser" | "tab",
    private readonly capabilities: Array<{ id: string; description: string }>,
    private readonly tabId?: string,
  ) {}

  async list(): Promise<Array<{ id: string; description: string }>> {
    return this.capabilities.map((value) => ({ ...value }));
  }

  async get(id: string): Promise<unknown> {
    const info = this.capabilities.find((capability) => capability.id === id);
    if (info === undefined) throw new Error(`Browser capability is not available: ${id}`);
    const base: Record<string, unknown> = {
      id,
      description: info.description,
      documentation: async () => `${id}: ${info.description}`,
    };
    if (this.scope === "tab" && this.tabId !== undefined) {
      const execute = (type: BrowserCommand, params: Params = {}) =>
        this.backend.execute(type, { tab_id: this.tabId, ...params });
      if (id === "cdp") {
        base.send = (method: string, params: Params = {}, options: Params = {}) => execute("tab_cdp_call", {
          method,
          params,
          target: options.target,
          timeout_ms: options.timeoutMs,
        });
        base.readEvents = (options: Params = {}) => execute("tab_cdp_events", {
          after_sequence: options.afterSequence,
          limit: options.limit,
          methods: options.methods,
          target: options.target,
          timeout_ms: options.timeoutMs,
        });
      } else if (id === "botDetection") {
        base.report = (options: Params) => execute("tab_bot_detection_report", options);
      } else if (id === "browserAuth") {
        base.request = (options: Params) => execute("tab_browser_auth_handoff", options);
      } else if (id === "pageAssets") {
        base.list = () => execute("tab_page_assets_list");
        base.bundle = (options: Params) => execute("tab_page_assets_bundle", options);
      }
    }
    return new Proxy(base, {
      get(target, property) {
        if (Reflect.has(target, property)) return Reflect.get(target, property);
        if (property === "then") return undefined;
        if (typeof property === "string") {
          return () => {
            throw new Error(`Browser capability ${id}.${property} has no executable sky-cua v1 raw-protocol mapping`);
          };
        }
      },
    });
  }
}

class BrowserUser {
  constructor(private readonly browser: Browser) {}
  async openTabs(): Promise<unknown[]> { return asArray(await this.browser.command("browser_user_open_tabs")); }
  async history(options: Params): Promise<unknown[]> {
    return asArray(await this.browser.command("browser_user_history", { options }));
  }
  async claimTab(tab: unknown): Promise<Tab> {
    const result = await this.browser.command("browser_user_claim_tab", {
      tab: typeof tab === "string" ? tab : asObject(tab),
    });
    return this.browser.tab(result);
  }
}

class Tabs {
  constructor(private readonly browser: Browser) {}
  async content(options: Params): Promise<unknown[]> {
    const urls = Array.isArray(options.urls) ? options.urls.map(String) : [];
    const contentType = String(options.contentType ?? "text");
    const results: unknown[] = [];
    for (const url of urls) {
      const tab = await this.new();
      try {
        await tab.goto(url);
        await tab.playwright.waitForLoadState({ timeoutMs: options.timeoutMs });
        const content = contentType === "domSnapshot"
          ? await tab.playwright.domSnapshot()
          : await tab.playwright.evaluate(
            contentType === "html"
              ? "() => document.documentElement.outerHTML"
              : "() => document.body?.innerText ?? ''",
          );
        results.push({ url: await tab.url() ?? url, title: await tab.title() ?? null, content });
      } catch {
        results.push({ url, title: null, content: null });
      } finally {
        await tab.close();
      }
    }
    return results;
  }
  async finalize(options: Params): Promise<void> {
    const keep = Array.isArray(options.keep) ? options.keep.map((entry) => {
      const object = asObject(entry);
      return { ...object, tab: idOf(object.tab) };
    }) : undefined;
    await this.browser.command("finalize_tabs", { ...options, ...(keep === undefined ? {} : { keep }) });
  }
  async get(id: string): Promise<Tab> { return this.browser.tab(id); }
  async list(): Promise<unknown[]> { return asArray(await this.browser.command("list_tabs")); }
  async new(): Promise<Tab> { return this.browser.tab(await this.browser.command("create_tab")); }
  async selected(): Promise<Tab | undefined> {
    const selected = await this.browser.command("selected_tab");
    return selected === null || selected === undefined ? undefined : this.browser.tab(selected);
  }
}

class Tab {
  readonly capabilities: CapabilityCollection;
  readonly clipboard: TabClipboardAPI;
  readonly content: ContentAPI;
  readonly cua: CUAAPI;
  readonly dev: TabDevAPI;
  readonly dom_cua: DomCUAAPI;
  readonly playwright: PlaywrightAPI;

  constructor(readonly id: string, private readonly browser: Browser) {
    this.capabilities = new CapabilityCollection(
      browser.backend,
      "tab",
      browser.info.capabilities.tab ?? [],
      id,
    );
    this.clipboard = new TabClipboardAPI(this);
    this.content = new ContentAPI(this);
    this.cua = new CUAAPI(this);
    this.dev = new TabDevAPI(this);
    this.dom_cua = new DomCUAAPI(this);
    this.playwright = new PlaywrightAPI(this);
  }

  command(type: BrowserCommand, params: Params = {}): Promise<unknown> {
    return this.browser.command(type, { tab_id: this.id, ...params });
  }
  async back(): Promise<void> { await this.command("navigate_tab_back"); }
  async close(): Promise<void> { await this.command("close_tab"); }
  async forward(): Promise<void> { await this.command("navigate_tab_forward"); }
  async goto(url: string): Promise<void> { await this.command("navigate_tab_url", { url }); }
  async reload(): Promise<void> { await this.command("navigate_tab_reload"); }
  async title(): Promise<string | undefined> {
    const tabs = await this.browser.tabs.list() as Array<Record<string, unknown>>;
    const title = tabs.find((tab) => String(tab.id) === this.id)?.title;
    return typeof title === "string" ? title : undefined;
  }
  async url(): Promise<string | undefined> {
    const tabs = await this.browser.tabs.list() as Array<Record<string, unknown>>;
    const url = tabs.find((tab) => String(tab.id) === this.id)?.url;
    return typeof url === "string" ? url : undefined;
  }
  async screenshot(options: Params = {}): Promise<Uint8Array> {
    return bytesOf(await this.command("tab_screenshot", { options }));
  }
  async markDeliverable(): Promise<void> {
    await this.browser.backend.raw("markTab", { tabId: rawTabId(this.id), status: "deliverable" });
  }
  async markHandoff(): Promise<void> {
    await this.browser.backend.raw("markTab", { tabId: rawTabId(this.id), status: "handoff" });
  }
  async getJsDialog(): Promise<Dialog | undefined> {
    const result = await this.command("tab_get_js_dialog");
    if (result === null || result === undefined) return undefined;
    return new Dialog(this, asObject(result));
  }
}

class Dialog {
  readonly type: string;
  private readonly id: string;
  constructor(private readonly tab: Tab, raw: Record<string, unknown>) {
    this.type = typeof raw.type === "string" ? raw.type : "alert";
    this.id = String(raw.id ?? "");
  }
  async accept(text?: string): Promise<void> {
    await this.tab.command("tab_handle_js_dialog", {
      action: "accept",
      dialog_id: this.id,
      ...(text === undefined ? {} : { prompt_text: text }),
    });
  }
  async dismiss(): Promise<void> {
    await this.tab.command("tab_handle_js_dialog", { action: "dismiss", dialog_id: this.id });
  }
}

class ContentAPI {
  constructor(private readonly tab: Tab) {}
  async export(): Promise<string> { return String(await this.tab.command("tab_content_export")); }
  async exportGsuite(type: "pdf" | "md" | "xlsx" | "csv" | "docx" | "pptx"): Promise<string> {
    return String(await this.tab.command("tab_content_export_gsuite", { export_type: type }));
  }
}

class CUAAPI {
  constructor(private readonly tab: Tab) {}
  async click(options: Params): Promise<void> { await this.tab.command("cua_click", options); }
  async double_click(options: Params): Promise<void> { await this.tab.command("cua_double_click", options); }
  async downloadMedia(options: Params): Promise<void> { await this.tab.command("cua_download_media", options); }
  async drag(options: Params): Promise<void> { await this.tab.command("cua_drag", options); }
  async keypress(options: Params): Promise<void> { await this.tab.command("cua_keypress", options); }
  async move(options: Params): Promise<void> { await this.tab.command("cua_move", options); }
  async scroll(options: Params): Promise<void> { await this.tab.command("cua_scroll", options); }
  async type(options: Params): Promise<void> { await this.tab.command("cua_type", options); }
}

class DomCUAAPI {
  constructor(private readonly tab: Tab) {}
  async click(options: Params): Promise<void> { await this.tab.command("dom_cua_click", options); }
  async double_click(options: Params): Promise<void> { await this.tab.command("dom_cua_double_click", options); }
  async downloadMedia(options: Params): Promise<void> { await this.tab.command("dom_cua_download_media", options); }
  async get_visible_dom(): Promise<unknown> { return this.tab.command("dom_cua_get_visible_dom"); }
  async keypress(options: Params): Promise<void> { await this.tab.command("dom_cua_keypress", options); }
  async scroll(options: Params): Promise<void> { await this.tab.command("dom_cua_scroll", options); }
  async type(options: Params): Promise<void> { await this.tab.command("dom_cua_type", options); }
}

type LocatorDescriptor = {
  kind: string;
  args?: unknown[];
  parent?: LocatorDescriptor;
};

class PlaywrightLocator {
  constructor(private readonly tab: Tab, readonly descriptor: LocatorDescriptor) {}
  private derive(kind: string, ...args: unknown[]): PlaywrightLocator {
    return new PlaywrightLocator(this.tab, { kind, args, parent: this.descriptor });
  }
  private call(command: BrowserCommand, params: Params = {}): Promise<unknown> {
    return this.tab.command(command, { locator: this.descriptor, ...params });
  }
  async all(): Promise<PlaywrightLocator[]> {
    const result = asArray(await this.call("playwright_locator_read_all"));
    return result.map((value, index) => new PlaywrightLocator(this.tab,
      asObject(value).kind === undefined ? { kind: "nth", args: [index], parent: this.descriptor } : value as LocatorDescriptor));
  }
  async allTextContents(options: Params = {}): Promise<string[]> { return asArray(await this.call("playwright_locator_all_text_contents", options)).map(String); }
  and(locator: PlaywrightLocator): PlaywrightLocator { return this.derive("and", locator.descriptor); }
  async check(options: Params = {}): Promise<void> { await this.call("playwright_locator_set_checked", { ...options, checked: true }); }
  async click(options: Params = {}): Promise<void> { await this.call("playwright_locator_click", options); }
  async count(): Promise<number> { return Number(await this.call("playwright_locator_count")); }
  async dblclick(options: Params = {}): Promise<void> { await this.call("playwright_locator_dblclick", options); }
  async downloadMedia(options: Params = {}): Promise<void> { await this.call("playwright_locator_download_media", options); }
  async evaluate<TResult, TArg>(pageFunction: unknown, arg?: TArg, options: Params = {}): Promise<TResult> {
    return await this.call("playwright_evaluate", { ...options, locator: this.descriptor, page_function: serializeFunction(pageFunction), arg }) as TResult;
  }
  async fill(value: string, options: Params = {}): Promise<void> { await this.call("playwright_locator_fill", { ...options, value }); }
  filter(options: Params): PlaywrightLocator { return this.derive("filter", normalizeLocatorOptions(options)); }
  first(): PlaywrightLocator { return this.derive("first"); }
  async getAttribute(name: string, options: Params = {}): Promise<string | null> {
    const value = await this.call("playwright_locator_get_attribute", { ...options, name });
    return value === null ? null : String(value);
  }
  getByLabel(text: unknown, options: Params = {}): PlaywrightLocator { return this.derive("getByLabel", text, options); }
  getByPlaceholder(text: unknown, options: Params = {}): PlaywrightLocator { return this.derive("getByPlaceholder", text, options); }
  getByRole(role: string, options: Params = {}): PlaywrightLocator { return this.derive("getByRole", role, options); }
  getByTestId(testId: string): PlaywrightLocator { return this.derive("getByTestId", testId); }
  getByText(text: unknown, options: Params = {}): PlaywrightLocator { return this.derive("getByText", text, options); }
  async innerText(options: Params = {}): Promise<string> { return String(await this.call("playwright_locator_inner_text", options)); }
  async isEnabled(): Promise<boolean> { return Boolean(await this.call("playwright_locator_is_enabled")); }
  async isVisible(): Promise<boolean> { return Boolean(await this.call("playwright_locator_is_visible")); }
  last(): PlaywrightLocator { return this.derive("last"); }
  locator(selector: string, options: Params = {}): PlaywrightLocator { return this.derive("locator", selector, normalizeLocatorOptions(options)); }
  nth(index: number): PlaywrightLocator { return this.derive("nth", index); }
  or(locator: PlaywrightLocator): PlaywrightLocator { return this.derive("or", locator.descriptor); }
  async press(value: string, options: Params = {}): Promise<void> { await this.call("playwright_locator_press", { ...options, value }); }
  async selectOption(value: unknown, options: Params = {}): Promise<void> { await this.call("playwright_locator_select_option", { ...options, value }); }
  async setChecked(checked: boolean, options: Params = {}): Promise<void> { await this.call("playwright_locator_set_checked", { ...options, checked }); }
  async textContent(options: Params = {}): Promise<string | null> {
    const value = await this.call("playwright_locator_text_content", options);
    return value === null ? null : String(value);
  }
  async type(value: string, options: Params = {}): Promise<void> { await this.call("playwright_locator_fill", { ...options, value, append: true }); }
  async uncheck(options: Params = {}): Promise<void> { await this.call("playwright_locator_set_checked", { ...options, checked: false }); }
  async waitFor(options: Params): Promise<void> { await this.call("playwright_locator_wait_for", options); }
}

function normalizeLocatorOptions(options: Params): Params {
  return Object.fromEntries(Object.entries(options).map(([key, value]) =>
    [key, value instanceof PlaywrightLocator ? value.descriptor : value]));
}

class PlaywrightFrameLocator {
  constructor(private readonly tab: Tab, private readonly descriptor: LocatorDescriptor) {}
  private locatorFor(kind: string, ...args: unknown[]): PlaywrightLocator {
    return new PlaywrightLocator(this.tab, { kind, args, parent: this.descriptor });
  }
  frameLocator(selector: string): PlaywrightFrameLocator { return new PlaywrightFrameLocator(this.tab, { kind: "frameLocator", args: [selector], parent: this.descriptor }); }
  getByLabel(text: unknown, options: Params = {}): PlaywrightLocator { return this.locatorFor("getByLabel", text, options); }
  getByPlaceholder(text: unknown, options: Params = {}): PlaywrightLocator { return this.locatorFor("getByPlaceholder", text, options); }
  getByRole(role: string, options: Params = {}): PlaywrightLocator { return this.locatorFor("getByRole", role, options); }
  getByTestId(testId: string): PlaywrightLocator { return this.locatorFor("getByTestId", testId); }
  getByText(text: unknown, options: Params = {}): PlaywrightLocator { return this.locatorFor("getByText", text, options); }
  locator(selector: string): PlaywrightLocator { return this.locatorFor("locator", selector); }
}

class PlaywrightDownload {
  constructor(private readonly tab: Tab, private readonly handle: unknown) {}
  async path(options: Params = {}): Promise<string | null> {
    const value = await this.tab.command("playwright_download_path", { ...options, download: this.handle });
    return value === null ? null : String(value);
  }
}

class PlaywrightFileChooser {
  constructor(private readonly tab: Tab, private readonly handle: unknown, private readonly multiple: boolean) {}
  isMultiple(): boolean { return this.multiple; }
  async setFiles(files: string | string[], options: Params = {}): Promise<void> {
    await this.tab.command("playwright_file_chooser_set_files", { ...options, chooser: this.handle, files });
  }
}

class PlaywrightAPI {
  constructor(private readonly tab: Tab) {}
  async domSnapshot(): Promise<string> { return String(await this.tab.command("playwright_dom_snapshot")); }
  async elementInfo(options: Params): Promise<unknown[]> { return asArray(await this.tab.command("playwright_element_info", options)); }
  async elementScreenshot(options: Params): Promise<Uint8Array> { return bytesOf(await this.tab.command("playwright_element_screenshot", options)); }
  async evaluate<TResult, TArg>(pageFunction: unknown, arg?: TArg, options: Params = {}): Promise<TResult> {
    return await this.tab.command("playwright_evaluate", { ...options, page_function: serializeFunction(pageFunction), arg }) as TResult;
  }
  async expectNavigation<T>(action: () => Promise<T>, options: Params): Promise<T> {
    const timeoutMs = options.timeoutMs === undefined ? 30_000 : Number(options.timeoutMs);
    if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
      throw new Error("expectNavigation timeoutMs must be a non-negative finite number");
    }
    await this.tab.command("tab_cdp_call", { method: "Page.enable" });
    const frameTree = asObject(await this.tab.command("tab_cdp_call", {
      method: "Page.getFrameTree",
    }));
    const mainFrameId = String(asObject(asObject(frameTree.frameTree).frame).id ?? "");
    if (mainFrameId === "") throw new Error("expectNavigation could not resolve the main frame");
    const navigation = this.waitForNavigationEvent(
      await this.currentCdpCursor(),
      timeoutMs,
      mainFrameId,
    );
    const [result] = await Promise.all([action(), navigation]);
    if (options.url !== undefined) await this.waitForURL(String(options.url), options);
    await this.waitForLoadState({ state: options.waitUntil ?? "load", timeoutMs: options.timeoutMs });
    return result;
  }
  private async currentCdpCursor(): Promise<number> {
    let cursor = 0;
    while (true) {
      const batch = asObject(await this.tab.command("tab_cdp_events", {
        after_sequence: cursor,
        limit: 1_000,
      }));
      cursor = Number(batch.cursor ?? cursor);
      if (batch.hasMore !== true) return cursor;
    }
  }
  private async waitForNavigationEvent(
    afterSequence: number,
    timeoutMs: number,
    mainFrameId: string,
  ): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    let cursor = afterSequence;
    while (true) {
      const remaining = Math.max(0, deadline - Date.now());
      const batch = asObject(await this.tab.command("tab_cdp_events", {
        after_sequence: cursor,
        limit: 1_000,
        methods: ["Page.frameNavigated", "Page.navigatedWithinDocument"],
        timeout_ms: remaining,
      }));
      const events = asArray(batch.events).map(asObject);
      if (events.some((event) => {
        const params = asObject(event.params);
        if (event.method === "Page.navigatedWithinDocument") {
          return String(params.frameId ?? "") === mainFrameId;
        }
        const frame = asObject(params.frame);
        return event.method === "Page.frameNavigated"
          && frame.parentId === undefined
          && String(frame.id ?? "") === mainFrameId;
      })) return;
      cursor = Number(batch.cursor ?? cursor);
      if (Date.now() >= deadline) throw new Error(`expectNavigation timed out after ${timeoutMs}ms`);
      await new Promise((resolve) => setTimeout(resolve, Math.min(25, Math.max(1, remaining))));
    }
  }
  frameLocator(selector: string): PlaywrightFrameLocator { return new PlaywrightFrameLocator(this.tab, { kind: "frameLocator", args: [selector] }); }
  getByLabel(text: unknown, options: Params = {}): PlaywrightLocator { return new PlaywrightLocator(this.tab, { kind: "getByLabel", args: [text, options] }); }
  getByPlaceholder(text: unknown, options: Params = {}): PlaywrightLocator { return new PlaywrightLocator(this.tab, { kind: "getByPlaceholder", args: [text, options] }); }
  getByRole(role: string, options: Params = {}): PlaywrightLocator { return new PlaywrightLocator(this.tab, { kind: "getByRole", args: [role, options] }); }
  getByTestId(testId: string): PlaywrightLocator { return new PlaywrightLocator(this.tab, { kind: "getByTestId", args: [testId] }); }
  getByText(text: unknown, options: Params = {}): PlaywrightLocator { return new PlaywrightLocator(this.tab, { kind: "getByText", args: [text, options] }); }
  locator(selector: string, options: Params = {}): PlaywrightLocator {
    return new PlaywrightLocator(this.tab, { kind: "locator", args: [selector, normalizeLocatorOptions(options)] });
  }
  async waitForEvent(event: "download" | "filechooser", options: Params = {}): Promise<PlaywrightDownload | PlaywrightFileChooser> {
    const command = event === "download" ? "playwright_wait_for_download" : "playwright_wait_for_file_chooser";
    const result = await this.tab.command(command, options);
    if (event === "download") return new PlaywrightDownload(this.tab, result);
    const object = asObject(result);
    return new PlaywrightFileChooser(this.tab, object.handle ?? result, object.multiple === true);
  }
  async waitForLoadState(options: Params): Promise<void> { await this.tab.command("playwright_wait_for_load_state", options); }
  async waitForTimeout(timeoutMs: number): Promise<void> { await this.tab.command("playwright_wait_for_timeout", { timeout_ms: timeoutMs }); }
  async waitForURL(url: string, options: Params): Promise<void> { await this.tab.command("playwright_wait_for_url", { ...options, url }); }
}

class TabClipboardAPI {
  constructor(private readonly tab: Tab) {}
  async read(): Promise<unknown[]> { return asArray(await this.tab.command("tab_clipboard_read")); }
  async readText(): Promise<string> { return String(await this.tab.command("tab_clipboard_read_text")); }
  async write(items: unknown[]): Promise<void> { await this.tab.command("tab_clipboard_write", { items }); }
  async writeText(text: string): Promise<void> { await this.tab.command("tab_clipboard_write_text", { text }); }
}

class TabDevAPI {
  constructor(private readonly tab: Tab) {}
  async logs(options: Params): Promise<unknown[]> { return asArray(await this.tab.command("tab_dev_logs", { options })); }
}

class Browser {
  readonly browserId: string;
  readonly capabilities: CapabilityCollection;
  readonly tabs: Tabs;
  readonly user: BrowserUser;
  readonly documentationApi: BrowserDocumentation;

  constructor(readonly backend: BrowserBackend) {
    this.browserId = backend.info.id;
    this.capabilities = new CapabilityCollection(backend, "browser", backend.info.capabilities.browser ?? []);
    this.tabs = new Tabs(this);
    this.user = new BrowserUser(this);
    this.documentationApi = new BrowserDocumentation(backend.info);
  }
  get info(): BrowserInfo { return this.backend.info; }
  command(type: BrowserCommand, params: Params = {}): Promise<unknown> { return this.backend.execute(type, params); }
  tab(value: unknown): Tab { return new Tab(idOf(value), this); }
  async documentation(): Promise<string> {
    return `${await this.documentationApi.api()}\n\nAvailable guidance:\n${this.documentationApi.lookupCatalog()}`;
  }
  async nameSession(name: string): Promise<void> { await this.command("name_session", { name }); }
}

const RUNTIME_INTERFACES: ReadonlyArray<readonly [abstract new (...args: never[]) => object, string]> = [
  [Browser, "Browser"],
  [BrowserUser, "BrowserUser"],
  [Tabs, "Tabs"],
  [Tab, "Tab"],
  [ContentAPI, "ContentAPI"],
  [CUAAPI, "CUAAPI"],
  [DomCUAAPI, "DomCUAAPI"],
  [PlaywrightAPI, "PlaywrightAPI"],
  [PlaywrightFrameLocator, "PlaywrightFrameLocator"],
  [PlaywrightLocator, "PlaywrightLocator"],
  [PlaywrightDownload, "PlaywrightDownload"],
  [PlaywrightFileChooser, "PlaywrightFileChooser"],
  [TabClipboardAPI, "TabClipboardAPI"],
  [TabDevAPI, "TabDevAPI"],
];

function supportedBrowserView(browser: Browser, info: BrowserInfo): Browser {
  const disabled = new Set<string>();
  for (const [interfaceName, members] of Object.entries(API_MANIFEST.interfaces)) {
    for (const [memberName, member] of Object.entries(members)) {
      const memberId = `${interfaceName}.${memberName}`;
      const supported = info.apiSupportOverrides?.[memberId]
        ?? member.unsupportedByDefaultIn?.includes(info.type) !== true;
      if (!supported) disabled.add(memberId);
    }
  }

  const wrapped = new WeakMap<object, object>();
  const original = new WeakMap<object, object>();
  const interfaceName = (value: object): string | undefined =>
    RUNTIME_INTERFACES.find(([constructor]) => value instanceof constructor)?.[1];
  const unwrap = (value: unknown): unknown => {
    if (value === null || typeof value !== "object") return value;
    const target = original.get(value);
    if (target !== undefined) return target;
    if (Array.isArray(value)) return value.map(unwrap);
    if (Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null) {
      return Object.fromEntries(Object.entries(value).map(([key, nested]) => [key, unwrap(nested)]));
    }
    return value;
  };
  const wrap = (value: unknown): unknown => {
    if (value === null || typeof value !== "object") return value;
    if (value instanceof Promise) return value.then(wrap);
    if (Array.isArray(value)) return value.map(wrap);
    const cached = wrapped.get(value);
    if (cached !== undefined) return cached;
    const name = interfaceName(value);
    if (name === undefined) return value;
    const functions = new Map<PropertyKey, unknown>();
    const allowed = (property: PropertyKey) =>
      typeof property !== "string" || !disabled.has(`${name}.${property}`);
    const proxy = new Proxy(value, {
      get(target, property) {
        if (!allowed(property)) return undefined;
        const member = Reflect.get(target, property, target) as unknown;
        if (typeof member !== "function") return wrap(member);
        const cachedFunction = functions.get(property);
        if (cachedFunction !== undefined) return cachedFunction;
        const method = (...args: unknown[]) => wrap(Reflect.apply(member, target, args.map(unwrap)));
        functions.set(property, method);
        return method;
      },
      getOwnPropertyDescriptor(target, property) {
        if (!allowed(property)) return undefined;
        return Reflect.getOwnPropertyDescriptor(target, property);
      },
      has(target, property) { return allowed(property) && Reflect.has(target, property); },
      ownKeys(target) { return Reflect.ownKeys(target).filter(allowed); },
    });
    wrapped.set(value, proxy);
    original.set(proxy, value);
    return proxy;
  };
  return wrap(browser) as Browser;
}

class Browsers {
  private readonly wrappers = new Map<string, Browser>();
  constructor(private readonly registry: BrowserBackendRegistry) {}
  async list(): Promise<BrowserInfo[]> { return (await this.registry.list()).map(({ info }) => ({ ...info })); }
  async get(id: string): Promise<Browser> {
    const backend = await this.registry.get(id);
    let browser = this.wrappers.get(backend.info.id);
    if (browser === undefined) {
      browser = supportedBrowserView(new Browser(backend), backend.info);
      this.wrappers.set(backend.info.id, browser);
    }
    return browser;
  }
  async getDefault(): Promise<Browser> {
    const first = (await this.registry.list())[0];
    if (first === undefined) throw new Error("No Browser backend is available");
    return this.get(first.info.id);
  }
  async getForUrl(_url: string): Promise<Browser> { return this.getDefault(); }
}

export class Agent {
  readonly browsers: Browsers;
  readonly documentation = new Documentation();
  constructor(registry: BrowserBackendRegistry) { this.browsers = new Browsers(registry); }
}

export type SetupBrowserRuntimeOptions = {
  globals: BrowserGlobals;
  elicitationDisplayName?: string;
};

let activeRegistry: BrowserBackendRegistry | undefined;

export async function setupBrowserRuntime({ globals }: SetupBrowserRuntimeOptions): Promise<void> {
  await activeRegistry?.close();
  const transportGlobals = Object.create(globals) as BrowserGlobals;
  Object.defineProperty(transportGlobals, "nodeRepl", {
    value: trustedNodeRepl(globals),
    enumerable: true,
  });
  activeRegistry = new BrowserBackendRegistry(transportGlobals);
  globals.agent = new Agent(activeRegistry);
  globals.display = async (value: unknown): Promise<void> => {
    if (value instanceof Uint8Array && typeof globals.nodeRepl?.emitImage === "function") {
      await globals.nodeRepl.emitImage(value);
      return;
    }
    if (typeof globals.nodeRepl?.write === "function") globals.nodeRepl.write(value);
    else globals.console.log(value);
  };
}

export const RUNTIME_API_SURFACE = API_SURFACE;
