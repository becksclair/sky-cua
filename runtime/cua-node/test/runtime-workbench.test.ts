import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { strict as assert } from "node:assert";
import { test } from "bun:test";
import { RuntimeManager } from "../src/host/runtime-manager.ts";

const candidates = [
  process.env.SKY_CUA_RELEASE_ROOT,
  resolve(import.meta.dir, "../out/linux-x64/cua_node"),
].filter((root): root is string => root !== undefined);
const runtimeRoot = candidates.find(
  (root) =>
    existsSync(`${root}/manifest.json`) &&
    existsSync(`${root}/bin/node`) &&
    existsSync(`${root}/lib/node_modules/pdfjs-dist/legacy/build/pdf.mjs`),
);

test.skipIf(runtimeRoot === undefined)(
  "exact Node runtime imports PDF.js and preserves loader/package identity without a prelude",
  async () => {
    assert.ok(runtimeRoot);
    const manager = new RuntimeManager({
      nodePath: `${runtimeRoot}/bin/node`,
      runtimeRoot,
      env: { NODE_REPL_NODE_MODULE_DIRS: `${runtimeRoot}/lib` },
    });
    try {
      const runtime = await manager.execute(
        "nodeRepl.write(JSON.stringify({version:nodeRepl.runtime.node.version,frozen:Object.isFrozen(nodeRepl.runtime),browser:nodeRepl.runtime.browser.executableKind,loaders:Object.keys(nodeRepl.loaders)}))",
        {
          requestId: 1,
          requestMeta: { session_id: "workbench", turn_id: "runtime" },
        },
      );
      assert.equal(runtime.ok, true, runtime.error ?? "runtime metadata failed");
      const runtimeEvidence = JSON.parse(runtime.output) as Record<string, unknown>;
      assert.equal(runtimeEvidence.version, "24.14.0");
      assert.equal(runtimeEvidence.frozen, true);
      assert.deepEqual(runtimeEvidence.loaders, [
        "canvas",
        "pdfjs",
        "pixelmatch",
        "playwright",
        "sharp",
        "tesseract",
      ]);

      const pdf = await manager.execute(
        "var pdfjsWorkbench=await nodeRepl.loaders.pdfjs(); nodeRepl.write(JSON.stringify({version:pdfjsWorkbench.version,domMatrix:typeof DOMMatrix,path2d:typeof Path2D,navigator:typeof navigator}))",
        {
          requestId: 2,
          requestMeta: { session_id: "workbench", turn_id: "pdf" },
        },
      );
      assert.equal(pdf.ok, true, pdf.error ?? "PDF.js direct import failed");
      assert.deepEqual(JSON.parse(pdf.output), {
        version: "5.4.624",
        domMatrix: "function",
        path2d: "function",
        navigator: "object",
      });

      const identity = await manager.execute(
        'var canvasLoader=await nodeRepl.loaders.canvas(); var canvasImport=await import("@napi-rs/canvas"); nodeRepl.write(JSON.stringify({namespace:canvasLoader===canvasImport,constructor:canvasImport.DOMMatrix===DOMMatrix}))',
        {
          requestId: 3,
          requestMeta: { session_id: "workbench", turn_id: "identity" },
        },
      );
      assert.equal(identity.ok, true, identity.error ?? "Canvas identity failed");
      assert.deepEqual(JSON.parse(identity.output), {
        namespace: true,
        constructor: true,
      });
    } finally {
      await manager.close();
    }
  },
);
