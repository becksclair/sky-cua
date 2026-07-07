# Plan 004: Stop the client retry path from double-executing non-idempotent actions

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-client/src/service_launcher.rs crates/sky-cua-platform/src/model/service.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED — this deliberately trades some auto-recovery for
  at-most-once semantics on mutating actions; the classification must be
  right or legitimate recovery regresses
- **Depends on**: none (002/003 recommended first for the safety net)
- **Category**: bug
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

The MCP client's service call path retries failed requests by re-sending the
identical request — in two places. For idempotent requests (health, listing,
screenshots) that's correct recovery. For mutating requests
(`ExecuteAction` = clicks/keystrokes/drags, mutating `Browser`/`Phone`
sub-requests), a failure like "connection closed before response" or a 60s
read timeout can occur *after the daemon already executed the action*. The
retry then executes it a second time: the agent sees one tool call, the
machine receives two clicks. Double-clicking a "Send" button or
double-typing text into a form is a real-world corruption, not a
theoretical race.

## Current state

- `crates/sky-cua-client/src/service_launcher.rs:247-259` — outer retry:

  ```rust
  pub fn call(&self, request: &ServiceRequest) -> Result<ServiceResponse> {
      match self.call_with_timeouts(request, SERVICE_READ_TIMEOUT, SERVICE_WRITE_TIMEOUT) {
          Ok(response) => Ok(response),
          Err(first_error) => {
              self.reap_exited_child()?;
              let launch_environment = self.recovery_launch_environment();
              self.spawn_service(&launch_environment)?;
              self.wait_for_startup_health(&launch_environment)...?;
              self.call_with_timeouts(request, ...)   // <-- unconditional re-send
          }
      }
  }
  ```

- Inner retry, same file (~:330-350): when the *cached* stream returns a
  stale-classified error, the call is re-attempted on a fresh connection:

  ```rust
  // If we got a non-stale error from the cached stream, return it directly
  // rather than retrying with a fresh connect.
  if let Some(Err(error)) = cached_response { return Err(error); }
  // Attempt 2: fresh connection.
  let stream = self.endpoint.connect()?;
  let (response, stream, owner_pid) = self.perform_call_on_stream(stream, request, ...)?;
  ```

- `is_stale_stream_error` (:820-828) classifies retryable errors by string:
  "broken pipe", "connection refused", "connection reset", "connection
  closed before response", "not connected", "unexpected eof". Of these,
  **"connection refused"** provably precedes daemon receipt; **"broken
  pipe"/"connection reset" on write** usually precede receipt but a reset
  after a completed write does not; **"connection closed before response"
  and "unexpected eof"** occur strictly *after* a successful write — the
  daemon may well have executed the request.
- `perform_call_on_stream` (:355-375) writes the full request, then reads one
  response line. There is no request id, so the daemon cannot dedup.
- `ServiceRequest` variants (`crates/sky-cua-platform/src/model/service.rs:15-66`):
  `Health, Doctor, SetupAccessibility, SetupWindowTargeting,
  LaunchApplication{..}, ListApps, ListWindows, FocusedWindow,
  ActivateWindow{..}, GetAppState{..}, Screenshot{..}, ResetPortalTokens,
  AgentCursorStatus, SetAgentCursor{..}, HideAgentCursor{..},
  ShowAgentCursor, Browser{request}, Phone{request},
  SessionPresence{action}, ExecuteAction{request}`.
- `BrowserRequest` and `PhoneRequest` are enums in the same model module
  family (`crates/sky-cua-platform/src/model/` — grep `enum BrowserRequest`
  and `enum PhoneRequest`); they mix read-only operations (status, snapshot,
  list) with mutating ones (click, input, navigate, gesture, app install).

## Design (decided by the advisor — implement as specified)

Add a `fn is_idempotent(&self) -> bool` classification on `ServiceRequest`
(and delegating helpers on `BrowserRequest`/`PhoneRequest`), then gate both
retry sites:

1. **Idempotent requests**: behavior unchanged (retry after respawn, retry on
   fresh connection).
2. **Mutating requests**: retry ONLY when the first failure provably
   precedes daemon receipt. Implement this by threading a failure-stage
   signal out of `perform_call_on_stream`: wrap its error as an enum or
   `anyhow` context marker distinguishing `FailedBeforeWrite` (connect,
   set_timeout, serialization, or the write itself failed) from
   `FailedAfterWrite` (write+flush succeeded; the read failed). Only
   `FailedBeforeWrite` errors are retryable for mutating requests. A
   respawn-then-retry for a mutating request is allowed only when the
   original failure was `FailedBeforeWrite` (e.g. connection refused because
   the daemon is down).
3. On a non-retryable mutating failure, return an error whose message states
   the ambiguity explicitly, e.g.:
   `action may or may not have executed: response was lost after the request was sent ({underlying}); not retrying a non-idempotent action — observe the current state before repeating it`.
   The agent-facing wording matters: it tells the model to re-observe
   instead of blindly re-clicking.

Classification table (implement exactly; when in doubt a variant is
NON-idempotent):

- Idempotent: `Health, Doctor, ListApps, ListWindows, FocusedWindow,
  GetAppState, Screenshot, AgentCursorStatus, SetAgentCursor,
  HideAgentCursor, ShowAgentCursor, SetupAccessibility,
  SetupWindowTargeting` (setups converge to the same state),
  `ActivateWindow` (focus-set converges).
- Non-idempotent: `ExecuteAction, LaunchApplication, ResetPortalTokens,
  SessionPresence`.
- `Browser{request}` / `Phone{request}`: delegate. Read every variant of
  both enums and classify: reads/status/snapshot/list/observe → idempotent;
  click/input/keys/scroll/navigate/tab-create/claim, gesture/tap/swipe/
  text/key/app-action/install/notification-action → non-idempotent.
  `navigate` and `scroll` are debatable; classify them non-idempotent
  (navigation triggers side effects; scroll accumulates).

Do NOT implement daemon-side request-id dedup in this plan — it's the more
invasive alternative; noted as deferred in Maintenance.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Client + platform tests | `cargo nextest run -p sky-cua-client -p sky-cua-platform` | all pass |
| Whole workspace | `cargo nextest run` | all pass |
| Format | `cargo fmt --check` | exit 0 |

## Scope

**In scope**:
- `crates/sky-cua-client/src/service_launcher.rs` — retry gating, failure-stage
  threading, tests
- `crates/sky-cua-platform/src/model/service.rs` — `is_idempotent` on
  `ServiceRequest` (pure additive method, no serde/wire change)
- The files defining `BrowserRequest`/`PhoneRequest` (same model family) —
  additive `is_idempotent` helpers only

**Out of scope** (do NOT touch):
- The daemon (`sky-cua-service`) — no request-id/dedup protocol changes.
- `is_stale_stream_error`'s string list (still used for *cache invalidation*
  — deciding to drop a cached stream is orthogonal to deciding to re-send).
- Wire format: `is_idempotent` must not add serde attributes or variants.
- `spawn_service` / health-wait logic.

## Git workflow

- Branch: `bex/advisor-004-retry-idempotency`
- Commits: `feat(platform): classify service requests by idempotency`, then
  `fix(client): never blind-retry non-idempotent requests after an ambiguous failure`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Classification methods

Add `impl ServiceRequest { pub fn is_idempotent(&self) -> bool { ... } }` per
the table above, with delegating impls on `BrowserRequest`/`PhoneRequest`.
Use exhaustive `match` (no `_ =>` arm) so new variants force a decision at
compile time. Write a unit test in the same file asserting the table for a
representative sample of every group.

**Verify**: `cargo nextest run -p sky-cua-platform` → pass, incl. new test.

### Step 2: Thread failure stage out of `perform_call_on_stream`

Introduce (client-side, private) `enum CallFailure { BeforeWrite(anyhow::Error), AfterWrite(anyhow::Error) }`
and change `perform_call_on_stream`'s error type (or wrap at the call
sites): connect/serialize/write/flush errors → `BeforeWrite`; read/parse
errors → `AfterWrite`. Keep the public `call()`/`call_with_timeouts`
signatures returning `anyhow::Result` — convert at the boundary.

**Verify**: `cargo build -p sky-cua-client` → exit 0.

### Step 3: Gate both retry sites

- Inner (fresh-connection attempt after a stale cached stream): allow attempt
  2 if `request.is_idempotent() || matches!(failure, BeforeWrite(_))`.
- Outer (`call()` respawn path): same predicate against the first failure.
  When the predicate is false, return the ambiguity error described in
  Design §3 (include the underlying error with `{first_error}` context, as
  the current code does).

**Verify**: `cargo nextest run -p sky-cua-client` → all pass.

### Step 4: Tests for the retry semantics

`service_launcher.rs` already has tests (grep `mod tests` / `is_stale_stream_error`
tests near :820). Following their style, add:

- idempotent request + AfterWrite failure → retried (assert two send
  attempts against a fake/mock endpoint if the existing tests have one;
  if no endpoint fake exists, test the predicate function directly:
  extract the gate into `fn should_retry(request: &ServiceRequest, failure: &CallFailure) -> bool`
  and table-test it).
- `ExecuteAction` + AfterWrite → NOT retried, error message contains
  "may or may not have executed".
- `ExecuteAction` + BeforeWrite (connection refused) → retried.
- `Browser{click}` non-idempotent vs `Browser{status}` idempotent routing.

**Verify**: `cargo nextest run -p sky-cua-client -E 'test(should_retry)'` → pass.

## Test plan

Covered in steps 1 and 4. Minimum: exhaustive-match classification test,
4 gate tests. Pattern: existing `service_launcher.rs` test module.

## Done criteria

- [ ] `cargo fmt --check && cargo nextest run` exits 0
- [ ] `ServiceRequest::is_idempotent` uses an exhaustive match (no `_` arm) — `grep -A40 "fn is_idempotent" crates/sky-cua-platform/src/model/service.rs` shows no `_ =>`
- [ ] The ambiguity error string is grep-able: `grep -rn "may or may not have executed" crates/sky-cua-client/` → ≥1 hit
- [ ] No wire-format change: `git diff crates/sky-cua-platform | grep -E "serde|rename"` → empty
- [ ] No files outside the in-scope list modified
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- `BrowserRequest`/`PhoneRequest` have variants whose mutating/read-only
  nature you cannot determine from their handlers — list them and stop
  rather than guessing.
- The existing test suite has integration tests that depend on blind retry
  of a mutating request (a test starts failing because retry no longer
  happens) — that's a semantic decision for the maintainer.
- Threading `CallFailure` requires changing a public API consumed outside
  `sky-cua-client`.

## Maintenance notes

- Deferred deliberately: daemon-side request-id dedup, which would restore
  safe auto-retry for mutating requests. If ambiguous-failure errors show up
  frequently in practice, that's the follow-up (client mints a UUID per call;
  daemon keeps a small LRU of executed ids → cached responses).
- New `ServiceRequest`/`BrowserRequest`/`PhoneRequest` variants now force an
  idempotency decision at compile time — reviewers must sanity-check each
  new classification.
- The agent-facing skills (`skills/computer-use/SKILL.md`) may eventually
  want a line about "response lost" errors meaning re-observe-first; out of
  scope here.
