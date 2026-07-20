import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

type JsonRecord = Record<string, unknown>;

export type SubsystemId =
  | "playwright"
  | "pdfjs"
  | "tesseract"
  | "sharp"
  | "canvas"
  | "webp"
  | "codecs"
  | "pixel-diff";

export type SubsystemResult = {
  id: SubsystemId;
  status: "passed" | "blocked" | "failed";
  parity: "locked-artifact-present" | "available-local-proof" | "not-claimed";
  detail: string;
  blocker?: string;
};

export type SubsystemHarnessReport = {
  status: "passed" | "blocked" | "failed";
  network: "disabled";
  user_cache: "empty-by-contract";
  parity_claim: "none";
  results: SubsystemResult[];
  local_proofs: {
    sharp: SubsystemResult;
    system_browser: SubsystemResult;
  };
};

export type SubsystemHarnessOptions = {
  runtimeRoot?: string;
  target?: string;
  sharpPackageRoot?: string;
  browserPath?: string;
  pixelmatchPath?: string;
  networkDisabled?: boolean;
  emptyUserCache?: boolean;
};

function packageRoot(runtimeRoot: string, name: string): string {
  return join(runtimeRoot, "lib/node_modules", name);
}

function packageVersion(
  runtimeRoot: string,
  name: string,
  expected: string,
): SubsystemResult {
  const path = packageRoot(runtimeRoot, name);
  const packageJson = join(path, "package.json");
  if (!existsSync(packageJson))
    return {
      id: idForPackage(name),
      status: "blocked",
      parity: "not-claimed",
      detail: `locked package is absent: ${name}@${expected}`,
      blocker: `${name}@${expected}`,
    };
  try {
    const value = JSON.parse(readFileSync(packageJson, "utf8")) as JsonRecord;
    if (value.name !== name || value.version !== expected)
      return {
        id: idForPackage(name),
        status: "failed",
        parity: "not-claimed",
        detail: `wrong package identity at ${packageJson}`,
        blocker: `${name}@${expected}`,
      };
    return {
      id: idForPackage(name),
      status: "passed",
      parity: "locked-artifact-present",
      detail: `${name}@${expected} is present`,
    };
  } catch (error) {
    return {
      id: idForPackage(name),
      status: "failed",
      parity: "not-claimed",
      detail: error instanceof Error ? error.message : `cannot read ${packageJson}`,
      blocker: `${name}@${expected}`,
    };
  }
}

function idForPackage(name: string): SubsystemId {
  if (name === "playwright" || name === "playwright-core") return "playwright";
  if (name === "pdfjs-dist") return "pdfjs";
  if (name === "tesseract.js" || name === "tesseract.js-core") return "tesseract";
  if (name === "sharp" || name.startsWith("@img/sharp")) return "sharp";
  if (name.startsWith("@napi-rs/canvas")) return "canvas";
  return "pixel-diff";
}

function pathResult(
  id: SubsystemId,
  path: string,
  detail: string,
  blocker: string,
): SubsystemResult {
  return existsSync(path)
    ? { id, status: "passed", parity: "locked-artifact-present", detail }
    : {
        id,
        status: "blocked",
        parity: "not-claimed",
        detail: `locked artifact is absent: ${path}`,
        blocker,
      };
}

function runSharpProof(sharpPackageRoot?: string): SubsystemResult {
  if (sharpPackageRoot === undefined || !existsSync(sharpPackageRoot))
    return {
      id: "sharp",
      status: "blocked",
      parity: "not-claimed",
      detail:
        "set CUA_NODE_SHARP_PATH to an approved local Sharp package to run the proof",
      blocker: "sharp-local-proof-input",
    };
  const node = process.env.CUA_NODE_PROOF_NODE ?? "node";
  const script =
    "const sharp=require(process.argv[1]); sharp({create:{width:1,height:1,channels:4,background:{r:255,g:0,b:0,alpha:1}}}).resize(2,2).webp().toBuffer().then(b=>{if(b.length<16)process.exit(2); console.log(JSON.stringify({bytes:b.length,format:'webp',parity:'local-proof-only'}))}).catch(()=>process.exit(3));";
  const result = spawnSync(node, ["-e", script, resolve(sharpPackageRoot)], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, npm_config_ignore_scripts: "true" },
  });
  if (result.status !== 0)
    return {
      id: "sharp",
      status: "failed",
      parity: "not-claimed",
      detail: `local Sharp proof failed: ${(result.stderr || result.stdout).trim() || `exit ${String(result.status)}`}`,
      blocker: "sharp-local-proof-failed",
    };
  return {
    id: "sharp",
    status: "passed",
    parity: "available-local-proof",
    detail: `Sharp resize/WebP proof passed without claiming bundled parity: ${(result.stdout || "").trim()}`,
  };
}

function runSystemBrowserProof(browserPath?: string): SubsystemResult {
  if (browserPath === undefined || !existsSync(browserPath))
    return {
      id: "playwright",
      status: "blocked",
      parity: "not-claimed",
      detail:
        "install Chromium, Brave, Chrome, or another configured Chromium-family executable",
      blocker: "playwright-system-browser",
    };
  const result = spawnSync(browserPath, ["--version"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, HOME: "/nonexistent-cua-node-proof" },
  });
  if (result.status !== 0)
    return {
      id: "playwright",
      status: "failed",
      parity: "not-claimed",
      detail: `system browser proof failed: ${(result.stderr || result.stdout).trim() || `exit ${String(result.status)}`}`,
      blocker: "playwright-system-browser-failed",
    };
  const firstLine = (result.stdout || "").split("\n")[0] ?? "browser";
  return {
    id: "playwright",
    status: "passed",
    parity: "available-local-proof",
    detail: `system Chromium-family browser is available: ${firstLine}`,
  };
}

export function runSubsystemHarness(
  options: SubsystemHarnessOptions = {},
): SubsystemHarnessReport {
  const results: SubsystemResult[] = [];
  const runtimeRoot =
    options.runtimeRoot === undefined ? undefined : resolve(options.runtimeRoot);
  if (options.target !== undefined && options.target !== "linux-x64")
    results.push({
      id: "playwright",
      status: "failed",
      parity: "not-claimed",
      detail: `target must be linux-x64, got ${options.target}`,
      blocker: "unsupported-target",
    });
  if (options.networkDisabled === false) {
    results.push({
      id: "playwright",
      status: "failed",
      parity: "not-claimed",
      detail: "network must be disabled for the offline subsystem harness",
      blocker: "network-enabled",
    });
  }
  if (options.emptyUserCache === false) {
    results.push({
      id: "playwright",
      status: "failed",
      parity: "not-claimed",
      detail: "user cache must be empty by contract",
      blocker: "user-cache-not-empty",
    });
  }

  if (runtimeRoot === undefined) {
    for (const id of [
      "playwright",
      "pdfjs",
      "tesseract",
      "sharp",
      "canvas",
      "webp",
      "codecs",
      "pixel-diff",
    ] as const)
      results.push({
        id,
        status: "blocked",
        parity: "not-claimed",
        detail: "no assembled runtime root was supplied",
        blocker: `runtime-root:${id}`,
      });
  } else {
    const playwright = packageVersion(runtimeRoot, "playwright", "1.57.0");
    const playwrightCore = packageVersion(runtimeRoot, "playwright-core", "1.57.0");
    const browser = runSystemBrowserProof(
      options.browserPath ??
        process.env.CUA_NODE_BROWSER_PATH ??
        ["/usr/bin/chromium", "/usr/bin/brave", "/usr/bin/google-chrome"].find(
          (path) => existsSync(path),
        ),
    );
    results.push(
      browser.status === "passed" &&
        playwright.status === "passed" &&
        playwrightCore.status === "passed"
        ? {
            id: "playwright",
            status: "passed",
            parity: "locked-artifact-present",
            detail: `${playwright.detail}; ${playwrightCore.detail}; ${browser.detail}`,
          }
        : {
            id: "playwright",
            status:
              playwright.status === "failed" || playwrightCore.status === "failed"
                ? "failed"
                : "blocked",
            parity: "not-claimed",
            detail: `${playwright.detail}; ${playwrightCore.detail}; ${browser.detail}`,
            blocker:
              playwright.blocker ?? playwrightCore.blocker ?? browser.blocker,
          },
    );

    const pdf = packageVersion(runtimeRoot, "pdfjs-dist", "5.4.624");
    const pdfData = pathResult(
      "pdfjs",
      join(runtimeRoot, "share/pdfjs"),
      "PDF.js local data root is present",
      "pdfjs-local-data",
    );
    results.push({
      id: "pdfjs",
      status:
        pdf.status === "passed" && pdfData.status === "passed"
          ? "passed"
          : pdf.status === "failed" || pdfData.status === "failed"
            ? "failed"
            : "blocked",
      parity:
        pdf.status === "passed" && pdfData.status === "passed"
          ? "locked-artifact-present"
          : "not-claimed",
      detail: `${pdf.detail}; ${pdfData.detail}`,
      blocker: pdf.blocker ?? pdfData.blocker,
    });

    const tesseract = packageVersion(runtimeRoot, "tesseract.js", "7.0.0");
    const tesseractCore = packageVersion(runtimeRoot, "tesseract.js-core", "7.0.0");
    const tessdata = pathResult(
      "tesseract",
      join(runtimeRoot, "share/tessdata/eng.traineddata"),
      "English tessdata is present",
      "tessdata-eng-approved-bundle",
    );
    results.push({
      id: "tesseract",
      status:
        tesseract.status === "passed" &&
        tesseractCore.status === "passed" &&
        tessdata.status === "passed"
          ? "passed"
          : tesseract.status === "failed" ||
              tesseractCore.status === "failed" ||
              tessdata.status === "failed"
            ? "failed"
            : "blocked",
      parity: "not-claimed",
      detail: `${tesseract.detail}; ${tesseractCore.detail}; ${tessdata.detail}`,
      blocker: tesseract.blocker ?? tesseractCore.blocker ?? tessdata.blocker,
    });

    const sharp = packageVersion(runtimeRoot, "sharp", "0.34.5");
    const sharpNative = pathResult(
      "sharp",
      join(runtimeRoot, "lib/node_modules/@img/sharp-linux-x64"),
      "Sharp Linux native package is present",
      "sharp-linux-x64-0.34.5",
    );
    const libvips = pathResult(
      "sharp",
      join(runtimeRoot, "lib/node_modules/@img/sharp-libvips-linux-x64"),
      "libvips Linux package is present",
      "sharp-libvips-linux-x64-1.2.4",
    );
    results.push({
      id: "sharp",
      status:
        sharp.status === "passed" &&
        sharpNative.status === "passed" &&
        libvips.status === "passed"
          ? "passed"
          : "blocked",
      parity: "not-claimed",
      detail: `${sharp.detail}; ${sharpNative.detail}; ${libvips.detail}`,
      blocker: sharp.blocker ?? sharpNative.blocker ?? libvips.blocker,
    });

    const canvas = packageVersion(runtimeRoot, "@napi-rs/canvas", "0.1.91");
    const canvasNative = packageVersion(
      runtimeRoot,
      "@napi-rs/canvas-linux-x64-gnu",
      "0.1.91",
    );
    results.push({
      id: "canvas",
      status:
        canvas.status === "passed" && canvasNative.status === "passed"
          ? "passed"
          : "blocked",
      parity: "not-claimed",
      detail: `${canvas.detail}; ${canvasNative.detail}`,
      blocker: canvas.blocker ?? canvasNative.blocker,
    });
    results.push(
      results.some((entry) => entry.id === "sharp" && entry.status === "passed")
        ? {
            id: "webp",
            status: "passed",
            parity: "locked-artifact-present",
            detail: "WebP is covered by the locked Sharp/libvips codec path",
          }
        : {
            id: "webp",
            status: "blocked",
            parity: "not-claimed",
            detail: "WebP proof is blocked until locked Sharp/libvips is present",
            blocker: "image-codecs-via-libvips",
          },
    );
    const pixelmatch =
      options.pixelmatchPath === undefined
        ? packageVersion(runtimeRoot, "pixelmatch", "7.1.0")
        : pathResult(
            "pixel-diff",
            options.pixelmatchPath,
            "pixelmatch implementation is present",
            "visual-diff-pixelmatch-7.1.0",
          );
    results.push(pixelmatch);
  }

  const localSharp = runSharpProof(
    options.sharpPackageRoot ?? process.env.CUA_NODE_SHARP_PATH,
  );
  const localBrowser = runSystemBrowserProof(
    options.browserPath ?? process.env.CUA_NODE_BROWSER_PATH,
  );
  const status =
    results.some((entry) => entry.status === "failed") ||
    localSharp.status === "failed" ||
    localBrowser.status === "failed"
      ? "failed"
      : results.some((entry) => entry.status === "blocked")
        ? "blocked"
        : "passed";
  return {
    status,
    network: "disabled",
    user_cache: "empty-by-contract",
    parity_claim: "none",
    results,
    local_proofs: { sharp: localSharp, system_browser: localBrowser },
  };
}

if (require.main === module) {
  let runtimeRoot: string | undefined;
  let sharpPackageRoot: string | undefined;
  let browserPath: string | undefined;
  let target: string | undefined;
  let networkDisabled = false;
  let emptyUserCache = false;
  const argv = process.argv.slice(2);
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg.startsWith("--root=")) runtimeRoot = arg.slice("--root=".length);
    else if (arg.startsWith("--sharp="))
      sharpPackageRoot = arg.slice("--sharp=".length);
    else if (arg.startsWith("--browser=")) browserPath = arg.slice("--browser=".length);
    else if (arg === "--target") {
      target = argv[index + 1];
      if (target === undefined) throw new Error("--target requires a value");
      index += 1;
    } else if (arg.startsWith("--target=")) target = arg.slice("--target=".length);
    else if (arg === "--network=disabled") networkDisabled = true;
    else if (arg === "--network=enabled") networkDisabled = false;
    else if (arg === "--empty-user-cache") emptyUserCache = true;
    else if (arg !== "--json") throw new Error(`unknown argument ${arg}`);
  }
  const report = runSubsystemHarness({
    runtimeRoot,
    target,
    sharpPackageRoot,
    browserPath,
    networkDisabled,
    emptyUserCache,
  });
  console.log(JSON.stringify(report, null, 2));
  if (report.status !== "passed") process.exitCode = 1;
}
