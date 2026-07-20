import { createHash } from "node:crypto";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gunzipSync } from "node:zlib";

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

  const tar = gunzipSync(readFileSync("out/sky-cua-0.1.0.tgz"));
  let offset = 0;
  let acceptanceMode: number | undefined;
  while (offset + 512 <= tar.length) {
    const header = tar.subarray(offset, offset + 512);
    const name = header.subarray(0, 100).toString("utf8").replace(/\0.*$/, "");
    if (name.length === 0) break;
    const mode = Number.parseInt(header.subarray(100, 108).toString("ascii").replace(/\0.*$/, ""), 8);
    const size = Number.parseInt(header.subarray(124, 136).toString("ascii").replace(/\0.*$/, ""), 8);
    if (name === "package/scripts/acceptance-actions.mjs") acceptanceMode = mode;
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  expect(acceptanceMode).toBe(0o755);
});

test("packaged Node 24 acceptance runner exercises the complete action fixture without lifecycle control", () => {
  const build = Bun.spawnSync(["bun", "run", "build"], {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe"
  });
  expect(build.exitCode).toBe(0);
  const result = Bun.spawnSync(["node", "scripts/acceptance-actions.mjs", "--dry-run"], {
    cwd: process.cwd(),
    stdout: "pipe",
    stderr: "pipe"
  });
  expect(result.exitCode).toBe(0);
  const evidence = JSON.parse(new TextDecoder().decode(result.stdout)) as {
    ok: boolean;
    service_lifecycle_control: boolean;
    webp_default: boolean;
    steps: { name: string; ok: boolean }[];
    fixture_request_types: string[];
  };
  expect(evidence.ok).toBe(true);
  expect(evidence.service_lifecycle_control).toBe(false);
  expect(evidence.webp_default).toBe(true);
  expect(evidence.steps.map((step) => step.name)).toEqual([
    "activate_window",
    "get_screenshot",
    "emit_screenshot",
    "move",
    "click",
    "drag",
    "scroll",
    "press_key",
    "type_text"
  ]);
  expect(evidence.steps.every((step) => step.ok)).toBe(true);
  expect(evidence.fixture_request_types).toEqual([
    "health",
    "activate_window",
    "get_screenshot",
    "move",
    "click",
    "drag",
    "scroll",
    "press_key",
    "type_text"
  ]);
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
