import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { arch, cpus, hostname, platform, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  verifyCuaNode,
  type VerificationReport,
} from "../tools/verify-cua-node";
import {
  processTreeRssBytes,
  startMcpSession,
  toolText,
} from "./web-workbench-acceptance-helper";

export type CuaNodeBenchmarkOptions = {
  repoRoot?: string;
  runtimeRoot?: string;
  target?: string;
  networkDisabled?: boolean;
  emptyUserCache?: boolean;
  iterations?: number;
  timeoutMs?: number;
  thresholds?: Partial<CuaNodeBenchmarkThresholds>;
  baselinePath?: string;
  baselineReport?: unknown;
  allowFixtureValues?: boolean;
  enforceLockPaths?: string[];
};

export type Samples = {
  samples_ms: number[];
  median_ms: number | null;
  p95_ms: number | null;
};
export type CuaNodeBenchmarkThresholds = {
  node_spawn_p95_ms: number;
  initialized_cold_start_ms: number;
  first_empty_cell_ms: number;
  steady_empty_cells_median_ms: number;
  steady_empty_cells_p95_ms: number;
  first_canvas_operation_ms: number;
  first_pdf_operation_ms: number;
  first_sharp_operation_ms: number;
  first_tesseract_operation_ms: number;
  bounded_shutdown_ms: number;
  idle_rss_bytes: number;
  post_use_rss_bytes: number;
};
export type CuaNodeBenchmarkMetrics = CuaNodeBenchmarkReport["metrics"];
export type CuaNodeBenchmarkBaseline = {
  schema: "com.heliasar.cua-node.benchmark-baseline";
  schema_version: 2;
  id: string;
  target: "linux-x64";
  machine: {
    platform: string;
    arch: string;
    hostname: string;
    cpu_model: string;
  };
  provenance: {
    recorded_at: string;
    source_commit: string;
    runtime_sha256: string;
    command: string;
    harness_state: string;
  };
  metrics: {
    initialized_cold_start_p95_ms: number;
    initialized_cold_start_samples: number;
    steady_empty_cells_median_ms: number;
    steady_empty_cells_p95_ms: number;
    steady_empty_cells_samples: number;
    idle_rss_bytes: number;
  };
};
export type BenchmarkVerdict = {
  actual: number | null;
  candidate: number | null;
  baseline: number | null;
  derived_threshold: number;
  safety_ceiling: number;
  threshold: number;
  unit: "ms" | "bytes";
  status: "passed" | "failed" | "unavailable";
};
export type CuaNodeBenchmarkReport = {
  schema: "com.heliasar.cua-node.benchmark";
  schema_version: 5;
  status: "passed" | "blocked" | "failed";
  target: "linux-x64";
  network: {
    mode: "caller-declared-disabled";
    enforcement: "not-sandboxed";
    detail: string;
  };
  user_cache: "disposable-empty";
  iterations: number;
  metrics: {
    node_spawn: Samples;
    initialized_cold_start: Samples;
    first_empty_cell: Samples;
    steady_empty_cells: Samples;
    first_canvas_operation: Samples;
    first_pdf_operation: Samples;
    first_sharp_operation: Samples;
    first_tesseract_operation: Samples;
    bounded_shutdown: Samples;
    idle_rss_bytes: number | null;
    post_use_rss_bytes: number | null;
  };
  strategy: { canvas: "lazy-per-generation"; loaders: "fixed-promoted" };
  baseline: {
    source: string;
    report: CuaNodeBenchmarkBaseline;
    formulas: {
      initialized_cold_start_ms: "absolute safety ceiling";
      steady_empty_cells_median_ms: "baseline*1.10";
      steady_empty_cells_p95_ms: "baseline*1.25";
      idle_rss_bytes: "baseline+20MiB";
    };
  } | null;
  verdicts: Record<keyof CuaNodeBenchmarkThresholds, BenchmarkVerdict>;
  operation_errors: Record<string, string>;
  detail: string;
  blockers: string[];
  verification?: VerificationReport;
};

const SCHEMA = "com.heliasar.cua-node.benchmark" as const;
const DEFAULT_RUNTIME_ROOT = "out/linux-x64/cua_node";
const DEFAULT_BASELINE_PATH = "production/benchmark-baseline-linux-x64.json";
const DEFAULT_ITERATIONS = 100;
const COLD_START_SAMPLES = 3;
const IDLE_RSS_ALLOWANCE_BYTES = 20 * 1024 * 1024;
const EMPTY_SAMPLES: Samples = {
  samples_ms: [],
  median_ms: null,
  p95_ms: null,
};
export const DEFAULT_BENCHMARK_THRESHOLDS: CuaNodeBenchmarkThresholds = {
  node_spawn_p95_ms: 500,
  initialized_cold_start_ms: 5_000,
  first_empty_cell_ms: 2_000,
  steady_empty_cells_median_ms: 500,
  steady_empty_cells_p95_ms: 500,
  first_canvas_operation_ms: 10_000,
  first_pdf_operation_ms: 10_000,
  first_sharp_operation_ms: 10_000,
  first_tesseract_operation_ms: 30_000,
  bounded_shutdown_ms: 2_000,
  idle_rss_bytes: 512 * 1024 * 1024,
  post_use_rss_bytes: 1024 * 1024 * 1024,
};

function resolveSafetyCeilings(
  overrides: Partial<CuaNodeBenchmarkThresholds> | undefined,
): { thresholds: CuaNodeBenchmarkThresholds; error: string | null } {
  const thresholds = { ...DEFAULT_BENCHMARK_THRESHOLDS };
  for (const key of Object.keys(thresholds) as Array<
    keyof CuaNodeBenchmarkThresholds
  >) {
    const override = overrides?.[key];
    if (override === undefined) continue;
    if (!Number.isFinite(override) || override < 0)
      return { thresholds, error: `invalid safety ceiling ${key}` };
    thresholds[key] = Math.min(thresholds[key], override);
  }
  return { thresholds, error: null };
}

function samples(values: number[]): Samples {
  const sorted = [...values].sort((a, b) => a - b);
  const midpoint = Math.floor(sorted.length / 2);
  const median =
    sorted.length === 0
      ? null
      : sorted.length % 2 === 0
        ? ((sorted[midpoint - 1] ?? 0) + (sorted[midpoint] ?? 0)) / 2
        : (sorted[midpoint] ?? null);
  const index = Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1);
  return {
    samples_ms: values,
    median_ms: median,
    p95_ms: sorted[index] ?? null,
  };
}

function emptyMetrics(): CuaNodeBenchmarkReport["metrics"] {
  return {
    node_spawn: { ...EMPTY_SAMPLES },
    initialized_cold_start: { ...EMPTY_SAMPLES },
    first_empty_cell: { ...EMPTY_SAMPLES },
    steady_empty_cells: { ...EMPTY_SAMPLES },
    first_canvas_operation: { ...EMPTY_SAMPLES },
    first_pdf_operation: { ...EMPTY_SAMPLES },
    first_sharp_operation: { ...EMPTY_SAMPLES },
    first_tesseract_operation: { ...EMPTY_SAMPLES },
    bounded_shutdown: { ...EMPTY_SAMPLES },
    idle_rss_bytes: null,
    post_use_rss_bytes: null,
  };
}

function verdict(
  candidate: number | null,
  baseline: number | null,
  derivedThreshold: number,
  safetyCeiling: number,
  unit: BenchmarkVerdict["unit"],
): BenchmarkVerdict {
  const threshold = Math.min(derivedThreshold, safetyCeiling);
  return {
    actual: candidate,
    candidate,
    baseline,
    derived_threshold: derivedThreshold,
    safety_ceiling: safetyCeiling,
    threshold,
    unit,
    status:
      candidate === null ? "unavailable" : candidate <= threshold ? "passed" : "failed",
  };
}

export function benchmarkVerdicts(
  metrics: CuaNodeBenchmarkMetrics,
  baseline: CuaNodeBenchmarkBaseline | null,
  safetyCeilings: CuaNodeBenchmarkThresholds = DEFAULT_BENCHMARK_THRESHOLDS,
): CuaNodeBenchmarkReport["verdicts"] {
  const baselineMetrics = baseline?.metrics;
  const steadyMedianThreshold =
    (baselineMetrics?.steady_empty_cells_median_ms ?? 0) * 1.1;
  const steadyP95Threshold =
    (baselineMetrics?.steady_empty_cells_p95_ms ?? 0) * 1.25;
  const idleRssThreshold =
    (baselineMetrics?.idle_rss_bytes ?? 0) + IDLE_RSS_ALLOWANCE_BYTES;
  return {
    node_spawn_p95_ms: verdict(
      metrics.node_spawn.p95_ms,
      null,
      safetyCeilings.node_spawn_p95_ms,
      safetyCeilings.node_spawn_p95_ms,
      "ms",
    ),
    initialized_cold_start_ms: verdict(
      metrics.initialized_cold_start.p95_ms,
      baselineMetrics?.initialized_cold_start_p95_ms ?? null,
      safetyCeilings.initialized_cold_start_ms,
      safetyCeilings.initialized_cold_start_ms,
      "ms",
    ),
    first_empty_cell_ms: verdict(
      metrics.first_empty_cell.p95_ms,
      null,
      safetyCeilings.first_empty_cell_ms,
      safetyCeilings.first_empty_cell_ms,
      "ms",
    ),
    steady_empty_cells_median_ms: verdict(
      metrics.steady_empty_cells.median_ms,
      baselineMetrics?.steady_empty_cells_median_ms ?? null,
      steadyMedianThreshold,
      safetyCeilings.steady_empty_cells_median_ms,
      "ms",
    ),
    steady_empty_cells_p95_ms: verdict(
      metrics.steady_empty_cells.p95_ms,
      baselineMetrics?.steady_empty_cells_p95_ms ?? null,
      steadyP95Threshold,
      safetyCeilings.steady_empty_cells_p95_ms,
      "ms",
    ),
    first_canvas_operation_ms: verdict(
      metrics.first_canvas_operation.p95_ms,
      null,
      safetyCeilings.first_canvas_operation_ms,
      safetyCeilings.first_canvas_operation_ms,
      "ms",
    ),
    first_pdf_operation_ms: verdict(
      metrics.first_pdf_operation.p95_ms,
      null,
      safetyCeilings.first_pdf_operation_ms,
      safetyCeilings.first_pdf_operation_ms,
      "ms",
    ),
    first_sharp_operation_ms: verdict(
      metrics.first_sharp_operation.p95_ms,
      null,
      safetyCeilings.first_sharp_operation_ms,
      safetyCeilings.first_sharp_operation_ms,
      "ms",
    ),
    first_tesseract_operation_ms: verdict(
      metrics.first_tesseract_operation.p95_ms,
      null,
      safetyCeilings.first_tesseract_operation_ms,
      safetyCeilings.first_tesseract_operation_ms,
      "ms",
    ),
    bounded_shutdown_ms: verdict(
      metrics.bounded_shutdown.p95_ms,
      null,
      safetyCeilings.bounded_shutdown_ms,
      safetyCeilings.bounded_shutdown_ms,
      "ms",
    ),
    idle_rss_bytes: verdict(
      metrics.idle_rss_bytes,
      baselineMetrics?.idle_rss_bytes ?? null,
      idleRssThreshold,
      safetyCeilings.idle_rss_bytes,
      "bytes",
    ),
    post_use_rss_bytes: verdict(
      metrics.post_use_rss_bytes,
      null,
      safetyCeilings.post_use_rss_bytes,
      safetyCeilings.post_use_rss_bytes,
      "bytes",
    ),
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function machineQualification(): CuaNodeBenchmarkBaseline["machine"] {
  return {
    platform: platform(),
    arch: arch(),
    hostname: hostname(),
    cpu_model: cpus()[0]?.model ?? "unknown",
  };
}

function validateBaseline(value: unknown): string | null {
  if (!isRecord(value)) return "baseline is not an object";
  if (
    value.schema !== "com.heliasar.cua-node.benchmark-baseline" ||
    value.schema_version !== 2 ||
    value.target !== "linux-x64" ||
    typeof value.id !== "string" ||
    !isRecord(value.machine) ||
    !isRecord(value.provenance) ||
    !isRecord(value.metrics)
  )
    return "baseline schema or required fields are malformed";
  const machine = value.machine;
  const currentMachine = machineQualification();
  for (const key of ["platform", "arch", "hostname", "cpu_model"] as const) {
    if (machine[key] !== currentMachine[key])
      return `baseline machine ${key} is incompatible: expected ${currentMachine[key]}`;
  }
  for (const key of [
    "recorded_at",
    "source_commit",
    "runtime_sha256",
    "command",
    "harness_state",
  ] as const) {
    if (typeof value.provenance[key] !== "string" || value.provenance[key].length === 0)
      return `baseline provenance ${key} is missing`;
  }
  for (const key of [
    "initialized_cold_start_p95_ms",
    "steady_empty_cells_median_ms",
    "steady_empty_cells_p95_ms",
    "idle_rss_bytes",
  ] as const) {
    const metric = value.metrics[key];
    if (typeof metric !== "number" || !Number.isFinite(metric) || metric <= 0)
      return `baseline metric ${key} is malformed`;
  }
  for (const key of [
    "initialized_cold_start_samples",
    "steady_empty_cells_samples",
  ] as const) {
    const count = value.metrics[key];
    if (!Number.isInteger(count) || (count as number) < COLD_START_SAMPLES)
      return `baseline metric ${key} requires at least ${COLD_START_SAMPLES} samples`;
  }
  return null;
}

function loadBaseline(
  repoRoot: string,
  options: CuaNodeBenchmarkOptions,
): { baseline: CuaNodeBenchmarkBaseline | null; source: string; error: string | null } {
  const source =
    options.baselineReport === undefined
      ? resolve(repoRoot, options.baselinePath ?? DEFAULT_BASELINE_PATH)
      : "explicit-report";
  let value = options.baselineReport;
  if (value === undefined) {
    if (!existsSync(source))
      return { baseline: null, source, error: "baseline is missing" };
    try {
      value = JSON.parse(readFileSync(source, "utf8")) as unknown;
    } catch (error) {
      const detail = error instanceof Error ? error.message : "invalid JSON";
      return { baseline: null, source, error: `baseline cannot be read: ${detail}` };
    }
  }
  const error = validateBaseline(value);
  return {
    baseline: error === null ? (value as CuaNodeBenchmarkBaseline) : null,
    source,
    error,
  };
}

function report(
  status: CuaNodeBenchmarkReport["status"],
  detail: string,
  blockers: string[],
  iterations: number,
  safetyCeilings: CuaNodeBenchmarkThresholds = DEFAULT_BENCHMARK_THRESHOLDS,
  baseline: CuaNodeBenchmarkBaseline | null = null,
  baselineSource = "unavailable",
): CuaNodeBenchmarkReport {
  const metrics = emptyMetrics();
  return {
    schema: SCHEMA,
    schema_version: 5,
    status,
    target: "linux-x64",
    network: {
      mode: "caller-declared-disabled",
      enforcement: "not-sandboxed",
      detail:
        "--network=disabled asserts benchmark intent; no network namespace or syscall sandbox is applied",
    },
    user_cache: "disposable-empty",
    iterations,
    metrics,
    strategy: { canvas: "lazy-per-generation", loaders: "fixed-promoted" },
    baseline:
      baseline === null
        ? null
        : {
            source: baselineSource,
            report: baseline,
            formulas: {
              initialized_cold_start_ms: "absolute safety ceiling",
              steady_empty_cells_median_ms: "baseline*1.10",
              steady_empty_cells_p95_ms: "baseline*1.25",
              idle_rss_bytes: "baseline+20MiB",
            },
          },
    verdicts: benchmarkVerdicts(metrics, baseline, safetyCeilings),
    operation_errors: {},
    detail,
    blockers,
  };
}

function elapsed(start: bigint): number {
  return Number(process.hrtime.bigint() - start) / 1_000_000;
}

const OPERATIONS = {
  canvas:
    'var benchmarkCanvas = await nodeRepl.loaders.canvas(); var benchmarkSurface = benchmarkCanvas.createCanvas(16, 16); benchmarkSurface.getContext("2d").fillRect(0,0,16,16); console.log(benchmarkSurface.width);',
  pdf: "var benchmarkPdf = await nodeRepl.loaders.pdfjs(); console.log(typeof benchmarkPdf.getDocument);",
  sharp:
    "var benchmarkSharpModule = await nodeRepl.loaders.sharp(); var benchmarkSharp = benchmarkSharpModule.default; var benchmarkSharpBytes = await benchmarkSharp({create:{width:8,height:8,channels:4,background:{r:1,g:2,b:3,alpha:1}}}).png().toBuffer(); console.log(benchmarkSharpBytes.length);",
  tesseract:
    'var benchmarkTesseract = await nodeRepl.loaders.tesseract(); var benchmarkWorker = await benchmarkTesseract.createWorker("eng", benchmarkTesseract.OEM.LSTM_ONLY, {langPath:"__TESSDATA_ROOT__",gzip:false,cacheMethod:"none",logger:function(){}}); await benchmarkWorker.terminate(); console.log("terminated");',
} as const;

export async function runCuaNodeBenchmark(
  options: CuaNodeBenchmarkOptions = {},
): Promise<CuaNodeBenchmarkReport> {
  const iterations = options.iterations ?? DEFAULT_ITERATIONS;
  const safetyCeilings = resolveSafetyCeilings(options.thresholds);
  const thresholds = safetyCeilings.thresholds;
  if (!Number.isInteger(iterations) || iterations < 1)
    return report(
      "failed",
      "iterations must be a positive integer",
      ["invalid-iterations"],
      DEFAULT_ITERATIONS,
    );
  if (safetyCeilings.error !== null)
    return report(
      "failed",
      safetyCeilings.error,
      ["invalid-safety-ceiling"],
      iterations,
      thresholds,
    );
  if (options.target !== "linux-x64")
    return report(
      "failed",
      "benchmark requires --target linux-x64",
      ["unsupported-or-missing-target"],
      iterations,
    );
  if (options.networkDisabled !== true)
    return report(
      "failed",
      "benchmark requires the caller assertion --network=disabled; networking is not sandboxed",
      ["network-enabled-or-unspecified"],
      iterations,
      thresholds,
    );
  if (options.emptyUserCache !== true)
    return report(
      "failed",
      "benchmark requires --empty-user-cache",
      ["user-cache-not-empty-or-unspecified"],
      iterations,
      thresholds,
    );
  const repoRoot = resolve(options.repoRoot ?? process.cwd());
  const baselineResult = loadBaseline(repoRoot, options);
  if (baselineResult.error !== null)
    return report(
      "failed",
      `benchmark baseline rejected: ${baselineResult.error}`,
      [`baseline:${baselineResult.error}`],
      iterations,
      thresholds,
    );
  const baseline = baselineResult.baseline;
  if (baseline === null)
    return report(
      "failed",
      "benchmark baseline rejected without a diagnostic",
      ["baseline:unknown-error"],
      iterations,
      thresholds,
    );
  const runtimeRoot = resolve(repoRoot, options.runtimeRoot ?? DEFAULT_RUNTIME_ROOT);
  if (!existsSync(runtimeRoot))
    return report(
      "blocked",
      `assembled runtime is absent: ${runtimeRoot}`,
      [`runtime-root:${runtimeRoot}`],
      iterations,
      thresholds,
      baseline,
      baselineResult.source,
    );
  const verification = verifyCuaNode({
    root: runtimeRoot,
    expectedTarget: "linux-x64-glibc",
    allowFixtureValues: options.allowFixtureValues,
    enforceLockPaths: options.enforceLockPaths ?? [
      join(repoRoot, "runtime-lock.json"),
      join(repoRoot, "native-assets.lock.json"),
    ],
  });
  if (verification.status !== "passed")
    return {
      ...report(
        verification.status === "blocked" ? "blocked" : "failed",
        "benchmark candidate failed verification preflight",
        verification.blockers,
        iterations,
        thresholds,
        baseline,
        baselineResult.source,
      ),
      verification,
    };

  const metrics = emptyMetrics();
  const nodePath = join(runtimeRoot, "bin/node");
  const nodeRepl = join(runtimeRoot, "bin/node_repl");
  const tempRoot = mkdtempSync(join(tmpdir(), "cua-node-benchmark-"));
  try {
    const nodeSamples: number[] = [];
    for (let index = 0; index < Math.min(iterations, 10); index += 1) {
      const start = process.hrtime.bigint();
      const result = spawnSync(nodePath, ["--version"], {
        stdio: "ignore",
        env: { ...process.env, HOME: tempRoot },
      });
      nodeSamples.push(elapsed(start));
      if (result.status !== 0)
        throw new Error(`bundled Node spawn exited ${String(result.status)}`);
    }
    metrics.node_spawn = samples(nodeSamples);
    const sessionEnvironment = (home: string) => ({
      ...process.env,
      HOME: home,
      XDG_CACHE_HOME: join(home, "cache"),
      NODE_REPL_NODE_PATH: nodePath,
      NODE_REPL_NODE_MODULE_DIRS: join(runtimeRoot, "lib/node_modules"),
      PLAYWRIGHT_BROWSERS_PATH: "0",
      TESSDATA_PREFIX: join(runtimeRoot, "share/tessdata"),
      NO_PROXY: "*",
      no_proxy: "*",
    });
    const initialize = async (home: string) => {
      const coldStart = process.hrtime.bigint();
      const coldSession = startMcpSession({
        executable: nodeRepl,
        cwd: home,
        timeoutMs: options.timeoutMs ?? 120_000,
        env: sessionEnvironment(home),
      });
      try {
        const initialized = await coldSession.request("initialize", {
          protocolVersion: "2025-11-25",
          capabilities: {},
          clientInfo: { name: "cua-node-benchmark", version: "4" },
        });
        if (initialized.error !== undefined)
          throw new Error(`initialize failed: ${JSON.stringify(initialized.error)}`);
        return { session: coldSession, elapsedMs: elapsed(coldStart) };
      } catch (error) {
        await coldSession.close(2_000).catch(() => undefined);
        throw error;
      }
    };
    const coldSamples: number[] = [];
    for (let index = 0; index < COLD_START_SAMPLES; index += 1) {
      const coldHome = join(tempRoot, `cold-${index + 1}`);
      mkdirSync(coldHome);
      const initialized = await initialize(coldHome);
      coldSamples.push(initialized.elapsedMs);
      try {
        const shutdown = await initialized.session.request(
          "shutdown",
          {},
          options.timeoutMs ?? 120_000,
        );
        if (shutdown.error !== undefined || shutdown.result !== null)
          throw new Error(`cold session shutdown failed: ${JSON.stringify(shutdown)}`);
        const exit = await initialized.session.close(2_000);
        if (exit.code !== 0 || exit.signal !== null)
          throw new Error(`unclean cold MCP exit: ${JSON.stringify(exit)}`);
      } catch (error) {
        await initialized.session.close(2_000).catch(() => undefined);
        throw error;
      }
    }
    metrics.initialized_cold_start = samples(coldSamples);
    const warmedHome = join(tempRoot, "warmed");
    mkdirSync(warmedHome);
    const initializedWarmed = await initialize(warmedHome);
    const session = initializedWarmed.session;
    const operationErrors: Record<string, string> = {};
    try {
      if (session.child.pid === undefined) throw new Error("MCP host has no PID");
      metrics.idle_rss_bytes = processTreeRssBytes(session.child.pid);
      const call = async (code: string, title: string) => {
        const start = process.hrtime.bigint();
        const response = await session.request(
          "tools/call",
          {
            name: "js",
            arguments: { code, title, timeout_ms: options.timeoutMs ?? 120_000 },
          },
          options.timeoutMs ?? 120_000,
        );
        toolText(response, title);
        return elapsed(start);
      };
      metrics.first_empty_cell = samples([
        await call("void 0", "Benchmark first empty cell"),
      ]);
      const steady: number[] = [];
      for (let index = 0; index < iterations; index += 1)
        steady.push(await call("void 0", `Benchmark steady cell ${index + 1}`));
      metrics.steady_empty_cells = samples(steady);
      const operation = async (id: string, code: string, title: string) => {
        const start = process.hrtime.bigint();
        try {
          await call(code, title);
        } catch (error) {
          operationErrors[id] =
            error instanceof Error ? error.message : "operation failed";
        }
        return elapsed(start);
      };
      const tesseractCode = OPERATIONS.tesseract.replace(
        '"__TESSDATA_ROOT__"',
        JSON.stringify(join(runtimeRoot, "share/tessdata")),
      );
      metrics.first_canvas_operation = samples([
        await operation(
          "first_canvas_operation",
          OPERATIONS.canvas,
          "Benchmark Canvas",
        ),
      ]);
      metrics.first_pdf_operation = samples([
        await operation("first_pdf_operation", OPERATIONS.pdf, "Benchmark PDF.js"),
      ]);
      metrics.first_sharp_operation = samples([
        await operation("first_sharp_operation", OPERATIONS.sharp, "Benchmark Sharp"),
      ]);
      metrics.first_tesseract_operation = samples([
        await operation(
          "first_tesseract_operation",
          tesseractCode,
          "Benchmark Tesseract",
        ),
      ]);
      metrics.post_use_rss_bytes = processTreeRssBytes(session.child.pid);
      const shutdownStart = process.hrtime.bigint();
      const shutdown = await session.request(
        "shutdown",
        {},
        options.timeoutMs ?? 120_000,
      );
      if (shutdown.error !== undefined || shutdown.result !== null)
        throw new Error(`shutdown failed: ${JSON.stringify(shutdown)}`);
      const exit = await session.close(2_000);
      metrics.bounded_shutdown = samples([elapsed(shutdownStart)]);
      if (exit.code !== 0 || exit.signal !== null)
        throw new Error(
          `unclean MCP exit: ${JSON.stringify(exit)} ${session.stderr()}`,
        );
    } catch (error) {
      await session.close(2_000).catch(() => undefined);
      throw error;
    }
    const verdicts = benchmarkVerdicts(metrics, baseline, thresholds);
    const failedVerdicts = Object.entries(verdicts)
      .filter(([, result]) => result.status !== "passed")
      .map(([name]) => `threshold:${name}`);
    const hasOperationErrors = Object.keys(operationErrors).length > 0;
    const final = report(
      hasOperationErrors ? "blocked" : failedVerdicts.length > 0 ? "failed" : "passed",
      hasOperationErrors
        ? "MCP benchmark completed with runtime capability blockers"
        : failedVerdicts.length > 0
          ? "benchmark exceeded the relative baseline contract or absolute safety ceiling"
          : "benchmark satisfied the relative baseline contract and absolute safety ceilings",
      hasOperationErrors
        ? ["nodeRepl.runtime-or-web-globals-not-integrated"]
        : failedVerdicts,
      iterations,
      thresholds,
      baseline,
      baselineResult.source,
    );
    final.operation_errors = operationErrors;
    return { ...final, metrics, verdicts, verification };
  } catch (error) {
    return {
      ...report(
        "failed",
        error instanceof Error ? error.message : "benchmark failed",
        ["mcp-kernel-benchmark-failed"],
        iterations,
        thresholds,
        baseline,
        baselineResult.source,
      ),
      metrics,
      verdicts: benchmarkVerdicts(metrics, baseline, thresholds),
      verification,
    };
  } finally {
    rmSync(tempRoot, { recursive: true, force: true });
  }
}

if (require.main === module) {
  const values = new Map(
    process.argv
      .slice(2)
      .filter((arg) => arg.includes("="))
      .map((arg) => arg.split(/=(.*)/su).slice(0, 2) as [string, string]),
  );
  void runCuaNodeBenchmark({
    target: values.get("--target"),
    runtimeRoot: values.get("--root"),
    iterations: values.has("--iterations")
      ? Number(values.get("--iterations"))
      : DEFAULT_ITERATIONS,
    timeoutMs: values.has("--timeout-ms")
      ? Number(values.get("--timeout-ms"))
      : undefined,
    networkDisabled: process.argv.includes("--network=disabled"),
    emptyUserCache: process.argv.includes("--empty-user-cache"),
  }).then((reportValue) => {
    console.log(JSON.stringify(reportValue, null, 2));
    if (reportValue.status !== "passed") process.exitCode = 1;
  });
}
