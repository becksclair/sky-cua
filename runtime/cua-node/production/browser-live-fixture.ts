import { createServer, type Server } from "node:http";

export type BrowserAcceptanceFixture = {
  origin: string;
  url: string;
  downloadUrl: string;
  close(): Promise<void>;
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
