import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createHash } from "node:crypto";
import {
  accessSync,
  constants,
  readdirSync,
  readFileSync,
  realpathSync,
} from "node:fs";
import { createInterface } from "node:readline";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { inflateSync } from "node:zlib";
import {
  createDisposableRuntime,
  startBrowserAcceptanceFixture,
  type BrowserAcceptanceFixture,
  type TrustNegativeCase,
} from "./browser-live-fixture.ts";

type Scenario = "iab" | "brave-origin-extension";
type JsonObject = Record<string, unknown>;

export function browserInfoMatchesAcceptance(
  entry: unknown,
  scenario: Scenario,
  sessionId: string,
): entry is JsonObject {
  if (entry === null || typeof entry !== "object" || Array.isArray(entry)) return false;
  const browser = entry as JsonObject;
  const metadata = browser.metadata;
  if (metadata === null || typeof metadata !== "object" || Array.isArray(metadata))
    return false;
  const browserMetadata = metadata as JsonObject;
  if (scenario === "iab") {
    return browser.type === "iab"
      && browser.transport === "host_provided_iab"
      && browserMetadata.skyCuaBridgeTransport === "host_provided_iab"
      && browserMetadata.skyCuaBridgeType !== "extension"
      && browserMetadata.codexSessionId === sessionId;
  }
  return browser.type === "extension"
    && browser.transport === "extension_native_host"
    && browserMetadata.skyCuaBridgeTransport === "extension_native_host";
}

export function selectBrowserInfoForAcceptance(
  availableBrowsers: unknown,
  scenario: Scenario,
  sessionId: string,
): JsonObject {
  if (!Array.isArray(availableBrowsers))
    throw new Error("Browser discovery did not return an array");
  const selected = availableBrowsers.find((entry) =>
    browserInfoMatchesAcceptance(entry, scenario, sessionId));
  if (selected === undefined) {
    const expected = scenario === "iab"
      ? "type=iab transport=host_provided_iab"
      : "type=extension transport=extension_native_host";
    throw new Error(`Required browser backend is unavailable: ${expected}`);
  }
  return selected as JsonObject;
}

export type BrowserAcceptanceOptions = {
  runtimeRoot: string;
  browserClient: string;
  nodeRepl: string;
  braveExecutable: string;
  scenario: Scenario;
  url: string;
  sessionId: string;
  turnId: string;
  inputTestId: string;
  readbackTestId: string;
  buttonName: string;
  typedText: string;
  timeoutMs: number;
  ownsFixture: boolean;
  trustNegative: "none" | TrustNegativeCase | "all";
};

export type InstalledSelection = {
  runtimeRoot: string;
  manifestPath: string;
  nodeRepl: string;
  browserClient: string;
  browserClientSha256: string;
  trustedBrowserClientSha256s: string[];
};

type Manifest = {
  runtime_name?: unknown;
  node_repl_path?: unknown;
  node_repl_sha256?: unknown;
  trusted_browser_client_sha256s?: unknown;
};

type McpResponse = {
  jsonrpc?: unknown;
  id?: unknown;
  result?: unknown;
  error?: unknown;
};

export type NodeReplProcess = {
  exitCode: number | null;
  signalCode: NodeJS.Signals | null;
  pid?: number | undefined;
  stdin: { end(): void };
  once(
    event: "exit",
    listener: (code: number | null, signal: NodeJS.Signals | null) => void,
  ): void;
  removeListener(
    event: "exit",
    listener: (code: number | null, signal: NodeJS.Signals | null) => void,
  ): void;
  kill(signal: NodeJS.Signals): boolean;
};

type ChildExit = {
  code: number | null;
  signal: NodeJS.Signals | null;
};

type DecodedPng = {
  width: number;
  height: number;
  pixels: Buffer;
};

const SCHEMA = "com.heliasar.cua-node.browser-live-acceptance" as const;
const SHA256 = /^[0-9a-f]{64}$/u;
const DEFAULT_RUNTIME_ROOT = resolve(__dirname, "../out/linux-x64/cua_node");
const DEFAULT_BRAVE_EXECUTABLE = "/opt/brave-origin-bin/brave";
const BRAVE_EXTENSION_ORIGIN =
  "chrome-extension://hehggadaopoacecdllhhajmbjkdcmajg/";
const PNG_SIGNATURE = Buffer.from("89504e470d0a1a0a", "hex");
const WEBP_RIFF = Buffer.from("RIFF", "ascii");
const WEBP_SIGNATURE = Buffer.from("WEBP", "ascii");
const NODE_REPL_EXIT_WAIT_MS = 5_000;

function errorText(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "unknown Browser Use acceptance failure";
}

function valueAfter(argument: string, name: string): string | null {
  const prefix = `${name}=`;
  return argument.startsWith(prefix) ? argument.slice(prefix.length) : null;
}

function nonempty(value: string | undefined, name: string): string {
  if (value === undefined || value.trim().length === 0)
    throw new Error(`${name} must be non-empty`);
  return value;
}

export function parseBrowserAcceptanceArgs(
  argv: string[],
  env: NodeJS.ProcessEnv = process.env,
): BrowserAcceptanceOptions {
  const values = new Map<string, string>();
  for (const argument of argv) {
    if (argument === "--json") continue;
    const names = [
      "--runtime-root",
      "--browser-client",
      "--node-repl",
      "--brave-executable",
      "--scenario",
      "--url",
      "--session-id",
      "--turn-id",
      "--input-testid",
      "--readback-testid",
      "--button-name",
      "--typed-text",
      "--timeout-ms",
      "--trust-negative",
    ];
    const name = names.find(
      (candidate) => valueAfter(argument, candidate) !== null,
    );
    if (name === undefined) throw new Error(`unknown argument: ${argument}`);
    values.set(name, valueAfter(argument, name) ?? "");
  }
  if (env.CODEX_BROWSER_PROVIDER?.trim().toLowerCase() === "skynet")
    throw new Error(
      "CODEX_BROWSER_PROVIDER=skynet is forbidden by installed Browser Use acceptance",
    );
  const scenario = nonempty(values.get("--scenario"), "--scenario");
  if (scenario !== "iab" && scenario !== "brave-origin-extension")
    throw new Error("--scenario must be iab or brave-origin-extension");
  const url = values.get("--url");
  const parsedUrl = url === undefined ? null : new URL(nonempty(url, "--url"));
  if (
    parsedUrl !== null &&
    parsedUrl.protocol !== "http:" &&
    parsedUrl.protocol !== "https:"
  )
    throw new Error("--url must use http or https");
  const trustNegative = values.get("--trust-negative") ?? "none";
  if (
    trustNegative !== "none" &&
    trustNegative !== "tampered" &&
    trustNegative !== "missing" &&
    trustNegative !== "wrong-manifest-hash" &&
    trustNegative !== "all"
  )
    throw new Error(
      "--trust-negative must be tampered, missing, wrong-manifest-hash, or all",
    );
  const timeoutText = values.get("--timeout-ms") ?? "60000";
  const timeoutMs = Number(timeoutText);
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1_000 || timeoutMs > 300_000)
    throw new Error("--timeout-ms must be an integer from 1000 through 300000");
  const runtimeRoot = resolve(
    values.get("--runtime-root") ?? DEFAULT_RUNTIME_ROOT,
  );
  const defaultBrowserClient = join(
    runtimeRoot,
    "lib/node_modules/@heliasar/browser-use/build/browser-client.mjs",
  );
  return {
    runtimeRoot,
    browserClient: resolve(
      values.get("--browser-client") ??
        env.CUA_NODE_BROWSER_CLIENT_PATH ??
        defaultBrowserClient,
    ),
    nodeRepl: resolve(
      values.get("--node-repl") ?? join(runtimeRoot, "bin/node_repl"),
    ),
    braveExecutable: resolve(
      values.get("--brave-executable") ?? DEFAULT_BRAVE_EXECUTABLE,
    ),
    scenario,
    url: parsedUrl?.href ?? "http://127.0.0.1/acceptance",
    sessionId: nonempty(values.get("--session-id"), "--session-id"),
    turnId: nonempty(values.get("--turn-id"), "--turn-id"),
    inputTestId: nonempty(
      values.get("--input-testid") ?? "browser-live-input",
      "--input-testid",
    ),
    readbackTestId: nonempty(
      values.get("--readback-testid") ?? "browser-live-readback",
      "--readback-testid",
    ),
    buttonName: nonempty(values.get("--button-name") ?? "OK", "--button-name"),
    typedText: nonempty(
      values.get("--typed-text") ?? "CUA NODE BROWSER ACCEPTANCE",
      "--typed-text",
    ),
    timeoutMs,
    ownsFixture: parsedUrl === null,
    trustNegative,
  };
}

export function verifyBraveOriginNativeHost(
  options: BrowserAcceptanceOptions,
  procRoot = "/proc",
): JsonObject {
  const braveExecutable = realpathSync(options.braveExecutable);
  assertExecutable(braveExecutable, "Brave Origin executable");
  const candidates: Array<{
    browserPid: number;
    browserExecutable: string;
    hostExecutable: string;
    hostPid: number;
  }> = [];
  for (const entry of readdirSync(procRoot, { withFileTypes: true })) {
    if (!entry.isDirectory() || !/^\d+$/u.test(entry.name)) continue;
    const hostRoot = join(procRoot, entry.name);
    try {
      const hostExecutable = realpathSync(join(hostRoot, "exe"));
      if (!hostExecutable.endsWith("/sky-cua-chrome-host")) continue;
      const commandLine = readFileSync(join(hostRoot, "cmdline"), "utf8");
      if (!commandLine.split("\0").includes(BRAVE_EXTENSION_ORIGIN)) continue;
      const status = readFileSync(join(hostRoot, "status"), "utf8");
      const parentMatch = /^PPid:\s+(\d+)$/mu.exec(status);
      if (parentMatch === null) continue;
      const parentPid = parentMatch[1] ?? "";
      candidates.push({
        browserPid: Number(parentPid),
        browserExecutable: realpathSync(join(procRoot, parentPid, "exe")),
        hostExecutable,
        hostPid: Number(entry.name),
      });
    } catch {
      // Processes can exit between directory enumeration and inspection.
    }
  }
  if (candidates.length > 1)
    throw new Error(
      `ambiguous ${BRAVE_EXTENSION_ORIGIN} native hosts: ${candidates.length} candidates`,
    );
  const candidate = candidates[0];
  if (candidate?.browserExecutable === braveExecutable)
    return {
      executable: braveExecutable,
      browser_pid: candidate.browserPid,
      native_host_pid: candidate.hostPid,
      native_host: candidate.hostExecutable,
      extension_origin: BRAVE_EXTENSION_ORIGIN,
    };
  throw new Error(
    `no ${BRAVE_EXTENSION_ORIGIN} native host is parented by ${braveExecutable}`,
  );
}

function readManifest(path: string): Manifest {
  const value: unknown = JSON.parse(readFileSync(path, "utf8"));
  if (value === null || typeof value !== "object" || Array.isArray(value))
    throw new Error(`runtime manifest must be an object: ${path}`);
  return value as Manifest;
}

function fileSha256(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function assertReadableFile(path: string, label: string): void {
  try {
    accessSync(path, constants.R_OK);
  } catch {
    throw new Error(`${label} is not readable: ${path}`);
  }
}

function assertExecutable(path: string, label: string): void {
  try {
    accessSync(path, constants.R_OK | constants.X_OK);
  } catch {
    throw new Error(`${label} is not executable: ${path}`);
  }
}

function rejectSkynetPath(path: string, label: string): void {
  const segments = resolve(path).toLowerCase().split("/");
  if (segments.some((segment) => segment.includes("skynet")))
    throw new Error(`${label} must not be a Skynet client path: ${path}`);
}

export function validateInstalledSelection(
  options: BrowserAcceptanceOptions,
): InstalledSelection {
  const runtimeRoot = realpathSync(options.runtimeRoot);
  const manifestPath = join(runtimeRoot, "manifest.json");
  assertReadableFile(manifestPath, "runtime manifest");
  const manifest = readManifest(manifestPath);
  if (manifest.runtime_name !== "cua_node")
    throw new Error(`runtime manifest is not cua_node: ${manifestPath}`);
  if (
    typeof manifest.node_repl_path !== "string" ||
    isAbsolute(manifest.node_repl_path)
  )
    throw new Error("runtime manifest node_repl_path must be relative");
  const manifestNodeRepl = realpathSync(
    join(runtimeRoot, manifest.node_repl_path),
  );
  const selectedNodeRepl = realpathSync(options.nodeRepl);
  if (selectedNodeRepl !== manifestNodeRepl)
    throw new Error(
      `selected node_repl is not the manifest executable: ${selectedNodeRepl}`,
    );
  assertExecutable(selectedNodeRepl, "installed node_repl");
  if (!SHA256.test(String(manifest.node_repl_sha256 ?? "")))
    throw new Error("runtime manifest node_repl_sha256 is invalid");
  if (fileSha256(selectedNodeRepl) !== manifest.node_repl_sha256)
    throw new Error(
      "installed node_repl SHA-256 does not match runtime manifest",
    );
  rejectSkynetPath(options.browserClient, "browser client");
  const browserClient = realpathSync(options.browserClient);
  rejectSkynetPath(browserClient, "resolved browser client");
  assertReadableFile(browserClient, "installed browser client");
  const hashes = manifest.trusted_browser_client_sha256s;
  if (
    !Array.isArray(hashes) ||
    hashes.length === 0 ||
    !hashes.every((item) => typeof item === "string" && SHA256.test(item))
  )
    throw new Error(
      "runtime manifest trusted_browser_client_sha256s is invalid or empty",
    );
  const trustedBrowserClientSha256s = hashes as string[];
  const browserClientSha256 = fileSha256(browserClient);
  if (!trustedBrowserClientSha256s.includes(browserClientSha256))
    throw new Error(
      `installed browser client SHA-256 ${browserClientSha256} is not trusted by runtime manifest`,
    );
  return {
    runtimeRoot,
    manifestPath,
    nodeRepl: selectedNodeRepl,
    browserClient,
    browserClientSha256,
    trustedBrowserClientSha256s,
  };
}

export type TrustNegativeEvidence = {
  runtime_root: string;
  browser_client: string;
  browser_client_sha256: string;
  cases: Array<{
    case: TrustNegativeCase;
    rejected: true;
    connection_attempted: false;
    hash_verified_before_connection: false;
    error: string;
  }>;
};

export function runPackagedRootTrustNegatives(
  options: BrowserAcceptanceOptions,
): TrustNegativeEvidence {
  const original = validateInstalledSelection(options);
  const requested: TrustNegativeCase[] =
    options.trustNegative === "all"
      ? ["tampered", "missing", "wrong-manifest-hash"]
      : options.trustNegative === "none"
        ? []
        : [options.trustNegative];
  if (requested.length === 0)
    throw new Error("packaged-root trust-negative mode was not selected");
  const cases = requested.map((testCase) => {
    const disposable = createDisposableRuntime(
      original.runtimeRoot,
      original.browserClient,
      original.nodeRepl,
      testCase,
    );
    try {
      let rejection: unknown;
      try {
        validateInstalledSelection({
          ...options,
          runtimeRoot: disposable.runtimeRoot,
          browserClient: disposable.browserClient,
          nodeRepl: disposable.nodeRepl,
          trustNegative: "none",
        });
      } catch (error) {
        rejection = error;
      }
      if (rejection === undefined)
        throw new Error(`trust-negative case ${testCase} did not reject`);
      const message = errorText(rejection);
      if (
        (testCase === "missing" && !message.includes("ENOENT")) ||
        (testCase !== "missing" &&
          !message.includes("is not trusted by runtime manifest"))
      )
        throw new Error(
          `trust-negative case ${testCase} rejected for the wrong reason: ${message}`,
        );
      return {
        case: testCase,
        rejected: true as const,
        connection_attempted: false as const,
        hash_verified_before_connection: false as const,
        error: message,
      };
    } finally {
      disposable.cleanup();
    }
  });
  if (fileSha256(original.browserClient) !== original.browserClientSha256)
    throw new Error("trust-negative mode modified the original Browser client");
  return {
    runtime_root: original.runtimeRoot,
    browser_client: original.browserClient,
    browser_client_sha256: original.browserClientSha256,
    cases,
  };
}

export function buildAcceptanceCode(
  options: BrowserAcceptanceOptions,
  selection: InstalledSelection,
): string {
  return `${buildAcceptanceSetupCode(options, selection)}\n${buildAcceptanceActionCode(options)}`;
}

export function buildAcceptanceSetupCode(
  options: BrowserAcceptanceOptions,
  selection: InstalledSelection,
): string {
  const selectorSource = browserInfoMatchesAcceptance.toString();
  return `
var browserClientUrl = ${JSON.stringify(pathToFileURL(selection.browserClient).href)};
var { setupBrowserRuntime } = await import(browserClientUrl);
await setupBrowserRuntime({ globals: globalThis });
var availableBrowsers = await agent.browsers.list();
var browserInfoMatchesAcceptance = ${selectorSource};
globalThis.selectedInfo = availableBrowsers.find((entry) => browserInfoMatchesAcceptance(entry, ${JSON.stringify(options.scenario)}, ${JSON.stringify(options.sessionId)}));
if (selectedInfo == null) throw new Error("Required browser backend is unavailable: " + ${JSON.stringify(options.scenario === "iab" ? "type=iab transport=host_provided_iab" : "type=extension transport=extension_native_host")});
globalThis.acceptanceBrowser = await agent.browsers.get(selectedInfo.id);
nodeRepl.write("BROWSER-SETUP-OK");`;
}

export function buildAcceptanceActionCode(
  options: BrowserAcceptanceOptions,
): string {
  return `
await acceptanceBrowser.nameSession("Browser live acceptance");
const acceptanceTab = await acceptanceBrowser.tabs.new();
try {
await acceptanceTab.goto(${JSON.stringify(options.url)});
await acceptanceTab.playwright.waitForLoadState({ state: "load", timeoutMs: ${options.timeoutMs} });
var navigationUrl = await acceptanceTab.url();
var input = acceptanceTab.playwright.getByTestId(${JSON.stringify(options.inputTestId)});
var screenshotViewport = await input.evaluate((element) => ({ width: element.ownerDocument.documentElement.clientWidth, height: element.ownerDocument.documentElement.clientHeight }), undefined, { timeoutMs: ${options.timeoutMs} });
var beforeScreenshot = await acceptanceTab.screenshot();
await nodeRepl.emitImage(beforeScreenshot);
await input.click({ timeoutMs: ${options.timeoutMs} });
await input.type(${JSON.stringify(options.typedText)}, { timeoutMs: ${options.timeoutMs} });
await input.press("End", { timeoutMs: ${options.timeoutMs} });
var typedValue = await input.evaluate((element) => element.value, undefined, { timeoutMs: ${options.timeoutMs} });
var okButton = acceptanceTab.playwright.getByRole("button", { name: ${JSON.stringify(options.buttonName)}, exact: true });
await okButton.click({ timeoutMs: ${options.timeoutMs} });
var readback = await acceptanceTab.playwright.getByTestId(${JSON.stringify(options.readbackTestId)}).innerText({ timeoutMs: ${options.timeoutMs} });
var afterScreenshot = await acceptanceTab.screenshot();
await nodeRepl.emitImage(afterScreenshot);
var requestMeta = nodeRepl.requestMeta;
nodeRepl.write(JSON.stringify({
  browser: { id: selectedInfo.id, name: selectedInfo.name, type: selectedInfo.type, transport: selectedInfo.transport, metadata: selectedInfo.metadata ?? {} },
  navigation: { requested_url: ${JSON.stringify(options.url)}, final_url: navigationUrl },
  keyboard: { method: "PlaywrightLocator.type+press", key: "End", text: ${JSON.stringify(options.typedText)}, value: typedValue },
  click: { actual: true, button_name: ${JSON.stringify(options.buttonName)} },
  readback,
  screenshot: { method: "Tab.screenshot", emitted: true, expected_width: screenshotViewport.width, expected_height: screenshotViewport.height, before_byte_length: beforeScreenshot.byteLength, after_byte_length: afterScreenshot.byteLength },
  request_meta: requestMeta,
}));
} finally {
  await acceptanceTab.close();
}`;
}

export function buildMcpRequests(
  options: BrowserAcceptanceOptions,
  selection: InstalledSelection,
): JsonObject[] {
  const requestMeta = {
    session_id: options.sessionId,
    turn_id: options.turnId,
    "x-codex-turn-metadata": {
      session_id: options.sessionId,
      turn_id: options.turnId,
    },
  };
  return [
    {
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-06-18",
        capabilities: { elicitation: { form: {} } },
        clientInfo: { name: "browser-live-acceptance", version: "1" },
      },
    },
    {
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: buildAcceptanceSetupCode(options, selection),
          timeout_ms: options.timeoutMs,
          title: "Browser live acceptance setup",
        },
        _meta: requestMeta,
      },
    },
    {
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: buildAcceptanceActionCode(options),
          timeout_ms: options.timeoutMs,
          title: "Browser live acceptance action",
        },
        _meta: requestMeta,
      },
    },
    { jsonrpc: "2.0", id: 4, method: "shutdown", params: {} },
  ];
}

function isObject(value: unknown): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function currentExit(child: NodeReplProcess): ChildExit | null {
  if (child.exitCode === null && child.signalCode === null) return null;
  return { code: child.exitCode, signal: child.signalCode };
}

function waitForExit(
  child: NodeReplProcess,
  timeoutMs: number,
): Promise<ChildExit | null> {
  const exited = currentExit(child);
  if (exited !== null) return Promise.resolve(exited);
  return new Promise((resolvePromise) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const finish = (exit: ChildExit | null): void => {
      if (timer !== undefined) clearTimeout(timer);
      child.removeListener("exit", onExit);
      resolvePromise(exit);
    };
    const onExit = (code: number | null, signal: NodeJS.Signals | null): void =>
      finish({ code, signal });
    child.once("exit", onExit);
    const racedExit = currentExit(child);
    if (racedExit !== null) {
      finish(racedExit);
      return;
    }
    timer = setTimeout(() => finish(currentExit(child)), timeoutMs);
  });
}

export function validateShutdownResponse(response: McpResponse): void {
  if (response.error !== undefined || response.result !== null)
    throw new Error(
      `node_repl shutdown response must be result:null with no error: ${JSON.stringify(response)}`,
    );
}

export async function closeNodeRepl(
  child: NodeReplProcess,
  waitMs = NODE_REPL_EXIT_WAIT_MS,
): Promise<ChildExit> {
  try {
    child.stdin.end();
  } catch {
    // PID-specific escalation below still guarantees bounded cleanup.
  }
  let exit = await waitForExit(child, waitMs);
  if (exit === null) {
    child.kill("SIGTERM");
    exit = await waitForExit(child, waitMs);
  }
  if (exit === null) {
    child.kill("SIGKILL");
    exit = await waitForExit(child, waitMs);
  }
  if (exit === null)
    throw new Error(
      `node_repl child PID ${child.pid ?? "unknown"} did not exit after SIGKILL`,
    );
  return exit;
}

export function combinePrimaryAndCleanupErrors(
  primaryFailure: unknown,
  cleanupFailure: unknown,
): AggregateError {
  const primary =
    primaryFailure instanceof Error
      ? primaryFailure
      : new Error("unknown Browser Use acceptance failure");
  const cleanup =
    cleanupFailure instanceof Error
      ? cleanupFailure
      : new Error("unknown node_repl cleanup failure");
  return new AggregateError(
    [primary, cleanup],
    `${primary.message}; node_repl cleanup also failed: ${cleanup.message}`,
    { cause: primary },
  );
}

function pngCrc32(bytes: Buffer): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1)
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function paethPredictor(
  left: number,
  above: number,
  upperLeft: number,
): number {
  const estimate = left + above - upperLeft;
  const leftDistance = Math.abs(estimate - left);
  const aboveDistance = Math.abs(estimate - above);
  const upperLeftDistance = Math.abs(estimate - upperLeft);
  if (leftDistance <= aboveDistance && leftDistance <= upperLeftDistance)
    return left;
  return aboveDistance <= upperLeftDistance ? above : upperLeft;
}

function decodePng(bytes: Buffer, label: string): DecodedPng {
  if (
    bytes.length < PNG_SIGNATURE.length ||
    !bytes.subarray(0, 8).equals(PNG_SIGNATURE)
  )
    throw new Error(`${label} screenshot is not a PNG`);
  let offset = PNG_SIGNATURE.length;
  let width = 0;
  let height = 0;
  let channels = 0;
  let sawHeader = false;
  let sawEnd = false;
  const compressed: Buffer[] = [];
  while (offset < bytes.length) {
    if (bytes.length - offset < 12)
      throw new Error(`${label} screenshot PNG is truncated`);
    const length = bytes.readUInt32BE(offset);
    const chunkEnd = offset + 12 + length;
    if (chunkEnd > bytes.length)
      throw new Error(`${label} screenshot PNG is truncated`);
    const typeBytes = bytes.subarray(offset + 4, offset + 8);
    const type = typeBytes.toString("ascii");
    const data = bytes.subarray(offset + 8, offset + 8 + length);
    const expectedCrc = bytes.readUInt32BE(offset + 8 + length);
    if (pngCrc32(Buffer.concat([typeBytes, data])) !== expectedCrc)
      throw new Error(
        `${label} screenshot PNG has an invalid ${type} checksum`,
      );
    if (!sawHeader && type !== "IHDR")
      throw new Error(`${label} screenshot PNG does not start with IHDR`);
    if (type === "IHDR") {
      if (sawHeader || length !== 13)
        throw new Error(`${label} screenshot PNG has an invalid IHDR`);
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      const bitDepth = data[8];
      const colorType = data[9];
      channels =
        colorType === 0
          ? 1
          : colorType === 2
            ? 3
            : colorType === 4
              ? 2
              : colorType === 6
                ? 4
                : 0;
      if (
        width < 1 ||
        height < 1 ||
        width > 16_384 ||
        height > 16_384 ||
        bitDepth !== 8 ||
        channels === 0 ||
        data[10] !== 0 ||
        data[11] !== 0 ||
        data[12] !== 0
      )
        throw new Error(
          `${label} screenshot PNG has an unsupported content shape`,
        );
      sawHeader = true;
    } else if (type === "IDAT") {
      if (!sawHeader || sawEnd)
        throw new Error(`${label} screenshot PNG has misplaced image data`);
      compressed.push(data);
    } else if (type === "IEND") {
      if (length !== 0 || compressed.length === 0)
        throw new Error(`${label} screenshot PNG has an invalid IEND`);
      sawEnd = true;
      offset = chunkEnd;
      break;
    }
    offset = chunkEnd;
  }
  if (!sawEnd || offset !== bytes.length)
    throw new Error(
      `${label} screenshot PNG is truncated or has trailing bytes`,
    );
  let scanlines: Buffer;
  try {
    scanlines = inflateSync(Buffer.concat(compressed));
  } catch {
    throw new Error(`${label} screenshot PNG image data cannot be decoded`);
  }
  const rowBytes = width * channels;
  if (scanlines.length !== (rowBytes + 1) * height)
    throw new Error(`${label} screenshot PNG has truncated scanlines`);
  const samples = Buffer.alloc(rowBytes * height);
  for (let row = 0; row < height; row += 1) {
    const sourceOffset = row * (rowBytes + 1);
    const filter = scanlines[sourceOffset];
    if (filter === undefined || filter > 4)
      throw new Error(`${label} screenshot PNG has an invalid row filter`);
    const outputOffset = row * rowBytes;
    for (let column = 0; column < rowBytes; column += 1) {
      const encoded = scanlines[sourceOffset + 1 + column] ?? 0;
      const left =
        column >= channels
          ? (samples[outputOffset + column - channels] ?? 0)
          : 0;
      const above =
        row > 0 ? (samples[outputOffset + column - rowBytes] ?? 0) : 0;
      const upperLeft =
        row > 0 && column >= channels
          ? (samples[outputOffset + column - rowBytes - channels] ?? 0)
          : 0;
      const predictor =
        filter === 0
          ? 0
          : filter === 1
            ? left
            : filter === 2
              ? above
              : filter === 3
                ? Math.floor((left + above) / 2)
                : paethPredictor(left, above, upperLeft);
      samples[outputOffset + column] = (encoded + predictor) & 0xff;
    }
  }
  const pixels = Buffer.alloc(width * height * 4);
  for (let pixel = 0; pixel < width * height; pixel += 1) {
    const source = pixel * channels;
    const target = pixel * 4;
    const grayscale = samples[source] ?? 0;
    pixels[target] = channels < 3 ? grayscale : (samples[source] ?? 0);
    pixels[target + 1] = channels < 3 ? grayscale : (samples[source + 1] ?? 0);
    pixels[target + 2] = channels < 3 ? grayscale : (samples[source + 2] ?? 0);
    pixels[target + 3] =
      channels === 2
        ? (samples[source + 1] ?? 0)
        : channels === 4
          ? (samples[source + 3] ?? 0)
          : 255;
  }
  return { width, height, pixels };
}

function readUint24Le(bytes: Buffer, offset: number): number {
  return (
    (bytes[offset] ?? 0) |
    ((bytes[offset + 1] ?? 0) << 8) |
    ((bytes[offset + 2] ?? 0) << 16)
  );
}

function decodeWebp(bytes: Buffer, label: string): DecodedPng {
  if (
    bytes.length < 20 ||
    !bytes.subarray(0, 4).equals(WEBP_RIFF) ||
    !bytes.subarray(8, 12).equals(WEBP_SIGNATURE)
  )
    throw new Error(`${label} screenshot is not a WebP image`);
  if (bytes.readUInt32LE(4) + 8 !== bytes.length)
    throw new Error(`${label} screenshot has an invalid WebP RIFF length`);
  const chunks: Array<{ type: string; start: number; size: number }> = [];
  let offset = 12;
  while (offset < bytes.length) {
    if (offset + 8 > bytes.length)
      throw new Error(`${label} screenshot has a truncated WebP chunk header`);
    const type = bytes.subarray(offset, offset + 4).toString("ascii");
    const size = bytes.readUInt32LE(offset + 4);
    const start = offset + 8;
    const end = start + size;
    if (end > bytes.length)
      throw new Error(`${label} screenshot has a truncated WebP ${type} chunk`);
    chunks.push({ type, start, size });
    offset = end + (size & 1);
  }
  if (offset !== bytes.length)
    throw new Error(`${label} screenshot has invalid WebP chunk padding`);
  const imageChunk = chunks.find((entry) => entry.type === "VP8 " || entry.type === "VP8L");
  if (imageChunk === undefined || imageChunk.size < 5)
    throw new Error(`${label} screenshot has no complete WebP image payload`);
  const extended = chunks.find((entry) => entry.type === "VP8X");
  const chunk = extended?.type ?? imageChunk.type;
  const start = extended?.start ?? imageChunk.start;
  let width: number;
  let height: number;
  if (chunk === "VP8X") {
    if ((extended?.size ?? 0) !== 10)
      throw new Error(`${label} screenshot has an invalid VP8X header`);
    width = readUint24Le(bytes, start + 4) + 1;
    height = readUint24Le(bytes, start + 7) + 1;
  } else if (chunk === "VP8L") {
    if (bytes[start] !== 0x2f)
      throw new Error(`${label} screenshot has an invalid VP8L header`);
    const packed = bytes.readUInt32LE(start + 1);
    width = (packed & 0x3fff) + 1;
    height = ((packed >>> 14) & 0x3fff) + 1;
  } else if (chunk === "VP8 ") {
    if (
      imageChunk.size < 10 ||
      !bytes.subarray(imageChunk.start + 3, imageChunk.start + 6).equals(
        Buffer.from([0x9d, 0x01, 0x2a]),
      )
    )
      throw new Error(`${label} screenshot has an invalid VP8 frame header`);
    width = bytes.readUInt16LE(imageChunk.start + 6) & 0x3fff;
    height = bytes.readUInt16LE(imageChunk.start + 8) & 0x3fff;
  } else {
    throw new Error(
      `${label} screenshot uses unsupported WebP chunk ${JSON.stringify(chunk)}`,
    );
  }
  if (width < 1 || height < 1 || width > 16_384 || height > 16_384)
    throw new Error(`${label} screenshot has invalid WebP dimensions`);
  return { width, height, pixels: bytes };
}

function decodeScreenshot(
  bytes: Buffer,
  mimeType: string,
  label: string,
): DecodedPng {
  if (mimeType === "image/png") return decodePng(bytes, label);
  if (mimeType === "image/webp") return decodeWebp(bytes, label);
  throw new Error(`${label} screenshot has unsupported MIME type ${mimeType}`);
}

export function parseToolResult(
  response: McpResponse,
  options: BrowserAcceptanceOptions,
): JsonObject {
  if (!isObject(response.result))
    throw new Error("node_repl tools/call returned no result");
  if (response.result.isError === true) {
    const content = Array.isArray(response.result.content)
      ? response.result.content
      : [];
    const message = content.find(isObject)?.text;
    throw new Error(
      typeof message === "string" ? message : "node_repl tools/call failed",
    );
  }
  const content = Array.isArray(response.result.content)
    ? response.result.content
    : [];
  const textItem = content.find(
    (item) => isObject(item) && item.type === "text",
  );
  if (!isObject(textItem) || typeof textItem.text !== "string")
    throw new Error("node_repl tools/call returned no text evidence");
  const imageItems = content.filter(
    (item) => isObject(item) && item.type === "image",
  );
  const beforeImage = imageItems[0];
  const afterImage = imageItems[1];
  if (
    imageItems.length !== 2 ||
    !isEmittedImage(beforeImage) ||
    !isEmittedImage(afterImage)
  )
    throw new Error(
      "browser screenshots were not emitted as two original images",
    );
  const evidence: unknown = JSON.parse(textItem.text);
  if (!isObject(evidence)) throw new Error("browser evidence is not an object");
  if (!browserInfoMatchesAcceptance(evidence.browser, options.scenario, options.sessionId))
    throw new Error(
      options.scenario === "iab"
        ? "browser identity is not a session-matching host_provided_iab"
        : "browser identity is not an extension_native_host extension",
    );
  const navigation = evidence.navigation;
  if (
    !isObject(navigation) ||
    navigation.requested_url !== options.url ||
    navigation.final_url !== options.url
  )
    throw new Error(
      "browser navigation did not reach the exact acceptance URL",
    );
  const keyboard = evidence.keyboard;
  if (
    !isObject(keyboard) ||
    keyboard.method !== "PlaywrightLocator.type+press" ||
    keyboard.key !== "End" ||
    keyboard.text !== options.typedText ||
    keyboard.value !== options.typedText
  )
    throw new Error(
      "browser keyboard input was not observed in the target control",
    );
  const click = evidence.click;
  if (
    !isObject(click) ||
    click.actual !== true ||
    click.button_name !== options.buttonName
  )
    throw new Error("the expected OK button click was not completed");
  if (evidence.readback !== options.typedText)
    throw new Error("browser readback does not match typed input");
  const screenshot = evidence.screenshot;
  if (
    !isObject(screenshot) ||
    screenshot.method !== "Tab.screenshot" ||
    screenshot.emitted !== true ||
    typeof screenshot.expected_width !== "number" ||
    !Number.isInteger(screenshot.expected_width) ||
    screenshot.expected_width < 1 ||
    typeof screenshot.expected_height !== "number" ||
    !Number.isInteger(screenshot.expected_height) ||
    screenshot.expected_height < 1 ||
    typeof screenshot.before_byte_length !== "number" ||
    !Number.isInteger(screenshot.before_byte_length) ||
    screenshot.before_byte_length < 1 ||
    typeof screenshot.after_byte_length !== "number" ||
    !Number.isInteger(screenshot.after_byte_length) ||
    screenshot.after_byte_length < 1
  )
    throw new Error("browser screenshot evidence is incomplete");
  const beforeImageBytes = Buffer.from(beforeImage.data, "base64");
  const afterImageBytes = Buffer.from(afterImage.data, "base64");
  if (
    beforeImageBytes.byteLength !== screenshot.before_byte_length ||
    afterImageBytes.byteLength !== screenshot.after_byte_length
  )
    throw new Error(
      "browser screenshot byte length does not match emitted image",
    );
  if (beforeImage.mimeType !== afterImage.mimeType)
    throw new Error("browser screenshots must use one consistent image format");
  const beforePng = decodeScreenshot(
    beforeImageBytes,
    beforeImage.mimeType,
    "before",
  );
  const afterPng = decodeScreenshot(
    afterImageBytes,
    afterImage.mimeType,
    "after",
  );
  const widthScale = beforePng.width / screenshot.expected_width;
  const heightScale = beforePng.height / screenshot.expected_height;
  if (
    beforePng.width !== afterPng.width ||
    beforePng.height !== afterPng.height ||
    widthScale < 1 ||
    widthScale > 4 ||
    Math.abs(widthScale - heightScale) > 0.01
  )
    throw new Error(
      `browser screenshot dimensions do not match the page viewport: expected ${screenshot.expected_width}x${screenshot.expected_height}, before ${beforePng.width}x${beforePng.height}, after ${afterPng.width}x${afterPng.height}`,
    );
  if (beforePng.pixels.equals(afterPng.pixels))
    throw new Error(
      "browser before/after screenshots have identical decoded pixels",
    );
  const requestMeta = evidence.request_meta;
  if (
    !isObject(requestMeta) ||
    requestMeta.session_id !== options.sessionId ||
    requestMeta.turn_id !== options.turnId ||
    !isObject(requestMeta["x-codex-turn-metadata"]) ||
    requestMeta["x-codex-turn-metadata"].session_id !== options.sessionId ||
    requestMeta["x-codex-turn-metadata"].turn_id !== options.turnId
  )
    throw new Error("session_id/turn_id were not preserved by node_repl");
  const metadata = response.result._meta;
  const toolSurface = isObject(metadata)
    ? metadata["codex/toolSurface"]
    : undefined;
  if (!isObject(toolSurface))
    throw new Error("codex/toolSurface metadata is missing");
  const expectedBackend = options.scenario === "iab" ? "iab" : "chrome";
  if (toolSurface.backend !== expectedBackend)
    throw new Error(
      `toolSurface backend must be ${expectedBackend}; got ${String(toolSurface.backend ?? "missing")}`,
    );
  const sanitizedToolSurface = {
    kind: toolSurface.kind,
    backend: toolSurface.backend,
    browserId: toolSurface.browserId,
    ...(Array.isArray(toolSurface.openTabIds)
      ? { openTabIds: toolSurface.openTabIds }
      : {}),
  };
  return {
    ...evidence,
    emitted_images: {
      before: {
        mime_type: beforeImage.mimeType,
        byte_length: beforeImageBytes.byteLength,
        width: beforePng.width,
        height: beforePng.height,
        detail: "original",
      },
      after: {
        mime_type: afterImage.mimeType,
        byte_length: afterImageBytes.byteLength,
        width: afterPng.width,
        height: afterPng.height,
        detail: "original",
      },
    },
    tool_surface: sanitizedToolSurface,
  };
}

function isEmittedImage(value: unknown): value is JsonObject & {
  data: string;
  mimeType: "image/jpeg" | "image/png" | "image/webp";
} {
  return (
    isObject(value) &&
    (value.mimeType === "image/jpeg" ||
      value.mimeType === "image/png" ||
      value.mimeType === "image/webp") &&
    typeof value.data === "string" &&
    value.data.length > 0 &&
    isObject(value._meta) &&
    value._meta["codex/imageDetail"] === "original"
  );
}

async function invokeInstalledNodeRepl(
  options: BrowserAcceptanceOptions,
  selection: InstalledSelection,
): Promise<JsonObject> {
  const requests = buildMcpRequests(options, selection);
  const childEnvironment: NodeJS.ProcessEnv = {
    ...process.env,
    NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S: selection.browserClientSha256,
  };
  delete childEnvironment.CODEX_BROWSER_PROVIDER;
  const child: ChildProcessWithoutNullStreams = spawn(selection.nodeRepl, [], {
    cwd: dirname(selection.manifestPath),
    env: childEnvironment,
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk: string) => {
    stderr += chunk;
  });
  const responses = new Map<number, McpResponse>();
  const write = (request: JsonObject): void => {
    child.stdin.write(`${JSON.stringify(request)}\n`);
  };
  const reader = createInterface({ input: child.stdout, crlfDelay: Infinity });
  reader.on("line", (line) => {
    const parsed: unknown = JSON.parse(line);
    if (
      isObject(parsed) &&
      parsed.method === "elicitation/create" &&
      (typeof parsed.id === "string" || typeof parsed.id === "number")
    ) {
      const params = parsed.params;
      const meta = isObject(params) ? params.meta : undefined;
      const expectedOrigin = new URL(options.url).origin;
      if (
        isObject(meta) &&
        meta.tool_name === "access_browser_origin" &&
        meta.origin === expectedOrigin
      ) {
        write({
          jsonrpc: "2.0",
          id: parsed.id,
          result: { action: "accept", content: {} },
        });
      } else {
        write({
          jsonrpc: "2.0",
          id: parsed.id,
          error: {
            code: -32602,
            message: "unexpected Browser Use elicitation",
          },
        });
      }
      return;
    }
    if (isObject(parsed) && typeof parsed.id === "number")
      responses.set(parsed.id, parsed);
  });
  const waitFor = async (id: number): Promise<McpResponse> => {
    const deadline = Date.now() + options.timeoutMs + 5_000;
    while (Date.now() < deadline) {
      const response = responses.get(id);
      if (response !== undefined) return response;
      if (child.exitCode !== null)
        throw new Error(
          `node_repl exited before response ${id}: ${stderr.trim()}`,
        );
      await new Promise((resolvePromise) => setTimeout(resolvePromise, 10));
    }
    throw new Error(`timed out waiting for node_repl response ${id}`);
  };
  let teardownAttempted = false;
  let primaryFailure: unknown;
  try {
    write(requests[0] ?? {});
    const initialized = await waitFor(1);
    if (initialized.error !== undefined)
      throw new Error("node_repl initialize failed");
    write(requests[1] ?? {});
    const setup = await waitFor(2);
    if (isObject(setup.result) && setup.result.isError === true)
      throw new Error("Browser live acceptance setup failed");
    write(requests[2] ?? {});
    const result = parseToolResult(await waitFor(3), options);
    write(requests[3] ?? {});
    validateShutdownResponse(await waitFor(4));
    teardownAttempted = true;
    await closeNodeRepl(child);
    return result;
  } catch (error) {
    primaryFailure = error;
    throw error;
  } finally {
    reader.close();
    if (!teardownAttempted) {
      try {
        await closeNodeRepl(child);
      } catch (cleanupFailure) {
        if (primaryFailure !== undefined)
          throw combinePrimaryAndCleanupErrors(primaryFailure, cleanupFailure);
        throw cleanupFailure;
      }
    }
  }
}

function report(
  status: "passed" | "failed",
  connectionAttempted: boolean,
  evidence: JsonObject,
  error?: string,
): JsonObject {
  return {
    schema: SCHEMA,
    schema_version: 1,
    status,
    connection_attempted: connectionAttempted,
    evidence,
    ...(error === undefined ? {} : { error }),
  };
}

export async function main(argv = process.argv.slice(2)): Promise<number> {
  let connectionAttempted = false;
  let fixture: BrowserAcceptanceFixture | undefined;
  try {
    const options = parseBrowserAcceptanceArgs(argv);
    if (options.trustNegative !== "none") {
      const trustEvidence = runPackagedRootTrustNegatives(options);
      process.stdout.write(
        `${JSON.stringify(
          report("passed", false, {
            mode: "packaged-root-trust-negative",
            source_hash_verified_before_copy: true,
            original_untouched: true,
            ...trustEvidence,
          }),
          null,
          2,
        )}\n`,
      );
      return 0;
    }
    const selection = validateInstalledSelection(options);
    const braveOrigin =
      options.scenario === "brave-origin-extension"
        ? verifyBraveOriginNativeHost(options)
        : undefined;
    if (options.ownsFixture) {
      fixture = await startBrowserAcceptanceFixture();
      options.url = fixture.url;
    }
    connectionAttempted = true;
    const liveEvidence = await invokeInstalledNodeRepl(options, selection);
    if (fixture !== undefined) await fixture.close();
    process.stdout.write(
      `${JSON.stringify(
        report("passed", true, {
          scenario: options.scenario,
          runtime_manifest: selection.manifestPath,
          node_repl: selection.nodeRepl,
          browser_client: selection.browserClient,
          browser_client_sha256: selection.browserClientSha256,
          hash_verified_before_connection: true,
          ...(fixture === undefined
            ? { fixture: { owned: false } }
            : {
                fixture: {
                  owned: true,
                  origin: fixture.origin,
                  url: fixture.url,
                  download_url: fixture.downloadUrl,
                  cleaned_up: true,
                },
              }),
          ...(braveOrigin === undefined ? {} : { brave_origin: braveOrigin }),
          ...liveEvidence,
        }),
        null,
        2,
      )}\n`,
    );
    return 0;
  } catch (error) {
    if (fixture !== undefined) {
      try {
        await fixture.close();
      } catch (cleanupError) {
        error = combinePrimaryAndCleanupErrors(error, cleanupError);
      }
    }
    const message = errorText(error);
    process.stdout.write(
      `${JSON.stringify(
        report(
          "failed",
          connectionAttempted,
          {
            hash_verified_before_connection: false,
            failure_phase: connectionAttempted ? "connection" : "preflight",
            wrong_hash_fail_closed:
              !connectionAttempted &&
              message.includes("is not trusted by runtime manifest"),
          },
          message,
        ),
        null,
        2,
      )}\n`,
    );
    return 1;
  }
}

if (import.meta.main) process.exitCode = await main();
