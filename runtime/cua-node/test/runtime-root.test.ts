import { test } from "bun:test";
import { strict as assert } from "node:assert";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  applyNodeReplEnvDefaults,
  resolveNodeReplRuntimeRoot,
} from "../src/host/runtime-root.ts";

const KEYS = [
  "CODEX_NODE_REPL_PATH",
  "NODE_REPL_NODE_PATH",
  "NODE_REPL_NODE_MODULE_DIRS",
  "PLAYWRIGHT_BROWSERS_PATH",
] as const;

function fakeRoot(): { root: string; nodePath: string } {
  const root = mkdtempSync(join(tmpdir(), "node-repl-root-"));
  const binDir = join(root, "bin");
  mkdirSync(binDir, { recursive: true });
  const nodePath = join(binDir, "node");
  writeFileSync(nodePath, "");
  writeFileSync(join(root, "manifest.json"), "{}");
  return { root, nodePath };
}

function saveEnv(): Array<readonly [string, string | undefined]> {
  return KEYS.map((key) => [key, process.env[key]] as const);
}

function restoreEnv(saved: Array<readonly [string, string | undefined]>): void {
  for (const [key, value] of saved) {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
}

test("resolveNodeReplRuntimeRoot finds the manifest ancestor", () => {
  const { root, nodePath } = fakeRoot();
  try {
    assert.equal(resolveNodeReplRuntimeRoot(nodePath), root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("applyNodeReplEnvDefaults derives the four paths from the runtime root", () => {
  const { root, nodePath } = fakeRoot();
  const saved = saveEnv();
  try {
    applyNodeReplEnvDefaults(nodePath);
    assert.equal(process.env["CODEX_NODE_REPL_PATH"], join(root, "bin/node_repl"));
    assert.equal(process.env["NODE_REPL_NODE_PATH"], join(root, "bin/node"));
    assert.equal(
      process.env["NODE_REPL_NODE_MODULE_DIRS"],
      join(root, "lib/node_modules"),
    );
    assert.equal(
      process.env["PLAYWRIGHT_BROWSERS_PATH"],
      join(root, "share/playwright"),
    );
  } finally {
    restoreEnv(saved);
    rmSync(root, { recursive: true, force: true });
  }
});

test("applyNodeReplEnvDefaults does not override an explicitly set var", () => {
  const { root, nodePath } = fakeRoot();
  const saved = saveEnv();
  process.env["NODE_REPL_NODE_PATH"] = "/custom/node";
  try {
    applyNodeReplEnvDefaults(nodePath);
    assert.equal(process.env["NODE_REPL_NODE_PATH"], "/custom/node");
    // Unset vars are still derived.
    assert.equal(
      process.env["NODE_REPL_NODE_MODULE_DIRS"],
      join(root, "lib/node_modules"),
    );
  } finally {
    restoreEnv(saved);
    rmSync(root, { recursive: true, force: true });
  }
});

test("applyNodeReplEnvDefaults is a no-op when no manifest ancestor exists", () => {
  const outside = mkdtempSync(join(tmpdir(), "node-repl-noroot-"));
  mkdirSync(join(outside, "bin"), { recursive: true });
  const outsideNode = join(outside, "bin", "node");
  writeFileSync(outsideNode, "");
  const saved = saveEnv();
  try {
    applyNodeReplEnvDefaults(outsideNode);
    for (const key of KEYS) {
      assert.equal(process.env[key], saved.find(([k]) => k === key)![1]);
    }
  } finally {
    restoreEnv(saved);
    rmSync(outside, { recursive: true, force: true });
  }
});
