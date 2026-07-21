# `cua_node` v1 contract

This directory freezes the Linux x86-64 glibc runtime seam used by the codex-desktop packaging lane. The contract is intentionally independent of the production assembler and launcher: Lane P can point its tests at `test/fixtures/fake-runtime/` before a real Node 24 assembly exists.

The machine-readable contracts are:

- [`runtime-manifest.schema.json`](runtime-manifest.schema.json): the installed `manifest.json` shape, version gates, required paths, component identities, and SHA-256 records.
- [`runtime-environment.contract.json`](runtime-environment.contract.json): environment selection and explicit legacy rollback behavior.
- [`computer-use-wrapper.contract.json`](computer-use-wrapper.contract.json): the AST-only migration target for the generated Computer Use wrapper.

## Installed runtime layout

The runtime root is `<resources>/cua_node`. The manifest is at the root and the following paths are fixed; these are not launcher guesses or platform-dependent aliases.

```text
cua_node/
  manifest.json
  bin/
    node
    node_repl
  lib/
    node_modules/
      @heliasar/sky-cua/
  share/
    playwright/
    tessdata/
    pdfjs/
  licenses/
  sbom.cdx.json
```

When supplied as an override, `NODE_REPL_NODE_MODULE_DIRS` contains direct module roots, not runtime roots. Normal installed startup does not require it: `bin/node_repl` executes its sibling `bin/node`, and the host derives `lib/node_modules` plus trusted Browser hashes from the adjacent validated manifest. The `@heliasar/sky-cua` package is therefore at `lib/node_modules/@heliasar/sky-cua`, and its public entrypoint is the package root. `share/playwright` contains the locked browser revision; `share/tessdata` contains locked OCR language data; and `share/pdfjs` contains local worker, standard-font, and CMap assets. Empty directories are still part of the fixture layout and must not be replaced with task-time downloads.

The manifest records `node_version: 24.14.0`, exact hashes for `bin/node` and `bin/node_repl`, the checksummed `@heliasar/sky-cua@0.1.0` tarball, browser revision, source commits, lock hashes, and checksums for shipped runtime files. The manifest itself is written last by assembly and is not included in its own `checksums.files` list; otherwise the manifest would be self-referential.

## Environment selection

Normal v1 selection is fail-closed and uses one manifest-compatible runtime root:

1. `CODEX_NODE_REPL_LEGACY_FALLBACK=1` selects explicit diagnostic legacy mode. In that mode, preserve an executable explicit `CODEX_NODE_REPL_PATH`, then use the existing legacy resolver in this order: `<resources>/node_repl`, `$XDG_CACHE_HOME/codex-runtimes/codex-primary-runtime/dependencies/bin/node_repl`, then `PATH` lookup for `node_repl`. Do not derive v1 paths in this mode.
2. Without the flag, a non-empty explicit v1 value is accepted only after validating its kind, adjacent v1 manifest, target/version, and exact manifest checksum. Invalid explicit values are not silently accepted.
3. Missing v1 values are derived from the bundled manifest: `bin/node_repl`, `bin/node`, `lib/node_modules`, the manifest's trusted digest list joined with commas, and `share/playwright`. After the native launcher executes `bin/node`, the host independently derives the runtime root from `process.execPath`; no selector variables need to be exported into the process.
4. If the bundled manifest exists but is corrupt, incompatible, or checksum-invalid, fail with an actionable error. Do not fall through to host Node, global modules, `PATH`, or the legacy executable. The only rollback is the explicit flag above.

These optional overrides and their exact separators are:

| Variable | Bundled value | Explicit value rule |
| --- | --- | --- |
| `CODEX_NODE_REPL_PATH` | `<root>/bin/node_repl` | Executable in a compatible v1 runtime root |
| `NODE_REPL_NODE_PATH` | `<root>/bin/node` | Executable in the same compatible v1 runtime root; `CODEX_BROWSER_USE_NODE_PATH` is the legacy alias only when this is unset |
| `NODE_REPL_NODE_MODULE_DIRS` | `<root>/lib/node_modules` | `:`-separated readable direct module roots, each owned by a compatible v1 manifest |
| `NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S` | Manifest digests joined with `,` | Lowercase 64-hex values, no empty entries, and every manifest digest must remain present |
| `PLAYWRIGHT_BROWSERS_PATH` | `<root>/share/playwright` | Readable directory containing the manifest's locked browser revision |

The selected `NODE_REPL_NODE_PATH` is also the value used to synchronize the legacy `CODEX_BROWSER_USE_NODE_PATH` path when that alias supplied the valid override. The resolver never creates a Playwright cache and never enables browser downloads.

## Computer Use wrapper migration

The generated source at `resources/upstream/plugins/openai-bundled/plugins/computer-use/scripts/computer-use-client.mjs` is patched by AST-derived ranges. The current stable locator is the `SKY_MAC_CLIENT_ENTRYPOINT` array, the `importPackagedCreateClient` declaration, and its call inside `setupComputerUseRuntime`; an ambiguous or missing seam is a patch error.

The v1 replacement performs the equivalent lazy named import:

```js
const { sky } = await import("@heliasar/sky-cua");
```

This is a dynamic import so the wrapper stays lazy; it is still the public named `sky` export, not a private deep path. The Node REPL resolver uses `NODE_REPL_NODE_MODULE_DIRS` to resolve the package. The wrapper publishes the exact exported value, without freezing, cloning, target arguments, or invoking a constructor, to all of these locations:

- `Symbol.for("openai.computer-use.runtime")`
- `globalThis.sky`
- `options.globals.sky` (or the default `globalThis` object)

If the symbol already contains a runtime, reuse that exact identity and republish it to the two property locations. The patch removes `@oai/sky`, `create_client`, the macOS `targets/mac/create_client.js` path, and the retired `import.meta.__codexNativePipe` dependency. It carries the sentinel `codex-cua-node-computer-use-wrapper-v1`, returns `already` on a second apply, and fails closed on source drift.

## Migration and rollback invariants

- The new runtime is selected only when its v1 manifest, target, paths, permissions, and checksums validate.
- A present-but-invalid v1 manifest never triggers an implicit legacy switch.
- Before Milestone 8, rollback is the explicit `CODEX_NODE_REPL_LEGACY_FALLBACK=1` mode. After Milestone 8, rollback is to the prior known-good application package, not a second indefinitely shipped runtime.
- Assembly is written to a temporary sibling, verified, then atomically replaced; an interrupted or failed build leaves the previous verified runtime untouched.
- The Computer Use patch is AST-only, idempotent, and reversible by restoring its recorded AST ranges. It does not edit generated bundle output directly.
- Runtime selection does not start or restart sky-cua, the Computer Use MCP server, or a browser daemon. The JavaScript package connects only to the already-running service.
