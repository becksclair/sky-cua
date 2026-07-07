# Plan 003: Land the small high-confidence correctness fixes (IPC error frames + caps, portal session race close, ydotool reap, cursor-adapter timeouts)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-service/src/ipc_server.rs crates/sky-cua-linux/src/portal/remote_desktop.rs crates/sky-cua-linux/src/virtual_input.rs crates/sky-cua-overlay-host/src/system_cursor.rs crates/sky-cua-client/src/service_launcher.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M (four independent S-fixes)
- **Risk**: LOW-MED (each fix is localized; the IPC change touches the hot path)
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

Four independently-verified defects, each small:

- **A.** One malformed IPC request line tears down the whole persistent
  client↔daemon connection instead of replying with an error frame, and both
  IPC ends read lines with no length cap (unbounded memory on a wedged peer).
- **B.** When two tasks race the first portal session start, the loser's
  fully-established RemoteDesktop+ScreenCast session (and PipeWire fd) is
  dropped without `close_session` — orphaned compositor-side session, and the
  operator may see two approval dialogs.
- **C.** `run_ydotool_command` kills a timed-out child without reaping it —
  one zombie process per timeout for the daemon's lifetime.
- **D.** The COSMIC cursor-bridge socket call and the KWin qdbus subprocess
  have no timeouts and run on the overlay render/tick path — a stuck helper
  hangs the overlay thread indefinitely.

## Current state

Fix A — `crates/sky-cua-service/src/ipc_server.rs:370-391`:

```rust
async fn handle_stream<S>(stream: S, daemon: Arc<ServiceDaemon>) -> Result<()>
where ... {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 { return Ok(()); }
        let request: ServiceRequest = serde_json::from_str(line.trim_end()).map_err(|error| {
            anyhow::anyhow!("failed to parse sky-cua IPC request as JSON: {error}")
        })?;                       // <-- parse failure closes the connection
        let response = daemon.handle(request).await;
        ...
    }
}
```

The client side reads responses at
`crates/sky-cua-client/src/service_launcher.rs:367-369` with the same
uncapped `read_line`. `ServiceResponse` is defined in
`crates/sky-cua-platform/src/model/service.rs`; check its variants for an
error shape (grep `Error` within that enum) — reuse the existing error
variant; do NOT invent a new response variant (wire-contract change).

Fix B — `crates/sky-cua-linux/src/portal/remote_desktop.rs:428-442`:

```rust
pub(crate) async fn ensure_session_started(&self) -> Result<(), BackendError> {
    {
        let state = self.inner.read().await;
        if state.session.is_some() { return Ok(()); }
    }
    let started = start_session_with_timeout(self.token_store.as_ref()).await?;
    let mut state = self.inner.write().await;
    if state.session.is_none() {
        state.pending_events.extend(started.lifecycle_events);
        state.set_session(started.session);
    }
    Ok(())     // <-- else branch drops started.session without close_session
}
```

The correct pattern exists in the same file: `reset_persisted_tokens`
(directly below, :445-451) calls `close_session(&session).await` on a
session it removes, and `capture_frame` has an `unused_session` branch that
closes a losing session. Mirror those.

Fix C — `crates/sky-cua-linux/src/virtual_input.rs:1310-1330`:

```rust
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();          // <-- no child.wait() after kill
                return Err(BackendError::new(...));
            }
            Err(error) => {
                let _ = child.kill();          // <-- same
                return Err(BackendError::new(...));
            }
        }
    }
```

The correct pattern is in the same file at `command_output_with_timeout`
(~:1413-1416): `kill()` then `wait()`.

Fix D — `crates/sky-cua-overlay-host/src/system_cursor.rs`:

- `cosmic_bridge_request` (:597-613) does `UnixStream::connect` +
  `write_all` + `read_to_string` with no `set_read_timeout`/
  `set_write_timeout`. The exemplar for socket timeouts is the input-helper
  client in `crates/sky-cua-linux/src/virtual_input.rs:790-795`.
- `call_qdbus` (:314-330) runs `Command::new(&self.qdbus)...output()` with no
  timeout. The exemplar for bounded subprocess waits is
  `command_output_with_timeout` in
  `crates/sky-cua-linux/src/virtual_input.rs` (~:1407) — but note
  `system_cursor.rs` is in a different crate; implement a small local
  spawn/try_wait/deadline helper in `system_cursor.rs` following the same
  shape (spawn → try_wait loop → kill+wait on deadline), don't add a
  cross-crate dependency for it.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Workspace tests | `cargo nextest run` | all pass |
| Targeted crates | `cargo nextest run -p sky-cua-service -p sky-cua-linux -p sky-cua-overlay-host` | all pass |
| Format | `cargo fmt --check` | exit 0 |
| Build | `cargo build` | exit 0 |

## Scope

**In scope**:
- `crates/sky-cua-service/src/ipc_server.rs` (fix A + its tests)
- `crates/sky-cua-client/src/service_launcher.rs` (fix A read cap only)
- `crates/sky-cua-linux/src/portal/remote_desktop.rs` (fix B)
- `crates/sky-cua-linux/src/virtual_input.rs` (fix C)
- `crates/sky-cua-overlay-host/src/system_cursor.rs` (fix D)

**Out of scope** (do NOT touch):
- `crates/sky-cua-platform/src/model/service.rs` — no wire-contract changes.
- The retry/respawn logic in `service_launcher.rs` (`call()`,
  `is_stale_stream_error`) — that's plan 004; only add the read cap here.
- `crates/sky-cua-cosmic-helper/` — the bridge *server*'s serial accept loop
  is a known limitation; the client-side timeout is the fix here.
- The chrome-host `process::exit` on Chrome-pipe write error — by design
  (documented at `host.rs:278-283`).

## Git workflow

- Branch: `bex/advisor-003-correctness-small-fixes`
- One commit per fix (A–D), style: `fix(ipc): reply with an error frame on malformed requests`,
  `fix(portal): close the losing session in ensure_session_started`, etc.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1 (Fix A): Error frames + read caps on the IPC

In `handle_stream` (`ipc_server.rs`):
1. On `serde_json::from_str` failure, build the existing error-shaped
   `ServiceResponse`, write it + newline, `continue` the loop. Only genuine
   transport errors (`read_line` Err, `read == 0`, write failures) may end
   the connection.
2. Cap the request line: use `(&mut reader).take(MAX_IPC_LINE_BYTES)` around
   the read, or check `line.len()` growth — simplest correct approach:
   `read_line` on a `.take()`-limited reader; if the limit is hit without a
   newline (`read == limit && !line.ends_with('\n')`), write an error frame
   and close (an oversized frame leaves the stream unsynchronized, so closing
   IS correct there — unlike parse errors). Set
   `const MAX_IPC_LINE_BYTES: u64 = 64 * 1024 * 1024;` (screenshots travel
   base64-inline in responses, requests are small — 64MiB is generous).
3. Client side (`service_launcher.rs:367-369`): apply the same `.take()` cap
   to the response read with the same constant value (responses carry inline
   images; do not set it lower).

Add tests in the existing `ipc_server.rs` `#[cfg(test)] mod tests`
(pattern: it already tests `handle_stream` with in-memory streams — grep
`mod tests` in the file): (a) malformed JSON line → error response returned
AND a subsequent valid request on the same stream still succeeds; (b) a line
exceeding the cap → connection closed with error frame.

**Verify**: `cargo nextest run -p sky-cua-service -E 'binary_id(sky-cua-service)' ` → all pass including 2 new tests.

### Step 2 (Fix B): Close the losing portal session

In `ensure_session_started`, restructure the tail:

```rust
let mut state = self.inner.write().await;
if state.session.is_none() {
    state.pending_events.extend(started.lifecycle_events);
    state.set_session(started.session);
    return Ok(());
}
drop(state);                                   // release before the await
close_session(&started.session).await;         // match capture_frame's unused-session handling
Ok(())
```

Check `close_session`'s exact signature/ownership in the file first and
match how `reset_persisted_tokens` calls it. Do not hold the write lock
across the `close_session` await.

**Verify**: `cargo nextest run -p sky-cua-linux` → all pass. (This path needs
a live portal for integration proof — state that in your report as a
not-run live gate, per AGENTS.md Definition of Done.)

### Step 3 (Fix C): Reap the killed ydotool child

Add `let _ = child.wait();` immediately after both `let _ = child.kill();`
lines in `run_ydotool_command` (`virtual_input.rs:1315-1328`).

**Verify**: `cargo build -p sky-cua-linux` → exit 0; `cargo nextest run -p sky-cua-linux` → all pass.

### Step 4 (Fix D): Timeouts on the cursor adapters

1. `cosmic_bridge_request`: after `UnixStream::connect`, set
   `stream.set_read_timeout(Some(Duration::from_secs(2)))?` and
   `set_write_timeout(Some(Duration::from_secs(2)))?` (mirror
   `virtual_input.rs:790-795`). A timeout surfaces as the existing
   `.context(...)` error — callers already handle bridge errors.
2. `call_qdbus`: replace `.output()` with spawn + deadline loop + kill/wait
   on timeout (5s deadline — KWin can be slow under load but the caller is a
   render tick heartbeat). Preserve the current stdout/stderr handling:
   after a successful in-deadline exit, use `wait_with_output()`.

**Verify**: `cargo nextest run -p sky-cua-overlay-host` → all pass;
`cargo build` (workspace) → exit 0.

## Test plan

- New: the two `handle_stream` tests from Step 1 (malformed-frame recovery,
  oversized-frame rejection) — pattern: existing `ipc_server.rs` tests.
- Fixes B–D are hard to unit-test without a live portal/compositor; rely on
  existing suites for no-regression plus a code-review-visible diff. Say in
  your final report that the portal-race and COSMIC/KWin timeout paths were
  not live-smoked.

## Done criteria

- [ ] `cargo fmt --check && cargo nextest run` exits 0
- [ ] Malformed-JSON test proves the connection survives a bad frame
- [ ] `grep -n "child.kill" crates/sky-cua-linux/src/virtual_input.rs` — every hit inside `run_ydotool_command` is followed by a `wait`
- [ ] `grep -n "set_read_timeout" crates/sky-cua-overlay-host/src/system_cursor.rs` → ≥1 hit
- [ ] `grep -c "\.output()" crates/sky-cua-overlay-host/src/system_cursor.rs` — the qdbus call no longer uses bare `.output()`
- [ ] No files outside the in-scope list modified (`git status`)
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- `ServiceResponse` has no existing error-shaped variant reusable for fix A —
  adding one is a wire-contract change touching the out-of-scope platform
  crate; report instead.
- `close_session`'s signature can't be called with `&started.session`
  (ownership mismatch) after reading the file — report the actual shape.
- Any existing test starts failing and the cause isn't obviously your diff.

## Maintenance notes

- Fix A's cap constant should stay ≥ the largest inline-image response;
  if screenshot delivery grows (e.g. multi-display inline), revisit.
- Fix B interacts with plan 004: after 004 lands, a retried first call can
  race here more often — the close-on-lose branch is what makes that safe.
- Reviewer scrutiny: fix A's "close on oversized but continue on malformed"
  asymmetry is deliberate (stream desync vs clean frame boundary) — check the
  implementation preserved it.
