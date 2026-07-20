import {
  accessSync,
  constants,
  existsSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { gzipSync } from "node:zlib";
import {
  processPidsContaining,
  processTreePids,
  runBoundedCleanup,
  startMcpSession,
  stopProcessesContaining,
  toolText,
  type McpSession,
} from "./web-workbench-acceptance-helper";

type Evidence = Record<string, string | number | boolean>;
export type WebWorkbenchReport = {
  schema: "com.heliasar.cua-node.web-workbench-acceptance";
  schema_version: 1;
  status: "passed" | "blocked" | "failed";
  target: "linux-x64-glibc";
  fixture: "loopback-only";
  checks: Array<{
    id: string;
    status: "passed" | "failed";
    evidence: Evidence;
    error?: string;
  }>;
  blockers: string[];
  error?: string;
};
export type AcceptanceOptions = {
  runtimeRoot: string;
  browserExecutable?: string;
  timeoutMs: number;
  cycles: number;
};
export type LoopbackFixture = {
  origin: string;
  wsUrl: string;
  requests: number;
  close(): Promise<void>;
};

const SCHEMA = "com.heliasar.cua-node.web-workbench-acceptance" as const;
const HTML = `<!doctype html><html><body><input id="text"><input id="upload" type="file"><output id="upload-state"></output><button id="apply" onclick="out.textContent=text.value">Apply</button><div id="out"></div><a id="download" download="fixture.txt" href="/download">Download</a><script>const uploadInput=document.getElementById("upload");const uploadState=document.getElementById("upload-state");uploadInput.addEventListener("change",async()=>{const file=uploadInput.files?.[0];uploadState.textContent=file===undefined?"":JSON.stringify({name:file.name,text:await file.text()});uploadState.dataset.observed="true";});</script></body></html>`;

type PlaywrightEvidence = {
  cycles?: unknown;
  readback?: unknown;
  upload?: unknown;
  crashSignals?: unknown;
};

type CrashSignal = {
  disconnected: boolean;
  command: string;
};

export function validatePlaywrightEvidence(
  evidence: PlaywrightEvidence,
  cycles: number,
): { uploadObserved: boolean; crashSignals: CrashSignal[] } {
  if (evidence.cycles !== cycles || evidence.readback !== "PLAYWRIGHT!")
    throw new Error(
      `Playwright interaction evidence mismatch: ${JSON.stringify(evidence)}`,
    );
  const upload = evidence.upload;
  if (upload === null || typeof upload !== "object" || Array.isArray(upload))
    throw new Error("Playwright upload was not observed by the page");
  const uploadRecord = upload as Record<string, unknown>;
  if (
    uploadRecord.observed !== true ||
    uploadRecord.name !== "upload marker.txt" ||
    uploadRecord.text !== "UPLOAD MARKER"
  )
    throw new Error(
      `Playwright upload evidence mismatch: ${JSON.stringify(uploadRecord)}`,
    );
  if (!Array.isArray(evidence.crashSignals) || evidence.crashSignals.length !== cycles)
    throw new Error("Playwright crash evidence count mismatch");
  const crashSignals = evidence.crashSignals.map((signal, index) => {
    if (signal === null || typeof signal !== "object" || Array.isArray(signal))
      throw new Error(`Playwright crash cycle ${index + 1} returned no signal`);
    const record = signal as Record<string, unknown>;
    if (record.disconnected !== true || typeof record.command !== "string")
      throw new Error(
        `Playwright crash cycle ${index + 1} did not disconnect: ${JSON.stringify(record)}`,
      );
    return { disconnected: true, command: record.command };
  });
  return { uploadObserved: true, crashSignals };
}

function executable(path: string): boolean {
  try {
    accessSync(path, constants.R_OK | constants.X_OK);
    return true;
  } catch {
    return false;
  }
}
export async function startLoopbackFixture(): Promise<LoopbackFixture> {
  let requests = 0;
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request, bunServer) {
      requests += 1;
      const remote = bunServer.requestIP(request)?.address;
      if (remote !== "127.0.0.1" && remote !== "::1")
        return new Response("forbidden", { status: 403 });
      const url = new URL(request.url);
      if (url.pathname === "/ws" && bunServer.upgrade(request)) return;
      if (url.pathname === "/")
        return new Response(HTML, { headers: { "content-type": "text/html" } });
      if (url.pathname === "/text") return new Response("loopback text");
      if (url.pathname === "/json")
        return Response.json({ ok: true, source: "loopback" });
      if (url.pathname === "/bytes")
        return new Response(new Uint8Array([0, 1, 2, 253, 254, 255]));
      if (url.pathname === "/stream")
        return new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(new TextEncoder().encode("one-"));
              setTimeout(() => {
                controller.enqueue(new TextEncoder().encode("two"));
                controller.close();
              }, 20);
            },
          }),
        );
      if (url.pathname === "/gzip")
        return new Response(gzipSync("compressed loopback"), {
          headers: { "content-encoding": "gzip" },
        });
      if (url.pathname === "/delay") {
        await new Promise((resolvePromise) => setTimeout(resolvePromise, 1_000));
        return new Response("late");
      }
      if (url.pathname === "/download")
        return new Response("download payload", {
          headers: {
            "content-disposition": 'attachment; filename="fixture.txt"',
          },
        });
      if (url.pathname === "/upload" && request.method === "POST") {
        const body = Buffer.from(await request.arrayBuffer());
        return Response.json({
          bytes: body.length,
          has_marker: body.includes(Buffer.from("UPLOAD MARKER")),
        });
      }
      return new Response("missing", { status: 404 });
    },
    websocket: {
      message(socket, message) {
        socket.send(message);
      },
    },
  });
  const origin = `http://127.0.0.1:${server.port}`;
  return {
    origin,
    wsUrl: `ws://127.0.0.1:${server.port}/ws`,
    get requests() {
      return requests;
    },
    async close() {
      await server.stop(true);
    },
  };
}

function parseJsonLine(text: string, label: string): Record<string, unknown> {
  const line = text.trim().split("\n").at(-1);
  if (line === undefined) throw new Error(`${label} returned no JSON`);
  const value: unknown = JSON.parse(line);
  if (value === null || typeof value !== "object" || Array.isArray(value))
    throw new Error(`${label} returned non-object JSON`);
  return value as Record<string, unknown>;
}

export function parseAcceptanceArgs(argv: string[]): AcceptanceOptions {
  let runtimeRoot: string | undefined;
  let browserExecutable: string | undefined;
  let timeoutMs = 30_000;
  let cycles = 2;
  for (const argument of argv) {
    if (argument === "--json") continue;
    if (argument.startsWith("--runtime-root="))
      runtimeRoot = resolve(argument.slice(15));
    else if (argument.startsWith("--browser-executable="))
      browserExecutable = resolve(argument.slice(21));
    else if (argument.startsWith("--timeout-ms="))
      timeoutMs = Number(argument.slice(13));
    else if (argument.startsWith("--cycles=")) cycles = Number(argument.slice(9));
    else throw new Error(`unknown argument: ${argument}`);
  }
  if (runtimeRoot === undefined) throw new Error("--runtime-root=PATH is required");
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1_000 || timeoutMs > 180_000)
    throw new Error("--timeout-ms must be 1000 through 180000");
  if (!Number.isInteger(cycles) || cycles < 1 || cycles > 10)
    throw new Error("--cycles must be 1 through 10");
  return { runtimeRoot, browserExecutable, timeoutMs, cycles };
}

function baseReport(status: WebWorkbenchReport["status"]): WebWorkbenchReport {
  return {
    schema: SCHEMA,
    schema_version: 1,
    status,
    target: "linux-x64-glibc",
    fixture: "loopback-only",
    checks: [],
    blockers: [],
  };
}

export async function runWebWorkbenchAcceptance(
  options: AcceptanceOptions,
): Promise<WebWorkbenchReport> {
  const report = baseReport("failed");
  const nodeRepl = join(options.runtimeRoot, "bin/node_repl");
  const node = join(options.runtimeRoot, "bin/node");
  if (!executable(nodeRepl) || !executable(node)) {
    report.status = "blocked";
    report.blockers.push("assembled-node-repl-runtime");
    report.error = `runtime executables are absent: ${options.runtimeRoot}`;
    return report;
  }
  const root = mkdtempSync(join(tmpdir(), "cua-node-web-workbench-"));
  let fixture: LoopbackFixture | undefined;
  let session: McpSession | undefined;
  let browserMarker: string | undefined;
  const uploadPath = join(root, "upload marker.txt");
  const screenshotPath = join(root, "standalone.png");
  const downloadPath = join(root, "download.txt");
  try {
    fixture = await startLoopbackFixture();
    writeFileSync(uploadPath, "UPLOAD MARKER");
    session = startMcpSession({
      executable: nodeRepl,
      cwd: root,
      timeoutMs: options.timeoutMs,
      env: {
        ...process.env,
        HOME: root,
        XDG_CACHE_HOME: join(root, "cache"),
        NODE_REPL_NODE_PATH: node,
        NODE_REPL_NODE_MODULE_DIRS: join(options.runtimeRoot, "lib/node_modules"),
        PLAYWRIGHT_BROWSERS_PATH: "0",
        NO_PROXY: "*",
        no_proxy: "*",
        HTTP_PROXY: "",
        HTTPS_PROXY: "",
        ALL_PROXY: "",
      },
    });
    const initialize = await session.request("initialize", {
      protocolVersion: "2025-11-25",
      capabilities: {},
      clientInfo: { name: "web-workbench-acceptance", version: "1" },
    });
    if (initialize.error !== undefined)
      throw new Error(`initialize failed: ${JSON.stringify(initialize.error)}`);
    let browserExecutable = options.browserExecutable;
    if (browserExecutable === undefined) {
      try {
        const metadata = parseJsonLine(
          toolText(
            await session.request("tools/call", {
              name: "js",
              arguments: {
                code: "console.log(JSON.stringify(nodeRepl.runtime))",
                title: "Resolve runtime metadata",
              },
            }),
            "runtime metadata",
          ),
          "runtime metadata",
        );
        const browser = metadata.browser;
        if (
          browser !== null &&
          typeof browser === "object" &&
          !Array.isArray(browser) &&
          typeof (browser as Record<string, unknown>).executablePath === "string"
        )
          browserExecutable = resolve(
            (browser as Record<string, unknown>).executablePath as string,
          );
      } catch {}
      if (browserExecutable === undefined) {
        report.status = "blocked";
        report.blockers.push("nodeRepl.runtime.browser.executablePath");
        report.error =
          "WP-03 runtime metadata is not integrated; pass --browser-executable=PATH only for standalone harness testing";
        return report;
      }
    }
    if (!executable(browserExecutable))
      throw new Error(
        `resolved browser executable is not executable: ${browserExecutable}`,
      );
    report.checks.push({
      id: "browser-resolution",
      status: "passed",
      evidence: {
        source:
          options.browserExecutable === undefined
            ? "nodeRepl.runtime"
            : "explicit-harness-option",
        executable: browserExecutable,
      },
    });
    const networkCode = String.raw`
var origin = ${JSON.stringify(fixture.origin)}; var wsUrl = ${JSON.stringify(fixture.wsUrl)};
var text = await (await fetch(origin + "/text")).text(); var json = await (await fetch(origin + "/json")).json();
var bytes = new Uint8Array(await (await fetch(origin + "/bytes")).arrayBuffer());
var form = new FormData(); form.append("file", new Blob(["UPLOAD MARKER"]), "marker.txt"); var upload = await (await fetch(origin + "/upload", {method:"POST", body:form})).json();
var streamed = await (await fetch(origin + "/stream")).text(); var compressed = await (await fetch(origin + "/gzip")).text();
var aborter = new AbortController(); var aborted = fetch(origin + "/delay", {signal:aborter.signal}).then(()=>false, error=>error.name === "AbortError"); aborter.abort();
var ws = await new Promise((resolvePromise,rejectPromise)=>{var socket=new WebSocket(wsUrl); socket.binaryType="arraybuffer"; var seen=[]; socket.onopen=()=>socket.send("hello"); socket.onmessage=(event)=>{seen.push(typeof event.data === "string" ? event.data : Array.from(new Uint8Array(event.data)).join(",")); if(seen.length===1)socket.send(new Uint8Array([4,5,6])); else socket.close(1000,"done");}; socket.onclose=(event)=>resolvePromise({seen:seen.join("|"),code:event.code}); socket.onerror=rejectPromise;});
console.log(JSON.stringify({text,json:json.ok,bytes:Array.from(bytes).join(","),upload:upload.has_marker,streamed,compressed,aborted:await aborted,ws:ws.seen,close:ws.code}));`;
    const network = parseJsonLine(
      toolText(
        await session.request(
          "tools/call",
          {
            name: "js",
            arguments: {
              code: networkCode,
              title: "Accept web globals",
              timeout_ms: options.timeoutMs,
            },
          },
          options.timeoutMs,
        ),
        "web globals",
      ),
      "web globals",
    );
    if (
      network.text !== "loopback text" ||
      network.json !== true ||
      network.bytes !== "0,1,2,253,254,255" ||
      network.upload !== true ||
      network.streamed !== "one-two" ||
      network.compressed !== "compressed loopback" ||
      network.aborted !== true ||
      network.ws !== "hello|4,5,6" ||
      network.close !== 1000
    )
      throw new Error(`web-global evidence mismatch: ${JSON.stringify(network)}`);
    report.checks.push({
      id: "web-globals",
      status: "passed",
      evidence: {
        fetch_forms: 7,
        websocket_text: true,
        websocket_binary: true,
        websocket_close_code: 1000,
      },
    });
    const baseline = processTreePids(session.child.pid!);
    browserMarker = `cua-node-workbench-${session.child.pid!}`;
    const markerArgument = `--cua-node-acceptance-marker=${browserMarker}`;
    const browserArgs = JSON.stringify([
      "--disable-background-networking",
      "--disable-component-update",
      "--no-first-run",
      "--no-sandbox",
      markerArgument,
    ]);
    const crashBrowserArgs = JSON.stringify(["--no-sandbox", markerArgument]);
    const playwrightCode = String.raw`
var pw = await import("playwright"); var cycles=${options.cycles}; var readback=""; var uploadEvidence; var crashSignals=[];
var bounded=async(label,promise)=>{var timer;try{return await Promise.race([promise,new Promise((resolvePromise,rejectPromise)=>{timer=setTimeout(()=>rejectPromise(new Error(label+" timed out")),2000);})]);}finally{clearTimeout(timer);}};
var closeBrowser=async(browser,label)=>{if(browser!==undefined&&browser.isConnected())await bounded(label,browser.close());};
for(var cycle=0;cycle<cycles;cycle+=1){var browser;try{browser=await pw.chromium.launch({executablePath:${JSON.stringify(browserExecutable)},headless:true,args:${browserArgs}});var page=await browser.newPage({acceptDownloads:true});await page.goto(${JSON.stringify(fixture.origin)});await page.locator("#text").click();await page.locator("#text").pressSequentially("PLAYWRIGHT");await page.locator("#text").press("End");await page.locator("#text").press("!");await page.locator("#upload").setInputFiles(${JSON.stringify(uploadPath)});await page.waitForFunction(()=>document.getElementById("upload-state").dataset.observed==="true");uploadEvidence=JSON.parse(await page.locator("#upload-state").textContent());uploadEvidence.observed=await page.locator("#upload-state").getAttribute("data-observed")==="true";await page.getByRole("button",{name:"Apply"}).click();readback=await page.locator("#out").textContent();var downloadPromise=page.waitForEvent("download");await page.locator("#download").click();var download=await downloadPromise;await download.saveAs(${JSON.stringify(downloadPath)});await page.screenshot({path:${JSON.stringify(screenshotPath)}});}finally{await closeBrowser(browser,"browser close");}}
for(var crashCycle=0;crashCycle<cycles;crashCycle+=1){var crashBrowser;try{crashBrowser=await pw.chromium.launch({executablePath:${JSON.stringify(browserExecutable)},headless:true,args:${crashBrowserArgs}});var disconnectedPromise=new Promise(resolvePromise=>crashBrowser.once("disconnected",()=>resolvePromise(true)));var cdp=await crashBrowser.newBrowserCDPSession();var command;try{await bounded("Browser.crash",cdp.send("Browser.crash"));command="resolved";}catch(error){var detail=error&&typeof error==="object"&&typeof error.message==="string"?error.message:typeof error==="string"?error:JSON.stringify(error);command="rejected:"+(detail||"unknown error");}var disconnected=await Promise.race([disconnectedPromise,new Promise(resolvePromise=>setTimeout(()=>resolvePromise(false),2000))]);if(disconnected!==true)throw new Error("Browser.crash did not disconnect ("+command+")");crashSignals.push({disconnected,command});}finally{await closeBrowser(crashBrowser,"crashed browser close");}}
console.log(JSON.stringify({cycles,readback,upload:uploadEvidence,crashSignals}));`;
    const playwright = parseJsonLine(
      toolText(
        await session.request(
          "tools/call",
          {
            name: "js",
            arguments: {
              code: playwrightCode,
              title: "Accept standalone Playwright",
              timeout_ms: options.timeoutMs,
            },
          },
          options.timeoutMs,
        ),
        "Playwright",
      ),
      "Playwright",
    );
    const playwrightEvidence = validatePlaywrightEvidence(playwright, options.cycles);
    if (
      readFileSync(downloadPath, "utf8") !== "download payload" ||
      !existsSync(screenshotPath) ||
      readFileSync(screenshotPath).length < 100
    )
      throw new Error(`Playwright evidence mismatch: ${JSON.stringify(playwright)}`);
    report.checks.push({
      id: "standalone-playwright",
      status: "passed",
      evidence: {
        cycles: options.cycles,
        click: true,
        type: true,
        key: true,
        upload_observed: playwrightEvidence.uploadObserved,
        download: true,
        screenshot_bytes: readFileSync(screenshotPath).length,
        crash_disconnects: playwrightEvidence.crashSignals.length,
        crash_command_outcomes: playwrightEvidence.crashSignals
          .map((signal) => signal.command)
          .join("|"),
      },
    });
    for (let cycle = 0; cycle < options.cycles; cycle += 1) {
      const cancellation = session.requestWithId(
        "tools/call",
        {
          name: "js",
          arguments: {
            code: "await new Promise(resolvePromise => setTimeout(resolvePromise, 30000))",
            title: `Cancel lifecycle cell ${cycle + 1}`,
            timeout_ms: options.timeoutMs,
          },
        },
        options.timeoutMs,
      );
      session.notify("notifications/cancelled", { requestId: cancellation.id });
      let cancelledText = "";
      try {
        cancelledText = toolText(await cancellation.response, "cancelled cell");
      } catch (error) {
        cancelledText = error instanceof Error ? error.message : "";
      }
      if (!/cancel/iu.test(cancelledText))
        throw new Error(
          `cancelled cell returned unexpected evidence: ${cancelledText}`,
        );
      await session.request(
        "tools/call",
        { name: "js_reset", arguments: {} },
        options.timeoutMs,
      );
      const reset = toolText(
        await session.request(
          "tools/call",
          {
            name: "js",
            arguments: {
              code: 'console.log(JSON.stringify({reset:typeof origin==="undefined"}))',
            },
          },
          options.timeoutMs,
        ),
        "reset",
      );
      if (parseJsonLine(reset, "reset").reset !== true)
        throw new Error("js_reset retained prior bindings");
    }
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 250));
    const after = processTreePids(session.child.pid!);
    const markedOrphans = processPidsContaining(browserMarker);
    if (after.length > baseline.length)
      throw new Error(
        `orphan process tree grew from ${baseline.join(",")} to ${after.join(",")}`,
      );
    if (markedOrphans.length > 0)
      throw new Error(`marked browser processes survived: ${markedOrphans.join(",")}`);
    report.checks.push({
      id: "lifecycle",
      status: "passed",
      evidence: {
        baseline_processes: baseline.length,
        final_processes: after.length,
        reset: true,
        cancellation_recovery: true,
        lifecycle_cycles: options.cycles,
        orphan_count: 0,
      },
    });
    const shutdown = await session.request("shutdown", {}, options.timeoutMs);
    if (shutdown.error !== undefined || shutdown.result !== null)
      throw new Error(`shutdown failed: ${JSON.stringify(shutdown)}`);
    const exit = await session.close(2_000);
    session = undefined;
    if (exit.code !== 0 || exit.signal !== null)
      throw new Error(`host shutdown was unclean: ${JSON.stringify(exit)}`);
    report.checks.push({
      id: "shutdown",
      status: "passed",
      evidence: {
        bounded_ms: 2000,
        code: 0,
        fixture_requests: fixture.requests,
      },
    });
    report.status = "passed";
    return report;
  } catch (error) {
    report.error =
      error instanceof Error ? error.message : "web workbench acceptance failed";
    report.blockers.push("web-workbench-acceptance-failed");
    return report;
  } finally {
    const cleanupErrors = await runBoundedCleanup(
      [
        {
          label: "MCP session cleanup",
          run: async () => {
            if (session !== undefined) await session.close(500);
          },
        },
        {
          label: "marked browser cleanup",
          run: async () => {
            if (browserMarker !== undefined)
              await stopProcessesContaining(browserMarker, 500);
          },
        },
        {
          label: "loopback fixture cleanup",
          run: async () => {
            if (fixture !== undefined) await fixture.close();
          },
        },
        {
          label: "temporary root cleanup",
          run: () => rm(root, { recursive: true, force: true }),
        },
      ],
      2_000,
    );
    if (cleanupErrors.length > 0) {
      report.status = "failed";
      report.error = [report.error, ...cleanupErrors].filter(Boolean).join("; ");
      if (!new Set(report.blockers).has("web-workbench-cleanup-failed"))
        report.blockers.push("web-workbench-cleanup-failed");
    }
  }
}

if (require.main === module)
  void (async () => {
    let report: WebWorkbenchReport;
    try {
      report = await runWebWorkbenchAcceptance(
        parseAcceptanceArgs(process.argv.slice(2)),
      );
    } catch (error) {
      report = baseReport("failed");
      report.error =
        error instanceof Error ? error.message : "invalid acceptance options";
      report.blockers.push("invalid-options");
    }
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    if (report.status !== "passed")
      process.exitCode = report.status === "blocked" ? 2 : 1;
  })();
