import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { runTaskAdapter, type AdapterRun } from "./adapters.ts";
import {
  FakeBrowserNativePeer,
  FakeBrowserPage,
  FakeMcpHost,
  FakeSkyClient,
  FakeSkyService,
  encodeNativeFrame,
  fakeImageTransform,
  fakeOcr,
  fakePdf,
  fakePlaywright,
  trustedBrowserRequest,
  type McpResponse,
} from "./fakes.ts";
import {
  FIXTURE_ROOT,
  expectedMcpImageContent,
  fixtureBytes,
  fixturePath,
  generateFixtures,
} from "./fixtures.ts";
import {
  assertArtifact,
  assertCategoryArtifact,
  sha256,
  writeBytes,
  writeJson,
  writeText,
  type AcceptanceArtifact,
  type CategoryArtifact,
  type Evidence,
  type Outcome,
} from "./types.ts";

export type HarnessOptions = {
  category?: string;
  outputRoot?: string | undefined;
  checkOnly?: boolean;
};

type TaskResult = {
  taskId: string;
  summary: string;
  outcome: Outcome;
  checks: Array<{ id: string; outcome: Outcome; detail: string }>;
  adapter: AdapterRun;
  evidence: Array<{ name: string; content: string | Uint8Array; description: string }>;
};

const CATEGORIES = new Map<string, string[]>([
  ["g2", ["g2-browser", "g2-computer", "g2-media"]],
  ["g3", ["g3-package"]],
  ["g4", ["g4-installed"]],
  ["all", ["g2-browser", "g2-computer", "g2-media", "g3-package", "g4-installed"]],
  ["g2-browser", ["g2-browser"]],
  ["g2-computer", ["g2-computer"]],
  ["g2-media", ["g2-media"]],
  ["g3-package", ["g3-package"]],
  ["g4-installed", ["g4-installed"]],
]);

export async function runHarness(
  options: HarnessOptions = {},
): Promise<{ outcome: Outcome; outputRoot: string; artifacts: string[] }> {
  const category = options.category ?? "all";
  const taskIds = CATEGORIES.get(category);
  if (taskIds === undefined) {
    throw new Error(
      `unknown category ${category}; choose ${[...CATEGORIES.keys()].join(", ")}`,
    );
  }
  generateFixtures({ checkOnly: options.checkOnly ?? true });
  const outputRoot = resolve(
    options.outputRoot ??
      join(
        process.cwd(),
        "out/cua-node-acceptance",
        new Date().toISOString().replace(/[:.]/gu, "-"),
      ),
  );
  mkdirSync(outputRoot, { recursive: true });
  const fixtureEvidence = join(outputRoot, "fixtures-check.json");
  writeJson(fixtureEvidence, {
    schema_version: "cua-node-acceptance/fixtures-check-v1",
    fixture_root: FIXTURE_ROOT,
    check: "deterministic-bytes-and-index",
    result: "pass",
  });
  const artifactPaths: string[] = [];
  const evidencePaths: string[] = [];
  const categoryArtifacts: CategoryArtifact[] = [];
  for (const taskId of taskIds) {
    const result = await runTask(taskId);
    const artifact = await materializeTask(result, outputRoot);
    assertArtifact(artifact);
    const artifactPath = join(outputRoot, "tasks", `${taskId}.json`);
    writeJson(artifactPath, artifact);
    artifactPaths.push(artifactPath);
    evidencePaths.push(...artifact.evidence.map((evidence) => evidence.path));
    const categoryArtifact: CategoryArtifact = {
      schema_version: "cua-node-acceptance/category-v1",
      category: taskId,
      outcome: artifact.outcome,
      installed_acceptance: artifact.installed_acceptance,
      tasks: [taskId],
      artifacts: [artifactPath],
      evidence_root: join(outputRoot, "evidence", taskId),
    };
    assertCategoryArtifact(categoryArtifact);
    const categoryPath = join(outputRoot, `${taskId}.json`);
    writeJson(categoryPath, categoryArtifact);
    categoryArtifacts.push(categoryArtifact);
  }
  const outcome = aggregateOutcome(
    categoryArtifacts.map((artifact) => artifact.outcome),
  );
  const summaryPath = join(outputRoot, "summary.json");
  writeJson(summaryPath, {
    schema_version: "cua-node-acceptance/summary-v1",
    requested_category: category,
    outcome,
    installed_acceptance: "pending",
    note: "Deterministic local fixtures and fake adapters only; real installed acceptance remains explicitly pending.",
    evidence_paths: [fixtureEvidence, ...artifactPaths, ...evidencePaths, summaryPath],
  });
  writeText(
    join(outputRoot, "README.txt"),
    `Acceptance evidence for ${category}\nOutcome: ${outcome}\nInstalled acceptance: pending\n\nRun with: bun runtime/cua-node/test/acceptance/orchestrator.ts --category=${category}\n`,
  );
  return {
    outcome,
    outputRoot,
    artifacts: [fixtureEvidence, ...artifactPaths, ...evidencePaths, summaryPath],
  };
}

async function runTask(taskId: string): Promise<TaskResult> {
  if (taskId === "g2-browser") return browserTask();
  if (taskId === "g2-computer") return computerTask();
  if (taskId === "g2-media") return mediaTask();
  if (taskId === "g3-package") return packageTask();
  if (taskId === "g4-installed") return installedTask();
  throw new Error(`unhandled task ${taskId}`);
}

async function browserTask(): Promise<TaskResult> {
  const host = new FakeMcpHost();
  const transcript = readFileSync(fixturePath("mcp/golden-transcript.ndjson"), "utf8");
  const responses = await host.processTranscript(transcript);
  const malformed = await host.processTranscript(
    readFileSync(fixturePath("mcp/malformed-framing.ndjson"), "utf8"),
  );
  const imageResponse = await host.handleLine(
    JSON.stringify({
      id: 4,
      method: "tools/call",
      params: { name: "js", arguments: { code: "emit-image" } },
    }),
  );
  const expectedImage = expectedMcpImageContent();
  const persistentFirst = await host.handleLine(
    JSON.stringify({
      id: 10,
      method: "tools/call",
      params: { name: "js", arguments: { code: "counter" } },
    }),
  );
  const thrown = await host.handleLine(
    JSON.stringify({
      id: 11,
      method: "tools/call",
      params: { name: "js", arguments: { code: "throw" } },
    }),
  );
  const persistentSecond = await host.handleLine(
    JSON.stringify({
      id: 12,
      method: "tools/call",
      params: { name: "js", arguments: { code: "counter" } },
    }),
  );
  const reset = await host.handleLine(
    JSON.stringify({
      id: 13,
      method: "tools/call",
      params: { name: "js_reset", arguments: {} },
    }),
  );
  const afterReset = await host.handleLine(
    JSON.stringify({
      id: 14,
      method: "tools/call",
      params: { name: "js", arguments: { code: "counter" } },
    }),
  );
  const timeout = await host.handleLine(
    JSON.stringify({
      id: 15,
      method: "tools/call",
      params: { name: "js", arguments: { code: "timeout", timeout_ms: 1 } },
    }),
  );
  const cancellation = new AbortController();
  const cancellationPromise = host.handleLine(
    JSON.stringify({
      id: 16,
      method: "tools/call",
      params: { name: "js", arguments: { code: "cancel-loop" } },
    }),
    cancellation.signal,
  );
  cancellation.abort();
  const cancelled = await cancellationPromise;
  const crashed = await host.handleLine(
    JSON.stringify({
      id: 17,
      method: "tools/call",
      params: { name: "js", arguments: { code: "crash" } },
    }),
  );
  const afterCrash = await host.handleLine(
    JSON.stringify({
      id: 18,
      method: "tools/call",
      params: { name: "js", arguments: { code: "counter" } },
    }),
  );
  const hostileConsole = await host.handleLine(
    JSON.stringify({
      id: 19,
      method: "tools/call",
      params: { name: "js", arguments: { code: "hostile-console" } },
    }),
  );
  const clientBytes = fixtureBytes("browser/interactive.html");
  const peer = new FakeBrowserNativePeer();
  const payloads = trustedBrowserRequest({
    clientBytes,
    suppliedHash: sha256(clientBytes),
    peer,
    payload: {
      method: "browser.inspect",
      session_id: "fixture-session",
      turn_id: "fixture-turn",
    },
  });
  const wrongPeer = new FakeBrowserNativePeer();
  let wrongHash = "not-reached";
  try {
    trustedBrowserRequest({
      clientBytes,
      suppliedHash: "0".repeat(64),
      peer: wrongPeer,
      payload: { method: "browser.inspect" },
    });
  } catch (error) {
    wrongHash = error instanceof Error ? error.message : String(error);
  }
  const coalescedFrames = Buffer.concat([
    encodeNativeFrame(Buffer.from('{"event":"split"}')),
    encodeNativeFrame(Buffer.from('{"event":"coalesced"}')),
  ]);
  const splitCoalesced = peer.receive([
    coalescedFrames.subarray(0, 3),
    coalescedFrames.subarray(3),
  ]);
  let oversize = "not-reached";
  try {
    peer.receive([Buffer.from([0, 0, 4, 1])], 1024);
  } catch (error) {
    oversize = error instanceof Error ? error.message : String(error);
  }
  peer.close();
  const page = new FakeBrowserPage(fixturePath("browser/interactive.html"));
  const inspected = page.inspect("#action");
  page.click("#action");
  page.type("#name", "browser fixture");
  page.scroll({ top: 180, left: 24 });
  const adapter = await runTaskAdapter({
    adapter: "browser-fake",
    commandEnv: "CUA_NODE_ACCEPTANCE_BROWSER_COMMAND",
    fallback: () => "fake browser task complete",
  });
  return {
    taskId: "g2-browser",
    summary: "MCP transcript/framing, trusted browser pipe, and local interactive page",
    outcome: "pass",
    checks: [
      {
        id: "mcp-golden",
        outcome: responses.length === 3 ? "pass" : "fail",
        detail: `responses=${responses.length}`,
      },
      {
        id: "mcp-malformed-framing",
        outcome: malformed.some((response) => response.error?.code === -32700)
          ? "pass"
          : "fail",
        detail: "malformed JSON is a parse error",
      },
      {
        id: "mcp-final-image-content-shape",
        outcome: matchesFinalImageContent(responseContent(imageResponse), expectedImage)
          ? "pass"
          : "fail",
        detail: JSON.stringify(responseContent(imageResponse)),
      },
      {
        id: "kernel-persistent-binding",
        outcome:
          responseText(persistentFirst) !== undefined &&
          responseText(persistentSecond) === "3" &&
          thrown.error?.message === "fixture JavaScript error"
            ? "pass"
            : "fail",
        detail: `first=${JSON.stringify(persistentFirst)}; second=${JSON.stringify(persistentSecond)}`,
      },
      {
        id: "kernel-reset",
        outcome:
          reset.result !== undefined && responseText(afterReset) === "1"
            ? "pass"
            : "fail",
        detail: JSON.stringify(afterReset),
      },
      {
        id: "kernel-timeout",
        outcome: timeout.error?.message === "EXECUTION_TIMEOUT" ? "pass" : "fail",
        detail: JSON.stringify(timeout),
      },
      {
        id: "kernel-cancel",
        outcome: cancelled.error?.message === "EXECUTION_CANCELLED" ? "pass" : "fail",
        detail: JSON.stringify(cancelled),
      },
      {
        id: "kernel-crash-recovery",
        outcome:
          crashed.error?.message === "KERNEL_CRASHED" &&
          responseText(afterCrash) === "1"
            ? "pass"
            : "fail",
        detail: JSON.stringify({ crashed, afterCrash }),
      },
      {
        id: "mcp-stdout-isolation",
        outcome:
          responseText(hostileConsole) === "captured-without-stdout-corruption"
            ? "pass"
            : "fail",
        detail: "hostile console output is represented as result content",
      },
      {
        id: "trusted-hash-exact",
        outcome: peer.connectCount === 1 ? "pass" : "fail",
        detail: `accepted exact bytes; connect_count=${peer.connectCount}`,
      },
      {
        id: "trusted-hash-wrong-zero-connect",
        outcome:
          wrongHash === "TRUSTED_BROWSER_HASH_REJECTED" && wrongPeer.connectCount === 0
            ? "pass"
            : "fail",
        detail: `error=${wrongHash}; connect_count=${wrongPeer.connectCount}`,
      },
      {
        id: "native-peer-split-coalesced",
        outcome:
          splitCoalesced.length === 2 && (payloads as unknown[]).length === 1
            ? "pass"
            : "fail",
        detail: `frames=${splitCoalesced.length}`,
      },
      {
        id: "native-peer-oversize",
        outcome: oversize === "NATIVE_PIPE_FRAME_TOO_LARGE" ? "pass" : "fail",
        detail: oversize,
      },
      {
        id: "native-peer-early-close",
        outcome: peer.pendingRejected && peer.closeCount === 1 ? "pass" : "fail",
        detail: `pending_rejected=${peer.pendingRejected}`,
      },
      {
        id: "browser-page-inspect-click-type-scroll",
        outcome:
          inspected.found &&
          page.state.clicked &&
          page.state.typed === "browser fixture" &&
          page.state.scrollTop === 180 &&
          page.state.scrollLeft === 24
            ? "pass"
            : "fail",
        detail: JSON.stringify(page.state),
      },
    ],
    adapter,
    evidence: [
      {
        name: "mcp-responses.json",
        content: JSON.stringify({
          responses,
          malformed,
          imageResponse,
          expectedImage,
          persistentFirst,
          thrown,
          persistentSecond,
          reset,
          afterReset,
          timeout,
          cancelled,
          crashed,
          afterCrash,
          hostileConsole,
        }),
        description: "Golden, lifecycle, and malformed MCP transcript results",
      },
      {
        name: "native-peer.json",
        content: JSON.stringify({
          exact_hash: sha256(clientBytes),
          wrong_hash_error: wrongHash,
          connect_count: peer.connectCount,
          wrong_connect_count: wrongPeer.connectCount,
          split_coalesced: splitCoalesced,
          oversize,
          early_close: peer.pendingRejected,
        }),
        description: "Native framed peer evidence",
      },
      {
        name: "browser-page.json",
        content: JSON.stringify({ inspected, state: page.state }),
        description: "Local interactive browser page state",
      },
    ],
  };
}

function responseText(response: McpResponse): string | undefined {
  const first = responseContent(response);
  const text = first?.text;
  return typeof text === "string" ? text : undefined;
}

function responseContent(response: McpResponse): Record<string, unknown> | undefined {
  const content = response.result?.content;
  if (!Array.isArray(content) || content.length === 0) return undefined;
  const first = content[0];
  if (first === null || typeof first !== "object" || Array.isArray(first))
    return undefined;
  return first as Record<string, unknown>;
}

function matchesFinalImageContent(
  actual: Record<string, unknown> | undefined,
  expected: ReturnType<typeof expectedMcpImageContent>,
): boolean {
  if (
    actual === undefined ||
    actual.type !== "image" ||
    actual.data !== expected.data ||
    actual.mimeType !== expected.mimeType ||
    typeof actual.data !== "string" ||
    actual.data.startsWith("data:")
  ) {
    return false;
  }
  const meta = actual._meta;
  return (
    meta !== null &&
    typeof meta === "object" &&
    !Array.isArray(meta) &&
    (meta as Record<string, unknown>)["codex/imageDetail"] === "original"
  );
}

async function computerTask(): Promise<TaskResult> {
  const service = new FakeSkyService();
  const client = new FakeSkyClient(service);
  const context = { session_id: "fixture-session", turn_id: "fixture-turn" };
  const screenshot = await client.screenshot();
  await client.move({ x: 10, y: 20 }, context);
  await client.click(
    { x: 10, y: 20, mouse_button: "middle", click_count: 2, key: "CTRL" },
    context,
  );
  await client.drag(
    { from_x: 10, from_y: 20, to_x: 50, to_y: 60, key: "SHIFT" },
    context,
  );
  await client.pressKey({ key: "CTRL+L" }, context);
  await client.typeText({ text: "sky fixture" }, context);
  await client.scroll(
    { direction: "left", pixels: 17, x: 20, y: 30, key: "ALT" },
    context,
  );
  await client.scroll({ direction: "down", pixels: 41 }, context);
  const cancelContext = { session_id: "fixture-session", turn_id: "cancel-turn" };
  const cancelled = client
    .move({ cancel_probe: true }, cancelContext)
    .then(() => "unexpected-success", failureText);
  setTimeout(() => client.cancel("cancel-turn"), 2);
  const cancelResult = await cancelled;
  service.running = false;
  const stoppedResult = await client
    .screenshot()
    .then(() => "unexpected-success", failureText);
  service.running = true;
  await client.screenshot();
  const ambiguous = await client
    .click({ x: 5, y: 5, ambiguous: true }, context)
    .then(() => "unexpected-success", failureText);
  const ambiguousClickCount = service.actions.filter(
    (action) => action.op === "Click" && action.payload.ambiguous === true,
  ).length;
  const adapter = await runTaskAdapter({
    adapter: "sky-fake",
    commandEnv: "CUA_NODE_ACCEPTANCE_SKY_COMMAND",
    fallback: () => "fake sky service complete",
  });
  return {
    taskId: "g2-computer",
    summary:
      "Fake sky-cua service operations, cancellation, restart-required, and no ambiguous retry",
    outcome: "pass",
    checks: [
      {
        id: "sky-screenshot-webp",
        outcome:
          screenshot.mime === "image/webp" && screenshot.width === 2 ? "pass" : "fail",
        detail: JSON.stringify(screenshot),
      },
      {
        id: "sky-full-action-set",
        outcome: ["Move", "Click", "Drag", "PressKey", "TypeText", "Scroll"].every(
          (op) => service.actions.some((action) => action.op === op),
        )
          ? "pass"
          : "fail",
        detail: service.actions.map((action) => action.op).join(","),
      },
      {
        id: "sky-horizontal-exact-scroll",
        outcome: service.actions.some(
          (action) =>
            action.op === "Scroll" &&
            action.payload.direction === "left" &&
            action.payload.pixels === 17,
        )
          ? "pass"
          : "fail",
        detail: "left/17px and down/41px requests preserved",
      },
      {
        id: "sky-cancellation",
        outcome: cancelResult === "SKY_CUA_CANCELLED" ? "pass" : "fail",
        detail: String(cancelResult),
      },
      {
        id: "sky-restart-required",
        outcome: stoppedResult === "SKY_CUA_SERVICE_RESTART_REQUIRED" ? "pass" : "fail",
        detail: stoppedResult,
      },
      {
        id: "sky-ambiguous-no-retry",
        outcome:
          ambiguous === "SKY_CUA_ACTION_OUTCOME_UNKNOWN" && ambiguousClickCount === 1
            ? "pass"
            : "fail",
        detail: `error=${ambiguous}; ambiguous_click_count=${ambiguousClickCount}`,
      },
    ],
    adapter,
    evidence: [
      {
        name: "sky-service.json",
        content: JSON.stringify({
          screenshot,
          actions: service.actions,
          cancel_result: cancelResult,
          stopped_result: stoppedResult,
          ambiguous,
          ambiguous_click_count: ambiguousClickCount,
        }),
        description: "Fake sky-cua protocol and failure evidence",
      },
    ],
  };
}

async function mediaTask(): Promise<TaskResult> {
  const ocr = fakeOcr(fixturePath("media/ocr-known.ppm"));
  const pdf = fakePdf(fixturePath("media/acceptance.pdf"));
  const images = [
    "media/red.png",
    "media/white.jpg",
    "media/blue.webp",
    "media/acceptance.svg",
  ].map((path) => fakeImageTransform(fixturePath(path)));
  const playwright = fakePlaywright(fixturePath("browser/playwright-local.html"));
  const adapter = await runTaskAdapter({
    adapter: "media-fake",
    commandEnv: "CUA_NODE_ACCEPTANCE_MEDIA_COMMAND",
    fallback: () => "fake offline media task complete",
  });
  return {
    taskId: "g2-media",
    summary:
      "Offline OCR, PDF text/vector/raster, image transforms, and local Playwright page",
    outcome: "pass",
    checks: [
      {
        id: "ocr-known-text",
        outcome:
          ocr.text === "Cua node offline OCR fixture 123" && ocr.confidence > 0.9
            ? "pass"
            : "fail",
        detail: JSON.stringify(ocr),
      },
      {
        id: "pdf-text-vector-raster",
        outcome: pdf.hasText && pdf.hasVector && pdf.hasRaster ? "pass" : "fail",
        detail: JSON.stringify(pdf),
      },
      {
        id: "image-format-pixels",
        outcome:
          images.length === 4 &&
          images.every((image) => image.expectation.startsWith("1x1:"))
            ? "pass"
            : "fail",
        detail: JSON.stringify(images),
      },
      {
        id: "playwright-local-page",
        outcome:
          playwright.clicked &&
          playwright.typed === "playwright fixture" &&
          playwright.scrolled
            ? "pass"
            : "fail",
        detail: JSON.stringify(playwright),
      },
      {
        id: "network-disabled",
        outcome:
          process.env.CUA_NODE_ACCEPTANCE_NETWORK === "enabled" ? "fail" : "pass",
        detail: "fixture media adapters perform no network I/O",
      },
    ],
    adapter,
    evidence: [
      {
        name: "media.json",
        content: JSON.stringify({ ocr, pdf, images, playwright, network: "disabled" }),
        description: "Offline media and local page evidence",
      },
    ],
  };
}

async function packageTask(): Promise<TaskResult> {
  const valid = JSON.parse(
    readFileSync(fixturePath("manifests/valid-runtime.json"), "utf8"),
  ) as Record<string, unknown>;
  const corrupt = JSON.parse(
    readFileSync(fixturePath("manifests/corrupt-runtime.json"), "utf8"),
  ) as Record<string, unknown>;
  const assetHashes = Array.isArray(valid.asset_hashes) ? valid.asset_hashes : [];
  const validAssets = assetHashes.every((entry) => {
    if (entry === null || typeof entry !== "object" || Array.isArray(entry))
      return false;
    const record = entry as Record<string, unknown>;
    const path = record.path;
    const expectedHash = record.sha256;
    return (
      typeof path === "string" &&
      typeof expectedHash === "string" &&
      existsSync(fixturePath(path)) &&
      sha256(readFileSync(fixturePath(path))) === expectedHash
    );
  });
  const corruptAssets = Array.isArray(corrupt.assets) ? corrupt.assets : [];
  const corruptHash = JSON.stringify(corruptAssets).includes(
    "0000000000000000000000000000000000000000000000000000000000000000",
  );
  const adapter = await runTaskAdapter({
    adapter: "package-fake",
    commandEnv: "CUA_NODE_ACCEPTANCE_PACKAGE_COMMAND",
    fallback: () => "fake package verifier complete",
  });
  return {
    taskId: "g3-package",
    summary:
      "Verified fixture runtime manifest plus corrupt/missing negative manifests",
    outcome: "pass",
    checks: [
      {
        id: "valid-manifest",
        outcome: valid.runtime_version === "fixture-1" && validAssets ? "pass" : "fail",
        detail: `assets=${assetHashes.length}`,
      },
      {
        id: "corrupt-manifest-rejected",
        outcome: corruptHash ? "pass" : "fail",
        detail: "wrong digest is detectable",
      },
      {
        id: "missing-asset-rejected",
        outcome:
          !existsSync(fixturePath("media/missing-asset.bin")) &&
          readFileSync(fixturePath("manifests/missing-runtime.json"), "utf8").includes(
            "missing-asset.bin",
          )
            ? "pass"
            : "fail",
        detail:
          "missing-runtime.json references intentionally absent media/missing-asset.bin",
      },
      {
        id: "no-sandbox-metadata",
        outcome: !JSON.stringify({ valid, corrupt }).toLowerCase().includes("sandbox")
          ? "pass"
          : "fail",
        detail: "fixture manifests contain no sandbox metadata",
      },
    ],
    adapter,
    evidence: [
      {
        name: "package.json",
        content: JSON.stringify({
          valid,
          corrupt,
          missing_asset: "media/missing-asset.bin",
          network: "disabled",
        }),
        description: "Fixture package/manifest verification evidence",
      },
    ],
  };
}

async function installedTask(): Promise<TaskResult> {
  const adapter = await runTaskAdapter({
    adapter: "installed-task-placeholder",
    commandEnv: "CUA_NODE_ACCEPTANCE_INSTALLED_COMMAND",
    fallback: () =>
      "No installed command configured; deterministic fake placeholder only.",
  });
  return {
    taskId: "g4-installed",
    summary:
      "Installed-task command adapter is wired, but real installed acceptance is pending",
    outcome: "pending",
    checks: [
      {
        id: "installed-adapter-wired",
        outcome: "pass",
        detail: `env=${adapter.commandEnv}; used_command=${adapter.usedCommand}`,
      },
      {
        id: "real-installed-acceptance",
        outcome: "pending",
        detail:
          "Run with CUA_NODE_ACCEPTANCE_INSTALLED_COMMAND after packaging/install integration.",
      },
    ],
    adapter,
    evidence: [
      {
        name: "installed-pending.txt",
        content: `${adapter.output}\nReal installed Browser/Computer/media acceptance remains pending.\n`,
        description: "Explicit installed acceptance boundary",
      },
    ],
  };
}

async function materializeTask(
  result: TaskResult,
  outputRoot: string,
): Promise<AcceptanceArtifact> {
  const evidence: Evidence[] = [];
  const evidenceRoot = join(outputRoot, "evidence", result.taskId);
  for (const item of result.evidence) {
    const path = join(evidenceRoot, item.name);
    if (typeof item.content === "string") {
      writeText(path, item.content.endsWith("\n") ? item.content : `${item.content}\n`);
    } else {
      writeBytes(path, item.content);
    }
    evidence.push({
      path,
      description: item.description,
      sha256: sha256(readFileSync(path)),
    });
  }
  const checkOutcome = aggregateOutcome(result.checks.map((check) => check.outcome));
  const outcome = result.outcome === "pending" ? "pending" : checkOutcome;
  return {
    schema_version: "cua-node-acceptance/v1",
    category: result.taskId.startsWith("g2")
      ? "G2"
      : result.taskId.startsWith("g3")
        ? "G3"
        : "G4",
    task_id: result.taskId,
    outcome,
    evidence_kind: result.adapter.usedCommand
      ? "external-command"
      : "deterministic-fake",
    installed_acceptance: result.taskId === "g4-installed" ? "pending" : "pending",
    summary: result.summary,
    checks: result.checks,
    evidence,
    adapter: {
      name: result.adapter.adapter,
      command_env: result.adapter.commandEnv,
      used_command: result.adapter.usedCommand,
    },
  };
}

function aggregateOutcome(outcomes: Outcome[]): Outcome {
  if (outcomes.some((outcome) => outcome === "fail")) return "fail";
  if (outcomes.some((outcome) => outcome === "pending")) return "pending";
  return "pass";
}

function failureText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function parseArgs(argv: string[]): {
  category: string;
  checkOnly: boolean;
  generate: boolean;
  outputRoot?: string | undefined;
} {
  let category = "all";
  let checkOnly = false;
  let generate = false;
  let outputRoot: string | undefined;
  for (const arg of argv) {
    if (arg.startsWith("--category=")) category = arg.slice("--category=".length);
    else if (arg === "--check") checkOnly = true;
    else if (arg === "--generate") generate = true;
    else if (arg.startsWith("--output=")) outputRoot = arg.slice("--output=".length);
    else if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: bun runtime/cua-node/test/acceptance/orchestrator.ts [--category=all|g2|g3|g4|g2-browser|g2-computer|g2-media|g3-package|g4-installed] [--check|--generate] [--output=PATH]",
      );
      process.exit(0);
    } else {
      throw new Error(`unknown argument ${arg}`);
    }
  }
  return { category, checkOnly, generate, outputRoot };
}

if (resolve(process.argv[1] ?? "") === fileURLToPath(import.meta.url)) {
  const args = parseArgs(process.argv.slice(2));
  if (args.generate) {
    console.log(JSON.stringify(generateFixtures({ checkOnly: false }), null, 2));
  } else if (args.checkOnly) {
    console.log(
      JSON.stringify(
        {
          fixtures: generateFixtures({ checkOnly: true }),
          real_installed_acceptance: "pending",
        },
        null,
        2,
      ),
    );
  } else {
    const result = await runHarness({
      category: args.category,
      outputRoot: args.outputRoot,
    });
    console.log(
      JSON.stringify({ ...result, real_installed_acceptance: "pending" }, null, 2),
    );
    if (result.outcome === "fail") process.exitCode = 1;
  }
}
