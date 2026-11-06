import { readFile } from "node:fs/promises";
import { isAbsolute, relative, resolve, sep } from "node:path";

let clientPromise;

async function loadClient(options) {
  // The standalone MCP registration binds the installed launcher. Its runtime
  // metadata supplies the same fixed root without requiring a process global
  // or exposing additional environment variables to this untrusted adapter.
  const root = options?.globals?.nodeRepl?.runtime?.root;
  if (typeof root !== "string" || !isAbsolute(root)) {
    throw new Error("sky-cua Browser fallback requires the installed Node REPL runtime");
  }
  const release = JSON.parse(await readFile(resolve(root, "RELEASE.json"), "utf8"));
  const semanticPath = release?.paths?.browser_client;
  if (typeof semanticPath !== "string" || semanticPath === "" || isAbsolute(semanticPath)) {
    throw new Error("sky-cua RELEASE.json has no valid browser_client semantic path");
  }
  const clientPath = resolve(root, semanticPath);
  const fromRoot = relative(root, clientPath);
  if (fromRoot === ".." || fromRoot.startsWith(`..${sep}`)) {
    throw new Error("sky-cua browser_client semantic path escapes the installed root");
  }
  // RELEASE's semantic copy is outside the runtime's trusted Browser directory.
  // Package resolution selects the canonical installed module; identical bytes
  // copied elsewhere must retain their ordinary, untrusted VM capabilities.
  const client = await import("@heliasar/browser-use");
  if (typeof client.setupBrowserRuntime !== "function") {
    throw new Error("sky-cua Browser client does not export setupBrowserRuntime");
  }
  return client;
}

export async function setupBrowserRuntime(options) {
  clientPromise ??= loadClient(options);
  const client = await clientPromise;
  return client.setupBrowserRuntime(options);
}
