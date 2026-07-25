# `node_repl` MCP output contract

## Status

Shipped. Upstream (`@oai/node-repl` / ChatGPT 26.707.72221 build-5307) contract recovered from extracted Darwin ELF, published skill instructions, and live probes. Last verified: 2026-07-25.

## Summary

The `node_repl` MCP server's `js` tool returns **only** what the call explicitly writes via `nodeRepl.write(value)` or `nodeRepl.emitImage(...)`. The value of the last top-level expression is **not** captured or returned. This is not a regression — it matches the upstream OpenAI contract exactly.

## Contract surface

Output channels (all other paths produce empty output):

| Channel | Behavior | Example |
|---|---|---|
| `nodeRepl.write(value)` | Appends `String(value)` to the tool result `content[].text`. Values other than strings are formatted with `util.inspect()`. | `nodeRepl.write(JSON.stringify({ a: 1 }))` |
| `nodeRepl.emitImage(imageLike)` | Adds an image to `content[].images`. Accepted input: data URL, raw PNG/JPEG/WebP bytes, or `{ bytes, mimeType }`. Multiple images per call are additive. | `await nodeRepl.emitImage(pngBuffer)` |
| `console.log(...)` / `.warn` / `.error` / `.info` / `.debug` | Captured as a side channel and appended to the text output with trailing newlines. Prefer `nodeRepl.write()` for final tool output; `console.log` is for debugging. | `console.log("state", state)` |

What does **not** return output:
- The last expression value (e.g. `1 + 1` without a wrapper produces `null` in `content[].text`).
- Assignment expressions, function calls without `nodeRepl.write`, Promise results without `nodeRepl.write`.
- Thrown exceptions produce an `isError` response with the error message in `content[].text`.

## Evidence

The upstream OpenAI contract is documented in four independent sources:

**`computer-use-node-repl.md:12`** (OpenAI computer-use plugin skill doc):
> For text output, use `nodeRepl.write(...)`. `nodeRepl.write(...)` takes a string. If you would like to read a whole object, wrap with `JSON.stringify(...)`.

**`unified-skill.md`** (OpenAI browser plugin skill doc, all examples):
- `nodeRepl.write(await iab.documentation());`
- `nodeRepl.write(await chrome.documentation());`
- `nodeRepl.write(await browser.documentation());`

And an explicit instruction at line 115: *"run the exact direct `nodeRepl.write(await <browser>.documentation());` call shown in the applicable scenario above. Do not assign the documentation to a variable, inspect its length..."*

**`research/skyshot.md:187-188`** (OpenAI architecture document):
```mermaid
K -->|"nodeRepl.write"| M
K -->|"nodeRepl.emitImage"| M
```
Only two output paths exist in the upstream architecture.

**Recovered fixture (`runtime/cua-node/test/fixtures/upstream-5307/kernel.json`):**
All recorded `output` values in the test fixture are produced by `nodeRepl.write()` calls. No output record exists from a bare expression.

## Implementation

- `runtime/cua-node/src/kernel/kernel.ts:571` — `await module.evaluate()` discards the completion value; only `exec.events` (populated by `outputWrite` / `outputLine`) is returned.
- `runtime/cua-node/src/host/mcp-server.ts:248-256` — the `callJs` handler reads `result.output` (the accumulated events text) and `result.images`, discarding everything else.
- The `instructions` field in the initialize response (`mcp-server.ts:57`) explicitly states: *"Use `nodeRepl.write(value)` to surface text output; the value of the last expression in your code is not returned — only explicit `nodeRepl.write(...)` calls and `nodeRepl.emitImage(...)` produce content in the tool result."*
- The `js` tool `description` in `tools/list` documents `nodeRepl.write()` and `console.log()` as output channels.

## Verification

```js
// This produces a tool result with content[].text = "42"
nodeRepl.write(String(6 * 7))

// This produces a tool result with content[].text = "" (null — bare expression)
// and DOES NOT return 42
6 * 7
```

The downstream test at `runtime/cua-node/test/fixtures/upstream-5307/fixtures.test.ts` verifies that the `tools/list` description and output shape match the upstream contract.

## Related

- `docs/research/2026-07-node-repl-output-contract.md` — investigation transcript and upstream evidence recovery.
