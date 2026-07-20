import { writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { API_MANIFEST, API_SURFACE } from "./api.ts";
import {
  COMMANDS,
  COMMAND_GROUPS,
  RAW_PROTOCOL_SUPPORTED_COMMANDS,
  RAW_PROTOCOL_UNSUPPORTED_COMMANDS,
} from "./commands.ts";

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const fixtureRoot = resolve(packageRoot, "fixtures");
await Bun.$`mkdir -p ${fixtureRoot}`.quiet();
await writeFile(resolve(fixtureRoot, "api-surface.json"), `${JSON.stringify({
  root: API_MANIFEST.root,
  interfaces: API_SURFACE,
  declarations: API_MANIFEST,
}, null, 2)}\n`);
await writeFile(resolve(fixtureRoot, "commands.json"), `${JSON.stringify({
  count: COMMANDS.length,
  commands: COMMANDS,
  groups: COMMAND_GROUPS,
  rawProtocol: {
    supported: RAW_PROTOCOL_SUPPORTED_COMMANDS,
    unsupported: RAW_PROTOCOL_UNSUPPORTED_COMMANDS,
  },
}, null, 2)}\n`);
