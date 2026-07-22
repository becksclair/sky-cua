import { strict as assert } from "node:assert";
import { posix, win32 } from "node:path";
import { test } from "bun:test";
import {
  createTrustedModulePolicyCore,
  openedTrustedFileRealPathCore,
  parseTrustedCodePathsCore,
  readTrustedCodeFileCore,
  renderTrustedModulePolicyCoreSource,
  resolveTrustedCodeRootsCore,
  trustedBytesEqualCore,
  trustedPathContainsCore,
  type TrustedModulePolicyCore,
  type TrustedModulePolicyCoreOptions,
} from "../src/host/trusted-module-policy-core.ts";
import { TRUSTED_MODULE_POLICY_KERNEL_SOURCE } from "../src/host/trusted-module-policy.ts";
import {
  DESCRIPTOR_REALPATH_VECTORS,
  POLICY_PARITY_VECTORS,
  ROOT_RESOLUTION_VECTORS,
  TRUSTED_FILE_ACQUISITION_VECTORS,
  type PolicyParityVector,
} from "./trusted-module-policy-vectors.ts";

interface EmbeddedPolicyCoreExports {
  createTrustedModulePolicyCore(
    options: TrustedModulePolicyCoreOptions,
  ): TrustedModulePolicyCore;
  openedTrustedFileRealPathCore: typeof openedTrustedFileRealPathCore;
  parseTrustedCodePathsCore: typeof parseTrustedCodePathsCore;
  readTrustedCodeFileCore: typeof readTrustedCodeFileCore;
  resolveTrustedCodeRootsCore: typeof resolveTrustedCodeRootsCore;
  trustedBytesEqualCore: typeof trustedBytesEqualCore;
  trustedPathContainsCore: typeof trustedPathContainsCore;
}

function loadEmbeddedPolicyCore(): EmbeddedPolicyCoreExports {
  const source = renderTrustedModulePolicyCoreSource();
  const exportExpression = String.raw`
return {
  createTrustedModulePolicyCore,
  openedTrustedFileRealPathCore,
  parseTrustedCodePathsCore,
  readTrustedCodeFileCore,
  resolveTrustedCodeRootsCore,
  trustedBytesEqualCore,
  trustedPathContainsCore,
};`;
  const factory = new Function(
    `${source}\n${exportExpression}`,
  ) as () => EmbeddedPolicyCoreExports;
  return factory();
}

function runPolicyVector(
  createCore: EmbeddedPolicyCoreExports["createTrustedModulePolicyCore"],
  bytesEqual: EmbeddedPolicyCoreExports["trustedBytesEqualCore"],
  vector: PolicyParityVector,
): readonly boolean[] {
  const files = new Map<string, Uint8Array>();
  for (const [path, bytes] of Object.entries(vector.initialFiles ?? {})) {
    files.set(path, Uint8Array.from(bytes));
  }
  const core = createCore({
    trustAllCode: vector.trustAllCode ?? false,
    bytesEqual,
    readTrustedCodeFile: (path) => files.get(path) ?? null,
    sha256Bytes: (bytes) => `hash:${[...bytes].join(",")}`,
  });
  const results: boolean[] = [];
  for (const action of vector.actions) {
    if (action.kind === "set-file") {
      if (action.bytes === null) files.delete(action.path);
      else files.set(action.path, Uint8Array.from(action.bytes ?? []));
      continue;
    }
    const result =
      action.kind === "probe"
        ? core.isTrustedDirectoryPath(action.path)
        : core.evaluate(
            action.path,
            Uint8Array.from(action.bytes ?? []),
            action.inheritedTrust ?? false,
          ).trusted;
    assert.equal(result, action.expected, `${vector.name}: ${action.kind}`);
    results.push(result);
  }
  return results;
}

test("rendered core is the exact implementation embedded in the kernel policy source", () => {
  const rendered = renderTrustedModulePolicyCoreSource();
  assert.equal(TRUSTED_MODULE_POLICY_KERNEL_SOURCE.includes(rendered), true);
  assert.equal(
    TRUSTED_MODULE_POLICY_KERNEL_SOURCE.includes("createHash('sha256')"),
    false,
    "the embedded policy must not hash every loaded module after digest trust was retired",
  );
  for (const name of [
    "parseTrustedCodePathsCore",
    "trustedPathContainsCore",
    "trustedBytesEqualCore",
    "openedTrustedFileRealPathCore",
    "resolveTrustedCodeRootsCore",
    "readTrustedCodeFileCore",
    "createTrustedModulePolicyCore",
  ]) {
    assert.equal(
      rendered.includes(`const ${name} = function`),
      true,
      `${name} has a stable embedded binding`,
    );
  }
  assert.doesNotThrow(() => loadEmbeddedPolicyCore());
});

test("filesystem trust acquisition vectors match typed and embedded implementations", () => {
  const embedded = loadEmbeddedPolicyCore();
  for (const implementation of [
    {
      name: "typed",
      openedRealPath: openedTrustedFileRealPathCore,
      resolveRoots: resolveTrustedCodeRootsCore,
      readFile: readTrustedCodeFileCore,
    },
    {
      name: "embedded",
      openedRealPath: embedded.openedTrustedFileRealPathCore,
      resolveRoots: embedded.resolveTrustedCodeRootsCore,
      readFile: embedded.readTrustedCodeFileCore,
    },
  ]) {
    for (const vector of DESCRIPTOR_REALPATH_VECTORS) {
      const calls: string[] = [];
      const result = implementation.openedRealPath(
        vector.fd,
        vector.originalPath,
        vector.platform,
        (path) => {
          calls.push(path);
          const resolved = vector.realpaths[path];
          if (resolved === undefined) throw new Error("missing fixture path");
          return resolved;
        },
      );
      assert.equal(
        result,
        vector.expected,
        `${vector.name}: ${implementation.name}`,
      );
      assert.deepEqual(
        calls,
        vector.expectedCalls,
        `${vector.name} calls: ${implementation.name}`,
      );
    }

    for (const vector of ROOT_RESOLUTION_VECTORS) {
      const directories = new Set(vector.directories);
      const roots = implementation.resolveRoots(
        vector.paths,
        (path) => {
          const resolved = vector.realpaths[path];
          if (resolved === undefined) throw new Error("missing fixture root");
          return resolved;
        },
        (path) => directories.has(path),
      );
      assert.deepEqual(
        roots,
        vector.expected,
        `${vector.name}: ${implementation.name}`,
      );
    }

    for (const vector of TRUSTED_FILE_ACQUISITION_VECTORS) {
      const descriptors = Object.keys(vector.descriptorPaths).map(Number);
      const fileDescriptors = new Set(vector.fileDescriptors);
      const closed: number[] = [];
      const bytes = implementation.readFile(
        "/requested/module.mjs",
        vector.roots,
        () => {
          const fd = descriptors.shift();
          if (fd === undefined) throw new Error("missing fixture descriptor");
          return fd;
        },
        (fd) => fileDescriptors.has(fd),
        (fd) => vector.descriptorPaths[String(fd)] ?? null,
        (file, root) =>
          trustedPathContainsCore(
            file,
            root,
            "linux",
            posix.relative,
            posix.isAbsolute,
          ),
        (fd) => Uint8Array.from(vector.bytes[String(fd)] ?? []),
        (fd) => closed.push(fd),
      );
      assert.deepEqual(
        resultBytes(bytes),
        vector.expected,
        `${vector.name}: ${implementation.name}`,
      );
      assert.deepEqual(
        closed,
        vector.expectedClosed,
        `${vector.name} closes: ${implementation.name}`,
      );
    }
  }
});

test("path parser and containment vectors match typed and embedded implementations", () => {
  const embedded = loadEmbeddedPolicyCore();
  const pathVectors = [
    {
      name: "posix paths",
      args: [" /alpha:/alpha:/beta:relative ", "/cwd", "linux", posix] as const,
      expected: ["/alpha", "/beta"],
    },
    {
      name: "windows paths",
      args: [
        " C:\\alpha;C:\\alpha;D:\\beta;relative ",
        "C:\\cwd",
        "win32",
        win32,
      ] as const,
      expected: ["C:\\alpha", "D:\\beta"],
    },
  ];
  for (const vector of pathVectors) {
    const [value, cwd, platform, pathApi] = vector.args;
    const args = [
      value,
      cwd,
      platform,
      pathApi.isAbsolute,
      pathApi.resolve,
    ] as const;
    assert.deepEqual(
      parseTrustedCodePathsCore(...args),
      vector.expected,
      `${vector.name}: typed`,
    );
    assert.deepEqual(
      embedded.parseTrustedCodePathsCore(...args),
      vector.expected,
      `${vector.name}: embedded`,
    );
  }

  const containmentVectors = [
    ["/root", "/root", "linux", posix, true],
    ["/root/child", "/root", "linux", posix, true],
    ["/root-sibling", "/root", "linux", posix, false],
    ["/outside", "/root", "linux", posix, false],
    ["C:\\root\\child", "C:\\root", "win32", win32, true],
    ["C:\\root-other", "C:\\root", "win32", win32, false],
  ] as const;
  for (const [file, root, platform, pathApi, expected] of containmentVectors) {
    const args = [
      file,
      root,
      platform,
      pathApi.relative,
      pathApi.isAbsolute,
    ] as const;
    assert.equal(trustedPathContainsCore(...args), expected, `${file}: typed`);
    assert.equal(
      embedded.trustedPathContainsCore(...args),
      expected,
      `${file}: embedded`,
    );
  }
});

test("policy decision vectors match typed and embedded implementations", () => {
  const embedded = loadEmbeddedPolicyCore();
  for (const vector of POLICY_PARITY_VECTORS) {
    const typed = runPolicyVector(
      createTrustedModulePolicyCore,
      trustedBytesEqualCore,
      vector,
    );
    const generated = runPolicyVector(
      embedded.createTrustedModulePolicyCore,
      embedded.trustedBytesEqualCore,
      vector,
    );
    assert.deepEqual(generated, typed, vector.name);
  }
});

function resultBytes(
  bytes: Readonly<Uint8Array> | null,
): readonly number[] | null {
  return bytes === null ? null : [...bytes];
}
