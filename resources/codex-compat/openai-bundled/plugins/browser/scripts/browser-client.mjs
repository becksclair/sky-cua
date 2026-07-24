import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, relative, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";

let clientPromise;

function installedRoot() {
  const dataHome = process.env.XDG_DATA_HOME?.trim();
  return resolve(dataHome || resolve(homedir(), ".local/share"), "sky-cua");
}

async function loadClient() {
  const root = installedRoot();
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
  const client = await import(pathToFileURL(clientPath).href);
  if (typeof client.setupBrowserRuntime !== "function") {
    throw new Error("sky-cua Browser client does not export setupBrowserRuntime");
  }
  return client;
}

export async function setupBrowserRuntime(options) {
  clientPromise ??= loadClient();
  const client = await clientPromise;
  return client.setupBrowserRuntime(options);
}
