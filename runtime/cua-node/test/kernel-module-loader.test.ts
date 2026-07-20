import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "bun:test";
import { parse } from "acorn";
import {
  KERNEL_MODULE_LOADING_SOURCE,
  KERNEL_MODULE_RESOLUTION_SOURCE,
} from "../src/kernel/kernel-module-loader.ts";
import { KERNEL_SOURCE } from "../src/kernel/kernel.ts";

const kernelOwner = readFileSync(
  join(import.meta.dir, "../src/kernel/kernel.ts"),
  "utf8",
);
const loaderOwner = readFileSync(
  join(import.meta.dir, "../src/kernel/kernel-module-loader.ts"),
  "utf8",
);

function occurrences(source: string, marker: string): number {
  return source.split(marker).length - 1;
}

test("module resolution and loading have one canonical source owner", () => {
  for (const marker of [
    "function resolveSpecifier(",
    "async function loadModule(",
    "function loadCommonJsValue(",
    "function createCommonJsRequire(",
  ]) {
    assert.equal(kernelOwner.includes(marker), false, marker);
    assert.equal(occurrences(loaderOwner, marker), 1, marker);
    assert.equal(occurrences(KERNEL_SOURCE, marker), 1, marker);
  }
  assert.match(kernelOwner, /\$\{KERNEL_MODULE_RESOLUTION_SOURCE\}/u);
  assert.match(kernelOwner, /\$\{KERNEL_MODULE_LOADING_SOURCE\}/u);
  assert.ok(kernelOwner.split("\n").length <= 650);
});

test("module-loader fragments compose as parseable kernel source", () => {
  assert.doesNotThrow(() =>
    parse(KERNEL_MODULE_RESOLUTION_SOURCE, {
      ecmaVersion: "latest",
      sourceType: "script",
    }),
  );
  assert.doesNotThrow(() =>
    parse(KERNEL_MODULE_LOADING_SOURCE, {
      ecmaVersion: "latest",
      sourceType: "script",
    }),
  );
  assert.doesNotThrow(() =>
    parse(KERNEL_SOURCE, { ecmaVersion: "latest", sourceType: "module" }),
  );
});
