import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { readFileSync } from "node:fs";
import { createInterface, type Interface } from "node:readline";

type JsonObject = Record<string, unknown>;
type McpResponse = { id?: unknown; result?: unknown; error?: unknown };

export type CleanupStep = {
  label: string;
  run(): void | Promise<void>;
};

export async function withTimeout<T>(
  label: string,
  timeoutMs: number,
  operation: Promise<T>,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      operation,
      new Promise<never>((_resolvePromise, rejectPromise) => {
        timer = setTimeout(
          () => rejectPromise(new Error(`${label} timed out after ${timeoutMs}ms`)),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

export async function runBoundedCleanup(
  steps: CleanupStep[],
  timeoutMs: number,
): Promise<string[]> {
  const errors: string[] = [];
  for (const step of steps) {
    try {
      await withTimeout(
        step.label,
        timeoutMs,
        Promise.resolve().then(() => step.run()),
      );
    } catch (error) {
      const detail = error instanceof Error ? error.message : "unknown error";
      errors.push(`${step.label}: ${detail}`);
    }
  }
  return errors;
}

function livePids(pids: number[]): number[] {
  return pids.filter((pid) => {
    try {
      process.kill(pid, 0);
      return true;
    } catch (error) {
      return !(
        error !== null &&
        typeof error === "object" &&
        "code" in error &&
        error.code === "ESRCH"
      );
    }
  });
}

function signalPid(pid: number, signal: NodeJS.Signals): void {
  try {
    process.kill(pid, signal);
  } catch (error) {
    if (
      error === null ||
      typeof error !== "object" ||
      !("code" in error) ||
      error.code !== "ESRCH"
    )
      throw error;
  }
}

async function waitForPidsToExit(pids: number[], timeoutMs: number): Promise<number[]> {
  const deadline = Date.now() + timeoutMs;
  let remaining = livePids(pids);
  while (remaining.length > 0 && Date.now() < deadline) {
    await new Promise((resolvePromise) => setTimeout(resolvePromise, 20));
    remaining = livePids(remaining);
  }
  return remaining;
}

export async function stopProcessesContaining(
  marker: string,
  timeoutMs: number,
): Promise<void> {
  const pids = processPidsContaining(marker);
  for (const pid of pids) signalPid(pid, "SIGTERM");
  const afterTerm = await waitForPidsToExit(pids, timeoutMs);
  for (const pid of afterTerm) signalPid(pid, "SIGKILL");
  const afterKill = await waitForPidsToExit(afterTerm, timeoutMs);
  if (afterKill.length > 0)
    throw new Error(`marked processes survived SIGKILL: ${afterKill.join(",")}`);
}

export type McpSession = {
  child: ChildProcessWithoutNullStreams;
  request(method: string, params: JsonObject, timeoutMs?: number): Promise<McpResponse>;
  requestWithId(
    method: string,
    params: JsonObject,
    timeoutMs?: number,
  ): {
    id: number;
    response: Promise<McpResponse>;
  };
  notify(method: string, params: JsonObject): void;
  close(
    timeoutMs?: number,
  ): Promise<{ code: number | null; signal: NodeJS.Signals | null }>;
  stderr(): string;
};

export type StoppableChild = Pick<
  ChildProcessWithoutNullStreams,
  "exitCode" | "signalCode" | "kill" | "once" | "removeListener"
> & {
  pid?: number;
  stdin: { end(): void };
};

export type ChildExit = {
  code: number | null;
  signal: NodeJS.Signals | null;
};

function exitOf(child: StoppableChild): ChildExit | null {
  return child.exitCode === null && child.signalCode === null
    ? null
    : { code: child.exitCode, signal: child.signalCode };
}

function waitForExit(child: StoppableChild, timeoutMs: number) {
  const current = exitOf(child);
  if (current !== null) return Promise.resolve(current);
  return new Promise<ReturnType<typeof exitOf>>((resolvePromise) => {
    const timer = setTimeout(() => finish(exitOf(child)), timeoutMs);
    const onExit = (code: number | null, signal: NodeJS.Signals | null) =>
      finish({ code, signal });
    const finish = (exit: ReturnType<typeof exitOf>) => {
      clearTimeout(timer);
      child.removeListener("exit", onExit);
      resolvePromise(exit);
    };
    child.once("exit", onExit);
    const racedExit = exitOf(child);
    if (racedExit !== null) finish(racedExit);
  });
}

export async function stopChildProcess(
  child: StoppableChild,
  timeoutMs: number,
  label = "MCP host",
): Promise<ChildExit> {
  const recordedPids = new Set<number>();
  const rememberProcessTree = (): void => {
    if (child.pid === undefined) return;
    for (const pid of processTreePids(child.pid)) recordedPids.add(pid);
  };
  rememberProcessTree();
  try {
    child.stdin.end();
  } catch {
    // Exact-PID escalation below remains authoritative.
  }
  let exit = await waitForExit(child, timeoutMs);
  if (exit === null) {
    rememberProcessTree();
    child.kill("SIGTERM");
    exit = await waitForExit(child, timeoutMs);
  }
  if (exit === null) {
    rememberProcessTree();
    child.kill("SIGKILL");
    exit = await waitForExit(child, timeoutMs);
  }
  const descendants = [...recordedPids].filter((pid) => pid !== child.pid);
  let survivingDescendants = livePids(descendants);
  for (const pid of survivingDescendants) signalPid(pid, "SIGTERM");
  survivingDescendants = await waitForPidsToExit(survivingDescendants, timeoutMs);
  for (const pid of survivingDescendants) signalPid(pid, "SIGKILL");
  survivingDescendants = await waitForPidsToExit(survivingDescendants, timeoutMs);
  const failures: Error[] = [];
  if (exit === null)
    failures.push(new Error(`${label} ${child.pid ?? "unknown"} survived SIGKILL`));
  if (survivingDescendants.length > 0)
    failures.push(
      new Error(
        `${label} descendants survived SIGKILL: ${survivingDescendants.join(",")}`,
      ),
    );
  if (failures.length === 1) throw failures[0];
  if (failures.length > 1)
    throw new AggregateError(failures, `${label} process-tree cleanup failed`);
  if (exit === null) throw new Error(`${label} exit state is unavailable`);
  return exit;
}

export function startMcpSession(options: {
  executable: string;
  cwd: string;
  env: NodeJS.ProcessEnv;
  timeoutMs: number;
}): McpSession {
  const child = spawn(options.executable, [], {
    cwd: options.cwd,
    env: options.env,
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk: string) => (stderr += chunk));
  const pending = new Map<number, (response: McpResponse) => void>();
  const reader: Interface = createInterface({
    input: child.stdout,
    crlfDelay: Infinity,
  });
  reader.on("line", (line) => {
    try {
      const value: unknown = JSON.parse(line);
      if (value !== null && typeof value === "object" && !Array.isArray(value)) {
        const response = value as McpResponse;
        if (typeof response.id === "number") pending.get(response.id)?.(response);
      }
    } catch {
      // Non-JSON output cannot satisfy a pending MCP request.
    }
  });
  let nextId = 1;
  const requestWithId = (
    method: string,
    params: JsonObject,
    timeoutMs = options.timeoutMs,
  ) => {
    const id = nextId++;
    const response = new Promise<McpResponse>((resolvePromise, rejectPromise) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        rejectPromise(new Error(`timed out waiting for ${method}`));
      }, timeoutMs);
      pending.set(id, (response) => {
        clearTimeout(timer);
        pending.delete(id);
        resolvePromise(response);
      });
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
    return { id, response };
  };
  return {
    child,
    request: (method, params, timeoutMs) =>
      requestWithId(method, params, timeoutMs).response,
    requestWithId,
    notify(method, params) {
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
    },
    async close(timeoutMs = options.timeoutMs) {
      try {
        return await stopChildProcess(child, timeoutMs);
      } finally {
        reader.close();
      }
    },
    stderr: () => stderr,
  };
}

export function toolText(response: McpResponse, label: string): string {
  if (response.error !== undefined)
    throw new Error(`${label}: ${JSON.stringify(response.error)}`);
  const result = response.result;
  if (result === null || typeof result !== "object" || Array.isArray(result))
    throw new Error(`${label}: missing tool result`);
  const record = result as JsonObject;
  if (record.isError === true)
    throw new Error(`${label}: ${JSON.stringify(record.content)}`);
  const content = Array.isArray(record.content) ? record.content : [];
  const text = content.find(
    (item) =>
      item !== null &&
      typeof item === "object" &&
      !Array.isArray(item) &&
      (item as JsonObject).type === "text" &&
      typeof (item as JsonObject).text === "string",
  ) as JsonObject | undefined;
  if (text === undefined) return "";
  return text.text as string;
}

export function processTreePids(rootPid: number): number[] {
  const pids = new Set([rootPid]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const entry of require("node:fs").readdirSync("/proc", {
      withFileTypes: true,
    })) {
      if (!entry.isDirectory() || !/^\d+$/u.test(entry.name)) continue;
      const pid = Number(entry.name);
      if (pids.has(pid)) continue;
      try {
        const status = readFileSync(`/proc/${pid}/status`, "utf8");
        const match = /^PPid:\s+(\d+)$/mu.exec(status);
        if (match !== null && pids.has(Number(match[1]))) {
          pids.add(pid);
          changed = true;
        }
      } catch {}
    }
  }
  return [...pids].sort((a, b) => a - b);
}

export function processTreeRssBytes(rootPid: number): number {
  return processTreePids(rootPid).reduce((total, pid) => {
    try {
      const match = /^VmRSS:\s+(\d+)\s+kB$/mu.exec(
        readFileSync(`/proc/${pid}/status`, "utf8"),
      );
      return total + (match === null ? 0 : Number(match[1]) * 1024);
    } catch {
      return total;
    }
  }, 0);
}

export function processPidsContaining(marker: string): number[] {
  const matches: number[] = [];
  for (const entry of require("node:fs").readdirSync("/proc", {
    withFileTypes: true,
  })) {
    if (!entry.isDirectory() || !/^\d+$/u.test(entry.name)) continue;
    try {
      if (readFileSync(`/proc/${entry.name}/cmdline`, "utf8").includes(marker))
        matches.push(Number(entry.name));
    } catch {}
  }
  return matches.sort((a, b) => a - b);
}
