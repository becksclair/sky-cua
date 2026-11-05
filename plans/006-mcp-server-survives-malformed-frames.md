# Plan 006: Answer malformed JSON-RPC frames with a parse error instead of ending the MCP session, and keep the request id on worker panics

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-client/src/mcp_server.rs`
> (no `..HEAD`, so uncommitted edits count). If the file changed since this
> plan was written, compare the "Current state" excerpts against the live
> code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts marked abridged are condensed from rustfmt multi-line
> output; formatting differences are not drift.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

The MCP stdio server's read loop propagates every `read_message` error with
`?`. A single line-delimited frame starting with `{`/`[`, or a Content-Length
body, that is not valid JSON therefore returns from `serve`, ending the whole
session: all in-flight tool calls are dropped, the
browser-task drain and the one-shot disconnect notification to the service are
skipped (they run only after the loop exits normally), and the host sees EOF.
JSON-RPC 2.0 specifies the opposite: reply with error code `-32700` ("Parse
error") and a `null` id, and keep reading. Only errors that lose framing
(oversized frame, EOF mid-header, non-UTF-8) justify closing the stream, and
even those should break out of the loop so the drain and disconnect run.

Separately, when a tool-call worker panics, the fallback response is built
with `"id": Value::Null` because the real `id` was moved into the closure.
The host cannot match that error to its request, so the call hangs until the
host's own timeout.

## Current state

`crates/sky-cua-client/src/mcp_server.rs` (2,125 lines; the server loop is
`pub async fn serve` at line 142):

- Read loop, lines 172-177:

  ```rust
  loop {
      let (message, framing) =
          match read_message(&mut reader, &mut read_line_buf, &mut read_payload_buf).await? {
              Some(v) => v,
              None => break,
          };
  ```

- Post-loop cleanup that the `?` bypasses, lines 286-315: drains
  `browser_tasks` with `join_next().await` (line 290), then calls
  `disconnect_request_once(...)` and sends it to the service via
  `spawn_blocking`, then `drop(response_tx); writer_task.await?; Ok(())`
  (`Ok(())` at line 314). The fast path at lines 278-282 shows the existing
  convention for a failed `response_tx.send`: `break`.

- Responses are sent through `response_tx: mpsc::Sender<(Value, MessageFraming)>`
  (line 153); a dedicated writer task serializes them (lines 157-170).
  `MessageFraming` (lines 45-48) is `enum MessageFraming { ContentLength, JsonLine }`
  and derives `PartialEq` (tests use `assert_eq!` on it).

- Worker panic fallback, lines 204-241 (abridged; the live `match` is
  multi-line rustfmt output):

  ```rust
  let id = message.get("id").cloned().unwrap_or(Value::Null);
  ...
  let response = tokio::task::spawn_blocking(move || {
      match handle_message(&service, &heuristics, &mut session, message, prepared_browser_call) {
          Ok(Some(response)) => Some(response),
          Ok(None) => None,
          Err(error) => Some(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32603, "message": error.to_string() } })),
      }
  })
  .await
  .unwrap_or_else(|_| {
      Some(json!({
          "jsonrpc": "2.0",
          "id": Value::Null,
          "error": { "code": -32603, "message": "service call panicked" }
      }))
  });
  ```

- `read_message` (line 1017) delegates to `read_message_with_limit` (lines
  1028-1089), which returns `Result<Option<(Value, MessageFraming)>>`. There
  is exactly one `return Ok(None)` (EOF, line 1039). The two body-parse sites:
  - line 1050 (line-delimited): `serde_json::from_str(trimmed).context("invalid line-delimited JSON-RPC message")?`,
    reached only when the line starts with `{` or `[` (line 1048)
  - line 1086 (Content-Length): `serde_json::from_slice(payload_buf)?,` after
    `reader.read_exact(payload_buf).await?` (line 1084)
  Framing-loss errors in the same function: `frame_too_large(...)` (line
  1150) for oversized lines/payloads, `anyhow!("unexpected EOF while reading MCP headers ...")`,
  `anyhow!("missing Content-Length header")`, non-UTF-8 contexts. Lines that
  start with neither `{` nor `[` go to `parse_header_line` (lines 1154-1169):
  a bare non-JSON word is fatal ("unexpected MCP header line"), and a stray
  `key: value` line is swallowed as a header. Both stay as they are.

- Existing error-shape helpers to copy: `already_initialized(id)` (lines
  824-836) and `not_initialized(id)` (lines 838-850) build
  `json!({"jsonrpc":"2.0","id":id,"error":{"code":..,"message":..,"data":{"code":".."}}})`.

- Tests: inline `mod tests` (starts line 1198; `use super::{...}` at
  1207-1216 imports `read_message, read_message_with_limit, write_message, MessageFraming`
  among others; `Cursor` is imported at 1199). Exemplars:
  `read_message_accepts_line_delimited_json` (1993),
  `read_message_accepts_content_length_framing` (1967), and
  `json_line_frame_limit_accepts_boundary_and_rejects_overflow` (fn at 2045).
  Five tests bind the result with `.await.unwrap().unwrap()` into a tuple
  pattern: `(message, framing)` at 1983 and 2007, `(_, framing)` at 2053,
  2080 and 2108. Three call sites use `.unwrap_err()` on oversize input
  (2061, 2090, 2116) and must keep compiling and passing. No test asserts that
  a malformed body returns `Err`. 375 tests in the crate at `e928a40`
  (`cargo nextest list -p sky-cua-client`), 33 in `mcp_server`.

Conventions: structured `data.code` on JSON-RPC errors; never break
line-delimited JSON-RPC (Codex depends on it) per
`crates/sky-cua-client/AGENTS.md`; conventional commits, e.g.
`fix(client): repair dead pinned graphical env instead of trusting it`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-client --all-targets -- -D warnings` | exit 0 |
| Server tests | `cargo nextest run -p sky-cua-client mcp_server` | all pass |
| Crate tests | `cargo nextest run -p sky-cua-client` | all pass (375 at `e928a40`, plus yours) |

## Scope

**In scope** (the only file you should modify):
- `crates/sky-cua-client/src/mcp_server.rs`

**Out of scope** (do NOT touch):
- `crates/sky-cua-client/src/mcp_tools.rs` and tool definitions.
- `write_message` and the writer task.
- The `MAX_MCP_FRAME_BYTES` limit (64 MiB) and the oversize behaviour.
- Any change to how valid messages are dispatched.
- Lines that do not start with `{` or `[` are still parsed as Content-Length
  headers (`parse_header_line`, 1154-1169). A bare non-JSON line stays fatal,
  and a stray `key: value` line still swallows following lines. Do not change
  this.

## Git workflow

- Branch: `bex/mcp-parse-error-resilience`
- Commit only if the operator authorised it; style:
  `fix(client): keep the MCP session open on malformed frames`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Distinguish body-parse errors from framing errors

Introduce a return type for the read functions that the loop can match on.
Place it directly after `enum MessageFraming` (line 48). It must derive
`Debug` because the existing oversize tests call `unwrap_err()` on
`Result<FrameRead>`, which requires `T: Debug`:

```rust
#[derive(Debug)]
enum FrameRead {
    /// A complete, parsed JSON-RPC message.
    Message(Value, MessageFraming),
    /// The frame delimiter was honoured but its body is not valid JSON;
    /// reply `-32700` and keep reading.
    Malformed { framing: MessageFraming, detail: String },
    /// Clean end of input.
    Eof,
}
```

Change `read_message_with_limit` to return `Result<FrameRead>`:

- At the line-delimited parse site (line 1050), replace the `?` with a match:
  `Ok(value) => Ok(FrameRead::Message(value, MessageFraming::JsonLine))`,
  `Err(error) => Ok(FrameRead::Malformed { framing: MessageFraming::JsonLine, detail: error.to_string() })`.
- At the Content-Length parse site (line 1086), the same, with
  `MessageFraming::ContentLength`. The preceding `read_exact` (line 1084)
  stays `?` (short read means framing is lost).
- The single `return Ok(None)` on EOF (line 1039) becomes `return Ok(FrameRead::Eof)`.
- Every other `Err` path (oversize, header EOF, missing Content-Length,
  non-UTF-8) stays an `Err`: those are framing losses.

Update `read_message` to the same return type. Then update the tests: add
`FrameRead` and `parse_error` to the `use super::{...}` list (1207-1216), and
replace each `.await.unwrap().unwrap()` tuple binding (1983, 2007, 2053, 2080,
2108) with

```rust
let FrameRead::Message(message, framing) =
    read_message(&mut reader, &mut line_buf, &mut payload_buf).await.unwrap()
else {
    panic!("expected a parsed message");
};
```

using `FrameRead::Message(_, framing)` at 2053, 2080 and 2108 and
`read_message_with_limit(...)` where the original did. The `unwrap_err()`
sites (2061, 2090, 2116) stay unchanged; oversize errors are raised before any
parse, so they still return `Err`.

**Verify**: none on its own. Changing the return type breaks the
`Some(v)`/`None` match in `serve` (174-177), so the crate does not compile
until Step 2 is applied. Steps 1 and 2 are verified together at the end of
Step 2.

### Step 2: Handle the three outcomes in the read loop and run cleanup on fatal errors

Replace lines 172-177 with:

```rust
let mut fatal_read_error: Option<anyhow::Error> = None;
loop {
    let (message, framing) =
        match read_message(&mut reader, &mut read_line_buf, &mut read_payload_buf).await {
            Ok(FrameRead::Message(message, framing)) => (message, framing),
            Ok(FrameRead::Malformed { framing, detail }) => {
                tracing::warn!(%detail, "malformed JSON-RPC frame; answering parse error");
                if response_tx.send((parse_error(detail), framing)).await.is_err() {
                    break;
                }
                continue;
            }
            Ok(FrameRead::Eof) => break,
            Err(error) => {
                fatal_read_error = Some(error);
                break;
            }
        };
```

Add the helper next to `already_initialized`:

```rust
fn parse_error(detail: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": {
            "code": -32700,
            "message": "Parse error",
            "data": { "code": "ParseError", "detail": detail }
        }
    })
}
```

At the end of `serve`, after `writer_task.await?;`, replace `Ok(())` with:

```rust
match fatal_read_error {
    Some(error) => Err(error),
    None => Ok(()),
}
```

so the browser-task drain and the disconnect notification run before a
framing error is reported. `response_tx` is still owned inside the loop (it
is dropped only at line 312), and `tracing::warn!(%detail, ...)` borrows
`detail`, so passing it to `parse_error(detail)` afterwards compiles. Run
`cargo fmt` after editing.

**Verify**: `cargo clippy -p sky-cua-client --all-targets -- -D warnings` → exit 0,
and `cargo nextest run -p sky-cua-client mcp_server` → all 33 existing tests pass.

### Step 3: Keep the request id when the worker panics

At line 204, after `let id = message.get("id").cloned().unwrap_or(Value::Null);`,
add `let panic_id = id.clone();` and use `panic_id` in the
`unwrap_or_else` fallback in place of `Value::Null`. Also add
`"data": { "code": "ServiceCallPanicked" }` to that error object for
consistency with the other structured errors.

**Verify**: `grep -n '"id": Value::Null' crates/sky-cua-client/src/mcp_server.rs`
→ the only remaining match is inside `parse_error` (where a null id is
correct).

### Step 4: Tests

Add to the inline `mod tests`:

1. `read_message_reports_malformed_json_line_as_recoverable` (`#[tokio::test]`):
   reader over `b"{not json}\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n"`.
   First call → `FrameRead::Malformed { framing: MessageFraming::JsonLine, .. }`.
   Second call on the same reader → `FrameRead::Message(message, MessageFraming::JsonLine)`
   with `message["id"] == 7`. This proves the stream stays in sync.
2. `read_message_reports_malformed_content_length_body_as_recoverable`
   (`#[tokio::test]`): use this exact input (`{nope` is 5 bytes; the ping
   body is 40 bytes):

   ```rust
   let mut reader = Cursor::new(
       b"Content-Length: 5\r\n\r\n{nopeContent-Length: 40\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"ping\"}".to_vec(),
   );
   ```

   First call → `FrameRead::Malformed { framing: MessageFraming::ContentLength, .. }`
   (`read_exact` consumed exactly `{nope`, so the next header read starts at
   `Content-Length: 40`). Second call → `FrameRead::Message(message, MessageFraming::ContentLength)`
   with `message["id"] == 8`.
3. `truncated_content_length_body_is_still_fatal` (`#[tokio::test]`): reader
   over `b"Content-Length: 10\r\n\r\n{}".to_vec()`;
   `read_message(...).await.unwrap_err()` (a short `read_exact` is a framing
   loss, not a recoverable parse error). The existing
   `json_line_frame_limit_accepts_boundary_and_rejects_overflow` stays as is.
4. `parse_error_response_shape` (`#[test]`): `let response = parse_error("x".into());`
   then `assert_eq!(response["error"]["code"], -32700)`,
   `assert_eq!(response["id"], Value::Null)`,
   `assert_eq!(response["error"]["data"]["code"], "ParseError")`. Do not build
   an expected `json!` literal containing `"id": Value::Null`; it would
   inflate the done-criteria grep count.

The `serve` loop itself reads real stdin and is not unit-tested at
`e928a40`; do not try to add a stdin harness. Steps 2 and 3 are covered by
the grep done-criteria and code review.

**Verify**: `cargo nextest run -p sky-cua-client mcp_server` → all pass,
37 tests (33 + 4 new).

## Test plan

- New tests listed in Step 4, in `mcp_server.rs` `mod tests`.
- Pattern: `read_message_accepts_line_delimited_json`.
- Verification: `cargo nextest run -p sky-cua-client` → all pass.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-client --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-client` exits 0; `cargo nextest list -p sky-cua-client --message-format json | python3 -c "import json,sys; d=json.load(sys.stdin); print(sum(len(s['testcases']) for s in d['rust-suites'].values()))"` prints 379 (375 at `e928a40` plus 4)
- [ ] `grep -n 'read_payload_buf).await?' crates/sky-cua-client/src/mcp_server.rs` prints nothing (that text occurs only at line 174 today)
- [ ] `grep -n '32700' crates/sky-cua-client/src/mcp_server.rs` matches in `parse_error` and in a test
- [ ] `grep -c '"id": Value::Null' crates/sky-cua-client/src/mcp_server.rs` is `1` (it is 1 today, at the panic fallback; after Steps 2-3 the only occurrence is inside `parse_error`)
- [ ] `git status --porcelain -- crates/` lists only `crates/sky-cua-client/src/mcp_server.rs` (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- `read_message` or `read_message_with_limit` are called from outside
  `mcp_server.rs` (`grep -rn 'read_message' crates/sky-cua-client/src --include='*.rs' | grep -v mcp_server.rs`);
  report the caller rather than changing its file.
- An existing test asserts that a malformed body returns `Err` (search the
  test module for `invalid line-delimited`); report it, since that test
  encodes the old behaviour and the maintainer must confirm the change.
- A Codex-compatibility concern: if any test or doc states that the server
  must exit on malformed input, stop and report.

## Maintenance notes

- Hosts that pipeline a bad frame followed by good ones now keep working;
  the `-32700` response has a `null` id per spec, so hosts cannot correlate
  it, which is expected.
- If a future framing mode is added (e.g. WebSocket), classify its errors the
  same way: body-parse errors are recoverable, delimiter errors are fatal.
- A reviewer should check that nothing after the loop awaits on
  `response_tx` after it is dropped, and that `fatal_read_error` is returned
  only after `writer_task` has flushed.
- A framing error now waits for in-flight browser tasks (as EOF already does)
  before `serve` returns; during a portal approval that can be tens of
  seconds. If `writer_task` panics, its `JoinError` is returned instead of
  `fatal_read_error`, which is acceptable.
- The Content-Length `Malformed` case is recoverable only if the declared
  length was right. If it was wrong, the next read starts mid-frame and ends
  in a fatal "missing Content-Length header", not a mis-dispatch.
- Codex pins rmcp `=3.0.0`, whose `JsonRpcError.id` is `Option<RequestId>`
  with `serde(default)`, so an `id: null` parse error deserializes cleanly
  and is logged at debug level and dropped. If `PROTOCOL_VERSION` moves to
  MCP 2026-07-28 or later, that revision says to omit `id` on parse errors
  instead of sending `null`; rmcp 3.0.0 accepts both.
