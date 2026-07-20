import { execFileSync } from "node:child_process";
import { isAbsolute } from "node:path";

const configured = process.env.CUA_NODE_EXACT_NODE_PATH;
const resolved =
  configured ??
  execFileSync("node", ["-p", "process.execPath"], {
    encoding: "utf8",
  }).trim();

if (!isAbsolute(resolved))
  throw new Error(`test Node path must be absolute: ${resolved}`);

export const TEST_NODE_PATH = resolved;
