# `node_repl` output contract: upstream evidence recovery

## Context

During verification of the standalone installer's OpenCode integration, a
verification agent reported that `node_repl`'s `js` tool did not return the
value of a bare last-expression evaluation (e.g. `({ now: ..., sum: 1 + 1 })`).
The agent had to wrap calls in `nodeRepl.write(JSON.stringify(...))` to surface
values. The question was whether this was a sky-cua regression or the upstream
contract.

## Investigation

### Source: OpenAI computer-use plugin skill doc

File: `~/projects/codex-desktop/resources/upstream/plugins/openai-bundled/plugins/computer-use/.codex-plugin/computer-use-node-repl.md`

Line 12:

> For text output, use `nodeRepl.write(...)`. `nodeRepl.write(...)` takes a
> string. If you would like to read a whole object, wrap with `JSON.stringify(...)`.

This is explicit: `nodeRepl.write` is the only text output path. No implicit
return.

### Source: OpenAI browser plugin skill doc

File: `~/projects/codex-desktop/resources/upstream/plugins/openai-bundled/plugins/browser/.codex-plugin/unified-skill.md`

Every setup example uses bare `nodeRepl.write(...)`:

```js
globalThis.iab = await agent.browsers.get("iab");
nodeRepl.write(await iab.documentation());
```

Line 115 explicitly warns agents *not* to assign the documentation to a
variable, inspect it, slice it, or emit an excerpt — only `nodeRepl.write()`
counts as output.

### Source: OpenAI skyshot architecture doc

File: `~/projects/codex-desktop/research/skyshot.md`, lines 187-188:

```mermaid
K -->|"nodeRepl.write"| M
K -->|"nodeRepl.emitImage"| M
```

Only two output paths exist in the architecture.

### Source: Recovered build-5307 fixture

File: `runtime/cua-node/test/fixtures/upstream-5307/`

The fixture README states it is "contract evidence, not a copy of upstream
implementation code." Every recorded output in `kernel.json` and
`mcp-transcripts.json` originates from a `nodeRepl.write()` call. No record
exists of a bare expression producing output.

### Implementation evidence

`runtime/cua-node/src/kernel/kernel.ts:571` evaluates a `vm.SourceTextModule`
and discards the completion value of `module.evaluate()`:

```typescript
await module.evaluate();
// ...return { output: outputText(exec?.events || []), ... }
```

Only `exec.events` — populated by `outputWrite` (called by `nodeRepl.write()`,
`console.log`, etc.) — is included in the result.

`runtime/cua-node/src/host/mcp-server.ts:256` reads `result.output`:

```typescript
if (result.output.length > 0) content.push({ type: "text", text: result.output });
```

If the user code wrote nothing, `result.output` is empty and the tool returns
`content: []`. The completion value is not consulted.

## Conclusion

The behavior is the upstream contract, not a sky-cua regression. The OpenAI
`node_repl` has never returned implicit last-expression values. The two output
channels are:

1. **`nodeRepl.write(value)`** — text, the only way to surface arbitrary values.
2. **`nodeRepl.emitImage(imageLike)`** — images, additive per call.

`console.log(...)` is captured as a side channel but is documented as a
debugging convenience, not a final-output path.

## Implications

- sky-cua's reimplementation matches the upstream contract faithfully.
- The `instructions` field in `mcp-server.ts:57` now explicitly states the
  negative: *"the value of the last expression in your code is not returned."*
  This prevents future verification agents from burning a round trip on
  bare-expression attempts.
- The `tools-list.json` fixture should not be modified — it is upstream
  evidence, and the instructions field is the right place for additions.
- The bundled skills (`skills/computer-use/`, `skills/browser-use/`,
  `skills/phone-use/`) do not need updating — they teach the `sky_cua` direct
  MCP tool surface, not `node_repl`.
