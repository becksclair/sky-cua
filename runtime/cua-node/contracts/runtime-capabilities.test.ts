import { readFileSync } from "node:fs";
import { join } from "node:path";
import { strict as assert } from "node:assert";
import { test } from "bun:test";
import { parse } from "acorn";
import { KERNEL_SOURCE } from "../src/kernel/kernel.ts";
import { RUNTIME_CAPABILITY_CONTRACT } from "../src/kernel/kernel-capabilities.ts";

type CapabilityContract = {
  schema_version: number;
  node_version: string;
  realms: string[];
  node_global_descriptors: Record<string, string[]>;
  canvas_lazy_globals: string[];
  deliberately_absent: string[];
  process_facades: {
    model: null;
    package: {
      identity: string[];
      operations: string[];
      forbidden: string[];
      stdio: {
        kind: string;
        fields: string[];
        forbidden: string[];
      };
      commonjs_compatibility: {
        facade: string;
        reason: string;
        host_process_identity: boolean;
        raw_stdio: boolean;
        forwarded_event_methods: string[];
      };
    };
  };
  loader_promotion: {
    fixed_names: string[];
    general_package_loader: boolean;
    required_properties: string[];
  };
};

const contract = JSON.parse(
  readFileSync(join(import.meta.dir, "runtime-capabilities.contract.json"), "utf8"),
) as CapabilityContract;

function sorted(values: readonly string[]): string[] {
  return [...values].sort((left, right) => left.localeCompare(right));
}

test("runtime capability contract freezes the Node and VM realm boundary", () => {
  assert.equal(contract.schema_version, 1);
  assert.equal(contract.node_version, "24.14.0");
  assert.deepEqual(contract.realms, ["model", "package", "trusted-browser"]);
  assert.equal(contract.process_facades.model, null);

  const globalGroups = Object.values(contract.node_global_descriptors).flat();
  assert.equal(new Set(globalGroups).size, globalGroups.length);
  for (const required of [
    "atob",
    "btoa",
    "fetch",
    "WebSocket",
    "URLPattern",
    "navigator",
    "PerformanceObserver",
    "CryptoKey",
  ])
    assert.equal(globalGroups.includes(required), true, required);

  assert.deepEqual(contract.canvas_lazy_globals, [
    "DOMMatrix",
    "DOMPoint",
    "DOMRect",
    "Image",
    "ImageData",
    "Path2D",
  ]);
  assert.equal(contract.deliberately_absent.includes("window"), true);
  assert.equal(contract.deliberately_absent.includes("OffscreenCanvas"), true);
});

test("runtime capability contract has no process or loader escape hatch", () => {
  assert.deepEqual(
    sorted(contract.process_facades.package.forbidden),
    sorted(["abort", "chdir", "exit", "kill", "send"]),
  );
  assert.equal(
    contract.process_facades.package.commonjs_compatibility.facade,
    "safe-shallow-mutable-clone",
  );
  assert.equal(
    contract.process_facades.package.commonjs_compatibility.host_process_identity,
    false,
  );
  assert.equal(
    contract.process_facades.package.commonjs_compatibility.raw_stdio,
    false,
  );
  assert.deepEqual(
    contract.process_facades.package.commonjs_compatibility.forwarded_event_methods,
    ["addListener", "off", "on", "once", "removeListener"],
  );
  assert.equal(contract.process_facades.package.stdio.kind, "frozen-metadata-only");
  assert.deepEqual(contract.process_facades.package.stdio.fields, ["fd", "isTTY"]);
  assert.deepEqual(
    sorted(contract.process_facades.package.stdio.forbidden),
    sorted(["destroy", "pipe", "read", "write"]),
  );
  assert.equal(
    contract.process_facades.package.operations.includes("getBuiltinModule"),
    true,
  );
  assert.equal(contract.loader_promotion.general_package_loader, false);
  assert.deepEqual(
    sorted(contract.loader_promotion.fixed_names),
    sorted([
      "acorn",
      "acornWalk",
      "canvas",
      "pdfjs",
      "pixelmatch",
      "playwright",
      "sharp",
      "tesseract",
    ]),
  );
  assert.equal(
    contract.loader_promotion.required_properties.includes("no-eager-native-load"),
    true,
  );
});

test("production kernel projects the canonical contract and composes parseable source", () => {
  assert.deepEqual(RUNTIME_CAPABILITY_CONTRACT, contract);
  for (const name of [
    ...Object.values(contract.node_global_descriptors).flat(),
    ...contract.canvas_lazy_globals,
    ...contract.loader_promotion.fixed_names,
    ...contract.process_facades.package.forbidden,
  ]) {
    assert.equal(KERNEL_SOURCE.includes(JSON.stringify(name)), true, name);
  }
  assert.doesNotThrow(() =>
    parse(KERNEL_SOURCE, { ecmaVersion: "latest", sourceType: "module" }),
  );
});
