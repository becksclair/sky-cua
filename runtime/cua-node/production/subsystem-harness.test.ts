import { strict as assert } from "node:assert";
import { chmodSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "bun:test";
import { runSubsystemHarness } from "./subsystem-harness";

test("offline subsystem harness reports actionable blockers without claiming parity", () => {
  const report = runSubsystemHarness({ networkDisabled: true, emptyUserCache: true });
  assert.equal(report.network, "disabled");
  assert.equal(report.user_cache, "empty-by-contract");
  assert.equal(report.parity_claim, "none");
  assert.equal(report.status, "blocked");
  assert.ok(report.results.every((result) => result.status === "blocked"));
  assert.ok(report.results.every((result) => result.blocker !== undefined));
});

test("subsystem harness rejects network or cache-enabled execution", () => {
  const report = runSubsystemHarness({ networkDisabled: false, emptyUserCache: false });
  assert.equal(report.status, "failed");
  assert.ok(report.results.some((result) => result.blocker === "network-enabled"));
  assert.ok(report.results.some((result) => result.blocker === "user-cache-not-empty"));
});

test("system Chromium satisfies the external standalone Playwright browser proof", () => {
  const root = mkdtempSync(join(tmpdir(), "cua-browser-proof-"));
  const browser = join(root, "chromium");
  writeFileSync(browser, "#!/bin/sh\nprintf 'Chromium 150.0.0\\n'\n");
  chmodSync(browser, 0o755);
  const report = runSubsystemHarness({
    browserPath: browser,
    networkDisabled: true,
    emptyUserCache: true,
  });
  assert.equal(report.local_proofs.system_browser.status, "passed");
  assert.match(report.local_proofs.system_browser.detail, /Chromium/u);
});
