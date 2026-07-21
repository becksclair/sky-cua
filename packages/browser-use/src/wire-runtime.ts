import { createHash, randomUUID } from "node:crypto";
import { mkdir, mkdtemp, rename, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, extname, join } from "node:path";
import type { BrowserCommand, CommandEnvelope } from "./commands.ts";

type RawBackend = {
  raw(method: string, params?: Record<string, unknown>): Promise<unknown>;
  finalizeStaleNodeReplSession?(sessionId: string): Promise<void>;
  onNotification?(method: string, listener: (params: unknown) => void): () => void;
};

type ObjectValue = Record<string, unknown>;

type CdpEvent = {
  sequence: number;
  source: ObjectValue;
  method: string;
  params?: ObjectValue;
};

type DialogState = { id: string; type: string; target: ObjectValue };
type FileChooserState = { id: string; backendNodeId: number; multiple: boolean };
type DownloadState = {
  id: string;
  filename?: string;
  path?: string;
  status: string;
  tabId: string;
  url?: string;
};

type BackendState = {
  cdpEvents: Map<string, CdpEvent[]>;
  dialogs: Map<string, DialogState>;
  downloadRoot: string | undefined;
  downloads: Map<string, DownloadState>;
  fileChooserHandles: Map<string, FileChooserState>;
  fileChoosers: Map<string, FileChooserState[]>;
  inventories: Map<string, ObjectValue>;
  logs: Map<string, ObjectValue[]>;
  nextSequence: number;
};

const BACKEND_STATES = new WeakMap<RawBackend, BackendState>();

function backendState(backend: RawBackend): BackendState {
  const existing = BACKEND_STATES.get(backend);
  if (existing !== undefined) return existing;
  const state: BackendState = {
    cdpEvents: new Map(),
    dialogs: new Map(),
    downloadRoot: undefined,
    downloads: new Map(),
    fileChooserHandles: new Map(),
    fileChoosers: new Map(),
    inventories: new Map(),
    logs: new Map(),
    nextSequence: 1,
  };
  backend.onNotification?.("onCDPEvent", (params) => recordCdpEvent(state, object(params)));
  backend.onNotification?.("onDownloadChange", (params) => recordDownload(state, object(params)));
  BACKEND_STATES.set(backend, state);
  return state;
}

function notificationTabId(source: ObjectValue): string | undefined {
  const value = source.tabId ?? source.tab_id;
  return typeof value === "string" || typeof value === "number" ? String(value) : undefined;
}

function recordCdpEvent(state: BackendState, notification: ObjectValue): void {
  const source = object(notification.source);
  const tab = notificationTabId(source);
  const method = typeof notification.method === "string" ? notification.method : undefined;
  if (tab === undefined || method === undefined) return;
  const params = object(notification.params);
  const event: CdpEvent = { sequence: state.nextSequence++, source, method, ...(notification.params === undefined ? {} : { params }) };
  const events = state.cdpEvents.get(tab) ?? [];
  events.push(event);
  if (events.length > 2_000) events.splice(0, events.length - 2_000);
  state.cdpEvents.set(tab, events);

  if (method === "Page.javascriptDialogOpening") {
    state.dialogs.set(tab, {
      id: `dialog-${event.sequence}`,
      type: String(params.type ?? "alert"),
      target: source,
    });
  } else if (method === "Page.javascriptDialogClosed") {
    state.dialogs.delete(tab);
  } else if (method === "Page.fileChooserOpened" && Number.isInteger(params.backendNodeId)) {
    const queue = state.fileChoosers.get(tab) ?? [];
    const chooser = {
      id: randomUUID(),
      backendNodeId: Number(params.backendNodeId),
      multiple: params.mode === "selectMultiple",
    };
    queue.push(chooser);
    state.fileChooserHandles.set(chooser.id, chooser);
    state.fileChoosers.set(tab, queue);
  } else if (method === "Runtime.consoleAPICalled") {
    const args = Array.isArray(params.args) ? params.args.map(object) : [];
    const message = args.map((arg) => arg.value === undefined ? String(arg.description ?? "") : String(arg.value)).join(" ");
    const logs = state.logs.get(tab) ?? [];
    logs.push({
      level: normalizeLogLevel(params.type),
      message,
      timestamp: new Date(Number(params.timestamp ?? Date.now())).toISOString(),
      ...(typeof source.url === "string" ? { url: source.url } : {}),
    });
    if (logs.length > 2_000) logs.splice(0, logs.length - 2_000);
    state.logs.set(tab, logs);
  } else if (method === "Browser.downloadWillBegin") {
    const id = String(params.guid ?? randomUUID());
    state.downloads.set(id, {
      id,
      ...(typeof params.suggestedFilename === "string" ? { filename: params.suggestedFilename } : {}),
      ...(state.downloadRoot === undefined ? {} : { path: join(state.downloadRoot, id) }),
      status: "started",
      tabId: tab,
      ...(typeof params.url === "string" ? { url: params.url } : {}),
    });
  } else if (method === "Browser.downloadProgress") {
    const id = String(params.guid ?? "");
    const current = state.downloads.get(id);
    if (current !== undefined) current.status = normalizeDownloadStatus(params.state);
  }
}

function normalizeLogLevel(value: unknown): string {
  const level = String(value ?? "log");
  return level === "warning" ? "warn" : ["debug", "info", "log", "warn", "error"].includes(level) ? level : "log";
}

function recordDownload(state: BackendState, params: ObjectValue): void {
  const id = String(params.id ?? params.guid ?? "");
  if (id === "") return;
  const existing = state.downloads.get(id);
  const notificationTab = notificationTabId(params);
  if (existing === undefined && notificationTab === undefined) return;
  if (existing !== undefined && notificationTab !== undefined && existing.tabId !== notificationTab) {
    return;
  }
  const current = existing ?? { id, status: "started", tabId: notificationTab! };
  if (typeof params.filename === "string") current.filename = params.filename;
  if (typeof params.path === "string") current.path = params.path;
  if (typeof params.status === "string") current.status = normalizeDownloadStatus(params.status);
  if (typeof params.url === "string") current.url = params.url;
  state.downloads.set(id, current);
}

function normalizeDownloadStatus(value: unknown): string {
  const status = String(value ?? "in_progress");
  if (status === "complete" || status === "completed") return "completed";
  if (status === "inProgress" || status === "in_progress") return "in_progress";
  if (status === "canceled" || status === "cancelled") return "canceled";
  return status;
}

function object(value: unknown): ObjectValue {
  return value !== null && typeof value === "object" ? value as ObjectValue : {};
}

function tabId(command: CommandEnvelope): string | number {
  const value = command.tab_id;
  if (typeof value !== "string" && typeof value !== "number") throw new Error(`${command.type} requires tab_id`);
  return value;
}

function rawTabId(value: string | number): string | number {
  if (typeof value === "number") return value;
  return /^\d+$/u.test(value) && Number.isSafeInteger(Number(value)) ? Number(value) : value;
}

function staleNodeReplOwner(error: unknown): string | undefined {
  const message = error instanceof Error ? error.message : String(error);
  return message.match(
    /already part of browser session (node-repl-[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})(?:\s|$)/iu,
  )?.[1];
}

function isUpfrontDebuggerUnattached(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes("Debugger unattached")
    || message.includes("Debugger is not attached")
  ) && !message.includes("Detached while handling command");
}

function withoutEnvelope(command: CommandEnvelope): ObjectValue {
  const { type: _type, browser_id: _browserId, ...params } = command;
  return params;
}

async function cdp(
  backend: RawBackend,
  tab: string | number,
  method: string,
  commandParams: ObjectValue = {},
  extra: ObjectValue = {},
): Promise<unknown> {
  const params = {
    target: { tabId: rawTabId(tab) },
    method,
    commandParams,
    ...extra,
  };
  try {
    return await backend.raw("executeCdp", params);
  } catch (error) {
    if (!isUpfrontDebuggerUnattached(error)) throw error;
    try {
      await backend.raw("detach", { tabId: rawTabId(tab) });
    } catch {
      // An upfront unattached response normally means there is nothing to
      // detach. The reset remains useful when extension bookkeeping drifted.
    }
    await backend.raw("attach", { tabId: rawTabId(tab) });
    await backend.raw("executeCdp", {
      target: { tabId: rawTabId(tab) },
      method: "Page.enable",
      commandParams: {},
    });
    return backend.raw("executeCdp", params);
  }
}

async function attachClaimedTab(backend: RawBackend, claimed: unknown): Promise<unknown> {
  const id = object(claimed).id;
  if (typeof id !== "string" && typeof id !== "number") {
    throw new Error("Browser claim returned no tab id");
  }
  await backend.raw("attach", { tabId: rawTabId(id) });
  await cdp(backend, id, "Page.enable");
  return claimed;
}

function unsupported(command: CommandEnvelope): never {
  throw new Error(`Browser command is not supported by the sky-cua v1 raw protocol: ${command.type}`);
}

function cdpValue(value: unknown): unknown {
  const response = object(value);
  const result = object(response.result);
  if ("value" in result) return result.value;
  if ("unserializableValue" in result) return result.unserializableValue;
  return value;
}

async function evaluate(
  backend: RawBackend,
  tab: string | number,
  expression: string,
  awaitPromise = true,
): Promise<unknown> {
  return cdpValue(await cdp(backend, tab, "Runtime.evaluate", {
    expression,
    awaitPromise,
    returnByValue: true,
  }));
}

function js(value: unknown): string {
  return JSON.stringify(value, (_key, item) => item instanceof RegExp
    ? { __skyCuaRegex: item.source, flags: item.flags }
    : item);
}

const LOCATOR_RUNTIME = String.raw`
const descriptor = __DESCRIPTOR__;
const text = (node) => (node.innerText || node.textContent || "").trim();
const matches = (actual, expected, exact=false) => expected?.__skyCuaRegex ? new RegExp(expected.__skyCuaRegex,expected.flags||"").test(actual) : exact ? actual===String(expected) : actual.includes(String(expected));
const visible = (node) => { const r=node.getBoundingClientRect(); const s=getComputedStyle(node); return r.width>0&&r.height>0&&s.visibility!=="hidden"&&s.display!=="none"; };
const roleOf = (node) => node.getAttribute("role") || ({BUTTON:"button",A:"link",INPUT:node.type==="checkbox"?"checkbox":"textbox",SELECT:"combobox",TEXTAREA:"textbox"}[node.tagName] || "");
const unique = (nodes) => [...new Set(nodes)];
const descendants = (roots) => unique(roots.flatMap((root) => [...root.querySelectorAll("*")]));
const relativeMatches = (node, locator) => resolve(locator, [node]).length > 0;
const filtered = (nodes, options={}) => nodes.filter((node) =>
  (options.visible !== true || visible(node)) &&
  (options.hasText == null || matches(text(node), options.hasText)) &&
  (options.hasNotText == null || !matches(text(node), options.hasNotText)) &&
  (options.has == null || relativeMatches(node, options.has)) &&
  (options.hasNot == null || !relativeMatches(node, options.hasNot))
);
const resolve = (d, roots=[document]) => {
  if (!d) return [];
  const base = d.parent ? resolve(d.parent, roots) : roots;
  const args = d.args || [];
  let nodes;
  if (d.kind === "locator") nodes = filtered(unique(base.flatMap((root) => [...root.querySelectorAll(args[0])])), args[1]);
  else if (d.kind === "getByRole") nodes = descendants(base).filter((node) => roleOf(node)===args[0] && (!args[1]?.name || matches(text(node),args[1].name,args[1].exact)));
  else if (d.kind === "getByText") nodes = descendants(base).filter((node) => matches(text(node),args[0],args[1]?.exact));
  else if (d.kind === "getByLabel") nodes = descendants(base).filter((node) => { const labelledBy=(node.getAttribute("aria-labelledby")||"").split(/\s+/).filter(Boolean).map((id)=>text(document.getElementById(id)||{})).join(" ");const names=[node.getAttribute("aria-label")||"",...[...(node.labels||[])].map(text),labelledBy].filter(Boolean);return names.some((name)=>matches(name,args[0],args[1]?.exact)); });
  else if (d.kind === "getByPlaceholder") nodes = descendants(base).filter((node) => matches(node.getAttribute("placeholder")||"",args[0],args[1]?.exact));
  else if (d.kind === "getByTestId") nodes = descendants(base).filter((node) => node.getAttribute("data-testid")===String(args[0]));
  else if (d.kind === "first") nodes = base.slice(0,1);
  else if (d.kind === "last") nodes = base.slice(-1);
  else if (d.kind === "nth") nodes = base.slice(Number(args[0]), Number(args[0])+1);
  else if (d.kind === "filter") nodes=filtered(base,args[0]);
  else if (d.kind === "and") { const other=resolve(args[0]); nodes=base.filter((node)=>other.includes(node)); }
  else if (d.kind === "or") nodes=[...new Set([...base,...resolve(args[0])])];
  else if (d.kind === "frameLocator") nodes=base.flatMap((root)=>[...root.querySelectorAll(args[0])]).flatMap((frame)=>frame.contentDocument?[frame.contentDocument]:[]);
  else nodes=base;
  return unique(nodes);
};
const nodes = resolve(descriptor);
`;

function locatorExpression(descriptor: unknown, body: string): string {
  return `(() => {${LOCATOR_RUNTIME.replace("__DESCRIPTOR__", js(descriptor))}${body}})()`;
}

async function locatorValue(backend: RawBackend, command: CommandEnvelope, body: string): Promise<unknown> {
  return evaluate(backend, tabId(command), locatorExpression(command.locator, body));
}

async function locatorPoint(backend: RawBackend, command: CommandEnvelope): Promise<{ x: number; y: number }> {
  const value = object(await locatorValue(backend, command,
    'const node=nodes[0]; if(!node) throw new Error("locator matched no element"); const r=node.getBoundingClientRect(); return {x:r.left+r.width/2,y:r.top+r.height/2};'));
  return { x: Number(value.x), y: Number(value.y) };
}

async function mouse(backend: RawBackend, tab: string | number, type: string, x: number, y: number, extra: ObjectValue = {}): Promise<void> {
  await cdp(backend, tab, "Input.dispatchMouseEvent", { type, x, y, ...extra });
}

async function click(backend: RawBackend, tab: string | number, x: number, y: number, count = 1, button = "left"): Promise<void> {
  await mouse(backend, tab, "mouseMoved", x, y);
  await mouse(backend, tab, "mousePressed", x, y, { button, clickCount: count });
  await mouse(backend, tab, "mouseReleased", x, y, { button, clickCount: count });
}

function button(value: unknown): string {
  return value === 2 || value === "middle" ? "middle" : value === 3 || value === "right" ? "right" : "left";
}

function modifierMask(key: string): number {
  if (key === "Alt") return 1;
  if (key === "Control" || key === "Ctrl") return 2;
  if (key === "Meta" || key === "Command") return 4;
  if (key === "Shift") return 8;
  return 0;
}

async function playwrightCommand(backend: RawBackend, command: CommandEnvelope): Promise<unknown> {
  switch (command.type) {
    case "playwright_dom_snapshot":
      return evaluate(backend, tabId(command), "document.documentElement.outerHTML");
    case "playwright_evaluate": {
      const source = js(command.page_function);
      const arg = js(command.arg);
      const descriptor = command.locator;
      return evaluate(backend, tabId(command), descriptor === undefined
        ? `(async()=>{const fn=(0,eval)('('+${source}+')');return await fn(${arg})})()`
        : locatorExpression(descriptor, `const fn=(0,eval)('('+${source}+')');return fn(nodes[0],${arg});`));
    }
    case "playwright_locator_count": return locatorValue(backend, command, "return nodes.length;");
    case "playwright_locator_read_all": return locatorValue(backend, command, "return nodes.map((_,index)=>({kind:'nth',args:[index],parent:descriptor}));");
    case "playwright_locator_all_text_contents": return locatorValue(backend, command, "return nodes.map((node)=>node.textContent||'');");
    case "playwright_locator_inner_text": return locatorValue(backend, command, "if(!nodes[0])throw new Error('locator matched no element');return nodes[0].innerText||'';");
    case "playwright_locator_text_content": return locatorValue(backend, command, "return nodes[0]?.textContent??null;");
    case "playwright_locator_get_attribute": return locatorValue(backend, command, `return nodes[0]?.getAttribute(${js(command.name)})??null;`);
    case "playwright_locator_is_enabled": return locatorValue(backend, command, "return !!nodes[0]&&!nodes[0].disabled;");
    case "playwright_locator_is_visible": return locatorValue(backend, command, "return !!nodes[0]&&visible(nodes[0]);");
    case "playwright_locator_click":
    case "playwright_locator_dblclick": {
      const point = await locatorPoint(backend, command);
      await click(backend, tabId(command), point.x, point.y, command.type.endsWith("dblclick") ? 2 : 1, button(command.button));
      return undefined;
    }
    case "playwright_locator_fill":
      return locatorValue(backend, command, `const node=nodes[0];if(!node)throw new Error('locator matched no element');node.focus();${command.append === true ? "" : "node.value='';"}node.value+=${js(command.value)};node.dispatchEvent(new Event('input',{bubbles:true}));node.dispatchEvent(new Event('change',{bubbles:true}));`);
    case "playwright_locator_set_checked":
      return locatorValue(backend, command, `const node=nodes[0];if(!node)throw new Error('locator matched no element');node.checked=${command.checked === true};node.dispatchEvent(new Event('input',{bubbles:true}));node.dispatchEvent(new Event('change',{bubbles:true}));`);
    case "playwright_locator_select_option":
      return locatorValue(backend, command, `const node=nodes[0];if(!node)throw new Error('locator matched no element');const values=${js(command.value)};const wanted=new Set((Array.isArray(values)?values:[values]).map((v)=>typeof v==='string'?v:v.value??v.label));for(const option of node.options)option.selected=wanted.has(option.value)||wanted.has(option.label);node.dispatchEvent(new Event('change',{bubbles:true}));return [...node.selectedOptions].map((o)=>o.value);`);
    case "playwright_locator_press":
      await locatorValue(backend, command, "const node=nodes[0];if(!node)throw new Error('locator matched no element');node.focus();");
      await cdp(backend, tabId(command), "Input.dispatchKeyEvent", { type: "keyDown", key: String(command.value) });
      await cdp(backend, tabId(command), "Input.dispatchKeyEvent", { type: "keyUp", key: String(command.value) });
      return undefined;
    case "playwright_locator_wait_for":
    case "playwright_wait_for_load_state":
    case "playwright_wait_for_url":
    case "playwright_wait_for_timeout":
      return waitCommand(backend, command);
    case "playwright_element_info": return elementInfo(backend, command);
    case "playwright_element_screenshot": {
      const token = `sky-cua-highlight-${randomUUID()}`;
      const overlay = await evaluate(backend, tabId(command), `(()=>{const token=${js(token)};const nodes=${command.includeNonInteractable === true ? "document.elementsFromPoint" : "document.elementFromPoint"}(${Number(command.x)},${Number(command.y)});const list=Array.isArray(nodes)?nodes:(nodes?[nodes]:[]);const root=document.createElement('div');root.id=token;root.style.cssText='position:fixed;inset:0;z-index:2147483647;pointer-events:none';for(const node of list.slice(0,12)){const r=node.getBoundingClientRect();const box=document.createElement('div');box.style.cssText='position:absolute;border:3px solid #ff2d55;background:rgba(255,45,85,.10);box-sizing:border-box';box.style.left=r.left+'px';box.style.top=r.top+'px';box.style.width=r.width+'px';box.style.height=r.height+'px';root.appendChild(box)}const dot=document.createElement('div');dot.style.cssText='position:absolute;width:12px;height:12px;border-radius:50%;background:#00e5ff;border:2px solid #001018;box-sizing:border-box';dot.style.left=(${Number(command.x)}-6)+'px';dot.style.top=(${Number(command.y)}-6)+'px';root.appendChild(dot);document.documentElement.appendChild(root);return token})()`);
      try {
        return await cdp(backend, tabId(command), "Page.captureScreenshot", { format: "webp" });
      } finally {
        await evaluate(backend, tabId(command), `document.getElementById(${js(String(overlay))})?.remove()`).catch(() => {});
      }
    }
    case "playwright_locator_download_media": {
      const point = await locatorPoint(backend, command);
      await click(backend, tabId(command), point.x, point.y);
      return undefined;
    }
    case "playwright_wait_for_file_chooser": {
      const state = backendState(backend);
      const tab = String(tabId(command));
      await cdp(backend, tabId(command), "Page.enable");
      await cdp(backend, tabId(command), "Page.setInterceptFileChooserDialog", { enabled: true });
      try {
        const chooser = await waitUntil(() => state.fileChoosers.get(tab)?.shift(), timeoutMs(command), "file chooser");
        return { handle: chooser.id, multiple: chooser.multiple };
      } finally {
        await cdp(backend, tabId(command), "Page.setInterceptFileChooserDialog", { enabled: false }).catch(() => {});
      }
    }
    case "playwright_file_chooser_set_files": {
      const state = backendState(backend);
      const id = String(command.chooser ?? command.file_chooser_id ?? "");
      const chooser = state.fileChooserHandles.get(id);
      if (chooser === undefined) throw new Error("File chooser is no longer active");
      const files = (Array.isArray(command.files) ? command.files : [command.files]).map(String);
      for (const file of files) {
        if (!(await stat(file)).isFile()) throw new Error(`File chooser path is not a regular file: ${file}`);
      }
      await cdp(backend, tabId(command), "DOM.setFileInputFiles", { backendNodeId: chooser.backendNodeId, files });
      state.fileChooserHandles.delete(id);
      return undefined;
    }
    case "playwright_wait_for_download": {
      const state = backendState(backend);
      const tab = String(tabId(command));
      state.downloadRoot ??= await sharedDownloadDirectory();
      const existing = new Set(
        [...state.downloads.values()]
          .filter((value) => value.tabId === tab)
          .map((value) => value.id),
      );
      await cdp(backend, tabId(command), "Browser.setDownloadBehavior", {
        behavior: "allowAndName",
        downloadPath: state.downloadRoot,
        eventsEnabled: true,
      });
      const download = await waitUntil(() => [...state.downloads.values()].find((value) =>
        value.tabId === tab && !existing.has(value.id) && value.status === "completed"),
      timeoutMs(command, 120_000), "download");
      return download.id;
    }
    case "playwright_download_path": {
      const state = backendState(backend);
      const id = String(command.download ?? command.download_id ?? "");
      const download = state.downloads.get(id);
      if (
        download === undefined
        || download.tabId !== String(tabId(command))
        || download.status !== "completed"
      ) return null;
      let path = download.path;
      if (path === undefined) return null;
      if (download.filename !== undefined && basename(path) !== safeFilename(download.filename, id)) {
        const named = join(dirname(path), safeFilename(download.filename, id));
        await rename(path, named).catch(() => {});
        if (await stat(named).then(() => true, () => false)) path = named;
      }
      return path;
    }
  }
  throw new Error(`Unhandled Playwright command: ${command.type}`);
}

async function waitCommand(backend: RawBackend, command: CommandEnvelope): Promise<void> {
  if (command.type === "playwright_wait_for_timeout") {
    await new Promise((resolve) => setTimeout(resolve, Number(command.timeout_ms ?? 0)));
    return;
  }
  const timeout = Number(command.timeoutMs ?? command.timeout_ms ?? 10_000);
  const requestedLoadState = command.type === "playwright_wait_for_url"
    ? command.waitUntil
    : command.state;
  const networkIdle = requestedLoadState === "networkidle"
    ? await startNetworkIdleTracker(backend, command)
    : undefined;
  const started = Date.now();
  while (Date.now() - started < timeout) {
    let ready = false;
    if (command.type === "playwright_locator_wait_for") {
      const counts = object(await locatorValue(backend, command, "return {attached:nodes.length,visible:nodes.filter(visible).length};"));
      const attached = Number(counts.attached);
      const visibleCount = Number(counts.visible);
      const state = String(command.state ?? "visible");
      ready = state === "attached" ? attached > 0
        : state === "detached" ? attached === 0
        : state === "hidden" ? attached === 0 || visibleCount === 0
        : visibleCount > 0;
    } else if (command.type === "playwright_wait_for_url") {
      const url = String(await evaluate(backend, tabId(command), "location.href"));
      ready = urlMatches(url, String(command.url));
      if (ready && command.waitUntil !== undefined) {
        ready = await loadStateReady(backend, command, String(command.waitUntil), networkIdle);
      }
    } else {
      ready = await loadStateReady(backend, command, String(command.state ?? "load"), networkIdle);
    }
    if (ready) return;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`${command.type} timed out`);
}

type NetworkIdleTracker = {
  cursor: number;
  inflight: Set<string>;
  quietSince: number | undefined;
};

async function startNetworkIdleTracker(
  backend: RawBackend,
  command: CommandEnvelope,
): Promise<NetworkIdleTracker> {
  const state = backendState(backend);
  await cdp(backend, tabId(command), "Network.enable");
  return { cursor: state.nextSequence - 1, inflight: new Set(), quietSince: undefined };
}

async function loadStateReady(
  backend: RawBackend,
  command: CommandEnvelope,
  requested: string,
  networkIdle?: NetworkIdleTracker,
): Promise<boolean> {
  if (!["domcontentloaded", "load", "networkidle"].includes(requested)) {
    throw new Error(`Unsupported load state: ${requested}`);
  }
  if (requested === "networkidle") {
    if (networkIdle === undefined) throw new Error("networkidle tracker was not initialized");
    const state = backendState(backend);
    const events = state.cdpEvents.get(String(tabId(command))) ?? [];
    for (const event of events) {
      if (event.sequence <= networkIdle.cursor) continue;
      networkIdle.cursor = event.sequence;
      const requestId = String(event.params?.requestId ?? "");
      if (requestId === "") continue;
      if (event.method === "Network.requestWillBeSent") {
        networkIdle.inflight.add(requestId);
        networkIdle.quietSince = undefined;
      } else if (
        event.method === "Network.loadingFinished"
        || event.method === "Network.loadingFailed"
      ) {
        networkIdle.inflight.delete(requestId);
      }
    }
    const readyState = String(await evaluate(backend, tabId(command), "document.readyState"));
    if (readyState !== "complete" || networkIdle.inflight.size !== 0) {
      networkIdle.quietSince = undefined;
      return false;
    }
    networkIdle.quietSince ??= Date.now();
    return Date.now() - networkIdle.quietSince >= 500;
  }
  const state = String(await evaluate(backend, tabId(command), "document.readyState"));
  return requested === "domcontentloaded"
    ? state === "interactive" || state === "complete"
    : state === "complete";
}

function urlMatches(actual: string, expected: string): boolean {
  if (!expected.includes("*")) return actual === expected;
  const pattern = expected
    .split("*")
    .map((part) => part.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&"))
    .join(".*");
  return new RegExp(`^${pattern}$`, "u").test(actual);
}

async function waitUntil<T>(read: () => T | undefined, timeoutMs: number, label: string): Promise<T> {
  const started = Date.now();
  while (Date.now() - started <= timeoutMs) {
    const value = read();
    if (value !== undefined) return value;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`${label} timed out after ${timeoutMs}ms`);
}

function timeoutMs(command: CommandEnvelope, fallback = 10_000): number {
  const value = Number(command.timeoutMs ?? command.timeout_ms ?? fallback);
  return Number.isFinite(value) && value >= 0 ? value : fallback;
}

function safeFilename(value: string, fallback: string): string {
  const cleaned = basename(value).replace(/[^A-Za-z0-9._-]+/gu, "_").replace(/^\.+/u, "");
  return cleaned === "" ? fallback : cleaned.slice(0, 160);
}

async function artifactDirectory(prefix: string): Promise<string> {
  return mkdtemp(join(tmpdir(), `${prefix}-`));
}

async function sharedDownloadDirectory(): Promise<string> {
  const uid = typeof process.getuid === "function" ? process.getuid() : "user";
  const root = join(tmpdir(), `sky-cua-browser-downloads-${uid}`);
  await mkdir(root, { recursive: true, mode: 0o700 });
  if (!(await stat(root)).isDirectory()) {
    throw new Error(`Browser download root is not a directory: ${root}`);
  }
  return root;
}

function decodedBase64(value: unknown): Uint8Array {
  return Uint8Array.from(Buffer.from(String(value ?? ""), "base64"));
}

async function grantClipboardPermissions(backend: RawBackend, command: CommandEnvelope): Promise<void> {
  const origin = String(await evaluate(backend, tabId(command), "location.origin"));
  await cdp(backend, tabId(command), "Browser.grantPermissions", {
    origin,
    permissions: ["clipboardReadWrite", "clipboardSanitizedWrite"],
  });
}

async function tabDetails(backend: RawBackend, tab: string | number): Promise<ObjectValue> {
  const raw = await backend.raw("getTabs");
  const tabs = tabsFromRaw(raw);
  return tabs.find((value) => String(value.id ?? value.tabId) === String(tab)) ?? {};
}

function tabsFromRaw(value: unknown): ObjectValue[] {
  return asArray(Array.isArray(value) ? value : object(value).tabs).map(object);
}

async function elementInfo(backend: RawBackend, command: CommandEnvelope): Promise<unknown[]> {
  const x = Number(command.x), y = Number(command.y);
  const value = await evaluate(backend, tabId(command), `(()=>{const node=document.elementFromPoint(${x},${y});if(!node)return [];const r=node.getBoundingClientRect();return [{tagName:node.tagName.toLowerCase(),preview:node.outerHTML.slice(0,300),visibleText:(node.innerText||node.textContent||'').trim(),role:node.getAttribute('role'),ariaName:node.getAttribute('aria-label'),testId:node.getAttribute('data-testid'),boundingBox:{x:r.x,y:r.y,width:r.width,height:r.height},selector:{primary:null,candidates:[]}}]})()`);
  return Array.isArray(value) ? value : [];
}

async function exportTabContent(backend: RawBackend, command: CommandEnvelope): Promise<string> {
  const payload = object(await evaluate(backend, tabId(command), `(()=>({html:'<!doctype html>\\n'+document.documentElement.outerHTML,title:document.title,url:location.href}))()`));
  const root = await artifactDirectory("sky-cua-content");
  const path = join(root, `${safeFilename(String(payload.title ?? "page"), "page")}.html`);
  await writeFile(path, String(payload.html ?? ""), "utf8");
  await writeFile(join(root, "source.json"), `${JSON.stringify({ url: payload.url ?? null }, null, 2)}\n`, "utf8");
  return path;
}

async function exportGsuiteContent(backend: RawBackend, command: CommandEnvelope): Promise<string> {
  const details = await tabDetails(backend, tabId(command));
  const url = new URL(String(details.url ?? ""));
  if (url.hostname !== "docs.google.com") throw new Error("Tab is not a Google Workspace document");
  const segments = url.pathname.split("/").filter(Boolean);
  const type = segments[0];
  const marker = segments.indexOf("d");
  const documentId = marker >= 0 ? segments[marker + 1] : undefined;
  if (!documentId || !["document", "spreadsheets", "presentation"].includes(String(type))) {
    throw new Error("Unable to identify Google Workspace document");
  }
  const format = String(command.export_type ?? command.format ?? "pdf");
  const allowed: Record<string, string[]> = {
    document: ["pdf", "md", "docx"],
    spreadsheets: ["pdf", "xlsx", "csv"],
    presentation: ["pdf", "pptx"],
  };
  if (!allowed[String(type)]?.includes(format)) throw new Error("GSuite export type is not supported for this tab");
  const exportUrl = type === "document"
    ? `https://docs.google.com/document/d/${documentId}/export?format=${format === "md" ? "txt" : format}`
    : type === "spreadsheets"
    ? `https://docs.google.com/spreadsheets/d/${documentId}/export?format=${format}`
    : `https://docs.google.com/presentation/d/${documentId}/export/${format}`;
  const fetched = object(await evaluate(backend, tabId(command), `(async()=>{const response=await fetch(${js(exportUrl)});if(!response.ok)throw new Error('GSuite export HTTP '+response.status);const bytes=new Uint8Array(await response.arrayBuffer());let binary='';for(let i=0;i<bytes.length;i+=32768)binary+=String.fromCharCode(...bytes.subarray(i,i+32768));return{base64:btoa(binary)}})()`));
  const root = await artifactDirectory("sky-cua-gsuite");
  const path = join(root, `${safeFilename(String(details.title ?? "document"), "document")}.${format}`);
  await writeFile(path, decodedBase64(fetched.base64));
  return path;
}

function assetKind(url: string, initiator = ""): string {
  const extension = extname(new URL(url, "https://invalid.local").pathname).toLowerCase();
  if ([".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".avif"].includes(extension) || initiator === "img") return "image";
  if ([".woff", ".woff2", ".ttf", ".otf"].includes(extension) || initiator === "font") return "font";
  if ([".css"].includes(extension) || initiator === "css" || initiator === "link") return "stylesheet";
  if ([".mp4", ".webm", ".mov"].includes(extension) || initiator === "video") return "video";
  if ([".js", ".mjs"].includes(extension) || initiator === "script") return "script";
  return "other";
}

async function listPageAssets(backend: RawBackend, command: CommandEnvelope, state: BackendState): Promise<ObjectValue> {
  const raw = object(await evaluate(backend, tabId(command), `(()=>{const entries=[...performance.getEntriesByType('resource')].map(e=>({url:e.name,initiator:e.initiatorType||'resource'}));for(const node of document.querySelectorAll('img[src],script[src],link[href],video[src],source[src]')){const url=node.src||node.href;if(url)entries.push({url,initiator:node.tagName.toLowerCase()})}return{pageUrl:location.href,entries,inlineSvgs:[...document.querySelectorAll('svg')].slice(0,100).map((svg,index)=>({id:'svg-'+(index+1),name:svg.getAttribute('aria-label')||svg.id||'inline-svg-'+(index+1),markup:svg.outerHTML}))}})()`));
  const seen = new Set<string>();
  const assets = asArray(raw.entries).map(object).flatMap((entry) => {
    const url = String(entry.url ?? "");
    if (url === "" || seen.has(url)) return [];
    seen.add(url);
    const kind = assetKind(url, String(entry.initiator ?? ""));
    return [{
      id: createHash("sha256").update(url).digest("hex").slice(0, 24),
      kind,
      name: safeFilename(decodeURIComponent(new URL(url, String(raw.pageUrl ?? "https://invalid.local")).pathname), kind),
      sources: [{ kind: "resource" }],
      url,
    }];
  });
  const inlineSvgs = asArray(raw.inlineSvgs).map(object);
  const byKind = Object.fromEntries(["font", "image", "script", "stylesheet", "video", "other"].map((kind) =>
    [kind, assets.filter((asset) => asset.kind === kind).length]));
  const inventory = {
    assets,
    id: randomUUID(),
    inlineSvgs,
    pageUrl: typeof raw.pageUrl === "string" ? raw.pageUrl : null,
    summary: { byKind, inlineSvgCount: inlineSvgs.length, totalCount: assets.length },
  };
  state.inventories.set(inventory.id, inventory);
  return inventory;
}

async function bundlePageAssets(backend: RawBackend, command: CommandEnvelope, state: BackendState): Promise<ObjectValue> {
  const inventoryId = String(command.inventoryId ?? command.inventory_id ?? "");
  const inventory = state.inventories.get(inventoryId);
  if (inventory === undefined) throw new Error(`Unknown page asset inventory: ${inventoryId}`);
  const requestedIds = Array.isArray(command.assetIds ?? command.asset_ids)
    ? new Set(asArray(command.assetIds ?? command.asset_ids).map(String))
    : undefined;
  const requestedKinds = Array.isArray(command.kinds) ? new Set(command.kinds.map(String)) : undefined;
  const requested = asArray(inventory.assets).map(object).filter((asset) =>
    (requestedIds === undefined || requestedIds.has(String(asset.id)))
    && (requestedKinds === undefined || requestedKinds.has(String(asset.kind))));
  const started = Date.now();
  const root = await artifactDirectory("sky-cua-assets");
  const assets: ObjectValue[] = [];
  const failures: ObjectValue[] = [];
  for (const asset of requested) {
    try {
      const fetched = object(await evaluate(backend, tabId(command), `(async()=>{const response=await fetch(${js(asset.url)});if(!response.ok)throw new Error('HTTP '+response.status);const bytes=new Uint8Array(await response.arrayBuffer());let binary='';for(let i=0;i<bytes.length;i+=32768)binary+=String.fromCharCode(...bytes.subarray(i,i+32768));return{base64:btoa(binary),contentType:response.headers.get('content-type')}})()`));
      const extension = extname(new URL(String(asset.url)).pathname);
      const filename = safeFilename(String(asset.name), String(asset.id));
      const path = join(
        root,
        `${String(asset.id)}-${filename}${extension && !filename.endsWith(extension) ? extension : ""}`,
      );
      await writeFile(path, decodedBase64(fetched.base64));
      assets.push({ ...asset, contentType: fetched.contentType ?? null, path });
    } catch (error) {
      failures.push({
        id: asset.id,
        name: asset.name,
        reason: error instanceof Error ? error.message : String(error),
        url: asset.url,
      });
    }
  }
  const manifestPath = join(root, "manifest.json");
  const result = {
    assets,
    directoryPath: root,
    failures,
    manifestPath,
    summary: {
      downloadedCount: assets.length,
      elapsedMs: Date.now() - started,
      failedCount: failures.length,
      requestedCount: requested.length,
    },
  };
  await writeFile(manifestPath, `${JSON.stringify(result, null, 2)}\n`, "utf8");
  return result;
}

export async function executeBrowserCommand(backend: RawBackend, command: CommandEnvelope): Promise<unknown> {
  const state = backendState(backend);
  switch (command.type) {
    case "browser_user_open_tabs": return backend.raw("getUserTabs");
    case "browser_user_history": return backend.raw("getUserHistory", object(command.options));
    case "browser_user_claim_tab": {
      const value = typeof command.tab === "string" ? command.tab : object(command.tab).id;
      if (typeof value !== "string" && typeof value !== "number") {
        throw new Error("browser_user_claim_tab requires a tab id");
      }
      const tabId = rawTabId(value);
      try {
        return await attachClaimedTab(backend, await backend.raw("claimUserTab", { tabId }));
      } catch (error) {
        if (object(command.options).reclaimStale !== true) throw error;
        const staleSessionId = staleNodeReplOwner(error);
        if (staleSessionId === undefined) throw error;
        if (backend.finalizeStaleNodeReplSession === undefined) {
          throw new Error("Browser backend cannot reclaim stale node_repl sessions", { cause: error });
        }
        await backend.finalizeStaleNodeReplSession(staleSessionId);
        return attachClaimedTab(backend, await backend.raw("claimUserTab", { tabId }));
      }
    }
    case "create_tab": return attachClaimedTab(backend, await backend.raw("createTab"));
    case "list_tabs": return tabsFromRaw(await backend.raw("getTabs"));
    case "selected_tab": return tabsFromRaw(await backend.raw("getTabs")).find((value) => value.active === true);
    case "close_tab": {
      const tab = tabId(command);
      const targets = object(await cdp(backend, tab, "Target.getTargets")).targetInfos;
      const target = asArray(targets).map(object).find((value) => String(value.tabId) === String(tab));
      if (typeof target?.targetId === "string") {
        return cdp(backend, tab, "Target.closeTarget", { targetId: target.targetId });
      }
      return cdp(backend, tab, "Page.close");
    }
    case "finalize_tabs": {
      const keep = (Array.isArray(command.keep) ? command.keep : []).map((value) => {
        const entry = object(value);
        const status = String(entry.status);
        if (status !== "deliverable" && status !== "handoff") {
          throw new Error(`finalize_tabs has unsupported status: ${status}`);
        }
        const tabValue = entry.tab;
        if (typeof tabValue !== "string" && typeof tabValue !== "number") {
          throw new Error("finalize_tabs keep entry requires a tab id");
        }
        return { tabId: rawTabId(tabValue), status };
      });
      await backend.raw("finalizeTabs", { keep });
      return undefined;
    }
    case "name_session": {
      const name = String(command.name ?? "").trim();
      if (name === "") throw new Error("name_session requires a name");
      await backend.raw("nameSession", { name });
      return undefined;
    }
    case "navigate_tab_url": return cdp(backend, tabId(command), "Page.navigate", { url: command.url });
    case "navigate_tab_reload": return cdp(backend, tabId(command), "Page.reload");
    case "navigate_tab_back": return cdp(backend, tabId(command), "Runtime.evaluate", { expression: "history.back()" });
    case "navigate_tab_forward": return cdp(backend, tabId(command), "Runtime.evaluate", { expression: "history.forward()" });
    case "tab_screenshot": {
      const options = object(command.options);
      return cdp(backend, tabId(command), "Page.captureScreenshot", {
        format: "webp",
        ...(options.clip === undefined ? {} : { clip: { ...object(options.clip), scale: 1 } }),
        ...(options.fullPage === true ? { captureBeyondViewport: true } : {}),
      });
    }
    case "tab_cdp_call": {
      const target = object(command.target);
      const rawTarget: ObjectValue = { tabId: rawTabId(tabId(command)) };
      if (typeof target.session_id === "string") rawTarget.sessionId = target.session_id;
      if (typeof target.target_id === "string") rawTarget.targetId = target.target_id;
      return backend.raw("executeCdp", {
        target: rawTarget,
        method: String(command.method),
        commandParams: object(command.params),
        ...(command.timeout_ms === undefined ? {} : { timeoutMs: command.timeout_ms }),
      });
    }
    case "tab_cdp_events": {
      const tab = String(tabId(command));
      const after = Number(command.after_sequence ?? 0);
      const limit = Math.min(1_000, Math.max(1, Number(command.limit ?? 100)));
      const methods = Array.isArray(command.methods) ? new Set(command.methods.map(String)) : undefined;
      const target = object(command.target);
      const read = () => (state.cdpEvents.get(tab) ?? []).filter((event) =>
        event.sequence > after
        && (methods === undefined || methods.has(event.method))
        && (typeof target.session_id !== "string" || event.source.sessionId === target.session_id)
        && (typeof target.target_id !== "string" || event.source.targetId === target.target_id));
      let matching = read();
      if (matching.length === 0 && timeoutMs(command, 0) > 0) {
        matching = await waitUntil(() => {
          const events = read();
          return events.length === 0 ? undefined : events;
        }, timeoutMs(command, 0), "CDP events").catch(() => []);
      }
      const events = matching.slice(0, limit);
      return {
        cursor: events.at(-1)?.sequence ?? Math.max(after, state.nextSequence - 1),
        events,
        hasMore: matching.length > events.length,
        truncated: after > 0 && (state.cdpEvents.get(tab)?.[0]?.sequence ?? after) > after + 1,
      };
    }
    case "tab_get_js_dialog": {
      await cdp(backend, tabId(command), "Page.enable");
      const dialog = state.dialogs.get(String(tabId(command)));
      return dialog === undefined ? null : { id: dialog.id, type: dialog.type };
    }
    case "tab_handle_js_dialog": {
      const tab = String(tabId(command));
      const dialog = state.dialogs.get(tab);
      if (dialog === undefined || dialog.id !== String(command.dialog_id ?? "")) {
        throw new Error("JavaScript dialog is no longer active");
      }
      const target: ObjectValue = { tabId: rawTabId(tabId(command)) };
      if (typeof dialog.target.sessionId === "string") target.sessionId = dialog.target.sessionId;
      if (typeof dialog.target.targetId === "string") target.targetId = dialog.target.targetId;
      await backend.raw("executeCdp", {
        target,
        method: "Page.handleJavaScriptDialog",
        commandParams: {
          accept: command.action === "accept",
          ...(command.prompt_text === undefined ? {} : { promptText: command.prompt_text }),
        },
      });
      state.dialogs.delete(tab);
      return undefined;
    }
    case "tab_dev_logs": {
      await cdp(backend, tabId(command), "Runtime.enable");
      const options = object(command.options);
      const levels = Array.isArray(options.levels) ? new Set(options.levels.map(normalizeLogLevel)) : undefined;
      const filter = typeof options.filter === "string" ? options.filter : undefined;
      const limit = Math.max(1, Number(options.limit ?? 100));
      return (state.logs.get(String(tabId(command))) ?? [])
        .filter((entry) => levels === undefined || levels.has(String(entry.level)))
        .filter((entry) => filter === undefined || String(entry.message).includes(filter))
        .slice(-limit);
    }
    case "tab_clipboard_read_text": {
      await grantClipboardPermissions(backend, command);
      return String(await evaluate(backend, tabId(command), "navigator.clipboard.readText()"));
    }
    case "tab_clipboard_write_text": {
      await grantClipboardPermissions(backend, command);
      await evaluate(backend, tabId(command), `navigator.clipboard.writeText(${js(command.text)})`);
      return undefined;
    }
    case "tab_clipboard_read": {
      await grantClipboardPermissions(backend, command);
      return asArray(await evaluate(backend, tabId(command), `(async()=>Promise.all((await navigator.clipboard.read()).map(async(item)=>({presentationStyle:item.presentationStyle,entries:await Promise.all(item.types.map(async(mimeType)=>{const blob=await item.getType(mimeType);const bytes=new Uint8Array(await blob.arrayBuffer());let binary='';for(let i=0;i<bytes.length;i+=32768)binary+=String.fromCharCode(...bytes.subarray(i,i+32768));return mimeType.startsWith('text/')?{mimeType,text:await blob.text()}:{mimeType,base64:btoa(binary)}}))}))))()`));
    }
    case "tab_clipboard_write": {
      await grantClipboardPermissions(backend, command);
      await evaluate(backend, tabId(command), `(async()=>{const items=${js(command.items)};await navigator.clipboard.write(items.map(item=>new ClipboardItem(Object.fromEntries(item.entries.map(entry=>[entry.mimeType,new Blob([entry.text!==undefined?entry.text:Uint8Array.from(atob(entry.base64||''),c=>c.charCodeAt(0))],{type:entry.mimeType})])),{presentationStyle:item.presentationStyle})))})()`);
      return undefined;
    }
    case "tab_content_export": return exportTabContent(backend, command);
    case "tab_content_export_gsuite": return exportGsuiteContent(backend, command);
    case "tab_page_assets_list": return listPageAssets(backend, command, state);
    case "tab_page_assets_bundle": return bundlePageAssets(backend, command, state);
    case "cua_move": return mouse(backend, tabId(command), "mouseMoved", Number(command.x), Number(command.y));
    case "cua_click": return click(backend, tabId(command), Number(command.x), Number(command.y), 1, button(command.button));
    case "cua_double_click": return click(backend, tabId(command), Number(command.x), Number(command.y), 2);
    case "cua_scroll": return mouse(backend, tabId(command), "mouseWheel", Number(command.x), Number(command.y), { deltaX: command.scrollX, deltaY: command.scrollY });
    case "cua_type": return cdp(backend, tabId(command), "Input.insertText", { text: command.text });
    case "cua_keypress": {
      const keys = (Array.isArray(command.keys) ? command.keys : []).map(String);
      let modifiers = 0;
      for (const key of keys) {
        modifiers |= modifierMask(key);
        await cdp(backend, tabId(command), "Input.dispatchKeyEvent", {
          type: "keyDown",
          key,
          modifiers,
        });
      }
      for (const key of [...keys].reverse()) {
        modifiers &= ~modifierMask(key);
        await cdp(backend, tabId(command), "Input.dispatchKeyEvent", {
          type: "keyUp",
          key,
          modifiers,
        });
      }
      return undefined;
    }
    case "cua_drag": {
      const points = Array.isArray(command.path) ? command.path.map(object) : [];
      if (points.length < 2) throw new Error("cua_drag requires at least two points");
      const first = points[0]!;
      await mouse(backend, tabId(command), "mousePressed", Number(first.x), Number(first.y), { button: "left" });
      for (const point of points.slice(1)) await mouse(backend, tabId(command), "mouseMoved", Number(point.x), Number(point.y), { button: "left" });
      const last = points.at(-1)!;
      return mouse(backend, tabId(command), "mouseReleased", Number(last.x), Number(last.y), { button: "left" });
    }
    case "dom_cua_get_visible_dom": return evaluate(backend, tabId(command), `(()=>{let i=0;return [...document.querySelectorAll('a,button,input,select,textarea,[role],[tabindex]')].filter((node)=>{const r=node.getBoundingClientRect();return r.width>0&&r.height>0}).map((node)=>{const id='sky-'+(++i);node.setAttribute('data-sky-cua-node-id',id);const r=node.getBoundingClientRect();return {node_id:id,tag:node.tagName.toLowerCase(),text:(node.innerText||node.value||node.getAttribute('aria-label')||'').trim(),bounds:{x:r.x,y:r.y,width:r.width,height:r.height}}})})()`);
    case "dom_cua_click":
    case "dom_cua_double_click": {
      const selector = `[data-sky-cua-node-id=${JSON.stringify(String(command.node_id))}]`;
      const point = object(await evaluate(backend, tabId(command), `(()=>{const n=document.querySelector(${js(selector)});if(!n)throw new Error('DOM node is stale or missing');const r=n.getBoundingClientRect();return {x:r.left+r.width/2,y:r.top+r.height/2}})()`));
      return click(backend, tabId(command), Number(point.x), Number(point.y), command.type === "dom_cua_double_click" ? 2 : 1);
    }
    case "dom_cua_type": return cdp(backend, tabId(command), "Input.insertText", { text: command.text });
    case "dom_cua_keypress": return executeBrowserCommand(backend, { ...command, type: "cua_keypress" });
    case "dom_cua_scroll": {
      const selector = `[data-sky-cua-node-id=${JSON.stringify(String(command.node_id))}]`;
      return cdp(backend, tabId(command), "Runtime.evaluate", { expression: command.node_id ? `document.querySelector(${js(selector)})?.scrollBy(${Number(command.x)},${Number(command.y)})` : `scrollBy(${Number(command.x)},${Number(command.y)})` });
    }
    case "tab_id": return tabId(command);
    case "cua_download_media": return click(backend, tabId(command), Number(command.x), Number(command.y));
    case "dom_cua_download_media": return executeBrowserCommand(backend, { ...command, type: "dom_cua_click" });
    case "tab_bot_detection_report": {
      const hostname = String(await evaluate(backend, tabId(command), "location.hostname"));
      return backend.raw("reportBotDetection", {
        tabId: rawTabId(tabId(command)),
        reason: String(command.reason),
        hostname,
      });
    }
    case "tab_browser_auth_handoff": {
      const { tab_id: _tabId, browser_id: _browserId, type: _type, ...request } = command;
      return backend.raw("browserAuthHandoff", { tabId: rawTabId(tabId(command)), ...request });
    }
    default:
      if (command.type.startsWith("playwright_")) return playwrightCommand(backend, command);
      return unsupported(command);
  }
}

function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}
