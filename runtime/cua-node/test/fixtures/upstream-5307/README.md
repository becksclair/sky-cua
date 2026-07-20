# Build-5307 `node_repl` contract fixtures

These fixtures freeze observable behavior recovered from the extracted ChatGPT
26.707.72221 Darwin runtime, its `SKY_API_REPORT.md`, the legacy Linux ELF,
and the checked-in Browser Use integration. They are contract evidence, not a
copy of upstream implementation code.

The fixture set is deliberately data-only. Runtime implementation lanes should
consume these files as golden inputs and must not infer new public behavior from
private host/kernel messages. Values marked `unknown` are limited to behavior
that cannot be executed or recovered from the pinned local artifacts.

Files:

- `contract.json` — index and consolidated typed contract.
- `tools-list.json` — build-5307 `tools/list` tool order, descriptions, and schemas.
- `provenance.json` — field-to-evidence mapping with exact local paths and selectors.
- `mcp-transcripts.json` — initialize, discovery, call, JS, reset, and module-dir cases.
- `kernel.json` — persistent ESM cells, module resolution, output, and process isolation.
- `node-repl-surfaces.json` — untrusted and trusted `nodeRepl` shapes.
- `output-metadata.json` — output, image, request metadata, and response metadata cases.
- `lifecycle.json` — timeout, cancellation, reset, and crash recovery cases.
- `trusted-helper.json` — exact-byte trust and privileged helper propagation.
- `native-pipe.json` — kernel bridge messages and browser socket framing/lifecycle.

Validation is intentionally dependency-free:

```sh
bun test runtime/cua-node/test/fixtures/upstream-5307/fixtures.test.ts
```

The extracted Darwin executable is ARM64 Mach-O and is not runnable on the
Linux host. Its readable strings are therefore cited as evidence; the legacy
Linux ELF transcript is separately recorded as a live probe.

The recovered upstream artifact also contains `sandbox_changed` and request
metadata examples containing sandbox fields. Those entries are retained only
as provenance evidence. Bex's production architecture runs bundled Node 24 as
a normal child; these upstream markers MUST NOT drive production launch,
kernel reset, or control-channel behavior.
