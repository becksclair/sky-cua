import { rmSync } from "node:fs";
import { join } from "node:path";

const PACKAGE_ROOT = join(import.meta.dir, "..");
const TYPESCRIPT_CLI = join(PACKAGE_ROOT, "node_modules", "typescript", "bin", "tsc");
const CHILD_ENV_ALLOWLIST = new Set(["HOME", "SystemRoot", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR"]);

function sanitizedChildEnvironment(): Record<string, string> {
  const environment: Record<string, string> = {};
  for (const [name, value] of Object.entries(process.env)) {
    if (CHILD_ENV_ALLOWLIST.has(name) && value !== undefined) {
      environment[name] = value;
    }
  }
  return environment;
}

rmSync(join(PACKAGE_ROOT, "dist"), { recursive: true, force: true });
const result = await Bun.build({
  entrypoints: [
    join(PACKAGE_ROOT, "src", "index.ts"),
    join(PACKAGE_ROOT, "src", "phone", "index.ts")
  ],
  outdir: join(PACKAGE_ROOT, "dist"),
  target: "node",
  format: "esm",
  sourcemap: "none",
  minify: false
});
if (!result.success) {
  throw new Error(result.logs.map((log) => log.message).join("\n"));
}

const typecheck = Bun.spawnSync([process.execPath, TYPESCRIPT_CLI, "-p", "tsconfig.build.json"], {
  cwd: PACKAGE_ROOT,
  env: sanitizedChildEnvironment(),
  stdout: "inherit",
  stderr: "inherit"
});
if (typecheck.exitCode !== 0) {
  throw new Error(`TypeScript declaration build failed with exit code ${typecheck.exitCode}.`);
}
