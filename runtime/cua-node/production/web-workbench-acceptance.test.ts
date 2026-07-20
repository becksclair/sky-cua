import { strict as assert } from "node:assert";
import { spawn, spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { test } from "bun:test";
import {
  parseAcceptanceArgs,
  startLoopbackFixture,
  validatePlaywrightEvidence,
} from "./web-workbench-acceptance";
import {
  processPidsContaining,
  runBoundedCleanup,
  stopProcessesContaining,
} from "./web-workbench-acceptance-helper";

const script = resolve(__dirname, "web-workbench-acceptance.ts");
const fixtureRuntime = resolve(__dirname, "../test/fixtures/fake-runtime");

test("argument parsing requires a runtime and bounds lifecycle cycles", () => {
  assert.throws(() => parseAcceptanceArgs([]), /runtime-root/u);
  assert.throws(
    () => parseAcceptanceArgs([`--runtime-root=${fixtureRuntime}`, "--cycles=0"]),
    /cycles must be 1 through 10/u,
  );
  const options = parseAcceptanceArgs([
    `--runtime-root=${fixtureRuntime}`,
    "--browser-executable=/bin/true",
    "--cycles=3",
  ]);
  assert.equal(options.cycles, 3);
  assert.equal(options.browserExecutable, "/bin/true");
});

test("loopback fixture deterministically serves fetch, upload, stream, compression, and abort", async () => {
  const fixture = await startLoopbackFixture();
  try {
    assert.equal(await (await fetch(`${fixture.origin}/text`)).text(), "loopback text");
    assert.deepEqual(await (await fetch(`${fixture.origin}/json`)).json(), {
      ok: true,
      source: "loopback",
    });
    assert.deepEqual(
      [...new Uint8Array(await (await fetch(`${fixture.origin}/bytes`)).arrayBuffer())],
      [0, 1, 2, 253, 254, 255],
    );
    assert.equal(await (await fetch(`${fixture.origin}/stream`)).text(), "one-two");
    assert.equal(
      await (await fetch(`${fixture.origin}/gzip`)).text(),
      "compressed loopback",
    );
    const upload = (await (
      await fetch(`${fixture.origin}/upload`, {
        method: "POST",
        body: "UPLOAD MARKER",
      })
    ).json()) as { has_marker: boolean };
    assert.equal(upload.has_marker, true);
    const controller = new AbortController();
    const delayed = fetch(`${fixture.origin}/delay`, {
      signal: controller.signal,
    });
    controller.abort();
    await assert.rejects(delayed, /abort/iu);
  } finally {
    await fixture.close();
  }
});

test("loopback fixture echoes WebSocket text and binary before a clean close", async () => {
  const fixture = await startLoopbackFixture();
  try {
    const url = new URL(fixture.wsUrl);
    const clientFrame = (payload: Buffer, opcode: number): Buffer => {
      const mask = Buffer.from([1, 2, 3, 4]);
      const masked = Buffer.from(payload);
      for (let index = 0; index < masked.length; index += 1)
        masked[index] ^= mask[index % mask.length]!;
      return Buffer.concat([
        Buffer.from([0x80 | opcode, 0x80 | payload.length]),
        mask,
        masked,
      ]);
    };
    const evidence = await new Promise<{ values: string[]; code: number }>(
      (resolvePromise, rejectPromise) => {
        const socket = connect(Number(url.port), url.hostname);
        const values: string[] = [];
        let upgraded = false;
        let pending = Buffer.alloc(0);
        const timer = setTimeout(() => {
          socket.destroy();
          rejectPromise(new Error("WebSocket fixture timed out"));
        }, 2_000);
        socket.once("connect", () =>
          socket.write(
            `GET /ws HTTP/1.1\r\nHost: ${url.host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: MDEyMzQ1Njc4OWFiY2RlZg==\r\nSec-WebSocket-Version: 13\r\n\r\n`,
          ),
        );
        socket.on("data", (chunk: Buffer) => {
          pending = Buffer.concat([pending, chunk]);
          if (!upgraded) {
            const boundary = pending.indexOf("\r\n\r\n");
            if (boundary < 0) return;
            assert.match(pending.subarray(0, boundary).toString(), /^HTTP\/1\.1 101/u);
            pending = pending.subarray(boundary + 4);
            upgraded = true;
            socket.write(clientFrame(Buffer.from("text-echo"), 1));
          }
          while (pending.length >= 2) {
            const opcode = pending[0]! & 0x0f;
            const length = pending[1]! & 0x7f;
            if (pending.length < 2 + length) return;
            const payload = pending.subarray(2, 2 + length);
            pending = pending.subarray(2 + length);
            if (opcode === 1) {
              values.push(payload.toString());
              socket.write(clientFrame(Buffer.from([7, 8, 9]), 2));
            } else if (opcode === 2) {
              values.push([...payload].join(","));
              socket.write(clientFrame(Buffer.from([3, 232]), 8));
            } else if (opcode === 8) {
              clearTimeout(timer);
              socket.end();
              resolvePromise({ values, code: payload.readUInt16BE(0) });
            }
          }
        });
        socket.once("error", rejectPromise);
      },
    );
    assert.deepEqual(evidence, {
      values: ["text-echo", "7,8,9"],
      code: 1000,
    });
  } finally {
    await fixture.close();
  }
});

test("missing nodeRepl.runtime browser metadata is a stable production blocker", () => {
  const rootsBefore = new Set(
    readdirSync(tmpdir()).filter((entry) =>
      entry.startsWith("cua-node-web-workbench-"),
    ),
  );
  const result = spawnSync(
    process.execPath,
    [script, `--runtime-root=${fixtureRuntime}`, "--timeout-ms=2000", "--json"],
    { encoding: "utf8", timeout: 10_000 },
  );
  assert.equal(result.status, 2, result.stderr);
  const report = JSON.parse(result.stdout) as {
    schema: string;
    schema_version: number;
    status: string;
    blockers: string[];
  };
  assert.equal(report.schema, "com.heliasar.cua-node.web-workbench-acceptance");
  assert.equal(report.schema_version, 1);
  assert.equal(report.status, "blocked");
  assert.deepEqual(report.blockers, ["nodeRepl.runtime.browser.executablePath"]);
  const leakedRoots = readdirSync(tmpdir()).filter(
    (entry) => entry.startsWith("cua-node-web-workbench-") && !rootsBefore.has(entry),
  );
  assert.deepEqual(leakedRoots, []);
});

test("bounded cleanup continues after rejection and timeout", async () => {
  const completed: string[] = [];
  const errors = await runBoundedCleanup(
    [
      {
        label: "rejecting browser",
        run: () => {
          completed.push("rejecting browser");
          throw new Error("close rejected");
        },
      },
      {
        label: "hanging server",
        run: () => {
          completed.push("hanging server");
          return new Promise<void>(() => undefined);
        },
      },
      {
        label: "temporary root",
        run: () => {
          completed.push("temporary root");
        },
      },
    ],
    20,
  );
  assert.deepEqual(completed, [
    "rejecting browser",
    "hanging server",
    "temporary root",
  ]);
  assert.equal(errors.length, 2);
  assert.match(errors[0]!, /rejecting browser: close rejected/u);
  assert.match(errors[1]!, /hanging server timed out after 20ms/u);
});

test("marked process cleanup terminates a leaked browser stand-in", async () => {
  const marker = `cua-node-cleanup-test-${process.pid}-${Date.now()}`;
  const child = spawn(
    process.execPath,
    ["-e", "setInterval(() => undefined, 1000)", marker],
    { stdio: "ignore" },
  );
  try {
    await new Promise<void>((resolvePromise, rejectPromise) => {
      child.once("spawn", resolvePromise);
      child.once("error", rejectPromise);
    });
    assert.deepEqual(processPidsContaining(marker), [child.pid!]);
    await stopProcessesContaining(marker, 500);
    assert.deepEqual(processPidsContaining(marker), []);
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  }
});

test("Playwright evidence rejects unobserved uploads and missing crash disconnects", () => {
  const accepted = {
    cycles: 1,
    readback: "PLAYWRIGHT!",
    upload: {
      observed: true,
      name: "upload marker.txt",
      text: "UPLOAD MARKER",
    },
    crashSignals: [{ disconnected: true, command: "rejected:Target closed" }],
  };
  assert.equal(validatePlaywrightEvidence(accepted, 1).crashSignals.length, 1);
  assert.throws(
    () =>
      validatePlaywrightEvidence(
        { ...accepted, upload: { ...accepted.upload, observed: false } },
        1,
      ),
    /upload evidence mismatch/u,
  );
  assert.throws(
    () =>
      validatePlaywrightEvidence(
        {
          ...accepted,
          crashSignals: [{ disconnected: false, command: "resolved" }],
        },
        1,
      ),
    /did not disconnect/u,
  );
});
