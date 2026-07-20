import { createHash } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

export const CODEX_PROJECTION_PATHS = [
  "openai-bundled/plugins/browser-use/scripts/browser-client.mjs",
  "openai-bundled/plugins/chrome/scripts/browser-client.mjs",
] as const;

export type ProjectionResult = {
  sha256: string;
  size: number;
  paths: string[];
};

export async function materializeCodexProjections(
  canonicalClientPath: string,
  projectionRoot: string,
): Promise<ProjectionResult> {
  const bytes = await readFile(canonicalClientPath);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const paths: string[] = [];
  for (const relativePath of CODEX_PROJECTION_PATHS) {
    const path = resolve(projectionRoot, relativePath);
    await mkdir(resolve(path, ".."), { recursive: true });
    await writeFile(path, bytes);
    paths.push(path);
  }
  return { sha256, size: bytes.byteLength, paths };
}

if (import.meta.main) {
  const [canonicalClientPath, projectionRoot] = process.argv.slice(2);
  if (canonicalClientPath === undefined || projectionRoot === undefined) {
    throw new Error("usage: bun src/projection.ts <canonical-browser-client.mjs> <projection-root>");
  }
  console.log(JSON.stringify(await materializeCodexProjections(canonicalClientPath, projectionRoot)));
}
