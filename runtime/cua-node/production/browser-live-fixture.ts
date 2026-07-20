import { createServer, type Server } from "node:http";
import {
  cpSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join, relative, sep } from "node:path";

export type BrowserAcceptanceFixture = {
  origin: string;
  url: string;
  downloadUrl: string;
  close(): Promise<void>;
};

export type TrustNegativeCase = "tampered" | "missing" | "wrong-manifest-hash";

export type DisposableRuntime = {
  runtimeRoot: string;
  browserClient: string;
  nodeRepl: string;
  cleanup(): void;
};

const ACCEPTANCE_PATH = "/acceptance";
const DOWNLOAD_PATH = "/download/browser-live-acceptance.txt";
const DOWNLOAD_BYTES = "sky-cua browser live acceptance\n";

const PAGE = `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>sky-cua Browser acceptance</title>
  <style>
    body { background: #f5f7fa; color: #172033; font: 18px sans-serif; }
    main { margin: 3rem auto; max-width: 42rem; padding: 2rem; background: white; }
    #browser-live-readback { min-height: 2rem; padding: 1rem; background: #dce7f8; }
    body.accepted { background: #16325c; }
    body.accepted #browser-live-readback { background: #baf5c8; }
  </style>
</head>
<body>
  <main>
    <label for="browser-live-input">Acceptance text</label>
    <input id="browser-live-input" data-testid="browser-live-input" autocomplete="off">
    <button id="browser-live-submit" type="button">OK</button>
    <output id="browser-live-readback" data-testid="browser-live-readback"></output>
    <a data-testid="browser-live-download" download href="${DOWNLOAD_PATH}">Download fixture</a>
  </main>
  <script>
    document.querySelector("#browser-live-submit").addEventListener("click", () => {
      document.querySelector("#browser-live-readback").textContent =
        document.querySelector("#browser-live-input").value;
      document.body.classList.add("accepted");
    });
  </script>
</body>
</html>`;

function closeServer(server: Server): Promise<void> {
  return new Promise((resolvePromise, reject) => {
    server.close((error) => {
      if (error === undefined) resolvePromise();
      else reject(error);
    });
  });
}

export async function startBrowserAcceptanceFixture(): Promise<BrowserAcceptanceFixture> {
  const server = createServer((request, response) => {
    const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
    response.setHeader("Cache-Control", "no-store");
    if (path === "/health") {
      response.writeHead(200, { "Content-Type": "application/json" });
      response.end('{"status":"ok"}\n');
      return;
    }
    if (path === DOWNLOAD_PATH) {
      response.writeHead(200, {
        "Content-Type": "text/plain; charset=utf-8",
        "Content-Disposition":
          'attachment; filename="browser-live-acceptance.txt"',
      });
      response.end(DOWNLOAD_BYTES);
      return;
    }
    if (path === "/" || path === ACCEPTANCE_PATH) {
      response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
      response.end(PAGE);
      return;
    }
    response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
    response.end("not found\n");
  });
  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      server.removeListener("error", reject);
      resolvePromise();
    });
  });
  const address = server.address();
  if (address === null || typeof address === "string") {
    await closeServer(server);
    throw new Error("loopback Browser acceptance fixture has no TCP address");
  }
  const origin = `http://127.0.0.1:${address.port}`;
  let closed = false;
  return {
    origin,
    url: `${origin}${ACCEPTANCE_PATH}`,
    downloadUrl: `${origin}${DOWNLOAD_PATH}`,
    close: async (): Promise<void> => {
      if (closed) return;
      closed = true;
      await closeServer(server);
    },
  };
}

function relativeInside(root: string, path: string, label: string): string {
  const item = relative(root, path);
  if (
    item.length === 0 ||
    item === ".." ||
    item.startsWith(`..${sep}`) ||
    isAbsolute(item)
  )
    throw new Error(`${label} must be inside packaged runtime root`);
  return item;
}

export function createDisposableRuntime(
  runtimeRootPath: string,
  browserClientPath: string,
  nodeReplPath: string,
  testCase: TrustNegativeCase,
): DisposableRuntime {
  const runtimeRoot = realpathSync(runtimeRootPath);
  const browserClient = realpathSync(browserClientPath);
  const nodeRepl = realpathSync(nodeReplPath);
  const browserRelative = relativeInside(runtimeRoot, browserClient, "browser client");
  const nodeReplRelative = relativeInside(runtimeRoot, nodeRepl, "node_repl");
  const temporaryRoot = mkdtempSync(join(tmpdir(), "sky-cua-browser-trust-negative-"));
  const copyRoot = join(temporaryRoot, "cua_node");
  try {
    cpSync(runtimeRoot, copyRoot, {
      recursive: true,
      dereference: false,
      preserveTimestamps: true,
    });
    const copiedBrowserClient = join(copyRoot, browserRelative);
    const copiedNodeRepl = join(copyRoot, nodeReplRelative);
    if (testCase === "tampered") {
      writeFileSync(
        copiedBrowserClient,
        Buffer.concat([readFileSync(copiedBrowserClient), Buffer.from("\n// tampered\n")]),
      );
    } else if (testCase === "missing") {
      rmSync(copiedBrowserClient);
    } else {
      const manifestPath = join(copyRoot, "manifest.json");
      const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as Record<
        string,
        unknown
      >;
      manifest.trusted_browser_client_sha256s = ["0".repeat(64)];
      writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
    }
    return {
      runtimeRoot: copyRoot,
      browserClient: copiedBrowserClient,
      nodeRepl: copiedNodeRepl,
      cleanup: (): void => rmSync(temporaryRoot, { recursive: true, force: true }),
    };
  } catch (error) {
    rmSync(temporaryRoot, { recursive: true, force: true });
    throw error;
  }
}
