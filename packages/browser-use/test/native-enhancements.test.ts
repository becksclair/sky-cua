import { strict as assert } from "node:assert";
import { test } from "bun:test";
import { createSkyBrowserRuntime, decorateSkyTab, type NativeNavigationTab, type NativeSetupOptions, type SkyTabEnhancements } from "../src/native-enhancements.ts";

type Event = { sequence: number; method: string; params: Record<string, unknown> };
function fixture() {
  const events: Event[] = [];
  const reads: Array<Record<string, unknown>> = [];
  const loadTimeouts: number[] = [];
  let sequence = 0, truncated = false, setupDelay = 0, loadDelay = 0, actionRan = false;
  const cdp = {
    async send(method: string, _params: unknown, options: { timeoutMs: number }) {
      assert.ok(Number.isSafeInteger(options.timeoutMs));
      if (setupDelay) await Bun.sleep(setupDelay);
      return method === "Page.getFrameTree" ? { frameTree: { frame: { id: "main" } } } : {};
    },
    async readEvents(options: Record<string, unknown>) {
      if (options.timeoutMs !== undefined) assert.ok(Number.isSafeInteger(options.timeoutMs));
      reads.push(options);
      if (options.afterSequence === undefined) return { cursor: sequence, events: [], hasMore: false, truncated: false };
      await Bun.sleep(2);
      return { cursor: sequence, events: events.filter(e => e.sequence > Number(options.afterSequence)), hasMore: false, truncated };
    },
  };
  const tab = {
    capabilities: { async get(id: string) { assert.equal(id, "cdp"); return cdp; } },
    playwright: {
      expectNavigation: async () => "native",
      async waitForURL(url: string, options: { timeoutMs: number }) {
        assert.equal(url, "https://fixture.test/next"); loadTimeouts.push(options.timeoutMs);
        if (loadDelay) await Bun.sleep(loadDelay);
      },
      async waitForLoadState(options: { timeoutMs: number; state: string }) {
        loadTimeouts.push(options.timeoutMs); assert.equal(options.state, "load");
        if (loadDelay) await Bun.sleep(loadDelay);
      },
    },
  };
  decorateSkyTab(tab);
  const enhanced = tab as typeof tab & { sky: SkyTabEnhancements };
  const emit = (method = "Page.frameNavigated", frame = "main", child = false) => events.push({
    sequence: ++sequence, method,
    params: method === "Page.navigatedWithinDocument" ? { frameId: frame } : { frame: { id: frame, ...(child ? { parentId: "main" } : {}) } },
  });
  return { tab: enhanced, emit, reads, loadTimeouts, cdp,
    setupDelay: (v: number) => { setupDelay = v; }, loadDelay: (v: number) => { loadDelay = v; },
    truncate: () => { truncated = true; }, action: () => { actionRan = true; }, actionRan: () => actionRan };
}

test("native Agent, setup options, existing decorator and native methods are preserved", async () => {
  const f = fixture(), agent = { browsers: { list: async () => ["iab", "mcpapps"] } };
  let count = 0, forwarded: NativeSetupOptions | undefined;
  const setup = createSkyBrowserRuntime(async options => { forwarded = options; options.decorateTab?.(f.tab); return agent; });
  assert.equal(await setup({ environment: "codex-app", undocumentedApiMembers: ["Tab.title"], decorateTab(tab) { assert.equal(tab, f.tab); count++; } }), agent);
  assert.deepEqual(await agent.browsers.list(), ["iab", "mcpapps"]);
  assert.equal(count, 1); assert.equal(forwarded?.environment, "codex-app");
  assert.deepEqual(forwarded?.undocumentedApiMembers, ["Tab.title"]);
  assert.equal(await f.tab.playwright.expectNavigation(), "native");
  const sky = f.tab.sky; decorateSkyTab(f.tab); assert.equal(f.tab.sky, sky);
});
for (const method of ["Page.frameNavigated", "Page.navigatedWithinDocument"]) {
  test(`fresh top-level ${method} returns the action result`, async () => {
    const f = fixture(), value = { success: true };
    assert.equal(await f.tab.sky.expectNavigation(() => {
      assert.ok(f.reads.length >= 2, "observation starts before action"); f.emit(method); return value;
    }, { timeoutMs: 200, url: "https://fixture.test/next" }), value);
    assert.equal(f.loadTimeouts.length, 2);
    assert.ok(f.loadTimeouts.every(v => v > 0 && v <= 200));
    assert.ok(f.reads.every(v => JSON.stringify(v.methods) === JSON.stringify(["Page.frameNavigated", "Page.navigatedWithinDocument"])));
  });
}
for (const kind of ["no-op", "already-buffered", "child-frame"]) {
  test(`${kind} does not satisfy a fresh main-frame navigation wait`, async () => {
    const f = fixture(); if (kind === "already-buffered") f.emit();
    await assert.rejects(f.tab.sky.expectNavigation(() => {
      if (kind === "child-frame") { f.emit("Page.frameNavigated", "child", true); f.emit("Page.navigatedWithinDocument", "child"); }
    }, { timeoutMs: 35 }), /timed out/);
    const count = f.reads.length; await Bun.sleep(25); assert.equal(f.reads.length, count);
  });
}
test("truncated history rejects even when it contains a matching navigation", async () => {
  const f = fixture();
  await assert.rejects(f.tab.sky.expectNavigation(() => { f.emit(); f.truncate(); }, { timeoutMs: 200 }), /history was truncated/);
});
test("original action rejection propagates and stops polling", async () => {
  const f = fixture(), failure = new Error("fixture action failed");
  await assert.rejects(f.tab.sky.expectNavigation(() => { throw failure; }, { timeoutMs: 200 }), e => e === failure);
  const count = f.reads.length; await Bun.sleep(25); assert.equal(f.reads.length, count);
});
test("deadline includes setup and expired setup never invokes the action", async () => {
  const f = fixture(); f.setupDelay(25);
  await assert.rejects(f.tab.sky.expectNavigation(f.action, { timeoutMs: 35 }), /timed out/);
  assert.equal(f.actionRan(), false);
});
test("URL and load waits share the original deadline", async () => {
  const f = fixture(); f.loadDelay(35);
  await assert.rejects(f.tab.sky.expectNavigation(() => f.emit(), { timeoutMs: 50, url: "https://fixture.test/next" }), /timed out/);
  assert.equal(f.loadTimeouts.length, 2); assert.ok(f.loadTimeouts[1]! < f.loadTimeouts[0]! - 20);
});
test("timeout stops observation without claiming to cancel the action", async () => {
  const f = fixture(); let finished = false;
  await assert.rejects(f.tab.sky.expectNavigation(async () => { await Bun.sleep(60); finished = true; }, { timeoutMs: 20 }), /timed out/);
  assert.equal(finished, false); await Bun.sleep(60); assert.equal(finished, true);
});
test("native capability and origin errors propagate before the action", async () => {
  const f = fixture(), denied = new Error("native origin authority denied");
  f.tab.capabilities.get = async () => { throw denied; };
  await assert.rejects(f.tab.sky.expectNavigation(f.action, { timeoutMs: 100 }), e => e === denied);
  assert.equal(f.actionRan(), false);
  f.tab.capabilities.get = async () => ({} as typeof f.cdp);
  await assert.rejects(f.tab.sky.expectNavigation(f.action, { timeoutMs: 100 }), /advertised native CDP/);
  assert.equal(f.actionRan(), false);
});
test("namespace collisions and invalid timeout values fail explicitly", async () => {
  const f = fixture(); assert.throws(() => decorateSkyTab({ ...f.tab, sky: {} } as NativeNavigationTab), /already defined/);
  for (const timeoutMs of [-1, NaN, Infinity]) await assert.rejects(f.tab.sky.expectNavigation(f.action, { timeoutMs }), /non-negative finite/);
  assert.equal(f.actionRan(), false);
});
