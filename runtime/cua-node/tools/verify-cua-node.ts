import { inspectCuaNodeLock } from "./verifier/lock-inspection";
import { verifyCuaNode } from "./verifier/runtime-verification";
import type {
  CuaNodeLockInspection,
  VerificationCheck,
  VerificationReport,
  VerifyCuaNodeOptions,
} from "./cua-node-verifier/types";

export { inspectCuaNodeLock, verifyCuaNode };
export type {
  CuaNodeLockInspection,
  VerificationCheck,
  VerificationReport,
  VerifyCuaNodeOptions,
};

export function assertVerified(report: VerificationReport): void {
  if (report.status !== "passed") throw new Error(JSON.stringify(report, null, 2));
}

function parseArgs(argv: string[]): VerifyCuaNodeOptions & { json: boolean } {
  let root = "out/linux-x64/cua_node";
  let json = false;
  let expectedTarget = "linux-x64-glibc";
  const enforceLockPaths: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--json") json = true;
    else if (arg.startsWith("--root=")) root = arg.slice("--root=".length);
    else if (arg.startsWith("--enforce-lock="))
      enforceLockPaths.push(arg.slice("--enforce-lock=".length));
    else if (arg === "--target") {
      const target = argv[index + 1];
      if (target === undefined) throw new Error("--target requires a value");
      expectedTarget = target === "linux-x64" ? "linux-x64-glibc" : target;
      index += 1;
    } else if (arg.startsWith("--target=")) {
      const target = arg.slice("--target=".length);
      expectedTarget = target === "linux-x64" ? "linux-x64-glibc" : target;
    } else if (arg === "--allow-fixture-values") continue;
    else if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: bun tools/verify-cua-node.ts [--root=PATH] [--target linux-x64] [--enforce-lock=PATH] [--allow-fixture-values] [--json]",
      );
      process.exit(0);
    } else throw new Error(`unknown argument ${arg}`);
  }
  return {
    root,
    expectedTarget,
    enforceLockPaths,
    allowFixtureValues: argv.includes("--allow-fixture-values"),
    json,
  };
}

if (require.main === module) {
  try {
    const options = parseArgs(process.argv.slice(2));
    const report = verifyCuaNode(options);
    const output = JSON.stringify(report, null, 2);
    if (options.json) console.log(output);
    else console.log(`${report.status}: ${report.root}`);
    if (report.status !== "passed") process.exitCode = 1;
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
