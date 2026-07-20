import {
  closeSync,
  lstatSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  statSync,
} from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { arrayValue, record, sha256, stringValue } from "./json";
import { inspectCuaNodeLock } from "./lock-inspection";
import { verifyManifest } from "./manifest-verification";
import { checkNativeFile, readExecutableIdentity } from "./native-audit";
import type { NativeCommandObserver } from "./native-audit";
import { findWrongPlatformOptionalDependencies } from "./package-metadata-audit";
import type {
  JsonRecord,
  VerificationCheck,
  VerificationReport,
  VerifyCuaNodeOptions,
} from "./types";

const REQUIRED_DIRECTORIES = [
  "bin",
  "lib",
  "lib/node_modules",
  "share",
  "share/playwright",
  "share/tessdata",
  "share/pdfjs",
  "licenses",
];

const REQUIRED_FILES = [
  "bin/node",
  "bin/node_repl",
  "lib/node_modules/@heliasar/browser-use/package.json",
  "lib/node_modules/@heliasar/sky-cua/package.json",
  "sbom.cdx.json",
];

const ELF_MAGIC = Buffer.from([0x7f, 0x45, 0x4c, 0x46]);

function hasElfMagic(path: string): boolean {
  const descriptor = openSync(path, "r");
  try {
    const magic = Buffer.allocUnsafe(ELF_MAGIC.length);
    return (
      readSync(descriptor, magic, 0, magic.length, 0) === magic.length &&
      magic.equals(ELF_MAGIC)
    );
  } finally {
    closeSync(descriptor);
  }
}

function relativeFiles(root: string): string[] {
  const files: string[] = [];
  const visit = (current: string): void => {
    const entries = readdirSync(current, { withFileTypes: true }).sort((a, b) =>
      a.name.localeCompare(b.name),
    );
    for (const entry of entries) {
      const absolute = join(current, entry.name);
      const name = relative(root, absolute).split(sep).join("/");
      if (entry.isSymbolicLink())
        throw new Error(`symlink is not allowed: ${name}`);
      if (entry.isDirectory()) visit(absolute);
      else if (entry.isFile()) files.push(name);
      else throw new Error(`unsupported filesystem entry: ${name}`);
    }
  };
  visit(root);
  return files;
}

function verifyRuntimeFiles(
  root: string,
  manifest: JsonRecord,
  checks: VerificationCheck[],
  allowFixtureValues: boolean,
  observeNativeCommand?: NativeCommandObserver,
): void {
  const topLevel = readdirSync(root, { withFileTypes: true })
    .map((entry) => entry.name)
    .sort((a, b) => a.localeCompare(b));
  const allowedTopLevel = new Set([
    "bin",
    "lib",
    "share",
    "licenses",
    "manifest.json",
    "sbom.cdx.json",
  ]);
  const unexpectedTopLevel = topLevel.filter(
    (entry) => !allowedTopLevel.has(entry),
  );
  const allowedFixtureEntries = new Set(["README.md", "fake-runtime.test.ts"]);
  const layoutUnexpected = allowFixtureValues
    ? unexpectedTopLevel.filter((entry) => !allowedFixtureEntries.has(entry))
    : unexpectedTopLevel;
  checks.push({
    id: "layout:top-level",
    status: layoutUnexpected.length === 0 ? "passed" : "failed",
    detail:
      layoutUnexpected.length === 0
        ? "exact top-level layout"
        : `unexpected entries: ${layoutUnexpected.join(", ")}`,
  });

  for (const directory of REQUIRED_DIRECTORIES) {
    const exists = lstatSync(join(root, directory)).isDirectory();
    checks.push({
      id: `layout:directory:${directory}`,
      status: exists ? "passed" : "failed",
      detail: exists ? "directory present" : "directory missing",
    });
  }
  for (const file of REQUIRED_FILES) {
    const exists = lstatSync(join(root, file)).isFile();
    checks.push({
      id: `layout:file:${file}`,
      status: exists ? "passed" : "failed",
      detail: exists ? "file present" : "file missing",
    });
  }

  const files = relativeFiles(root).filter((file) => file !== "manifest.json");
  const checksumContainer = record(manifest.checksums, "manifest.checksums");
  const checksumEntries = arrayValue(
    checksumContainer.files,
    "manifest.checksums.files",
  ).map((value, index) => record(value, `manifest.checksums.files[${index}]`));
  const declared = checksumEntries
    .map((entry) => stringValue(entry.path, "checksum path"))
    .sort((a, b) => a.localeCompare(b));
  const actual = [...files].sort((a, b) => a.localeCompare(b));
  const checksumActual = allowFixtureValues
    ? actual.filter((entry) => declared.includes(entry))
    : actual;
  checks.push({
    id: "checksums:file-set",
    status:
      JSON.stringify(declared) === JSON.stringify(checksumActual)
        ? "passed"
        : "failed",
    detail: `declared=${declared.length}; actual=${actual.length}`,
  });
  for (const entry of checksumEntries) {
    const file = stringValue(entry.path, "checksum path");
    const expectedHash = stringValue(entry.sha256, `checksum ${file}`);
    const expectedSize = entry.size_bytes;
    if (file === "manifest.json") {
      checks.push({
        id: `checksums:path:${file}`,
        status: "failed",
        detail: "checksum entries cannot include manifest.json",
      });
      continue;
    }
    const path = join(root, file);
    const withinRoot = resolve(path).startsWith(`${root}${sep}`);
    if (!withinRoot || !statSync(path).isFile()) {
      checks.push({
        id: `checksums:file:${file}`,
        status: "failed",
        detail: "checksum target is missing or escapes runtime root",
      });
      continue;
    }
    const valid =
      expectedHash === sha256(path) && expectedSize === statSync(path).size;
    checks.push({
      id: `checksums:file:${file}`,
      status: valid ? "passed" : "failed",
      detail: valid ? "SHA-256 and size match" : "SHA-256 or size mismatch",
    });
  }

  const allEntries = [...files, ...REQUIRED_DIRECTORIES];
  const modeFailures: string[] = [];
  for (const entry of allEntries) {
    const path = join(root, entry);
    const isDirectory = statSync(path).isDirectory();
    const mode = statSync(path).mode & 0o777;
    const expected = isDirectory
      ? 0o755
      : entry.startsWith("bin/") || entry.endsWith(".node")
        ? 0o755
        : 0o644;
    if (mode !== expected)
      modeFailures.push(
        `${entry}=0${mode.toString(8)} expected 0${expected.toString(8)}`,
      );
  }
  checks.push({
    id: "permissions:layout",
    status: modeFailures.length === 0 ? "passed" : "failed",
    detail:
      modeFailures.length === 0
        ? "directories 0755 and files 0644/0755"
        : modeFailures.join("; "),
  });

  const forbiddenNames = files.filter((file) =>
    /(?:^|\/)(?:fsevents|.*musl.*)(?:\/|$)/iu.test(file),
  );
  checks.push({
    id: "dependencies:no-forbidden-platform-packages",
    status: forbiddenNames.length === 0 ? "passed" : "failed",
    detail:
      forbiddenNames.length === 0
        ? "no fsevents or musl package paths"
        : forbiddenNames.join(", "),
  });
  const wrongPlatformOptionalDependencies: string[] = [];
  for (const file of files.filter((entry) => entry.endsWith("package.json"))) {
    const packageJson = record(
      JSON.parse(readFileSync(join(root, file), "utf8")) as unknown,
      file,
    );
    wrongPlatformOptionalDependencies.push(
      ...findWrongPlatformOptionalDependencies(packageJson, file).map(
        ({ dependency, platform }) => `${file}: ${dependency} (${platform})`,
      ),
    );
    const serialized = JSON.stringify(packageJson);
    if (/fsevents/iu.test(serialized))
      checks.push({
        id: `dependencies:fsevents:${file}`,
        status: "failed",
        detail: "fsevents is forbidden in the Linux production graph",
      });
    if (packageJson.scripts !== undefined) {
      const scripts = record(packageJson.scripts, `${file}.scripts`);
      const lifecycle = ["preinstall", "install", "postinstall"].filter(
        (name) => typeof scripts[name] === "string",
      );
      if (lifecycle.length > 0)
        checks.push({
          id: `dependencies:lifecycle:${file}`,
          status: "failed",
          detail: `install lifecycle scripts are forbidden: ${lifecycle.join(", ")}`,
        });
    }
  }
  checks.push({
    id: "dependencies:no-wrong-platform-optional-dependencies",
    status:
      wrongPlatformOptionalDependencies.length === 0 ? "passed" : "failed",
    detail:
      wrongPlatformOptionalDependencies.length === 0
        ? "optional dependency names contain no Darwin, Windows, ARM, or musl targets"
        : wrongPlatformOptionalDependencies.join(", "),
  });
  const packagePath = join(
    root,
    "lib/node_modules/@heliasar/sky-cua/package.json",
  );
  const skyPackage = record(
    JSON.parse(readFileSync(packagePath, "utf8")) as unknown,
    "@heliasar/sky-cua/package.json",
  );
  const validSkyPackage =
    skyPackage.name === "@heliasar/sky-cua" && skyPackage.version === "0.1.0";
  checks.push({
    id: "package:sky-cua-identity",
    status: validSkyPackage ? "passed" : "failed",
    detail: `name=${String(skyPackage.name)}; version=${String(skyPackage.version)}`,
  });
  const components = record(manifest.components, "manifest.components");
  const browserUseComponent = record(
    components.browser_use,
    "manifest.components.browser_use",
  );
  const browserUsePackagePath = join(
    root,
    "lib/node_modules/@heliasar/browser-use/package.json",
  );
  const browserUsePackage = record(
    JSON.parse(readFileSync(browserUsePackagePath, "utf8")) as unknown,
    "@heliasar/browser-use/package.json",
  );
  const validBrowserUsePackage =
    browserUsePackage.name === browserUseComponent.package_name &&
    browserUsePackage.version === browserUseComponent.package_version;
  checks.push({
    id: "package:browser-use-identity",
    status: validBrowserUsePackage ? "passed" : "failed",
    detail: `name=${String(browserUsePackage.name)}; version=${String(browserUsePackage.version)}`,
  });
  const browserUseEntrypoint = stringValue(
    browserUseComponent.entrypoint,
    "manifest.components.browser_use.entrypoint",
  );
  const browserUseEntrypointPath = join(root, browserUseEntrypoint);
  const browserUseEntrypointWithinRoot = resolve(
    browserUseEntrypointPath,
  ).startsWith(`${root}${sep}`);
  const browserUseEntrypointMatches =
    browserUseEntrypointWithinRoot &&
    statSync(browserUseEntrypointPath).isFile() &&
    sha256(browserUseEntrypointPath) === browserUseComponent.entrypoint_sha256;
  checks.push({
    id: "package:browser-use-entrypoint",
    status: browserUseEntrypointMatches ? "passed" : "failed",
    detail: browserUseEntrypointMatches
      ? "canonical Browser entrypoint bytes match the component and trust hash"
      : "canonical Browser entrypoint bytes do not match the component hash",
  });

  const sbom = record(
    JSON.parse(readFileSync(join(root, "sbom.cdx.json"), "utf8")) as unknown,
    "sbom.cdx.json",
  );
  checks.push({
    id: "sbom:cyclonedx",
    status:
      sbom.bomFormat === "CycloneDX" &&
      typeof sbom.specVersion === "string" &&
      Array.isArray(sbom.components)
        ? "passed"
        : "failed",
    detail: "CycloneDX SBOM shape",
  });
  const licenseFiles = relativeFiles(join(root, "licenses"));
  checks.push({
    id: "licenses:notices",
    status: licenseFiles.length > 0 ? "passed" : "failed",
    detail: `${licenseFiles.length} notice files`,
  });

  for (const file of files) {
    const path = join(root, file);
    if (
      file.endsWith(".node") ||
      file === "bin/node" ||
      file === "bin/node_repl"
    ) {
      checkNativeFile(
        path,
        file,
        checks,
        allowFixtureValues,
        observeNativeCommand,
      );
    } else if (hasElfMagic(path))
      checkNativeFile(
        path,
        file,
        checks,
        allowFixtureValues,
        observeNativeCommand,
      );
  }
}

export function verifyCuaNode(
  options: VerifyCuaNodeOptions,
  observeNativeCommand?: NativeCommandObserver,
): VerificationReport {
  const root = resolve(options.root);
  const checks: VerificationCheck[] = [];
  const blockers: string[] = [];
  let manifest: JsonRecord | null = null;
  try {
    const rootStat = statSync(root);
    checks.push({
      id: "root:exists",
      status: rootStat.isDirectory() ? "passed" : "failed",
      detail: root,
    });
    const parsedManifest = JSON.parse(
      readFileSync(join(root, "manifest.json"), "utf8"),
    ) as unknown;
    const verifiedManifest = verifyManifest(
      parsedManifest,
      checks,
      options.expectedTarget ?? "linux-x64-glibc",
      options.allowFixtureValues ?? false,
    );
    if (verifiedManifest !== null) {
      manifest = verifiedManifest;
      verifyRuntimeFiles(
        root,
        manifest,
        checks,
        options.allowFixtureValues ?? false,
        observeNativeCommand,
      );
    }
  } catch (error) {
    const detail =
      error instanceof Error ? error.message : "verification failed";
    blockers.push(detail);
    checks.push({ id: "verification:exception", status: "failed", detail });
  }

  const enforcedLocks = (options.enforceLockPaths ?? []).map((lockPath) => {
    const inspection = inspectCuaNodeLock(lockPath);
    if (inspection.status !== "passed") blockers.push(...inspection.blockers);
    checks.push({
      id: `lock:${inspection.path}`,
      status: inspection.status,
      detail: inspection.detail,
    });
    return inspection;
  });
  if (options.allowFixtureValues !== true) {
    const requiredKinds = ["runtime", "native-assets"] as const;
    const missingKinds = requiredKinds.filter(
      (kind) => !enforcedLocks.some((inspection) => inspection.kind === kind),
    );
    const duplicateKinds = requiredKinds.filter(
      (kind) =>
        enforcedLocks.filter((inspection) => inspection.kind === kind).length >
        1,
    );
    if (missingKinds.length > 0 || duplicateKinds.length > 0) {
      const detail = [
        missingKinds.length > 0
          ? `missing ${missingKinds.join(", ")} lock`
          : null,
        duplicateKinds.length > 0
          ? `duplicate ${duplicateKinds.join(", ")} lock`
          : null,
      ]
        .filter((entry): entry is string => entry !== null)
        .join("; ");
      blockers.push(detail);
      checks.push({
        id: "locks:complete-provenance",
        status: "failed",
        detail: `production verification requires exactly one runtime and native-assets lock (${detail})`,
      });
    }
  }
  if (manifest !== null) {
    const checksums = record(manifest.checksums, "manifest.checksums");
    const lockHashes = record(
      checksums.lock_hashes,
      "manifest.checksums.lock_hashes",
    );
    for (const inspection of enforcedLocks) {
      const manifestKey =
        inspection.kind === "runtime"
          ? "runtime_lock_sha256"
          : inspection.kind === "native-assets"
            ? "native_assets_lock_sha256"
            : null;
      const expectedHash =
        manifestKey === null ? null : lockHashes[manifestKey];
      const matches =
        inspection.sha256.length > 0 &&
        typeof expectedHash === "string" &&
        expectedHash === inspection.sha256;
      const skippedFixtureCheck = options.allowFixtureValues === true;
      checks.push({
        id: `lock-manifest-hash:${inspection.path}`,
        status: skippedFixtureCheck || matches ? "passed" : "failed",
        detail: skippedFixtureCheck
          ? "fixture lock hash comparison bypassed"
          : matches
            ? "manifest lock hash matches the enforced lock bytes"
            : "manifest lock hash does not match the enforced lock bytes",
      });
    }
  }

  const preflightProblems = checks.filter(
    (check) => check.status === "failed" || check.status === "blocked",
  );
  if (manifest !== null && preflightProblems.length === 0) {
    const nodeIdentity = readExecutableIdentity(join(root, "bin/node"));
    const expectedNodeIdentity = `v${stringValue(
      manifest.node_version,
      "manifest.node_version",
    )}`;
    checks.push({
      id: "identity:node-version",
      status:
        nodeIdentity.ok && nodeIdentity.output === expectedNodeIdentity
          ? "passed"
          : "failed",
      detail: nodeIdentity.ok
        ? `expected ${expectedNodeIdentity}, got ${nodeIdentity.output}`
        : nodeIdentity.output,
    });

    const nodeReplIdentity = readExecutableIdentity(
      join(root, "bin/node_repl"),
    );
    const expectedNodeReplVersion = stringValue(
      manifest.node_repl_version,
      "manifest.node_repl_version",
    );
    checks.push({
      id: "identity:node-repl-version",
      status:
        nodeReplIdentity.ok &&
        nodeReplIdentity.output.endsWith(`/${expectedNodeReplVersion}`)
          ? "passed"
          : "failed",
      detail: nodeReplIdentity.ok
        ? `expected */${expectedNodeReplVersion}, got ${nodeReplIdentity.output}`
        : nodeReplIdentity.output,
    });
  } else {
    checks.push({
      id: "execution:preflight",
      status: preflightProblems.some((check) => check.status === "blocked")
        ? "blocked"
        : "failed",
      detail:
        "candidate executables were not run because static preflight did not pass",
    });
  }

  const status = checks.some((check) => check.status === "failed")
    ? "failed"
    : checks.some((check) => check.status === "blocked") || blockers.length > 0
      ? "blocked"
      : "passed";
  return { status, root, checks, blockers };
}
