/** Additive helpers for the maintained native Browser client. No transport of its own. */

type ObjectValue = Record<string, unknown>;

export type SkyNavigationOptions = {
  timeoutMs?: number;
  url?: string;
  waitUntil?: "load" | "domcontentloaded" | "networkidle";
};

type CdpEvent = {
  sequence: number;
  method: string;
  params?: ObjectValue;
};

type CdpBatch = {
  cursor: number;
  events: CdpEvent[];
  hasMore: boolean;
  truncated: boolean;
};

type CdpCapability = {
  send(method: string, params?: ObjectValue, options?: { timeoutMs: number }): Promise<unknown>;
  readEvents(options: {
    methods: string[];
    limit: number;
    afterSequence?: number;
    timeoutMs?: number;
  }): Promise<CdpBatch>;
};

export type NativeNavigationTab = {
  capabilities: { get(id: string): Promise<unknown> };
  playwright: {
    waitForURL(url: string, options: { timeoutMs: number; waitUntil?: string }): Promise<void>;
    waitForLoadState(options: { timeoutMs: number; state: string }): Promise<void>;
  };
};

export type SkyTabEnhancements = {
  expectNavigation<T>(action: () => T | Promise<T>, options?: SkyNavigationOptions): Promise<T>;
};

export type NativeSetupOptions = {
  decorateTab?: (tab: NativeNavigationTab) => void;
  [key: string]: unknown;
};

const NAVIGATION_METHODS = ["Page.frameNavigated", "Page.navigatedWithinDocument"];
const decoratedTabs = new WeakSet<object>();

function object(value: unknown): ObjectValue {
  return value !== null && typeof value === "object" ? value as ObjectValue : {};
}

function remaining(deadline: number, timeoutMs: number): number {
  const value = deadline - performance.now();
  if (value <= 0) throw new Error(`Sky expectNavigation timed out after ${timeoutMs}ms`);
  // Native Browser command timeouts are integer milliseconds. The enclosing
  // deadline race still bounds the whole operation, including this rounding.
  return Math.ceil(value);
}

async function withinDeadline<T>(work: Promise<T>, deadline: number, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      work,
      new Promise<never>((_resolve, reject) => {
        timer = setTimeout(() => reject(new Error(`Sky expectNavigation timed out after ${timeoutMs}ms`)),
          Math.max(0, deadline - performance.now()));
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

function cursorOf(batch: CdpBatch): number {
  if (batch.truncated) throw new Error("Sky expectNavigation lost navigation evidence: CDP event history was truncated");
  if (!Number.isSafeInteger(batch.cursor) || batch.cursor < 0) {
    throw new Error("Sky expectNavigation received an invalid CDP event cursor");
  }
  return batch.cursor;
}

async function expectNavigation<T>(
  tab: NativeNavigationTab,
  action: () => T | Promise<T>,
  options: SkyNavigationOptions = {},
): Promise<T> {
  const timeoutMs = options.timeoutMs ?? 30_000;
  if (!Number.isFinite(timeoutMs) || timeoutMs < 0) {
    throw new Error("Sky expectNavigation timeoutMs must be a non-negative finite number");
  }
  if (typeof action !== "function") throw new TypeError("Sky expectNavigation requires an action callback");
  const deadline = performance.now() + timeoutMs;
  const cdp = await withinDeadline(tab.capabilities.get("cdp"), deadline, timeoutMs) as CdpCapability;
  if (typeof cdp?.send !== "function" || typeof cdp?.readEvents !== "function") {
    throw new Error("Sky expectNavigation requires the advertised native CDP capability");
  }
  await withinDeadline(cdp.send("Page.enable", {}, { timeoutMs: remaining(deadline, timeoutMs) }), deadline, timeoutMs);
  const tree = object(await withinDeadline(cdp.send("Page.getFrameTree", {}, {
    timeoutMs: remaining(deadline, timeoutMs),
  }), deadline, timeoutMs));
  const mainFrameId = object(object(tree.frameTree).frame).id;
  if (typeof mainFrameId !== "string" || mainFrameId === "") {
    throw new Error("Sky expectNavigation could not resolve the main frame");
  }
  let batch = await withinDeadline(cdp.readEvents({ methods: NAVIGATION_METHODS, limit: 1_000 }), deadline, timeoutMs);
  let cursor = cursorOf(batch);
  // The native API normally returns the current cursor without history here.
  // If it pages, finish capturing the pre-action position with the same filters.
  while (batch.hasMore) {
    remaining(deadline, timeoutMs);
    batch = await withinDeadline(cdp.readEvents({ methods: NAVIGATION_METHODS, limit: 1_000, afterSequence: cursor }), deadline, timeoutMs);
    cursor = cursorOf(batch);
  }
  let stopped = false;
  const navigation = (async () => {
    while (!stopped) {
      const availableMs = remaining(deadline, timeoutMs);
      const events = await cdp.readEvents({
        methods: NAVIGATION_METHODS,
        limit: 1_000,
        afterSequence: cursor,
        timeoutMs: Math.min(100, availableMs),
      });
      if (stopped) return;
      const nextCursor = cursorOf(events);
      if (events.events.some((event) => {
        if (event.sequence <= cursor) return false;
        const params = object(event.params);
        if (event.method === "Page.navigatedWithinDocument") return params.frameId === mainFrameId;
        const frame = object(params.frame);
        return event.method === "Page.frameNavigated" && frame.id === mainFrameId && frame.parentId === undefined;
      })) return;
      cursor = nextCursor;
      if (!events.hasMore) {
        await new Promise((resolve) => setTimeout(resolve, Math.min(20, remaining(deadline, timeoutMs))));
      }
    }
  })();
  try {
    const [result] = await withinDeadline(Promise.all([Promise.resolve().then(action), navigation]), deadline, timeoutMs);
    if (options.url !== undefined) {
      await withinDeadline(tab.playwright.waitForURL(options.url, {
        timeoutMs: remaining(deadline, timeoutMs),
        ...(options.waitUntil === undefined ? {} : { waitUntil: options.waitUntil }),
      }), deadline, timeoutMs);
    }
    await withinDeadline(tab.playwright.waitForLoadState({
      timeoutMs: remaining(deadline, timeoutMs),
      state: options.waitUntil ?? "load",
    }), deadline, timeoutMs);
    return result;
  } finally {
    stopped = true;
  }
}

export function decorateSkyTab(tab: NativeNavigationTab): void {
  if (decoratedTabs.has(tab)) return;
  if ("sky" in tab) throw new Error("Cannot install Sky Browser helpers: tab.sky is already defined");
  const helpers: SkyTabEnhancements = {
    expectNavigation: (action, options) => expectNavigation(tab, action, options),
  };
  Object.defineProperty(tab, "sky", { value: Object.freeze(helpers), enumerable: true, configurable: true });
  decoratedTabs.add(tab);
}

/** Preserve native setup, returned Agent, options and any caller decorator. */
export function createSkyBrowserRuntime<T>(
  setupNativeBrowserRuntime: (options: NativeSetupOptions) => Promise<T>,
): (options?: NativeSetupOptions) => Promise<T> {
  return async (options = {}) => setupNativeBrowserRuntime({
    ...options,
    decorateTab(tab) {
      decorateSkyTab(tab);
      options.decorateTab?.(tab);
    },
  });
}
