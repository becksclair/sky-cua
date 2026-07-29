import { strict as assert } from "node:assert";
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough } from "node:stream";
import { test } from "bun:test";
import { KernelChild } from "../src/host/kernel-child.ts";
import { McpServer } from "../src/host/mcp-server.ts";
import { RuntimeManager } from "../src/host/runtime-manager.ts";
import { TEST_NODE_PATH } from "./test-node-path.ts";

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function waitFor(condition: () => boolean, timeoutMs = 1_000): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() >= deadline) throw new Error("condition was not met");
    await delay(5);
  }
}

function processIsAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

test("kernel termination rejects pending work and aborts every privileged callback", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-child-stop-"));
  const fakeNode = join(directory, "fake-node");
  const callbackSignals: AbortSignal[] = [];
  try {
    await writeFile(
      fakeNode,
      `#!/usr/bin/env node
const token = 'fixture-token';
process.send?.({ version: 'cua-kernel-control-v2', type: 'privileged_bridge_handshake', token });
process.on('message', (message) => {
  if (message.type !== 'exec') return;
  const common = { version: 'cua-kernel-control-v2', type: 'privileged_request', exec_id: message.id, token, generation: token };
  process.send?.({ ...common, id: 'fetch-fixture', op: 'authenticated_fetch', input: 'data:text/plain,fixture' });
  process.send?.({ ...common, id: 'elicitation-fixture', op: 'elicitation', request: {} });
  process.send?.({ ...common, id: 'config-fixture', op: 'config', config_op: 'read' });
});
setInterval(() => {}, 1000);
`,
      "utf8",
    );
    await chmod(fakeNode, 0o755);
    const pendingCallback = (signal: AbortSignal): Promise<never> => {
      callbackSignals.push(signal);
      return new Promise(() => undefined);
    };
    const child = new KernelChild({
      nodePath: fakeNode,
      cwd: directory,
      env: { ...process.env },
      onAuthenticatedFetch: (_input, _init, signal) => pendingCallback(signal),
      onElicitation: (_request, signal) => pendingCallback(signal),
      onConfig: (_operation, _payload, signal) => pendingCallback(signal),
    });
    const state = child as unknown as {
      handlePrivilegedRequest(message: unknown): Promise<void>;
      moduleDirPending: Map<string, unknown>;
    };
    const originalHandler = state.handlePrivilegedRequest.bind(child);
    const privilegedHandlers: Promise<void>[] = [];
    state.handlePrivilegedRequest = (message) => {
      const handler = originalHandler(message);
      privilegedHandlers.push(handler);
      return handler;
    };
    const execution = child.execute("exec-fixture", "await 1", null);
    const moduleDir = child.addNodeModuleDir(directory);
    const executionRejected = assert.rejects(execution, /fixture termination/u);
    const moduleDirRejected = assert.rejects(moduleDir, /fixture termination/u);
    await waitFor(() => callbackSignals.length === 3);
    await child.terminate("fixture termination");
    await Promise.all([executionRejected, moduleDirRejected, ...privilegedHandlers]);
    assert.equal(
      callbackSignals.every((signal) => signal.aborted),
      true,
    );
    assert.equal(new Set(callbackSignals).size, 1);
    assert.equal(state.moduleDirPending.size, 0);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("module-dir work rejects a child detached during startup", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-detached-child-"));
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  const state = manager as unknown as {
    child: KernelChild | null;
    ensureChild(): Promise<KernelChild>;
  };
  try {
    const detached = await state.ensureChild();
    const detachedPid = detached.pid;
    assert.notEqual(detachedPid, undefined);
    state.ensureChild = async () => {
      await manager.close();
      return detached;
    };
    await assert.rejects(
      manager.addNodeModuleDir(directory),
      /kernel generation changed/u,
    );
    assert.equal(state.child, null);
    assert.equal(processIsAlive(detachedPid!), false);
  } finally {
    await manager.close();
    await rm(directory, { recursive: true, force: true });
  }
});

test("close during delayed runtime metadata does not spawn a child", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-metadata-close-"));
  const fakeNode = join(directory, "fake-node");
  const pidFile = join(directory, "pid");
  let releaseMetadata = (): void => {
    throw new Error("metadata gate was not initialized");
  };
  let metadataEntered = false;
  const metadataGate = new Promise<void>((resolve) => {
    releaseMetadata = resolve;
  });
  try {
    await writeFile(
      fakeNode,
      `#!/usr/bin/env node
import { writeFileSync } from 'node:fs';
writeFileSync(${JSON.stringify(pidFile)}, String(process.pid));
setInterval(() => {}, 1000);
`,
      "utf8",
    );
    await chmod(fakeNode, 0o755);
    const manager = new RuntimeManager({ nodePath: fakeNode, cwd: directory });
    const state = manager as unknown as {
      ensureChild(): Promise<KernelChild>;
      resolveRuntimeMetadata(nodePath: string): Promise<null>;
    };
    state.resolveRuntimeMetadata = async () => {
      metadataEntered = true;
      await metadataGate;
      return null;
    };
    const startup = state.ensureChild();
    await waitFor(() => metadataEntered);
    const close = manager.close();
    releaseMetadata();
    await assert.rejects(startup, /shutting down|generation changed/u);
    await close;
    await assert.rejects(readFile(pidFile, "utf8"), /ENOENT/u);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("reset during delayed runtime metadata starts only the new generation", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-metadata-reset-"));
  const fakeNode = join(directory, "fake-node");
  const pidFile = join(directory, "pids");
  let releaseMetadata = (): void => {
    throw new Error("metadata gate was not initialized");
  };
  let metadataCalls = 0;
  const metadataGate = new Promise<void>((resolve) => {
    releaseMetadata = resolve;
  });
  try {
    await writeFile(
      fakeNode,
      `#!/usr/bin/env node
import { appendFileSync } from 'node:fs';
appendFileSync(${JSON.stringify(pidFile)}, String(process.pid) + '\\n');
process.send?.({ version: 'cua-kernel-control-v2', type: 'privileged_bridge_handshake', token: 'fixture-token' });
setInterval(() => {}, 1000);
`,
      "utf8",
    );
    await chmod(fakeNode, 0o755);
    const manager = new RuntimeManager({ nodePath: fakeNode, cwd: directory });
    try {
      const state = manager as unknown as {
        resolveRuntimeMetadata(nodePath: string): Promise<null>;
      };
      state.resolveRuntimeMetadata = async () => {
        metadataCalls += 1;
        if (metadataCalls === 1) await metadataGate;
        return null;
      };
      const execution = manager.execute("await new Promise(() => {})", {
        requestId: "metadata-reset",
        requestMeta: null,
      });
      const executionRejected = assert.rejects(execution, /reset/u);
      await waitFor(() => metadataCalls === 1);
      const reset = manager.reset();
      releaseMetadata();
      await Promise.all([executionRejected, reset]);
      const pids = (await readFile(pidFile, "utf8")).trim().split("\n").map(Number);
      assert.equal(pids.length, 1);
      assert.equal(processIsAlive(pids[0]!), true);
      await manager.close();
      assert.equal(processIsAlive(pids[0]!), false);
    } finally {
      releaseMetadata();
      await manager.close();
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("malformed implicit runtime manifest fails before child spawn", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-malformed-manifest-"));
  const binDirectory = join(directory, "bin");
  const fakeNode = join(binDirectory, "fake-node");
  const pidFile = join(directory, "pid");
  try {
    await mkdir(binDirectory);
    await writeFile(join(directory, "manifest.json"), "{ malformed", "utf8");
    await writeFile(
      fakeNode,
      `#!/usr/bin/env node
import { writeFileSync } from 'node:fs';
writeFileSync(${JSON.stringify(pidFile)}, String(process.pid));
setInterval(() => {}, 1000);
`,
      "utf8",
    );
    await chmod(fakeNode, 0o755);
    const manager = new RuntimeManager({
      nodePath: fakeNode,
      cwd: directory,
      allowHostNode: true,
    });
    try {
      await assert.rejects(
        manager.execute("nodeRepl.write('never')", {
          requestId: "malformed-manifest",
          requestMeta: null,
        }),
        /JSON|property name|position/u,
      );
      await assert.rejects(readFile(pidFile, "utf8"), /ENOENT/u);
    } finally {
      await manager.close();
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

for (const fixture of [
  {
    name: "schema-invalid",
    mutate: (manifest: Record<string, unknown>): void => {
      delete manifest.schema_version;
    },
    expected: /canonical schema.*schema_version/u,
  },
  {
    name: "target-drift",
    mutate: (manifest: Record<string, unknown>): void => {
      manifest.target = "darwin-arm64";
    },
    expected: /canonical schema.*target/u,
  },
] as const) {
  test(`${fixture.name} implicit runtime manifest fails before child spawn`, async () => {
    const directory = await mkdtemp(join(tmpdir(), `cua-node-${fixture.name}-`));
    const binDirectory = join(directory, "bin");
    const fakeNode = join(binDirectory, "fake-node");
    const pidFile = join(directory, "pid");
    try {
      await mkdir(binDirectory);
      const manifest = JSON.parse(
        await readFile(
          join(import.meta.dir, "fixtures", "fake-runtime", "manifest.json"),
          "utf8",
        ),
      ) as Record<string, unknown>;
      fixture.mutate(manifest);
      await writeFile(
        join(directory, "manifest.json"),
        `${JSON.stringify(manifest)}\n`,
        "utf8",
      );
      await writeFile(
        fakeNode,
        `#!/usr/bin/env node
import { writeFileSync } from 'node:fs';
writeFileSync(${JSON.stringify(pidFile)}, String(process.pid));
setInterval(() => {}, 1000);
`,
        "utf8",
      );
      await chmod(fakeNode, 0o755);
      const manager = new RuntimeManager({
        nodePath: fakeNode,
        cwd: directory,
        allowHostNode: true,
      });
      try {
        await assert.rejects(
          manager.execute("nodeRepl.write('never')", {
            requestId: fixture.name,
            requestMeta: null,
          }),
          fixture.expected,
        );
        await assert.rejects(readFile(pidFile, "utf8"), /ENOENT/u);
      } finally {
        await manager.close();
      }
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
}

test("runtime authenticated fetch forwards its child abort signal", async () => {
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  const controller = new AbortController();
  const state = manager as unknown as {
    authenticatedFetch(
      input: string,
      init: Record<string, unknown> | undefined,
      signal: AbortSignal,
    ): Promise<unknown>;
  };
  const originalFetch = globalThis.fetch;
  let receivedSignal: AbortSignal | null = null;
  globalThis.fetch = ((_input: string | URL | Request, init?: RequestInit) => {
    receivedSignal = init?.signal ?? null;
    return Promise.resolve(new Response("fixture"));
  }) as typeof fetch;
  try {
    await state.authenticatedFetch(
      "data:text/plain,should-not-load",
      undefined,
      controller.signal,
    );
    assert.equal(receivedSignal, controller.signal);
  } finally {
    globalThis.fetch = originalFetch;
    await manager.close();
  }
});

test("MCP client requests abort without retaining stale generation handlers", async () => {
  const output = new PassThrough();
  const server = new McpServer({ output });
  await server.dispatch({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: "2025-11-25",
      capabilities: { elicitation: { form: {} } },
    },
  });
  const controller = new AbortController();
  const request = server.requestClient("elicitation/create", {}, controller.signal);
  controller.abort();
  await assert.rejects(request);
  const state = server as unknown as { clientRequests: Map<unknown, unknown> };
  assert.equal(state.clientRequests.size, 0);
  await server.close();
});

test("production nodeRepl elicitation round-trips through the MCP client", async () => {
  const output = new PassThrough();
  let written = "";
  output.on("data", (chunk) => { written += chunk.toString(); });
  const server = new McpServer({ output });
  try {
    await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: { elicitation: { form: {} } },
      },
    });
    const managerState = server.manager as unknown as {
      options: {
        onElicitation(request: Record<string, unknown>, signal: AbortSignal): Promise<unknown>;
      };
    };
    const execution = managerState.options.onElicitation(
      { message: "Choose", requestedSchema: { type: "object" } },
      new AbortController().signal,
    );
    await waitFor(() => written.includes('"method":"elicitation/create"'));
    const outbound = written
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line) as Record<string, unknown>)
      .find((message) => message.method === "elicitation/create");
    assert.equal(typeof outbound?.id, "string");
    await server.handleLine(JSON.stringify({
      jsonrpc: "2.0",
      id: outbound?.id,
      result: { action: "accept", content: { answer: "yes" } },
    }));
    assert.deepEqual(await execution, {
      action: "accept",
      content: { answer: "yes" },
    });
  } finally {
    await server.close();
  }
});

test("production nodeRepl preserves MCP client rejection details", async () => {
  const output = new PassThrough();
  let written = "";
  output.on("data", (chunk) => { written += chunk.toString(); });
  const server = new McpServer({ output });
  try {
    await server.dispatch({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-11-25",
        capabilities: { elicitation: { form: {} } },
      },
    });
    const request = server.requestClient("elicitation/create", {
      message: "Choose",
    });
    await waitFor(() => written.includes('"method":"elicitation/create"'));
    const outbound = written
      .trim()
      .split("\n")
      .map((line) => JSON.parse(line) as Record<string, unknown>)
      .find((message) => message.method === "elicitation/create");
    await server.handleLine(JSON.stringify({
      jsonrpc: "2.0",
      id: outbound?.id,
      error: { code: -32602, message: "requestedSchema is required" },
    }));
    await assert.rejects(
      request,
      /MCP client request failed \(-32602\): requestedSchema is required/u,
    );
  } finally {
    await server.close();
  }
});

test("client disconnect rejects production elicitation and lets the server stop", async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  let written = "";
  output.on("data", (chunk) => { written += chunk.toString(); });
  const server = new McpServer({ input, output });
  const started = server.start();
  await server.dispatch({
    jsonrpc: "2.0",
    id: 1,
    method: "initialize",
    params: {
      protocolVersion: "2025-11-25",
      capabilities: { elicitation: { form: {} } },
    },
  });
  const managerState = server.manager as unknown as {
    options: {
      onElicitation(request: Record<string, unknown>, signal: AbortSignal): Promise<unknown>;
    };
  };
  const execution = managerState.options.onElicitation(
    { message: "Choose", requestedSchema: { type: "object" } },
    new AbortController().signal,
  );
  await waitFor(() => written.includes('"method":"elicitation/create"'));
  input.end();
  await assert.rejects(execution, /MCP host closed/);
  await started;
  await assert.rejects(
    server.requestClient("elicitation/create", {}),
    /MCP host closed/,
  );
  const state = server as unknown as { clientRequests: Map<unknown, unknown> };
  assert.equal(state.clientRequests.size, 0);
});

test("cancellation wins while timeout teardown is pending", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-timeout-race-"));
  const fakeNode = join(directory, "fake-node");
  const pidFile = join(directory, "pids");
  try {
    await writeFile(
      fakeNode,
      `#!/usr/bin/env node
import { appendFileSync, readFileSync } from 'node:fs';
appendFileSync(${JSON.stringify(pidFile)}, String(process.pid) + '\\n');
const generation = readFileSync(${JSON.stringify(pidFile)}, 'utf8').trim().split('\\n').length;
process.on('SIGTERM', () => {});
process.send?.({ version: 'cua-kernel-control-v2', type: 'privileged_bridge_handshake', token: 'fixture-token' });
process.on('message', (message) => {
  if (message.type === 'exec' && generation > 1) {
    process.send?.({ version: 'cua-kernel-control-v2', type: 'exec_result', id: message.id, ok: true, output: 'fresh', images: [], response_meta: null });
  }
});
setInterval(() => {}, 1000);
`,
      "utf8",
    );
    await chmod(fakeNode, 0o755);
    const manager = new RuntimeManager({
      nodePath: fakeNode,
      cwd: directory,
      allowHostNode: true,
    });
    try {
      const first = manager.execute("await new Promise(() => {})", {
        requestId: "timeout-race",
        requestMeta: null,
        timeoutMs: 100,
      });
      const firstRejected = assert.rejects(first, /cancelled/u);
      const state = manager as unknown as { termination: Promise<void> | null };
      await waitFor(() => state.termination !== null);
      manager.cancel("timeout-race");
      await firstRejected;

      const second = await manager.execute("nodeRepl.write('fresh')", {
        requestId: "next-generation",
        requestMeta: null,
        timeoutMs: 2_000,
      });
      assert.equal(second.output, "fresh");
      const pids = (await readFile(pidFile, "utf8")).trim().split("\n").map(Number);
      assert.equal(pids.length, 2);
      assert.equal(processIsAlive(pids[0]!), false);
    } finally {
      await manager.close();
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("protocol failure kills the old child before the next generation starts", async () => {
  const directory = await mkdtemp(join(tmpdir(), "cua-node-protocol-failure-"));
  const fakeNode = join(directory, "fake-node");
  const pidFile = join(directory, "pids");
  try {
    await writeFile(
      fakeNode,
      `#!/usr/bin/env node
import { appendFileSync } from 'node:fs';
appendFileSync(${JSON.stringify(pidFile)}, String(process.pid) + '\\n');
process.send?.({ version: 'cua-kernel-control-v1', type: 'privileged_bridge_handshake', token: 'fake-token' });
process.on('message', () => process.send?.({ version: 'wrong-kernel-version', type: 'protocol_error', error: 'fixture protocol failure' }));
setInterval(() => {}, 1000);
`,
      "utf8",
    );
    await chmod(fakeNode, 0o755);
    const manager = new RuntimeManager({
      nodePath: fakeNode,
      cwd: directory,
      allowHostNode: true,
    });
    try {
      await assert.rejects(
        manager.execute("nodeRepl.write('never')", {
          requestId: 1,
          requestMeta: null,
        }),
        /fixture protocol failure|unsupported kernel control protocol/u,
      );
      const firstPid = Number((await readFile(pidFile, "utf8")).trim());
      assert.equal(processIsAlive(firstPid), false);
      await assert.rejects(
        manager.execute("nodeRepl.write('never')", {
          requestId: 2,
          requestMeta: null,
        }),
        /fixture protocol failure|unsupported kernel control protocol/u,
      );
      const pids = (await readFile(pidFile, "utf8")).trim().split("\n").map(Number);
      assert.equal(pids.length, 2);
      assert.equal(processIsAlive(pids[0]!), false);
      assert.equal(processIsAlive(pids[1]!), false);
    } finally {
      await manager.close();
    }
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
