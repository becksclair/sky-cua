export interface TrustedModulePolicyCoreOptions {
  readonly trustAllCode: boolean;
  readonly bytesEqual: (
    left: Readonly<Uint8Array>,
    right: Readonly<Uint8Array>,
  ) => boolean;
  readonly readTrustedCodeFile: (path: string) => Readonly<Uint8Array> | null;
  readonly sha256Bytes: (bytes: Readonly<Uint8Array>) => string;
}

export interface TrustedModulePolicyCore {
  isTrustedDirectoryPath(path: string): boolean;
  evaluate(
    path: string,
    bytes: Readonly<Uint8Array>,
    inheritedTrust: boolean,
  ): { readonly sha256: string; readonly trusted: boolean };
}

type ParseTrustedCodePaths = (
  value: string | undefined,
  cwd: string,
  platform: NodeJS.Platform,
  isAbsolutePath: (path: string) => boolean,
  resolvePath: (cwd: string, path: string) => string,
) => readonly string[];

type TrustedPathContains = (
  file: string,
  root: string,
  platform: NodeJS.Platform,
  relativePath: (root: string, file: string) => string,
  isAbsolutePath: (path: string) => boolean,
) => boolean;

type TrustedBytesEqual = (
  left: Readonly<Uint8Array>,
  right: Readonly<Uint8Array>,
) => boolean;

type OpenedTrustedFileRealPath = (
  fd: number,
  originalPath: string,
  platform: NodeJS.Platform,
  realpathPath: (path: string) => string,
) => string | null;

type ResolveTrustedCodeRoots = (
  paths: readonly string[],
  realpathPath: (path: string) => string,
  isDirectoryPath: (path: string) => boolean,
) => readonly string[];

type ReadTrustedCodeFile = (
  file: string,
  roots: readonly string[],
  openFile: (path: string) => number,
  isFileDescriptor: (fd: number) => boolean,
  openedFileRealPath: (fd: number, path: string) => string | null,
  pathContains: (file: string, root: string) => boolean,
  readFileBytes: (fd: number) => Readonly<Uint8Array>,
  closeFile: (fd: number) => void,
) => Readonly<Uint8Array> | null;

type CreateTrustedModulePolicyCore = (
  options: TrustedModulePolicyCoreOptions,
) => TrustedModulePolicyCore;

/**
 * These named functions intentionally use contextually typed parameters so
 * their runtime source is valid JavaScript. The kernel renderer embeds these
 * exact policy decisions instead of maintaining a second implementation.
 */
export const parseTrustedCodePathsCore: ParseTrustedCodePaths =
  function parseTrustedCodePathsCore(
    value,
    cwd,
    platform,
    isAbsolutePath,
    resolvePath,
  ) {
    const delimiter = platform === "win32" ? ";" : ":";
    const paths = [];
    const seen = new Set();
    for (const token of (value ?? "").split(delimiter)) {
      const trimmed = token.trim();
      if (trimmed.length === 0 || !isAbsolutePath(trimmed)) continue;
      const normalized = resolvePath(cwd, trimmed);
      if (seen.has(normalized)) continue;
      seen.add(normalized);
      paths.push(normalized);
    }
    return paths;
  };

export const trustedPathContainsCore: TrustedPathContains =
  function trustedPathContainsCore(
    file,
    root,
    platform,
    relativePath,
    isAbsolutePath,
  ) {
    const child = relativePath(root, file);
    return (
      child === "" ||
      (child !== ".." &&
        !child.startsWith(`..${platform === "win32" ? "\\" : "/"}`) &&
        !isAbsolutePath(child))
    );
  };

export const trustedBytesEqualCore: TrustedBytesEqual =
  function trustedBytesEqualCore(left, right) {
    if (left.byteLength !== right.byteLength) return false;
    for (let index = 0; index < left.byteLength; index += 1) {
      if (left[index] !== right[index]) return false;
    }
    return true;
  };

export const openedTrustedFileRealPathCore: OpenedTrustedFileRealPath =
  function openedTrustedFileRealPathCore(
    fd,
    originalPath,
    platform,
    realpathPath,
  ) {
    const fdRoots =
      platform === "linux" ? ["/proc/self/fd"] : ["/dev/fd", "/proc/self/fd"];
    for (const fdRoot of fdRoots) {
      try {
        return realpathPath(`${fdRoot}/${fd}`);
      } catch {
        // Try the next descriptor namespace before failing closed.
      }
    }
    if (platform === "win32") {
      try {
        return realpathPath(originalPath);
      } catch {
        return null;
      }
    }
    return null;
  };

export const resolveTrustedCodeRootsCore: ResolveTrustedCodeRoots =
  function resolveTrustedCodeRootsCore(paths, realpathPath, isDirectoryPath) {
    const roots = [];
    for (const configuredPath of paths) {
      try {
        const realPath = realpathPath(configuredPath);
        if (isDirectoryPath(realPath)) roots.push(realPath);
      } catch {
        // A configured root that is unavailable is not a trust grant.
      }
    }
    return roots;
  };

export const readTrustedCodeFileCore: ReadTrustedCodeFile =
  function readTrustedCodeFileCore(
    file,
    roots,
    openFile,
    isFileDescriptor,
    openedFileRealPath,
    pathContains,
    readFileBytes,
    closeFile,
  ) {
    for (const root of roots) {
      let fd = null;
      try {
        fd = openFile(file);
        if (!isFileDescriptor(fd)) continue;
        const realPath = openedFileRealPath(fd, file);
        if (realPath === null || !pathContains(realPath, root)) continue;
        return readFileBytes(fd);
      } catch {
        // A path that cannot be opened and contained is not trusted.
      } finally {
        if (fd !== null) closeFile(fd);
      }
    }
    return null;
  };

export const createTrustedModulePolicyCore: CreateTrustedModulePolicyCore =
  function createTrustedModulePolicyCore(options) {
    const pendingDirectoryBytes = new Map();
    return {
      isTrustedDirectoryPath(path) {
        const bytes = options.readTrustedCodeFile(path);
        if (bytes === null) {
          pendingDirectoryBytes.delete(path);
          return false;
        }
        pendingDirectoryBytes.set(path, bytes);
        return true;
      },
      evaluate(path, bytes, inheritedTrust) {
        const sha256 = options.sha256Bytes(bytes);
        const pendingBytes = pendingDirectoryBytes.get(path);
        pendingDirectoryBytes.delete(path);
        const openedBytes =
          pendingBytes === undefined ? options.readTrustedCodeFile(path) : null;
        const directoryTrusted =
          pendingBytes === undefined
            ? openedBytes !== null && options.bytesEqual(openedBytes, bytes)
            : options.bytesEqual(pendingBytes, bytes);
        return {
          sha256,
          trusted: inheritedTrust || options.trustAllCode || directoryTrusted,
        };
      },
    };
  };

const EMBEDDED_CORE_FUNCTIONS = [
  {
    name: "parseTrustedCodePathsCore",
    implementation: parseTrustedCodePathsCore,
  },
  { name: "trustedPathContainsCore", implementation: trustedPathContainsCore },
  { name: "trustedBytesEqualCore", implementation: trustedBytesEqualCore },
  {
    name: "openedTrustedFileRealPathCore",
    implementation: openedTrustedFileRealPathCore,
  },
  {
    name: "resolveTrustedCodeRootsCore",
    implementation: resolveTrustedCodeRootsCore,
  },
  {
    name: "readTrustedCodeFileCore",
    implementation: readTrustedCodeFileCore,
  },
  {
    name: "createTrustedModulePolicyCore",
    implementation: createTrustedModulePolicyCore,
  },
] as const;

export function renderTrustedModulePolicyCoreSource(): string {
  return EMBEDDED_CORE_FUNCTIONS.map(
    ({ name, implementation }) =>
      `const ${name} = ${implementation.toString()};`,
  ).join("\n\n");
}
