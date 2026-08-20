import { existsSync } from "node:fs";
import { dirname, join } from "node:path";

/**
 * `node_repl` is installed as `<root>/bin/node_repl` (a freestanding C
 * launcher that execs `<root>/bin/node <root>/lib/node_repl/cli.js`). The
 * bundled runtime root is therefore the ancestor of the node binary that
 * contains `manifest.json` — the same `runtime_root` the contract
 * (`runtime-environment.contract.json`) defines as "the ancestor containing
 * `bin/` and `manifest.json`".
 *
 * The host MCP configs used to pin the four paths below. They are now optional
 * overrides only: when unset we derive them from the runtime root so the server
 * works regardless of install location. This matches the contract's
 * `unset_behavior: derive_from_manifest_or_fail` and the existing
 * `resolveBundledNodePath` derivation.
 */
const NODE_REPL_ENV_DEFAULTS = [
  ["CODEX_NODE_REPL_PATH", "bin/node_repl"],
  ["NODE_REPL_NODE_PATH", "bin/node"],
  ["NODE_REPL_NODE_MODULE_DIRS", "lib/node_modules"],
  ["PLAYWRIGHT_BROWSERS_PATH", "share/playwright"],
] as const;

export function resolveNodeReplRuntimeRoot(
  startPath: string = process.execPath,
): string | undefined {
  let current = dirname(startPath);
  for (;;) {
    if (existsSync(join(current, "manifest.json"))) return current;
    const parent = dirname(current);
    if (parent === current) return undefined;
    current = parent;
  }
}

export function applyNodeReplEnvDefaults(
  startPath: string = process.execPath,
): void {
  const runtimeRoot = resolveNodeReplRuntimeRoot(startPath);
  if (runtimeRoot === undefined) return;
  for (const [key, relative] of NODE_REPL_ENV_DEFAULTS) {
    const existing = process.env[key];
    if (existing === undefined || existing.length === 0) {
      process.env[key] = join(runtimeRoot, relative);
    }
  }
}

applyNodeReplEnvDefaults();
