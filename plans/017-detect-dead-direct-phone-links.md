# Plan 017: Detect dead direct-LAN phone links on the host instead of waiting for TCP to give up

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-service/src/phone/direct/mod.rs android/phone-companion/app/src/main/java/com/skycua/phonecompanion/direct/OkHttpDirectSocket.kt`
> (no `..HEAD`, so uncommitted edits count). If either file changed since
> this plan was written, compare the "Current state" excerpts against the
> live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts are verbatim unless marked abridged.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW-MED
- **Depends on**: none (plan 002 touches `content_transfer.rs` and `manager/appshot.rs`, not this file)
- **Category**: bug
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

The authenticated direct-LAN websocket loop in `serve_socket` has no read
deadline: it only ends on peer EOF, a socket error, cancellation, or a
protocol violation. When the phone leaves Wi-Fi, sleeps, or loses its link
without a clean close, the host keeps the device registered as connected
until the kernel's TCP retransmission timer gives up, typically minutes.
During that window every phone request waits out its full per-request
deadline and fails with `DeadlineExceeded` instead of `NotConnected`, and
the agent sees a "connected" device that answers nothing. The companion
pings every 20 seconds (`OkHttpClient.pingInterval(20, SECONDS)`) on a fixed
schedule regardless of other traffic, and OkHttp drops its own socket when a
pong is late, so a live phone always produces an inbound frame within 20 s
and silence longer than about a minute is a dead link. The post-`AuthOk`
fallback loop in the same file (used when the registry has no handle)
already applies a 30 s idle timeout to `socket.next()`; the main
authenticated loop should apply a comparable bound, tolerant of the ping
cadence. A second, smaller hole: `request()` sends the frame with
`link.control.send(message).await` outside the `deadline`, so if the 64-slot
control channel is full because the socket is wedged, the caller can block
past its own deadline.

## Current state

`crates/sky-cua-service/src/phone/direct/mod.rs` (3,416 lines at `e928a40`;
no other plan in this set edits it):

- Imports (lines 8-33) include `std::time::Duration`, `futures_util::{SinkExt, StreamExt}`,
  `tokio::{...}`; `tokio::time::timeout` is already used at lines 439, 560,
  1677, 1710. The lockfile resolves tokio 1.53.3 (workspace minimum 1.48),
  which has `tokio::pin!`, `Sleep::reset`, `Sleep::deadline`, `timeout_at`
  and `JoinHandle::is_finished`.
- `DirectRuntimeError` (lines 46-53): `NotConnected, LinkEpochChanged { .. }, DeadlineExceeded, Disconnected, Protocol(String), Remote { code, message }`.
- `request()` (lines 386-448). Lines 431-447 (verbatim):

  ```rust
  let message = tokio_tungstenite::tungstenite::Message::Text(encoded.into());
  if link.control.send(message).await.is_err() {
      link.pending
          .lock()
          .expect("pending mutex poisoned")
          .remove(&request_id);
      return Err(DirectRuntimeError::Disconnected);
  }
  match tokio::time::timeout(deadline, rx).await {
      Ok(Ok(result)) => result,
      Ok(Err(_)) => Err(DirectRuntimeError::Disconnected),
      Err(_) => {
          link.pending
              .lock()
              .expect("pending mutex poisoned")
              .remove(&request_id);
          Err(DirectRuntimeError::DeadlineExceeded)
      }
  }
  ```

  `link.control` is an `mpsc::Sender` with capacity 64 (`serve_socket`,
  line 619: `let (control_tx, mut control_rx) = mpsc::channel(64);`).

- `serve_socket` (lines 610-718). The loop (lines 637-698) is two
  `tokio::select! { biased; ... }` blocks (one when `control_burst >= 8`,
  one otherwise); each has the arms `_ = &mut cancel => break`,
  `message = socket.next(), if <inbound guard> => { if !matches!(self.handle_inbound(..).await, Ok(true)) { break; } ... }`,
  and the `control_rx.recv()` / `bulk_rx.recv()` send arms, which set
  `terminal_error = Some(io::Error::new(io::ErrorKind::BrokenPipe, error))`
  on a failed `socket.send` (four `socket.send(..).await` calls in total).
  After the loop (lines 699-717) the link is removed from `self.links` if the
  epoch still matches, pending requests are failed with `Disconnected`, the
  content store aborts the epoch, and the function returns `terminal_error`
  as `Err` or `Ok(())`. That result propagates through `accept_socket` and
  is logged at line 1244 as
  `tracing::warn!(%peer, %error, "CompanionDirect connection rejected")`;
  an idle timeout will therefore appear in the log as "connection rejected".
  rustfmt leaves `select!` bodies alone (existing lines there already exceed
  100 columns), so the grep-based done criteria below are stable.
- `handle_inbound` (lines 720-770, abridged): `None` → `Ok(false)` (EOF);
  `Err` → `InvalidData`; `Binary` → content chunk; non-`Text` frames
  (`Ping`, `Pong`, `Close`, `Frame`) → `Ok(true)`; `Text` → decoded control
  frame. After a `Close` the next read yields `None`. tungstenite 0.30
  queues a `Pong` when it reads a `Ping` (`protocol/mod.rs:668-674`) and
  writes it at the start of the next `read()`/`poll_next` (`mod.rs:456-466`)
  or on any write, and the application still sees the `Ping` from
  `socket.next()`; since this loop polls `next()` on at least every other
  pass, each companion ping is an inbound event here AND gets its pong
  promptly.
- The post-`AuthOk` fallback loop (used when `registry.handle` is `None`)
  at lines 1670-1680 shows the existing idle-timeout idiom:

  ```rust
  message = tokio::time::timeout(Duration::from_secs(30), socket.next()) => {
      message.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "authenticated link idle timeout"))?
  }
  ```

- Tests (inline `mod tests`, which has `use super::*` so `SinkExt`/`StreamExt`
  are available; the flood test already uses `sink.send`):
  `live_socket_scheduler_exits_and_removes_link_on_peer_eof` (fn at line
  3112, attribute 3111) is the exemplar for a real-TCP `serve_socket` test: it binds
  `TcpListener` on `127.0.0.1:0`, spawns `serve_socket(socket, peer, "device-a", 1, cancel_rx)`,
  connects a `tokio_tungstenite::connect_async` client, waits until
  `handle.snapshot("device-a").is_some()`, drops the client, and asserts
  the server task ends within 1 s and the snapshot is gone. `.config/nextest.toml`
  does not currently serialize `phone::direct::tests` (audit `TESTS-13`); the
  new test binds an ephemeral port like its siblings.

`android/.../direct/OkHttpDirectSocket.kt:14`: `.pingInterval(20, TimeUnit.SECONDS)`.
The companion never sends application-level keepalives beyond websocket
pings.

Conventions: `cargo nextest run`; conventional commits, e.g.
`fix(browser): ack directly replied settlements so the host queue drains`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-service --all-targets -- -D warnings` | exit 0 |
| Direct tests | `cargo nextest run -p sky-cua-service phone::direct::tests` | all pass |
| Crate tests | `cargo nextest run -p sky-cua-service` | all pass (786 at `e928a40`, plus yours) |

## Scope

**In scope** (the only file you should modify):
- `crates/sky-cua-service/src/phone/direct/mod.rs` (`request`, `serve_socket`, one new constant, tests)
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- The Android companion (its 20 s ping is the contract this plan relies on;
  do not change it).
- Host-initiated pings. tokio-tungstenite can send `Ping` frames, but the
  companion already pings, and adding host pings would make the idle gate
  depend on two timers; keep it read-idle only.
- `handle_inbound`, the content store, the enrollment/auth code.
- The post-`AuthOk` fallback loop (lines 1670-1680) and its 30 s timeout.
- The unbounded `link.bulk.send(..).await` calls in the host-to-phone
  content push (lines ~517-557), which have the same shape as the Step 2
  hole; follow-up.
- `.config/nextest.toml` serialization (`TESTS-13`).

## Git workflow

- Branch: `bex/direct-link-idle-timeout`
- Commit only if the operator authorised it; style:
  `fix(phone): time out silent direct links and bound control sends by the request deadline`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add the idle bound to `serve_socket`

1. Add a constant near `DEFAULT_ENROLLMENT_TTL` (line 40):

   ```rust
   /// A link that produces no inbound frame (data, ping, pong, or close) for
   /// this long is dead: the companion pings every 20 s
   /// (`OkHttpDirectSocket.kt`), so this is three missed pings plus slack.
   /// Without it a phone that drops off Wi-Fi stays "connected" until TCP
   /// retransmission gives up (minutes), and every request burns its full
   /// deadline instead of failing fast with `NotConnected`.
   pub(crate) const DIRECT_LINK_IDLE_TIMEOUT: Duration = Duration::from_secs(75);
   ```

2. In `serve_socket`, before the `loop` (after `terminal_error` is declared
   at line 636), add:

   ```rust
   let idle = tokio::time::sleep(DIRECT_LINK_IDLE_TIMEOUT);
   tokio::pin!(idle);
   ```

3. In BOTH `select!` blocks, add an arm immediately AFTER the `socket.next()`
   arm and BEFORE the `control_rx`/`bulk_rx` send arms (order: cancel,
   socket.next, idle, then the send arms). With `biased;`, placing idle
   before `socket.next()` would let a companion ping that is already
   buffered lose to an expired idle timer after a long send-arm body and kill
   a live link; placing it after the send arms would let outbound traffic
   starve it.

   ```rust
   () = &mut idle => {
       terminal_error = Some(io::Error::new(
           io::ErrorKind::TimedOut,
           "direct link idle timeout: no inbound frame",
       ));
       break;
   }
   ```

4. In BOTH `socket.next()` arms, reset the timer when any inbound frame
   arrives (first statement inside the arm body, before `handle_inbound`):

   ```rust
   idle.as_mut().reset(tokio::time::Instant::now() + DIRECT_LINK_IDLE_TIMEOUT);
   ```

   A `Ping`, `Pong`, `Binary`, `Text` or `Close` all count as liveness; EOF
   (`None`) still ends the loop through `handle_inbound` returning `Ok(false)`.

5. Bound the four `socket.send(..).await` calls by the idle deadline too.
   `select!` cannot interrupt an arm body: if the phone vanishes during a
   host-to-phone content push (256 KiB `Binary` chunks over `link.bulk`),
   the kernel send buffer fills, `socket.send` blocks inside the arm, and the
   idle arm is never polled. Replace each
   `if let Err(error) = socket.send(X).await { terminal_error = Some(io::Error::new(io::ErrorKind::BrokenPipe, error)); break; }`
   (where `X` is `message` in the control arms and `frame.message` in the
   bulk arms) with:

   ```rust
   match tokio::time::timeout_at(idle.deadline(), socket.send(X)).await {
       Ok(Ok(())) => {}
       Ok(Err(error)) => {
           terminal_error = Some(io::Error::new(io::ErrorKind::BrokenPipe, error));
           break;
       }
       Err(_) => {
           terminal_error = Some(io::Error::new(
               io::ErrorKind::TimedOut,
               "direct link idle timeout: send stalled",
           ));
           break;
       }
   }
   ```

   Keep the `if let Some(sent) = frame.sent { ... }` and burst bookkeeping
   that follow each send unchanged.

The post-loop cleanup already removes the link, fails pending requests with
`Disconnected`, and returns `terminal_error`, so a timeout is reported like a
broken pipe; `accept_socket` logs it at line 1244 as "CompanionDirect
connection rejected". Keep that unchanged.

**Verify**: `cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-service phone::direct::tests` → all existing
tests pass (the EOF and flood tests finish well inside 75 s and still pass).

### Step 2: Bound the control send in `request()` by the request deadline

Replace lines 432-439 (the `if link.control.send(message).await.is_err() { ... }`
block AND the following `match tokio::time::timeout(deadline, rx).await {`
line) with:

```rust
let send_deadline = tokio::time::Instant::now() + deadline;
match tokio::time::timeout_at(send_deadline, link.control.send(message)).await {
    Ok(Ok(())) => {}
    Ok(Err(_)) => {
        link.pending.lock().expect("pending mutex poisoned").remove(&request_id);
        return Err(DirectRuntimeError::Disconnected);
    }
    Err(_) => {
        link.pending.lock().expect("pending mutex poisoned").remove(&request_id);
        return Err(DirectRuntimeError::DeadlineExceeded);
    }
}
match tokio::time::timeout_at(send_deadline, rx).await {
```

and keep the existing match arms for the response wait (the three arms that
followed the old `match` line). The whole request (enqueue plus response)
now shares one absolute deadline instead of the deadline starting only after
the enqueue succeeded. Run `cargo fmt`.

**Verify**: `cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-service phone::direct::tests` → all pass
(existing request/deadline tests such as `disconnect_is_ambiguous_and_request_is_not_replayed`
at line 2972 must be unaffected: a request that was enqueued waits for its
response until the same absolute deadline, i.e. the remainder after a
near-instant enqueue; no test asserts `DeadlineExceeded` timing).

### Step 3: Regression test for the idle timeout

The 75 s constant is too long for a unit test, so make the timeout
injectable without changing production call sites. Rename the existing
`serve_socket` to `serve_socket_with_idle_timeout`, add a trailing parameter
`idle_timeout: Duration`, and in its body replace ALL uses of
`DIRECT_LINK_IDLE_TIMEOUT` (the `tokio::time::sleep(..)` and both `reset(..)`
calls; the Step 1.5 sends use `idle.deadline()` and need no change) with
`idle_timeout`. Then add the production wrapper exactly as follows (no `mut`
on its parameters, or `-D warnings` fails on `unused_mut`):

```rust
async fn serve_socket(
    &self,
    socket: WebSocketStream<TcpStream>,
    peer: SocketAddr,
    device_id: String,
    epoch: u64,
    cancel: tokio::sync::oneshot::Receiver<()>,
) -> io::Result<()> {
    self.serve_socket_with_idle_timeout(socket, peer, device_id, epoch, cancel, DIRECT_LINK_IDLE_TIMEOUT)
        .await
}
```

After this, `grep -n DIRECT_LINK_IDLE_TIMEOUT` shows exactly the constant's
definition and this one call. Then add, modelled on
`live_socket_scheduler_exits_and_removes_link_on_peer_eof`:

```rust
#[tokio::test]
async fn live_socket_scheduler_times_out_a_silent_link_and_fails_pending_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = DirectRuntimeHandle::new();
    let server_handle = handle.clone();
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        server_handle
            .serve_socket_with_idle_timeout(
                socket,
                "127.0.0.1:0".parse().unwrap(),
                "device-a".into(),
                1,
                cancel_rx,
                Duration::from_millis(500),
            )
            .await
    });
    // Connect but never send anything (a silent peer).
    let _client = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .unwrap()
        .0;
    tokio::time::timeout(Duration::from_secs(1), async {
        while handle.snapshot("device-a").is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    // A request issued against the silent link must fail once the link is
    // declared dead, not wait out its own (longer) deadline.
    let request = handle.request(
        "device-a",
        1,
        "companion.status",
        serde_json::json!({}),
        true,
        Duration::from_secs(5),
    );
    let (request_result, server_result) = tokio::join!(
        request,
        tokio::time::timeout(Duration::from_secs(2), server)
    );
    let error = server_result.expect("server must stop on idle timeout").unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
    assert_eq!(request_result, Err(DirectRuntimeError::Disconnected));
    assert!(handle.snapshot("device-a").is_none());
}
```

The 500 ms idle window (rather than something tighter) keeps a small race
negligible: the idle timer starts when the link is inserted, so if the test
took longer than the window to issue the request, the request would see
`NotConnected` instead of `Disconnected`; keep the strict `Disconnected`
assertion.

Also add the liveness counterpart, `live_socket_scheduler_treats_pings_as_liveness`:
same setup with a 500 ms idle timeout; `let mut client = ...connect_async(...).await.unwrap().0;`
then a loop that sends `tokio_tungstenite::tungstenite::Message::Ping(Default::default())`
every 100 ms for 1.5 s via `client.send(..).await.unwrap()`; after the loop
assert `!server.is_finished()` (the link survived three idle windows of
pings); then `drop(client)` and
`tokio::time::timeout(Duration::from_secs(1), server).await.expect("server must stop after peer EOF").unwrap().unwrap();`
(EOF is a clean `Ok(())`).

`request`'s parameters are `(device_id, expected_epoch, method, params, idempotent, deadline)`
(lines 386-394); it needs only a `links` entry with a matching epoch, no
capabilities or method registration, and the pending entry is inserted
before the send, so the teardown's `Disconnected` reaches it.

**Verify**: `cargo nextest run -p sky-cua-service phone::direct::tests` → all
pass including the 2 new tests; the idle test completes in under 2 s.

### Step 4: Full crate run

**Verify**: `cargo fmt --check` → exit 0;
`cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-service` → all pass.

## Test plan

- `live_socket_scheduler_times_out_a_silent_link_and_fails_pending_requests`
  and `live_socket_scheduler_treats_pings_as_liveness` (Step 3).
- Existing `phone::direct::tests` unchanged and passing; the EOF/flood
  scheduler tests double as regression coverage for the select-arm edits.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-service --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-service` exits 0 with the 2 new tests
- [ ] `grep -c 'DIRECT_LINK_IDLE_TIMEOUT' crates/sky-cua-service/src/phone/direct/mod.rs` is `2` (the definition and the wrapper's call)
- [ ] `grep -c 'idle.as_mut().reset' crates/sky-cua-service/src/phone/direct/mod.rs` is `2` (both select blocks)
- [ ] `grep -c 'timeout_at(idle.deadline(), socket.send' crates/sky-cua-service/src/phone/direct/mod.rs` is `4`
- [ ] `grep -c 'timeout_at(send_deadline' crates/sky-cua-service/src/phone/direct/mod.rs` is `2`
- [ ] `git diff --name-only e928a40 -- crates/` prints only `crates/sky-cua-service/src/phone/direct/mod.rs`
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- An existing scheduler test fails after Step 1 (the select arms were
  mis-edited; the flood test is the sensitive one).
- `tokio::pin!` or `Sleep::reset` are unavailable (they are in tokio 1.48's
  `time` feature, which the workspace enables).
- The idle test cannot observe `Disconnected` on the pending request because
  `request()` rejects the device before enqueueing (report what gate it hit).
- The ping-liveness test is flaky on the dev host: do not raise the ping
  interval or lower the test idle timeout; report timings instead.

## Maintenance notes

- `DIRECT_LINK_IDLE_TIMEOUT` is tied to the companion's 20 s ping. If the
  companion's `pingInterval` changes, change this constant with it (keep it
  at roughly three missed pings plus slack).
- A link killed by the idle timeout is reported as `io::ErrorKind::TimedOut`
  to the accept path; the companion reconnects on its own and
  re-authenticates with a new epoch. Pending requests fail with
  `Disconnected`, which the manager already treats as "not retried, caller
  decides".
- The `timeout_at` change means a request's deadline now covers the enqueue
  wait. If a caller passes a very short deadline (hundreds of ms) while the
  control queue is briefly full, it may now see `DeadlineExceeded` where
  before it would have blocked longer and then succeeded; that is the
  intended contract.
- A reviewer should check both `select!` blocks received identical arms (in
  the order cancel, socket.next, idle, sends), that all four sends are
  wrapped, and that the `idle` future is polled only inside `select!` (a
  pinned `Sleep` that is never polled never fires).
- Plan 002 cites `release_content_artifact` at `mod.rs:211-231`; this plan's
  new constant near line 40 shifts those lines down by about 8. That is
  expected drift for 002, not a semantic change.
