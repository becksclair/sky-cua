import { strict as assert } from "node:assert";
import { EventEmitter } from "node:events";
import { test } from "bun:test";
import { KernelChild } from "../src/host/kernel-child.ts";
import { RuntimeManager } from "../src/host/runtime-manager.ts";
import { TEST_NODE_PATH } from "./test-node-path.ts";

const FINAL_EXIT_ERROR = /node_repl kernel did not exit within 250ms after SIGKILL/u;
const SETTLEMENT_DEADLINE_MS = 1_000;

class UnstoppableChild extends EventEmitter {
  public readonly signals: NodeJS.Signals[] = [];
  public readonly stdin = { destroyed: false };
  public exitCode: number | null = null;
  public signalCode: NodeJS.Signals | null = null;

  public kill(signal: NodeJS.Signals): boolean {
    this.signals.push(signal);
    return true;
  }

  public send(_message: unknown, _callback?: (error: Error | null) => void): boolean {
    return true;
  }
}

type KernelChildState = {
  child: UnstoppableChild | null;
};

type RuntimeManagerState = {
  active: unknown;
  child: KernelChild | null;
};

function createUnstoppableKernel(): {
  child: UnstoppableChild;
  kernel: KernelChild;
} {
  const child = new UnstoppableChild();
  const kernel = new KernelChild({
    nodePath: TEST_NODE_PATH,
    cwd: process.cwd(),
    env: { ...process.env },
  });
  (kernel as unknown as KernelChildState).child = child;
  return { child, kernel };
}

async function settlesWithin<T>(promise: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const deadline = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(
      () => reject(new Error("fixture settlement deadline exceeded")),
      SETTLEMENT_DEADLINE_MS,
    );
  });
  try {
    return await Promise.race([promise, deadline]);
  } finally {
    if (timer !== null) clearTimeout(timer);
  }
}

test("KernelChild termination rejects after the final SIGKILL exit deadline", async () => {
  const { child, kernel } = createUnstoppableKernel();

  await assert.rejects(
    settlesWithin(kernel.terminate("fixture termination")),
    FINAL_EXIT_ERROR,
  );
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
  assert.equal(child.listenerCount("exit"), 0);
});

test("runtime timeout propagates teardown failure and blocks a replacement", async () => {
  const { child, kernel } = createUnstoppableKernel();
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  const state = manager as unknown as RuntimeManagerState;
  state.child = kernel;

  const execution = manager.execute("await new Promise(() => {})", {
    requestId: "unstoppable-timeout",
    requestMeta: null,
    timeoutMs: 5,
  });
  await assert.rejects(settlesWithin(execution), FINAL_EXIT_ERROR);
  assert.equal(state.active, null);
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);

  await assert.rejects(
    settlesWithin(
      manager.execute("nodeRepl.write('replacement')", {
        requestId: "blocked-replacement",
        requestMeta: null,
        timeoutMs: 5,
      }),
    ),
    FINAL_EXIT_ERROR,
  );
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
  await assert.rejects(settlesWithin(manager.close()), FINAL_EXIT_ERROR);
});

test("runtime execution failure latches teardown failure and blocks a replacement", async () => {
  const { child, kernel } = createUnstoppableKernel();
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  const state = manager as unknown as RuntimeManagerState;
  state.child = kernel;
  kernel.execute = async () => {
    throw new Error("fixture control-channel failure");
  };

  await assert.rejects(
    settlesWithin(
      manager.execute("nodeRepl.write('first')", {
        requestId: "unstoppable-execution-failure",
        requestMeta: null,
      }),
    ),
    FINAL_EXIT_ERROR,
  );
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
  assert.equal(state.active, null);

  await assert.rejects(
    settlesWithin(
      manager.execute("nodeRepl.write('replacement')", {
        requestId: "blocked-after-execution-failure",
        requestMeta: null,
      }),
    ),
    FINAL_EXIT_ERROR,
  );
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
  await assert.rejects(settlesWithin(manager.close()), FINAL_EXIT_ERROR);
});

test("runtime close propagates teardown failure within the final deadline", async () => {
  const { child, kernel } = createUnstoppableKernel();
  const manager = new RuntimeManager({
    allowHostNode: true,
    nodePath: TEST_NODE_PATH,
    runtimeMetadata: null,
  });
  (manager as unknown as RuntimeManagerState).child = kernel;

  await assert.rejects(settlesWithin(manager.close()), FINAL_EXIT_ERROR);
  assert.deepEqual(child.signals, ["SIGTERM", "SIGKILL"]);
  assert.equal(child.listenerCount("exit"), 0);
});
