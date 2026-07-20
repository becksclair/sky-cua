# Fake `cua_node` runtime

This is a small executable fixture for launcher, packaging, and Computer Use wrapper tests. It mirrors the frozen installed layout without shipping Node, browser binaries, OCR data, or native addons.

The fixture contract is intentionally deterministic:

- `bin/node --version` reports the contract version `v24.14.0` and otherwise delegates JavaScript execution to the host Node selected by `FAKE_RUNTIME_SYSTEM_NODE` (default `/usr/bin/node`).
- `bin/node_repl --version` reports `node_repl-fake/1.0.0`.
- `bin/node_repl --print-env` prints only the five v1 runtime variables as JSON.
- `bin/node_repl --smoke` validates the five environment values and imports `@heliasar/sky-cua` from `NODE_REPL_NODE_MODULE_DIRS` without network access.
- With no flag, `bin/node_repl` accepts one JSON object per line and returns one JSON object per line. It implements the minimum `initialize`, `tools/list`, `tools/call`, and `shutdown` responses needed for launcher tests; `js` returns a deterministic fixture result and `js_reset` acknowledges reset. The third tool is named `js_add_node_module_dir`, matching the build-5307 contract.

The fake package is pure ESM and exposes exactly the named root export `sky`. It is a lazy `Proxy`: importing the package performs no socket, process, or filesystem I/O; accessing an operation records a deterministic call in memory. This is enough to prove that the generated wrapper imports the public package and preserves its identity without requiring the sky-cua service.

Run the fixture smoke directly from the repository root:

```sh
runtime/cua-node/test/fixtures/fake-runtime/bin/node --version
runtime/cua-node/test/fixtures/fake-runtime/bin/node_repl --version
NODE_REPL_NODE_PATH="$PWD/runtime/cua-node/test/fixtures/fake-runtime/bin/node" \
NODE_REPL_NODE_MODULE_DIRS="$PWD/runtime/cua-node/test/fixtures/fake-runtime/lib/node_modules" \
NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S="$(printf '%064d' 0)" \
PLAYWRIGHT_BROWSERS_PATH="$PWD/runtime/cua-node/test/fixtures/fake-runtime/share/playwright" \
CODEX_NODE_REPL_PATH="$PWD/runtime/cua-node/test/fixtures/fake-runtime/bin/node_repl" \
runtime/cua-node/test/fixtures/fake-runtime/bin/node_repl --smoke
```

The manifest is checked by `runtime-manifest.test.ts`, including each recorded fixture checksum and executable mode. The fixture is not a production fallback and must not be copied into packaged resources.
