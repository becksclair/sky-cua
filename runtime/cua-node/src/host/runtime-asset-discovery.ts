import { accessSync, constants } from "node:fs";
import { access, readdir, realpath, stat } from "node:fs/promises";
import {
  basename,
  delimiter,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from "node:path";
import { pathToFileURL } from "node:url";

export type BrowserExecutableKind = "brave-origin" | "brave" | "chrome" | "chromium";

export interface BrowserExecutableResolution {
  readonly executablePath: string;
  readonly executableKind: BrowserExecutableKind;
}

/**
 * The manifest fields consumed by runtime discovery after the caller has
 * validated the complete record against contracts/runtime-manifest.schema.json.
 * This intentionally does not parse or reproduce the canonical schema.
 */
export interface ValidatedRuntimeManifestRecord {
  readonly manifest_version: number;
  readonly node_version: string;
  readonly node_path: string;
  readonly node_modules: string;
  readonly data: Readonly<{
    playwright: string;
    tessdata: string;
    pdfjs: string;
    licenses: string;
    sbom: string;
  }>;
}

export interface RuntimeAssetDiscoveryInput {
  readonly runtimeRoot: string;
  readonly manifest: ValidatedRuntimeManifestRecord;
  readonly resolveBrowserExecutable?:
    | (() => BrowserExecutableResolution | null)
    | undefined;
}

export interface RuntimeAssetMetadata {
  readonly version: number;
  readonly root: string;
  readonly node: Readonly<{
    version: string;
    execPath: string;
  }>;
  readonly modules: Readonly<{
    root: string;
  }>;
  readonly browser: Readonly<{
    playwrightRoot: string;
    executablePath: string | null;
    executableKind: BrowserExecutableKind | null;
  }>;
  readonly pdfjs: Readonly<{
    root: string;
    cMapUrl: string;
    standardFontDataUrl: string;
    wasmUrl: string | null;
    workerSrc: string;
  }>;
  readonly tesseract: Readonly<{
    tessdataRoot: string;
    languages: readonly string[];
  }>;
  readonly licenses: Readonly<{
    root: string;
  }>;
  readonly sbomPath: string;
}

export type RuntimeAssetDiscoveryErrorCode =
  | "INVALID_RUNTIME_ROOT"
  | "PATH_TRAVERSAL"
  | "MISSING_PATH"
  | "WRONG_PATH_TYPE"
  | "NOT_EXECUTABLE"
  | "INVALID_BROWSER_RESOLUTION";

export class RuntimeAssetDiscoveryError extends Error {
  public readonly code: RuntimeAssetDiscoveryErrorCode;
  public readonly asset: string;

  public constructor(
    code: RuntimeAssetDiscoveryErrorCode,
    asset: string,
    detail: string,
  ) {
    super(`runtime asset ${asset}: ${detail}`);
    this.name = "RuntimeAssetDiscoveryError";
    this.code = code;
    this.asset = asset;
  }
}

const BROWSER_KINDS = new Set<BrowserExecutableKind>([
  "brave-origin",
  "brave",
  "chrome",
  "chromium",
]);

const BROWSER_EXECUTABLE_CANDIDATES: ReadonlyArray<
  readonly [path: string, kind: BrowserExecutableKind]
> = [
  ["/opt/brave-origin-bin/brave", "brave-origin"],
  ["/opt/google/chrome/chrome", "chrome"],
];

function executableKind(path: string): BrowserExecutableKind {
  const normalizedPath = path.toLowerCase();
  const name = basename(normalizedPath);
  if (normalizedPath.includes("brave-origin")) return "brave-origin";
  if (name.includes("brave")) return "brave";
  if (name.includes("chrome")) return "chrome";
  return "chromium";
}

function isExecutable(path: string): boolean {
  try {
    accessSync(path, constants.R_OK | constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

export function resolveDefaultBrowserExecutable(
  env: NodeJS.ProcessEnv = process.env,
  cwd = process.cwd(),
): BrowserExecutableResolution | null {
  const configured = env.CUA_NODE_CHROMIUM_EXECUTABLE;
  if (configured !== undefined && isAbsolute(configured) && isExecutable(configured)) {
    return {
      executablePath: configured,
      executableKind: executableKind(configured),
    };
  }
  for (const [path, kind] of BROWSER_EXECUTABLE_CANDIDATES) {
    if (isExecutable(path)) return { executablePath: path, executableKind: kind };
  }
  return resolveBrowserExecutableFromPath(env, cwd);
}

export function resolveBrowserExecutableFromPath(
  env: NodeJS.ProcessEnv = process.env,
  cwd = process.cwd(),
): BrowserExecutableResolution | null {
  const names: ReadonlyArray<readonly [string, BrowserExecutableKind]> = [
    ["brave-origin", "brave-origin"],
    ["brave-browser", "brave"],
    ["brave", "brave"],
    ["google-chrome-stable", "chrome"],
    ["google-chrome", "chrome"],
    ["chromium", "chromium"],
    ["chromium-browser", "chromium"],
  ];
  for (const directory of (env.PATH ?? "").split(delimiter)) {
    if (directory.length === 0) continue;
    const absoluteDirectory = isAbsolute(directory)
      ? directory
      : resolve(cwd, directory);
    for (const [name, kind] of names) {
      const path = join(absoluteDirectory, name);
      if (isExecutable(path)) return { executablePath: path, executableKind: kind };
    }
  }
  return null;
}

function isWithinRoot(root: string, candidate: string): boolean {
  const pathFromRoot = relative(root, candidate);
  return (
    pathFromRoot === "" ||
    (pathFromRoot !== ".." && !pathFromRoot.startsWith(`..${sep}`))
  );
}

async function canonicalRuntimeRoot(path: string): Promise<string> {
  if (!isAbsolute(path)) {
    throw new RuntimeAssetDiscoveryError(
      "INVALID_RUNTIME_ROOT",
      "runtime root",
      `path must be absolute: ${path}`,
    );
  }
  const normalized = resolve(path);
  try {
    const root = await realpath(normalized);
    if (!(await stat(root)).isDirectory()) {
      throw new RuntimeAssetDiscoveryError(
        "WRONG_PATH_TYPE",
        "runtime root",
        `expected directory: ${normalized}`,
      );
    }
    return root;
  } catch (error) {
    if (error instanceof RuntimeAssetDiscoveryError) throw error;
    throw new RuntimeAssetDiscoveryError(
      "INVALID_RUNTIME_ROOT",
      "runtime root",
      `directory is unavailable: ${normalized}`,
    );
  }
}

function resolveManifestPath(
  root: string,
  manifestPath: string,
  asset: string,
): string {
  if (manifestPath.length === 0 || isAbsolute(manifestPath)) {
    throw new RuntimeAssetDiscoveryError(
      "PATH_TRAVERSAL",
      asset,
      `manifest path must be a non-empty relative path: ${manifestPath}`,
    );
  }
  const candidate = resolve(root, manifestPath);
  if (!isWithinRoot(root, candidate)) {
    throw new RuntimeAssetDiscoveryError(
      "PATH_TRAVERSAL",
      asset,
      `manifest path escapes runtime root: ${manifestPath}`,
    );
  }
  return candidate;
}

async function requiredPath(
  root: string,
  manifestPath: string,
  asset: string,
  expectedType: "directory" | "file",
): Promise<string> {
  const candidate = resolveManifestPath(root, manifestPath, asset);
  let resolved: string;
  try {
    resolved = await realpath(candidate);
  } catch {
    throw new RuntimeAssetDiscoveryError(
      "MISSING_PATH",
      asset,
      `required ${expectedType} is missing: ${candidate}`,
    );
  }
  if (!isWithinRoot(root, resolved)) {
    throw new RuntimeAssetDiscoveryError(
      "PATH_TRAVERSAL",
      asset,
      `resolved path escapes runtime root: ${candidate} -> ${resolved}`,
    );
  }
  const details = await stat(resolved);
  const valid = expectedType === "directory" ? details.isDirectory() : details.isFile();
  if (!valid) {
    throw new RuntimeAssetDiscoveryError(
      "WRONG_PATH_TYPE",
      asset,
      `expected ${expectedType}: ${candidate}`,
    );
  }
  return resolved;
}

async function requiredExecutable(
  root: string,
  manifestPath: string,
  asset: string,
): Promise<string> {
  const path = await requiredPath(root, manifestPath, asset, "file");
  try {
    await access(path, constants.R_OK | constants.X_OK);
  } catch {
    throw new RuntimeAssetDiscoveryError(
      "NOT_EXECUTABLE",
      asset,
      `file is not readable and executable: ${path}`,
    );
  }
  return path;
}

async function optionalDirectory(
  root: string,
  path: string,
  asset: string,
): Promise<string | null> {
  try {
    return await requiredPath(root, path, asset, "directory");
  } catch (error) {
    if (error instanceof RuntimeAssetDiscoveryError && error.code === "MISSING_PATH") {
      return null;
    }
    throw error;
  }
}

async function tessdataLanguages(tessdataRoot: string): Promise<string[]> {
  const entries = await readdir(tessdataRoot, { withFileTypes: true });
  const suffix = ".traineddata";
  const languages = entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(suffix))
    .map((entry) => entry.name.slice(0, -suffix.length))
    .filter((language) => language.length > 0)
    .sort((left, right) => left.localeCompare(right));
  if (languages.length === 0) {
    throw new RuntimeAssetDiscoveryError(
      "MISSING_PATH",
      "tessdata languages",
      `required *.traineddata files are missing: ${tessdataRoot}`,
    );
  }
  return languages;
}

async function browserMetadata(
  resolveBrowserExecutable: (() => BrowserExecutableResolution | null) | undefined,
): Promise<Pick<RuntimeAssetMetadata["browser"], "executablePath" | "executableKind">> {
  const resolution = resolveBrowserExecutable?.() ?? null;
  if (resolution === null) {
    return { executablePath: null, executableKind: null };
  }
  if (
    !isAbsolute(resolution.executablePath) ||
    !BROWSER_KINDS.has(resolution.executableKind)
  ) {
    throw new RuntimeAssetDiscoveryError(
      "INVALID_BROWSER_RESOLUTION",
      "browser executable",
      "resolver must return an absolute path and a supported executable kind",
    );
  }
  let executablePath: string;
  try {
    executablePath = await realpath(resolve(resolution.executablePath));
    const details = await stat(executablePath);
    if (!details.isFile()) throw new Error("not a file");
    await access(executablePath, constants.R_OK | constants.X_OK);
  } catch {
    throw new RuntimeAssetDiscoveryError(
      "INVALID_BROWSER_RESOLUTION",
      "browser executable",
      `resolved file is not readable and executable: ${resolution.executablePath}`,
    );
  }
  return { executablePath, executableKind: resolution.executableKind };
}

function deepFreeze<T>(value: T): Readonly<T> {
  if (value !== null && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) deepFreeze(child);
    Object.freeze(value);
  }
  return value;
}

export async function discoverRuntimeAssets(
  input: RuntimeAssetDiscoveryInput,
): Promise<RuntimeAssetMetadata> {
  const root = await canonicalRuntimeRoot(input.runtimeRoot);
  const { manifest } = input;
  const nodePath = await requiredExecutable(
    root,
    manifest.node_path,
    "Node executable",
  );
  const moduleRoot = await requiredPath(
    root,
    manifest.node_modules,
    "Node module root",
    "directory",
  );
  const playwrightRoot = await requiredPath(
    root,
    manifest.data.playwright,
    "Playwright root",
    "directory",
  );
  const tessdataRoot = await requiredPath(
    root,
    manifest.data.tessdata,
    "tessdata root",
    "directory",
  );
  const pdfjsRoot = await requiredPath(
    root,
    manifest.data.pdfjs,
    "PDF.js root",
    "directory",
  );
  const licensesRoot = await requiredPath(
    root,
    manifest.data.licenses,
    "licenses root",
    "directory",
  );
  const sbomPath = await requiredPath(root, manifest.data.sbom, "SBOM", "file");
  const cMapUrl = await requiredPath(pdfjsRoot, "cmaps", "PDF.js CMaps", "directory");
  const standardFontDataUrl = await requiredPath(
    pdfjsRoot,
    "standard_fonts",
    "PDF.js standard fonts",
    "directory",
  );
  const wasmUrl = await optionalDirectory(pdfjsRoot, "wasm", "PDF.js WASM");
  const pdfWorkerPath = await requiredPath(
    moduleRoot,
    "pdfjs-dist/legacy/build/pdf.worker.mjs",
    "PDF.js worker module",
    "file",
  );
  const languages = await tessdataLanguages(tessdataRoot);
  const browser = await browserMetadata(input.resolveBrowserExecutable);

  const directoryUrl = (path: string | null): string | null =>
    path === null ? null : path.endsWith(sep) ? path : `${path}${sep}`;
  const metadata: RuntimeAssetMetadata = {
    version: manifest.manifest_version,
    root,
    node: { version: manifest.node_version, execPath: nodePath },
    modules: { root: moduleRoot },
    browser: { playwrightRoot, ...browser },
    pdfjs: {
      root: pdfjsRoot,
      cMapUrl: directoryUrl(cMapUrl) as string,
      standardFontDataUrl: directoryUrl(standardFontDataUrl) as string,
      wasmUrl: directoryUrl(wasmUrl),
      workerSrc: pathToFileURL(pdfWorkerPath).href,
    },
    tesseract: { tessdataRoot, languages },
    licenses: { root: licensesRoot },
    sbomPath,
  };
  deepFreeze(metadata);
  return metadata;
}
