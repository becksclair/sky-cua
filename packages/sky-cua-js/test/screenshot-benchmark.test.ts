import { expect, test } from "bun:test";

test("Node screenshot benchmark self-test proves calculations and connection semantics", () => {
  const result = Bun.spawnSync([
    "node",
    "--expose-gc",
    "scripts/benchmark-screenshot.mjs",
    "--self-test"
  ], {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe"
  });
  const stdout = new TextDecoder().decode(result.stdout);
  const stderr = new TextDecoder().decode(result.stderr);
  expect(result.exitCode).toBe(0);
  expect(stderr).toBe("");
  expect(stdout.includes("self-test: PASS")).toBe(true);
  expect(stdout.includes("one health each")).toBe(true);
  expect(stdout.includes("exact WebP parity")).toBe(true);
});
