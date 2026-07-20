import { createStdioServer } from "./host/mcp-server.ts";

export const NODE_REPL_VERSION = "0.1.0";

export async function main(argv = process.argv.slice(2)): Promise<void> {
  if (argv.includes("--version")) {
    process.stdout.write(`node_repl/${NODE_REPL_VERSION}\n`);
    return;
  }
  const server = createStdioServer();
  const close = (): void => {
    void server.close();
  };
  process.once("SIGTERM", close);
  process.once("SIGINT", close);
  await server.start();
}

if (import.meta.main) {
  void main().catch((error: unknown) => {
    const message = error instanceof Error ? error.stack ?? error.message : String(error);
    process.stderr.write(`${message}\n`);
    process.exitCode = 1;
  });
}
