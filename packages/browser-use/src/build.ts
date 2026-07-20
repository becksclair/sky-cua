import { createHash } from "node:crypto";
import { cp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { API_MANIFEST } from "./api.ts";
import { generateDeclarations } from "./generate-declarations.ts";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputRoot = resolve(process.env.BROWSER_USE_BUILD_DIR ?? resolve(packageRoot, "build"));
await rm(outputRoot, { recursive: true, force: true });
await mkdir(outputRoot, { recursive: true });

const result = await Bun.build({
  entrypoints: [resolve(packageRoot, "src/index.ts")],
  outdir: outputRoot,
  naming: "browser-client.mjs",
  target: "node",
  format: "esm",
  minify: true,
  sourcemap: "none",
  packages: "bundle",
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}
await writeFile(resolve(outputRoot, "index.d.ts"), generateDeclarations(API_MANIFEST));
const projection = await Bun.build({
  entrypoints: [resolve(packageRoot, "src/projection.ts")],
  outdir: outputRoot,
  naming: "projection.mjs",
  target: "node",
  format: "esm",
  minify: true,
  sourcemap: "none",
  packages: "bundle",
});
if (!projection.success) {
  for (const log of projection.logs) console.error(log);
  process.exit(1);
}
await cp(resolve(packageRoot, "fixtures"), resolve(outputRoot, "fixtures"), { recursive: true });
const sourcePackage = JSON.parse(await readFile(resolve(packageRoot, "package.json"), "utf8"));
await writeFile(resolve(outputRoot, "package.json"), `${JSON.stringify({
  name: sourcePackage.name,
  version: sourcePackage.version,
  private: true,
  type: "module",
  exports: {
    ".": { types: "./index.d.ts", import: "./browser-client.mjs" },
    "./projection": "./projection.mjs",
  },
  engines: sourcePackage.engines,
}, null, 2)}\n`);
const clientBytes = await readFile(resolve(outputRoot, "browser-client.mjs"));
await writeFile(resolve(outputRoot, "BROWSER_COMPONENT.json"), `${JSON.stringify({
  schemaVersion: 1,
  package: "@heliasar/browser-use",
  version: sourcePackage.version,
  runtime: "node",
  nodeVersion: sourcePackage.engines.node,
  entrypoint: "browser-client.mjs",
  sha256: createHash("sha256").update(clientBytes).digest("hex"),
  size: clientBytes.byteLength,
  interfaceCount: Object.keys(API_MANIFEST.interfaces).length,
  typeCount: Object.keys(API_MANIFEST.types).length,
}, null, 2)}\n`);
