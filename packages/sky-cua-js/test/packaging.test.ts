import { createHash } from "node:crypto";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { expect, test } from "bun:test";

function packHash(): string {
  const result = Bun.spawnSync(["bun", "run", "pack"], {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe"
  });
  if (result.exitCode !== 0) {
    const stderr = new TextDecoder().decode(result.stderr);
    throw new Error(`deterministic pack failed: ${stderr}`);
  }
  return createHash("sha256")
    .update(readFileSync("out/sky-cua-0.1.0.tgz"))
    .digest("hex");
}

test("pack is an exact deterministic alias and produces identical archives twice", () => {
  const packageJson = JSON.parse(readFileSync("package.json", "utf8")) as {
    scripts?: Record<string, unknown>;
  };
  expect(packageJson.scripts?.pack).toBe("bun run pack:deterministic");
  expect(packHash()).toBe(packHash());
});

test("build ignores an injected ambient tsc and sanitizes its declaration child", () => {
  const fakeBin = mkdtempSync(join(tmpdir(), "sky-cua-fake-tsc-"));
  const fakeTsc = join(fakeBin, "tsc");
  writeFileSync(fakeTsc, "#!/bin/sh\nexit 91\n");
  chmodSync(fakeTsc, 0o755);
  try {
    const result = Bun.spawnSync([process.execPath, "scripts/build.ts"], {
      cwd: process.cwd(),
      env: {
        ...process.env,
        PATH: `${fakeBin}:${process.env.PATH ?? ""}`,
        NODE_PATH: join(fakeBin, "injected-node-modules"),
        TS_NODE_PROJECT: join(fakeBin, "injected-tsconfig.json")
      },
      stdout: "pipe",
      stderr: "pipe"
    });
    const stderr = new TextDecoder().decode(result.stderr);
    expect(result.exitCode).toBe(0);
    expect(stderr.includes("exit code 91")).toBe(false);
  } finally {
    rmSync(fakeBin, { recursive: true, force: true });
  }
});
