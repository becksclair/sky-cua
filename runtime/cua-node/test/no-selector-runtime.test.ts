import { spawn, spawnSync } from "node:child_process";
import {
  chmod,
  copyFile,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { test } from "bun:test";
import { strict as assert } from "node:assert";
import { TEST_NODE_PATH } from "./test-node-path.ts";

const SELECTOR_ENV = [
  "SKY_CUA_RELEASE_ROOT",
  "SKY_CUA_RELEASE_ID",
  "CODEX_BROWSER_USE_MODULE_SEARCH_ROOT",
  "CODEX_BROWSER_USE_NODE_PATH",
  "PLAYWRIGHT_BROWSERS_PATH",
  "NODE_REPL_NODE_PATH",
  "NODE_REPL_NODE_MODULE_DIRS",
  "CUA_NODE_BROWSER_CLIENT_PATH",
] as const;

test("installed node_repl self-discovers Node, modules, and fixed Browser code", async () => {
  const runtimeRoot = await mkdtemp(join(tmpdir(), "cua-node-self-discovery-"));
  try {
    await cp(join(import.meta.dir, "fixtures", "fake-runtime"), runtimeRoot, {
      recursive: true,
    });
    const nodePath = join(runtimeRoot, "bin", "node");
    await copyFile(TEST_NODE_PATH, nodePath);
    await chmod(nodePath, 0o755);
    await mkdir(join(runtimeRoot, "lib", "node_repl"), { recursive: true });
    const built = await Bun.build({
      entrypoints: [join(import.meta.dir, "..", "src", "cli.ts")],
      outdir: join(runtimeRoot, "lib", "node_repl"),
      naming: "cli.js",
      target: "node",
    });
    assert.equal(built.success, true, built.logs.map(String).join("\n"));
    await mkdir(join(runtimeRoot, "share", "pdfjs", "cmaps"), {
      recursive: true,
    });
    await mkdir(join(runtimeRoot, "share", "pdfjs", "standard_fonts"), {
      recursive: true,
    });
    await mkdir(
      join(runtimeRoot, "lib", "node_modules", "pdfjs-dist", "legacy", "build"),
      { recursive: true },
    );
    await writeFile(
      join(
        runtimeRoot,
        "lib",
        "node_modules",
        "pdfjs-dist",
        "legacy",
        "build",
        "pdf.worker.mjs",
      ),
      "export {};\n",
      "utf8",
    );
    await writeFile(
      join(runtimeRoot, "share", "tessdata", "eng.traineddata"),
      "fixture\n",
      "utf8",
    );
    const browserClient = join(
      runtimeRoot,
      "lib",
      "node_modules",
      "@heliasar",
      "browser-use",
      "build",
      "browser-client.mjs",
    );
    await writeFile(
      browserClient,
      "export const hasNativePipe = typeof nodeRepl.nativePipe?.createConnection === 'function';\n",
      "utf8",
    );

    const launcher = join(runtimeRoot, "bin", "node_repl");
    const compile = spawnSync(
      "cc",
      [
        "-Os",
        "-nostdlib",
        "-static",
        "-ffreestanding",
        "-fno-builtin",
        "-fno-stack-protector",
        "-fno-pie",
        "-no-pie",
        "-Wl,--build-id=none",
        "-o",
        launcher,
        join(import.meta.dir, "..", "native", "node_repl.c"),
      ],
      { encoding: "utf8" },
    );
    assert.equal(compile.status, 0, compile.stderr);

    const env = { ...process.env };
    for (const name of SELECTOR_ENV) delete env[name];
    for (const name of SELECTOR_ENV) {
      assert.equal(env[name], undefined, `${name} must be absent at launch`);
    }
    const child = spawn(launcher, [], {
      cwd: runtimeRoot,
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });
    child.stderr.setEncoding("utf8");
    let stderr = "";
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
    });
    const reader = createInterface({
      input: child.stdout,
      crlfDelay: Infinity,
    });
    const lines: string[] = [];
    const waiters: Array<(line: string) => void> = [];
    reader.on("line", (line) => {
      const waiter = waiters.shift();
      if (waiter === undefined) lines.push(line);
      else waiter(line);
    });
    const nextLine = (): Promise<string> => {
      const line = lines.shift();
      if (line !== undefined) return Promise.resolve(line);
      return new Promise((resolvePromise) => waiters.push(resolvePromise));
    };
    const request = async (value: object): Promise<Record<string, unknown>> => {
      child.stdin.write(`${JSON.stringify(value)}\n`);
      return JSON.parse(await nextLine()) as Record<string, unknown>;
    };
    await request({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} });
    const toolResponse = await request({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: {
        name: "js",
        arguments: {
          code: "var browser = await import('@heliasar/browser-use'); nodeRepl.write(JSON.stringify({ node: nodeRepl.runtime.node.execPath, modules: nodeRepl.runtime.modules.root, trusted: browser.hasNativePipe }))",
        },
      },
    });
    const shutdown = await request({
      jsonrpc: "2.0",
      id: 3,
      method: "shutdown",
      params: {},
    });
    assert.equal(shutdown.result, null);
    child.stdin.end();
    const exitCode = await new Promise<number | null>((resolvePromise) =>
      child.once("exit", resolvePromise),
    );
    assert.equal(exitCode, 0, stderr);
    const result = toolResponse.result as {
      content: Array<{ text: string }>;
    };
    const text = result.content[0]?.text;
    assert.ok(text !== undefined);
    const evidence = JSON.parse(text);
    assert.deepEqual(evidence, {
      node: nodePath,
      modules: join(runtimeRoot, "lib", "node_modules"),
      trusted: true,
    });
  } finally {
    await rm(runtimeRoot, { recursive: true, force: true });
  }
});
