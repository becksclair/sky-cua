import { strict as assert } from "node:assert";
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { arch, cpus, hostname, platform } from "node:os";
import { resolve } from "node:path";
import { test } from "bun:test";
import {
  benchmarkVerdicts,
  defaultBenchmarkLockPaths,
  runCuaNodeBenchmark,
  type CuaNodeBenchmarkBaseline,
  type CuaNodeBenchmarkMetrics,
} from "./benchmark";
import { stopChildProcess } from "./web-workbench-acceptance-helper";

const repoRoot = resolve(__dirname, "..");
const fixtureRoot = resolve(__dirname, "../test/fixtures/fake-runtime");

test("installed benchmarks enforce the candidate generation's own lock bytes", () => {
  assert.deepEqual(defaultBenchmarkLockPaths("/installed/cua_node"), [
    "/installed/cua_node/share/locks/runtime-lock.json",
    "/installed/cua_node/share/locks/native-assets.lock.json",
  ]);
});

function baseline(
  metrics: Partial<CuaNodeBenchmarkBaseline["metrics"]> = {},
): CuaNodeBenchmarkBaseline {
  return {
    schema: "com.heliasar.cua-node.benchmark-baseline",
    schema_version: 2,
    id: "synthetic-test-baseline",
    target: "linux-x64",
    machine: {
      platform: platform(),
      arch: arch(),
      hostname: hostname(),
      cpu_model: cpus()[0]?.model ?? "unknown",
    },
    provenance: {
      recorded_at: "2026-07-19T00:00:00.000Z",
      source_commit: "test",
      runtime_sha256: "test",
      command: "synthetic benchmark test",
      harness_state: "synthetic",
    },
    metrics: {
      initialized_cold_start_p95_ms: 1_000,
      initialized_cold_start_samples: 3,
      steady_empty_cells_median_ms: 100,
      steady_empty_cells_p95_ms: 100,
      steady_empty_cells_samples: 3,
      idle_rss_bytes: 128 * 1024 * 1024,
      ...metrics,
    },
  };
}

function syntheticMetrics(
  values: {
    cold?: number;
    steady?: number;
    idleRss?: number;
  } = {},
): CuaNodeBenchmarkMetrics {
  const sample = (value: number) => ({
    samples_ms: [value],
    median_ms: value,
    p95_ms: value,
  });
  return {
    node_spawn: sample(1),
    initialized_cold_start: sample(values.cold ?? 1),
    first_empty_cell: sample(1),
    steady_empty_cells: sample(values.steady ?? 1),
    first_canvas_operation: sample(1),
    first_pdf_operation: sample(1),
    first_sharp_operation: sample(1),
    first_tesseract_operation: sample(1),
    bounded_shutdown: sample(1),
    idle_rss_bytes: values.idleRss ?? 1,
    post_use_rss_bytes: 1,
  };
}

test("benchmark enforces the target, offline, and empty-cache contract", async () => {
  const report = await runCuaNodeBenchmark({
    target: "linux-x64",
    networkDisabled: false,
    emptyUserCache: true,
  });
  assert.equal(report.status, "failed");
  assert.ok(report.blockers.includes("network-enabled-or-unspecified"));
  assert.deepEqual(report.network, {
    mode: "caller-declared-disabled",
    enforcement: "not-sandboxed",
    detail:
      "--network=disabled asserts benchmark intent; no network namespace or syscall sandbox is applied",
  });
});

test("benchmark emits stable MCP/kernel metrics for the verified fixture", async () => {
  const report = await runCuaNodeBenchmark({
    repoRoot,
    runtimeRoot: fixtureRoot,
    target: "linux-x64",
    networkDisabled: true,
    emptyUserCache: true,
    iterations: 3,
    baselineReport: baseline({
      initialized_cold_start_p95_ms: 10_000,
      steady_empty_cells_median_ms: 10_000,
      steady_empty_cells_p95_ms: 10_000,
      idle_rss_bytes: 512 * 1024 * 1024 - 20 * 1024 * 1024,
    }),
    allowFixtureValues: true,
    enforceLockPaths: [],
  });
  assert.equal(report.status, "passed", JSON.stringify(report, null, 2));
  assert.equal(report.iterations, 3);
  assert.equal(report.schema, "com.heliasar.cua-node.benchmark");
  assert.equal(report.schema_version, 5);
  assert.equal(report.metrics.initialized_cold_start.samples_ms.length, 3);
  assert.equal(report.metrics.steady_empty_cells.samples_ms.length, 3);
  assert.equal(typeof report.metrics.first_canvas_operation.p95_ms, "number");
  assert.equal(typeof report.metrics.idle_rss_bytes, "number");
  assert.equal(typeof report.metrics.post_use_rss_bytes, "number");
  assert.equal(typeof report.metrics.bounded_shutdown.p95_ms, "number");
  assert.deepEqual(report.strategy, {
    canvas: "lazy-per-generation",
    loaders: "fixed-promoted",
  });
  assert.equal(report.baseline?.report.id, "synthetic-test-baseline");
  assert.equal(report.verdicts.steady_empty_cells_p95_ms.status, "passed");
  assert.equal(report.verdicts.post_use_rss_bytes.unit, "bytes");
  assert.ok(
    Object.values(report.verdicts).every((verdict) => verdict.status === "passed"),
  );
});

test("locked warm median and p95 thresholds pass at their exact boundaries", () => {
  const frozen = baseline({
    initialized_cold_start_p95_ms: 100,
    steady_empty_cells_median_ms: 40,
    steady_empty_cells_p95_ms: 40,
  });
  const verdicts = benchmarkVerdicts(
    syntheticMetrics({ cold: 140, steady: 44 }),
    frozen,
  );
  assert.deepEqual(
    {
      baseline: verdicts.initialized_cold_start_ms.baseline,
      candidate: verdicts.initialized_cold_start_ms.candidate,
      derived: verdicts.initialized_cold_start_ms.derived_threshold,
      status: verdicts.initialized_cold_start_ms.status,
    },
    { baseline: 100, candidate: 140, derived: 5_000, status: "passed" },
  );
  assert.equal(verdicts.steady_empty_cells_median_ms.derived_threshold, 44);
  assert.equal(verdicts.steady_empty_cells_median_ms.status, "passed");
  assert.equal(verdicts.steady_empty_cells_p95_ms.derived_threshold, 50);
  assert.equal(verdicts.steady_empty_cells_p95_ms.status, "passed");
});

test("warm median over 110% and warm p95 over 125% fail below absolute caps", () => {
  const frozen = baseline({
    steady_empty_cells_median_ms: 10,
    steady_empty_cells_p95_ms: 10,
  });
  const verdicts = benchmarkVerdicts(syntheticMetrics({ steady: 13 }), frozen);
  assert.equal(verdicts.steady_empty_cells_median_ms.derived_threshold, 11);
  assert.equal(verdicts.steady_empty_cells_median_ms.status, "failed");
  assert.equal(verdicts.steady_empty_cells_p95_ms.derived_threshold, 12.5);
  assert.equal(verdicts.steady_empty_cells_p95_ms.status, "failed");
  assert.ok(
    verdicts.steady_empty_cells_p95_ms.candidate! <
      verdicts.steady_empty_cells_p95_ms.safety_ceiling,
  );
});

test("idle RSS allows exactly 20 MiB and rejects one additional byte", () => {
  const baselineRss = 128 * 1024 * 1024;
  const threshold = baselineRss + 20 * 1024 * 1024;
  const frozen = baseline({ idle_rss_bytes: baselineRss });
  const exact = benchmarkVerdicts(
    syntheticMetrics({ idleRss: threshold }),
    frozen,
  ).idle_rss_bytes;
  const over = benchmarkVerdicts(
    syntheticMetrics({ idleRss: threshold + 1 }),
    frozen,
  ).idle_rss_bytes;
  assert.equal(exact.status, "passed");
  assert.equal(over.status, "failed");
});

test("benchmark fails closed when its baseline is missing", async () => {
  const report = await runCuaNodeBenchmark({
    repoRoot,
    target: "linux-x64",
    networkDisabled: true,
    emptyUserCache: true,
    baselinePath: "production/does-not-exist.json",
  });
  assert.equal(report.status, "failed");
  assert.ok(report.blockers.includes("baseline:baseline is missing"));
});

test("benchmark fails closed on malformed or incompatible baselines", async () => {
  const malformed = await runCuaNodeBenchmark({
    repoRoot,
    target: "linux-x64",
    networkDisabled: true,
    emptyUserCache: true,
    baselineReport: { schema: "wrong" },
  });
  assert.equal(malformed.status, "failed");
  assert.match(malformed.detail, /malformed/u);

  const incompatible = baseline();
  incompatible.machine.hostname = "another-machine";
  const incompatibleReport = await runCuaNodeBenchmark({
    repoRoot,
    target: "linux-x64",
    networkDisabled: true,
    emptyUserCache: true,
    baselineReport: incompatible,
  });
  assert.equal(incompatibleReport.status, "failed");
  assert.match(incompatibleReport.detail, /incompatible/u);
});

test("baseline cold sample count must represent at least three sessions", async () => {
  const tooSmall = baseline({ initialized_cold_start_samples: 1 });
  const report = await runCuaNodeBenchmark({
    repoRoot,
    target: "linux-x64",
    networkDisabled: true,
    emptyUserCache: true,
    baselineReport: tooSmall,
  });
  assert.equal(report.status, "failed");
  assert.match(report.detail, /requires at least 3 samples/u);
});

test("benchmark CLI helper is a checked-in runtime entrypoint", () => {
  assert.equal(existsSync(resolve(__dirname, "benchmark.ts")), true);
});

test("benchmark process cleanup reaps a forced MCP host descendant", async () => {
  const fixture = String.raw`
const { spawn } = require("node:child_process");
const descendant = spawn(process.execPath, ["-e", "process.on('SIGTERM', () => {}); setInterval(() => {}, 1000)"], { stdio: "ignore" });
process.stdout.write(String(descendant.pid) + "\n");
process.on("SIGTERM", () => {});
setInterval(() => {}, 1000);
`;
  const child = spawn(process.execPath, ["-e", fixture], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  const descendantPid = await new Promise<number>((resolvePromise, rejectPromise) => {
    child.once("error", rejectPromise);
    child.stdout.once("data", (chunk: Buffer) =>
      resolvePromise(Number(chunk.toString("utf8").trim())),
    );
  });
  try {
    const exit = await stopChildProcess(child, 500, "benchmark fixture host");
    assert.deepEqual(exit, { code: null, signal: "SIGKILL" });
    for (const pid of [child.pid, descendantPid]) {
      assert.throws(
        () => process.kill(pid!, 0),
        (error: unknown) =>
          error !== null &&
          typeof error === "object" &&
          "code" in error &&
          error.code === "ESRCH",
      );
    }
  } finally {
    for (const pid of [child.pid, descendantPid]) {
      if (pid === undefined) continue;
      try {
        process.kill(pid, "SIGKILL");
      } catch {}
    }
  }
});
