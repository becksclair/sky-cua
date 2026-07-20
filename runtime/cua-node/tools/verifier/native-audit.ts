import { execFileSync } from "node:child_process";
import type { VerificationCheck } from "./types";

export type NativeCommandObserver = (command: string, path: string) => void;

export function runCommand(
  command: string,
  args: string[],
  path: string,
  observeCommand?: NativeCommandObserver,
): { ok: boolean; output: string } {
  observeCommand?.(command, path);
  try {
    return {
      ok: true,
      output: execFileSync(command, [...args, path], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      }),
    };
  } catch (error) {
    const detail = error instanceof Error ? error.message : "unknown command failure";
    return { ok: false, output: detail };
  }
}

export function readExecutableIdentity(path: string): {
  ok: boolean;
  output: string;
} {
  try {
    return {
      ok: true,
      output: execFileSync(path, ["--version"], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      }).trim(),
    };
  } catch (error) {
    return {
      ok: false,
      output: error instanceof Error ? error.message : "version probe failed",
    };
  }
}

export function checkNativeFile(
  path: string,
  relativePath: string,
  checks: VerificationCheck[],
  allowFixtureValues: boolean,
  observeCommand?: NativeCommandObserver,
): void {
  const fileResult = runCommand("file", ["-b"], path, observeCommand);
  if (!fileResult.ok) {
    checks.push({
      id: `native-file:${relativePath}`,
      status: "failed",
      detail: fileResult.output,
    });
    return;
  }
  const description = fileResult.output;
  if (!/ELF 64-bit/iu.test(description) || !/x86-64/iu.test(description)) {
    if (
      allowFixtureValues &&
      (relativePath === "bin/node" || relativePath === "bin/node_repl")
    ) {
      checks.push({
        id: `native-audit:${relativePath}`,
        status: "passed",
        detail:
          "fixture executable accepted; production verification requires an ELF64 x86-64 binary",
      });
      return;
    }
    checks.push({
      id: `native-architecture:${relativePath}`,
      status: "failed",
      detail: `expected ELF64 x86-64, got ${description.trim()}`,
    });
    return;
  }
  if (/musl/iu.test(description)) {
    checks.push({
      id: `native-libc:${relativePath}`,
      status: "failed",
      detail: "musl artifact is not accepted for linux-x64-glibc",
    });
    return;
  }

  const dynamic = runCommand("readelf", ["-d"], path, observeCommand);
  if (!dynamic.ok) {
    checks.push({
      id: `native-dynamic:${relativePath}`,
      status: "failed",
      detail: dynamic.output,
    });
    return;
  }
  const pathTags = [
    ...dynamic.output.matchAll(/\((?:RPATH|RUNPATH)\).*?\[([^\]]*)\]/gu),
  ].flatMap((match) => match[1]?.split(":") ?? []);
  const unsafePaths = pathTags.filter(
    (entry) =>
      entry.length > 0 && (!entry.startsWith("$ORIGIN") || entry.includes("musl")),
  );
  if (unsafePaths.length > 0) {
    checks.push({
      id: `native-rpath:${relativePath}`,
      status: "failed",
      detail: `unexpected RPATH/RUNPATH: ${unsafePaths.join(",")}`,
    });
    return;
  }

  const versions = runCommand("readelf", ["--version-info"], path, observeCommand);
  if (!versions.ok) {
    checks.push({
      id: `native-versions:${relativePath}`,
      status: "failed",
      detail: versions.output,
    });
    return;
  }
  const glibcVersions = [...versions.output.matchAll(/GLIBC_(\d+)\.(\d+)/gu)].map(
    (match) => Number(match[1]) * 100 + Number(match[2]),
  );
  if (glibcVersions.some((version) => version > 228)) {
    checks.push({
      id: `native-glibc:${relativePath}`,
      status: "failed",
      detail: "requires GLIBC newer than the 2.28 baseline",
    });
    return;
  }

  const ldd = runCommand("ldd", [], path, observeCommand);
  const lddOutput = ldd.output;
  if (!ldd.ok && !/not a dynamic executable|statically linked/iu.test(lddOutput)) {
    checks.push({
      id: `native-ldd:${relativePath}`,
      status: "failed",
      detail: lddOutput,
    });
    return;
  }
  if (/not found|musl/iu.test(lddOutput)) {
    checks.push({
      id: `native-ldd:${relativePath}`,
      status: "failed",
      detail: `missing or incompatible library: ${lddOutput.trim()}`,
    });
    return;
  }
  checks.push({
    id: `native-audit:${relativePath}`,
    status: "passed",
    detail: "ELF64 x86-64, glibc baseline, RPATH/RUNPATH, and ldd checks passed",
  });
}
