import { strict as assert } from "node:assert";
import {
  chmodSync,
  cpSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, describe, test } from "bun:test";
import {
  discoverRuntimeAssets,
  resolveBrowserExecutableFromPath,
  resolveDefaultBrowserExecutable,
  RuntimeAssetDiscoveryError,
  type RuntimeAssetMetadata,
  type ValidatedRuntimeManifestRecord,
} from "../src/host/runtime-asset-discovery.ts";

const roots: string[] = [];

const manifest: ValidatedRuntimeManifestRecord = {
  manifest_version: 1,
  node_version: "24.14.0",
  node_path: "bin/node",
  node_modules: "lib/node_modules",
  data: {
    playwright: "share/playwright",
    tessdata: "share/tessdata",
    pdfjs: "share/pdfjs",
    licenses: "licenses",
    sbom: "sbom.cdx.json",
  },
};

function tempRoot(label: string): string {
  const root = mkdtempSync(join(tmpdir(), `cua-node-${label}-`));
  roots.push(root);
  return root;
}

function write(path: string, content = "fixture\n"): void {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
}

function buildRuntime(root: string, withWasm = true): void {
  write(join(root, "bin/node"), "#!/bin/sh\nexit 0\n");
  chmodSync(join(root, "bin/node"), 0o755);
  mkdirSync(join(root, "lib/node_modules"), { recursive: true });
  write(join(root, "lib/node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs"));
  write(join(root, "share/playwright/system-browser-required.json"), "{}\n");
  write(join(root, "share/tessdata/osd.traineddata"));
  write(join(root, "share/tessdata/eng.traineddata"));
  write(join(root, "share/pdfjs/cmaps/Identity-H.bcmap"));
  write(join(root, "share/pdfjs/standard_fonts/FoxitSans.pfb"));
  if (withWasm) write(join(root, "share/pdfjs/wasm/openjpeg.wasm"));
  write(join(root, "licenses/NOTICE.txt"));
  write(join(root, "sbom.cdx.json"), "{}\n");
}

function assertDeepFrozen(value: unknown): void {
  if (value === null || typeof value !== "object") return;
  assert.equal(Object.isFrozen(value), true);
  for (const child of Object.values(value)) assertDeepFrozen(child);
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe("runtime asset discovery", () => {
  test("discovers and deeply freezes source-style runtime metadata", async () => {
    const root = tempRoot("source");
    buildRuntime(root);

    const result = await discoverRuntimeAssets({ runtimeRoot: root, manifest });

    assert.equal(result.root, resolve(root));
    assert.equal(result.node.version, "24.14.0");
    assert.equal(result.node.execPath, join(root, "bin/node"));
    assert.equal(result.modules.root, join(root, "lib/node_modules"));
    assert.equal(result.browser.playwrightRoot, join(root, "share/playwright"));
    assert.equal(result.browser.executablePath, null);
    assert.equal(result.browser.executableKind, null);
    assert.equal(result.pdfjs.cMapUrl, `${join(root, "share/pdfjs/cmaps")}/`);
    assert.equal(
      result.pdfjs.standardFontDataUrl,
      `${join(root, "share/pdfjs/standard_fonts")}/`,
    );
    assert.equal(result.pdfjs.wasmUrl, `${join(root, "share/pdfjs/wasm")}/`);
    assert.equal(
      result.pdfjs.workerSrc,
      `file://${join(root, "lib/node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs")}`,
    );
    assert.deepEqual(result.tesseract.languages, ["eng", "osd"]);
    assertDeepFrozen(result);
    assert.equal(Reflect.set(result.node, "version", "changed"), false);
    assert.equal(Reflect.set(result.tesseract.languages, "0", "changed"), false);
  });

  test("derives identical metadata from relocated and assembled-style roots", async () => {
    const sourceRoot = tempRoot("relocation-source");
    buildRuntime(sourceRoot, false);
    const relocationParent = tempRoot("relocated");
    const relocatedRoot = join(relocationParent, "different-prefix/runtime");
    cpSync(sourceRoot, relocatedRoot, { recursive: true });
    const assembledParent = tempRoot("assembled");
    const assembledRoot = join(assembledParent, "resources/cua_node");
    cpSync(sourceRoot, assembledRoot, { recursive: true });

    const relocated = await discoverRuntimeAssets({
      runtimeRoot: relocatedRoot,
      manifest,
    });
    const assembled = await discoverRuntimeAssets({
      runtimeRoot: assembledRoot,
      manifest,
    });

    for (const result of [relocated, assembled]) {
      assert.equal(result.node.execPath.startsWith(`${result.root}/`), true);
      assert.equal(result.modules.root.startsWith(`${result.root}/`), true);
      assert.equal(result.sbomPath.startsWith(`${result.root}/`), true);
      assert.equal(result.pdfjs.wasmUrl, null);
    }
    assert.notEqual(relocated.root, assembled.root);
  });

  test("accepts a reusable external browser resolution without rooting it", async () => {
    const root = tempRoot("browser-runtime");
    buildRuntime(root);
    const browserRoot = tempRoot("browser-external");
    const browserPath = join(browserRoot, "brave");
    write(browserPath, "#!/bin/sh\nexit 0\n");
    chmodSync(browserPath, 0o755);

    const result = await discoverRuntimeAssets({
      runtimeRoot: root,
      manifest,
      resolveBrowserExecutable: () => ({
        executablePath: browserPath,
        executableKind: "brave-origin",
      }),
    });

    assert.deepEqual(result.browser, {
      playwrightRoot: join(root, "share/playwright"),
      executablePath: browserPath,
      executableKind: "brave-origin",
    });
    assert.equal(Object.isFrozen(result.browser), true);
  });

  test("resolves relative PATH entries to absolute executable paths", () => {
    const root = tempRoot("relative-browser-path");
    const browserPath = join(root, "bin/brave-origin");
    write(browserPath, "#!/bin/sh\nexit 0\n");
    chmodSync(browserPath, 0o755);

    assert.deepEqual(resolveBrowserExecutableFromPath({ PATH: "bin" }, root), {
      executablePath: browserPath,
      executableKind: "brave-origin",
    });
  });

  test("classifies configured Brave Origin by its absolute installation path", () => {
    const root = tempRoot("configured-brave-origin");
    const browserPath = join(root, "opt/brave-origin-bin/brave");
    write(browserPath, "#!/bin/sh\nexit 0\n");
    chmodSync(browserPath, 0o755);

    assert.deepEqual(
      resolveDefaultBrowserExecutable(
        { CUA_NODE_CHROMIUM_EXECUTABLE: browserPath, PATH: "" },
        root,
      ),
      {
        executablePath: browserPath,
        executableKind: "brave-origin",
      },
    );
  });

  for (const [path, asset, kind] of [
    ["bin/node", "Node executable", "file"],
    ["lib/node_modules", "Node module root", "directory"],
    ["share/playwright", "Playwright root", "directory"],
    ["share/tessdata", "tessdata root", "directory"],
    ["share/pdfjs", "PDF.js root", "directory"],
    ["licenses", "licenses root", "directory"],
    ["sbom.cdx.json", "SBOM", "file"],
    ["share/pdfjs/cmaps", "PDF.js CMaps", "directory"],
    ["share/pdfjs/standard_fonts", "PDF.js standard fonts", "directory"],
    [
      "lib/node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs",
      "PDF.js worker module",
      "file",
    ],
  ] as const) {
    test(`reports a precise missing error for ${asset}`, async () => {
      const root = tempRoot("missing");
      buildRuntime(root);
      rmSync(join(root, path), { recursive: true });

      await assert.rejects(
        discoverRuntimeAssets({ runtimeRoot: root, manifest }),
        (error: unknown) => {
          assert.ok(error instanceof RuntimeAssetDiscoveryError);
          assert.equal(error.code, "MISSING_PATH");
          assert.equal(error.asset, asset);
          assert.equal(
            error.message,
            `runtime asset ${asset}: required ${kind} is missing: ${join(root, path)}`,
          );
          return true;
        },
      );
    });
  }

  test("rejects a tessdata root with no languages", async () => {
    const root = tempRoot("missing-languages");
    buildRuntime(root);
    rmSync(join(root, "share/tessdata"), { recursive: true });
    mkdirSync(join(root, "share/tessdata"), { recursive: true });

    await assert.rejects(
      discoverRuntimeAssets({ runtimeRoot: root, manifest }),
      (error: unknown) => {
        assert.ok(error instanceof RuntimeAssetDiscoveryError);
        assert.equal(error.code, "MISSING_PATH");
        assert.equal(error.asset, "tessdata languages");
        assert.match(error.message, /required \*\.traineddata files are missing/u);
        return true;
      },
    );
  });

  test("rejects lexical and symlink path traversal", async () => {
    const root = tempRoot("traversal");
    buildRuntime(root);
    const escapedManifest: ValidatedRuntimeManifestRecord = {
      ...manifest,
      data: { ...manifest.data, pdfjs: "../outside/pdfjs" },
    };

    await assert.rejects(
      discoverRuntimeAssets({ runtimeRoot: root, manifest: escapedManifest }),
      (error: unknown) => {
        assert.ok(error instanceof RuntimeAssetDiscoveryError);
        assert.equal(error.code, "PATH_TRAVERSAL");
        assert.equal(error.asset, "PDF.js root");
        assert.match(error.message, /manifest path escapes runtime root/u);
        return true;
      },
    );

    const outside = tempRoot("symlink-outside");
    mkdirSync(join(outside, "node_modules"), { recursive: true });
    rmSync(join(root, "lib/node_modules"), { recursive: true });
    symlinkSync(join(outside, "node_modules"), join(root, "lib/node_modules"));
    await assert.rejects(
      discoverRuntimeAssets({ runtimeRoot: root, manifest }),
      (error: unknown) => {
        assert.ok(error instanceof RuntimeAssetDiscoveryError);
        assert.equal(error.code, "PATH_TRAVERSAL");
        assert.equal(error.asset, "Node module root");
        assert.match(error.message, /resolved path escapes runtime root/u);
        return true;
      },
    );
  });

  test("rejects invalid external browser results precisely", async () => {
    const root = tempRoot("invalid-browser");
    buildRuntime(root);

    await assert.rejects(
      discoverRuntimeAssets({
        runtimeRoot: root,
        manifest,
        resolveBrowserExecutable: () => ({
          executablePath: "relative/browser",
          executableKind: "chromium",
        }),
      }),
      (error: unknown) => {
        assert.ok(error instanceof RuntimeAssetDiscoveryError);
        assert.equal(error.code, "INVALID_BROWSER_RESOLUTION");
        assert.equal(error.asset, "browser executable");
        return true;
      },
    );
  });

  test("returns a serialization-safe record", async () => {
    const root = tempRoot("serialization");
    buildRuntime(root);
    const result: RuntimeAssetMetadata = await discoverRuntimeAssets({
      runtimeRoot: root,
      manifest,
    });

    assert.deepEqual(JSON.parse(JSON.stringify(result)), result);
  });
});
