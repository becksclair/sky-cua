import { Buffer } from "node:buffer";
import { createHash } from "node:crypto";
import {
  closeSync,
  constants,
  fstatSync,
  openSync,
  readFileSync,
  realpathSync,
  statSync,
} from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";
import {
  createTrustedModulePolicyCore,
  openedTrustedFileRealPathCore,
  parseTrustedCodePathsCore,
  readTrustedCodeFileCore,
  renderTrustedModulePolicyCoreSource,
  resolveTrustedCodeRootsCore,
  trustedBytesEqualCore,
  trustedPathContainsCore,
  type TrustedModulePolicyCore,
} from "./trusted-module-policy-core.ts";

export const TRUSTED_CODE_PATHS_ENV = "NODE_REPL_TRUSTED_CODE_PATHS";
export const TRUST_ALL_CODE_ENV = "NODE_REPL_TRUST_ALL_CODE";

export interface TrustedModulePolicyOptions {
  env?: NodeJS.ProcessEnv;
  cwd?: string;
  platform?: NodeJS.Platform;
}

export interface TrustedModuleLoad {
  readonly path: string;
  readonly bytes: Readonly<Uint8Array>;
  readonly sha256: string;
  readonly trusted: boolean;
}

const EMPTY_ENV: NodeJS.ProcessEnv = {};
const NO_FOLLOW = (constants as { O_NOFOLLOW?: number }).O_NOFOLLOW ?? 0;

function sha256Bytes(bytes: Uint8Array): string {
  return createHash("sha256").update(bytes).digest("hex");
}

export function parseTrustedCodePaths(
  value: string | undefined,
  cwd = process.cwd(),
  platform: NodeJS.Platform = process.platform,
): readonly string[] {
  return parseTrustedCodePathsCore(value, cwd, platform, isAbsolute, resolve);
}

function isWithinPath(
  file: string,
  root: string,
  platform = process.platform,
): boolean {
  return trustedPathContainsCore(file, root, platform, relative, isAbsolute);
}

function openedFileRealPath(fd: number, originalPath: string): string | null {
  return openedTrustedFileRealPathCore(
    fd,
    originalPath,
    process.platform,
    realpathSync,
  );
}

function openExactFile(path: string): Buffer {
  const fd = openSync(path, constants.O_RDONLY | NO_FOLLOW);
  try {
    if (!fstatSync(fd).isFile())
      throw new Error("trusted module path is not a file");
    return Buffer.from(readFileSync(fd));
  } finally {
    closeSync(fd);
  }
}

function resolveTrustedCodeRoots(paths: readonly string[]): readonly string[] {
  return resolveTrustedCodeRootsCore(paths, realpathSync, (path) =>
    statSync(path).isDirectory(),
  );
}

/**
 * The policy is deliberately independent of module resolution. Directory
 * grants snapshot bytes from an opened descriptor, and the loader-provided
 * bytes must match that snapshot before compilation receives trust.
 */
export class TrustedModulePolicy {
  public readonly trustedCodePaths: readonly string[];
  public readonly trustAllCode: boolean;
  private readonly trustedCodeRoots: readonly string[];
  private readonly core: TrustedModulePolicyCore;
  public constructor(options: TrustedModulePolicyOptions = {}) {
    const env = options.env ?? EMPTY_ENV;
    const cwd = options.cwd ?? process.cwd();
    this.trustedCodePaths = parseTrustedCodePaths(
      env[TRUSTED_CODE_PATHS_ENV],
      cwd,
      options.platform ?? process.platform,
    );
    this.trustedCodeRoots = resolveTrustedCodeRoots(this.trustedCodePaths);
    this.trustAllCode = env[TRUST_ALL_CODE_ENV] === "1";
    this.core = createTrustedModulePolicyCore({
      trustAllCode: this.trustAllCode,
      bytesEqual: trustedBytesEqualCore,
      readTrustedCodeFile: (path) => this.readTrustedCodeFile(path),
      sha256Bytes,
    });
  }

  public isTrustedDirectoryPath(path: string): boolean {
    return this.core.isTrustedDirectoryPath(path);
  }

  public evaluate(
    path: string,
    bytes: Readonly<Uint8Array>,
    _packageEntrypoint: boolean,
    inheritedTrust: boolean,
  ): TrustedModuleLoad {
    const decision = this.core.evaluate(path, bytes, inheritedTrust);
    return { path, bytes, ...decision };
  }

  public readEntrypoint(
    path: string,
    inheritedTrust = false,
  ): TrustedModuleLoad {
    const bytes = openExactFile(path);
    return this.evaluate(path, bytes, true, inheritedTrust);
  }

  private readTrustedCodeFile(path: string): Readonly<Uint8Array> | null {
    return readTrustedCodeFileCore(
      path,
      this.trustedCodeRoots,
      (file) => openSync(file, constants.O_RDONLY | NO_FOLLOW),
      (fd) => fstatSync(fd).isFile(),
      openedFileRealPath,
      isWithinPath,
      (fd) => Buffer.from(readFileSync(fd)),
      closeSync,
    );
  }
}

/**
 * This source is embedded in the isolated Node kernel. Keep its decisions
 * byte-oriented and aligned with TrustedModulePolicy above; the host-side
 * class remains the directly testable implementation seam.
 */
export const TRUSTED_MODULE_POLICY_KERNEL_SOURCE = String.raw`
${renderTrustedModulePolicyCoreSource()}

function trustedPathContains(file, root) {
  return trustedPathContainsCore(file, root, process.platform, relative, isAbsolute);
}

const {
  closeSync: closeTrustedFile,
  constants: trustedFsConstants,
  fstatSync: fstatTrustedFile,
  openSync: openTrustedFile,
  readFileSync: readTrustedFileBytes,
  realpathSync: realpathTrustedFile,
  statSync: statTrustedPath,
} = await import('node:fs');
const trustedNoFollow = trustedFsConstants.O_NOFOLLOW || 0;

function openedTrustedFileRealPath(fd, originalPath) {
  return openedTrustedFileRealPathCore(fd, originalPath, process.platform, realpathTrustedFile);
}

function resolveTrustedRoots(paths) {
  return resolveTrustedCodeRootsCore(paths, realpathTrustedFile, (path) => statTrustedPath(path).isDirectory());
}

function readTrustedCodeFile(file, roots) {
  return readTrustedCodeFileCore(
    file,
    roots,
    (path) => openTrustedFile(path, trustedFsConstants.O_RDONLY | trustedNoFollow),
    (fd) => fstatTrustedFile(fd).isFile(),
    openedTrustedFileRealPath,
    trustedPathContains,
    (fd) => Buffer.from(readTrustedFileBytes(fd)),
    closeTrustedFile,
  );
}

function createTrustedModulePolicy() {
  const trustedCodePaths = parseTrustedCodePathsCore(process.env.NODE_REPL_TRUSTED_CODE_PATHS, cwd, process.platform, isAbsolute, resolve);
  const trustedRoots = resolveTrustedRoots(trustedCodePaths);
  const core = createTrustedModulePolicyCore({
    trustAllCode: process.env.NODE_REPL_TRUST_ALL_CODE === '1',
    bytesEqual: trustedBytesEqualCore,
    readTrustedCodeFile(file) { return readTrustedCodeFile(file, trustedRoots); },
    sha256Bytes() { return ''; },
  });
  return {
    isTrustedDirectoryPath: core.isTrustedDirectoryPath,
    evaluate(file, bytes, packageEntrypoint, inheritedTrust) {
      return core.evaluate(file, bytes, inheritedTrust);
    },
  };
}
`;
