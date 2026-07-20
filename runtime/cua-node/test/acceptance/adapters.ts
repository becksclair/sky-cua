import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

export type AdapterRun = {
  adapter: string;
  commandEnv: string;
  usedCommand: boolean;
  output: string;
};

export async function runTaskAdapter(options: {
  adapter: string;
  commandEnv: string;
  fallback: () => Promise<string> | string;
}): Promise<AdapterRun> {
  const command = process.env[options.commandEnv];
  if (command === undefined || command.trim() === "") {
    return {
      adapter: options.adapter,
      commandEnv: options.commandEnv,
      usedCommand: false,
      output: await options.fallback(),
    };
  }
  const { stdout, stderr } = await execFileAsync("/bin/sh", ["-c", command], {
    env: { ...process.env, CUA_NODE_ACCEPTANCE_NETWORK: "disabled" },
    maxBuffer: 4 * 1024 * 1024,
  });
  return {
    adapter: options.adapter,
    commandEnv: options.commandEnv,
    usedCommand: true,
    output: `${stdout}${stderr}`,
  };
}
