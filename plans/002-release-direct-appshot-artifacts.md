# Plan 002: Release direct-lane AppShot content artifacts so the phone link stops dying after 16-32 observes

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-service/src/phone/manager/appshot.rs crates/sky-cua-service/src/phone/direct/content_transfer.rs crates/sky-cua-service/src/phone/direct/mod.rs crates/sky-cua-service/src/phone/direct/provider.rs crates/sky-cua-service/src/phone/manager/features.rs crates/sky-cua-service/src/phone/manager/tests.rs`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts below are abridged (rustfmt multi-line code is shown
> condensed); formatting differences are not drift, changed behaviour is.
> If plan 017 landed first, `direct/mod.rs` line numbers are shifted down by
> about 8 (a new constant near line 40); that is expected drift.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

When a phone is connected over the CompanionDirect (direct-LAN websocket) lane,
every `phone_observe` asks the companion for an AppShot. The companion ships
the screenshot (and, for large trees, the full accessibility tree) as content
transfers that the host commits into a per-link `InboundContentStore`. The
host AppShot producer reads those artifacts and copies them into its own
artifact directory, but it never releases them. The store caps committed
artifacts at 32 and expires entries only lazily, when that exact `content_id`
is looked up again. A failed commit returns `Err` from `handle_inbound`,
which breaks the socket loop and tears down the authenticated link. Net
effect: after 16 to 32 observes on one link, the next commit fails, the link
drops, every pending request fails with `Disconnected`, and the phone has to
reconnect and re-authenticate. The companion sends a separate
`full_tree_content_ref` only when the windows JSON exceeds 256 KiB
(`android/.../direct/DirectDispatcher.kt:190`), so the typical case is one
artifact per observe (link dies on observe 33) and the worst case two (dies on
observe 17). Lazy expiry never rescues them: their random UUID content ids are
never looked up again.

A second, smaller accounting bug in the same store makes the byte cap
(256 MiB) drift: re-committing a `content_id` that is already committed
overwrites the entry (and the file, whose name is derived from the id) but
adds the new size to `committed_bytes` without subtracting the old one. This
one is latent: the real companion uses random UUID content ids
(`ContentTransfer.kt:49,65`), so only test or fake producers re-use ids.

## Current state

- `crates/sky-cua-service/src/phone/manager/appshot.rs` — the direct-lane
  AppShot producer (`PhoneManager::direct_appshot`, starts line 28). The
  companion response is bound at `let value = provider.dispatch(...)...?;`
  (statement ends line 61). Lines 127-159 today (abridged):

  ```rust
  let tree_bytes = if let Some(reference) = value.get("full_tree_content_ref") {
      read_direct_content(provider, reference, &device_id, epoch)?
  } else {
      serde_json::to_vec(&value).map_err(|e| DiagnosticEntry { ... })?
  };
  let tree = write_artifact(&id, "tree.json", "application/json", &tree_bytes, &device_id, epoch)?;
  let image =
      if let Some(reference) = value.get("screenshot").and_then(|v| v.get("content_ref")) {
          let bytes = read_direct_content(provider, reference, &device_id, epoch)?;
          let mime = ...unwrap_or("image/png");
          write_artifact(&id, "screen", mime, &bytes, &device_id, epoch)?
      } else {
          return Err(DiagnosticEntry { code: "PhoneAppShotPartial".into(), ... });
      };
  ```

  `read_direct_content` (lines 316-344) parses the JSON into a `ContentRef`,
  checks `device_id`/`link_epoch`, then calls
  `provider.runtime().read_content_artifact(device_id, epoch, &reference)`.
  `provider.runtime()` returns an owned clone of `DirectRuntimeHandle`
  (`crates/sky-cua-service/src/phone/direct/provider.rs:22-24`), and
  `crate::phone::direct::DirectRuntimeHandle` is the public path (the tests
  import it that way at `manager/tests.rs:29`). Nothing in this file calls
  `release_content_artifact`; confirm with
  `grep -n release_content_artifact crates/sky-cua-service/src/phone/manager/appshot.rs`
  (no matches at `e928a40`). Every early `?` return between line 61 and the
  final `Ok(AppShotEnvelope {..})` (tree read, tree write, screenshot read,
  screenshot write, dimension checks, mapping) is an exit path on which the
  companion artifacts must also be released.

- `crates/sky-cua-service/src/phone/direct/mod.rs` — `DirectRuntimeHandle`.
  Lines 211-231 define the release API you will call:

  ```rust
  pub(crate) fn release_content_artifact(
      &self,
      device_id: &str,
      epoch: u64,
      content_id: &str,
  ) -> io::Result<()>
  ```

  It locks the link, checks the epoch, and calls
  `InboundContentStore::release_artifact`. Lines 757-762 show why a failed
  commit is fatal: `.commit(commit, epoch)?` inside `handle_inbound`, whose
  `Err` makes the `serve_socket` loop `break` (lines 646-647, 674-675) and
  fail every pending request with `Disconnected` (711-713). Each link has its
  own store (line 626), so epoch checks on release are sound. The content
  tools in `crates/sky-cua-service/src/phone/manager/features.rs:111-116`
  call `runtime.release_content_artifact(&device_id, epoch, &content_id)`
  only on an explicit `phone_content` Release request; that is the intended
  lifecycle for camera/storage content.

- `crates/sky-cua-service/src/phone/direct/content_transfer.rs` — the store.
  - Lines 24-25: `const MAX_COMMITTED_ARTIFACTS: usize = 32;` and
    `const MAX_COMMITTED_BYTES: u64 = 256 * 1024 * 1024;`
  - Lines 54-60: `InboundContentStore { active, committed: HashMap<String, CommittedArtifact>, active_bytes, committed_bytes, now_override }`.
    `CommittedArtifact` (lines 36-43) has `path, sha256, mime_type, size, epoch, expires_at_ms`.
  - `commit` (ends around line 282). The capacity check and insert, lines 258-281:

    ```rust
    if self.committed.len() >= MAX_COMMITTED_ARTIFACTS
        || self.committed_bytes.saturating_add(d.content.size_bytes) > MAX_COMMITTED_BYTES
    {
        return Err(io::Error::other("committed artifact bound exceeded"));
    }
    fs::rename(&transfer.temp_path, &final_path)?;
    self.committed_bytes = self.committed_bytes.saturating_add(d.content.size_bytes);
    let expires = d.content.expires_at_ms.unwrap_or_else(|| self.now().saturating_add(15 * 60 * 1000));
    self.committed.insert(d.content.content_id.clone(), CommittedArtifact { ... });
    Ok(d.content.content_id.clone())
    ```

  - `remove_if_expired` (lines 400-415) is the only expiry path; it is
    per-`content_id` and only runs when that id is described or read.
  - `release_artifact` (lines 348-367) removes the entry, subtracts its size
    and deletes the file. Reuse its body for the new eviction helper.
  - Tests live inline at the bottom of the file (`mod tests`, 7 tests). The
    test `expired_artifact_is_removed_and_capacity_released` (line 648) shows
    the fixture helpers `declaration(size, sha)`, `chunk(transfer_id, index, offset, chunk_bytes, bytes)`,
    `s.set_now(..)` and how to drive `declare`/`chunk`/`commit`. Note that
    `declaration()` (lines 435-455) hard-codes `transfer_id: "t"`,
    `content_id: "shot"` and `chunk_bytes = 3`; tests that need distinct ids
    must override `d.transfer_id` and `d.content.content_id`, and a payload
    over 3 bytes needs more than one `chunk(...)` call.

- `crates/sky-cua-service/src/phone/manager/tests.rs` — manager tests. The
  direct-lane AppShot exemplar is
  `direct_observe_registers_its_appshot_as_an_actionable_phone_snapshot`
  (line 775). It uses `DirectRuntimeHandle::register_fake("direct-device", 9)`
  and the helper `direct_appshot_wire_result(&direct, appshot_id, device_id, epoch, package, seq)`
  (line 735), which calls `direct.commit_fake_content(device_id, epoch, &format!("{appshot_id}-screen"), "image/png", ContentSource::Screenshot, &[0])`
  and embeds the returned `ContentRef` as `screenshot.content_ref`. So after
  an observe in that test, the committed artifact id is `"<appshot_id>-screen"`.
  `commit_fake_content` writes into the same per-link store that
  `release_content_artifact` removes from (mod.rs 266, 351, 227-230), with a
  15-minute fake expiry (mod.rs 330), so before the fix the artifact is still
  describable after the observe. `DirectRuntimeHandle::describe_content_artifact(device_id, epoch, content_id)`
  (mod.rs lines 189-209) returns `Err` once an artifact is gone.

Conventions: errors in this crate are `DiagnosticEntry` with a stable `code`
string (see `read_direct_content`). Tests run with `cargo nextest run`. Commit
messages are conventional, for example
`fix(browser): ack directly replied settlements so the host queue drains`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-service --all-targets -- -D warnings` | exit 0 |
| Store tests | `cargo nextest run -p sky-cua-service phone::direct::content_transfer` | all pass |
| Manager tests | `cargo nextest run -p sky-cua-service phone::manager` | all pass |
| Crate tests | `cargo nextest run -p sky-cua-service` | all pass (786 at `e928a40`, plus yours) |

## Scope

**In scope** (the only files you should modify):
- `crates/sky-cua-service/src/phone/manager/appshot.rs`
- `crates/sky-cua-service/src/phone/direct/content_transfer.rs`
- `crates/sky-cua-service/src/phone/manager/tests.rs` (add tests)

**Out of scope** (do NOT touch):
- `crates/sky-cua-service/src/phone/direct/mod.rs`. Making a commit failure a
  non-fatal error reply instead of a link teardown is a protocol-behaviour
  change; it is noted under Maintenance, not done here.
- `MAX_COMMITTED_ARTIFACTS` / `MAX_COMMITTED_BYTES` values. Raising caps
  would hide the leak rather than fix it.
- The Android companion (`android/phone-companion`).
- `features.rs` content tools; they already release correctly.

## Git workflow

- Branch: `bex/direct-appshot-artifact-release`
- Commit only if the operator authorised it; style:
  `fix(phone): release direct AppShot content artifacts after copying them`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Evict expired artifacts before the capacity check, and fix duplicate-id accounting

In `content_transfer.rs`:

1. Add a private method on `InboundContentStore`:

   ```rust
   /// Drop every committed artifact whose expiry has passed, releasing its
   /// bytes and file. Called before capacity checks so lazily-expired entries
   /// cannot consume slots.
   fn evict_expired(&mut self) {
       let now = self.now();
       let expired: Vec<String> = self
           .committed
           .iter()
           .filter(|(_, artifact)| artifact.expires_at_ms <= now)
           .map(|(id, _)| id.clone())
           .collect();
       for id in expired {
           if let Some(artifact) = self.committed.remove(&id) {
               self.committed_bytes = self.committed_bytes.saturating_sub(artifact.size);
               let _ = fs::remove_file(artifact.path);
           }
       }
   }
   ```

2. In `commit`, replace the capacity check and the byte accounting (today
   lines 258-266: the `if self.committed.len() >= MAX_COMMITTED_ARTIFACTS ...`
   block, `fs::rename`, and `self.committed_bytes = ...saturating_add(...)`)
   with:

   ```rust
   self.evict_expired();
   let previous_size = self.committed.get(&d.content.content_id).map(|a| a.size);
   let slots = self.committed.len() - usize::from(previous_size.is_some());
   let projected = self
       .committed_bytes
       .saturating_sub(previous_size.unwrap_or(0))
       .saturating_add(d.content.size_bytes);
   if slots >= MAX_COMMITTED_ARTIFACTS || projected > MAX_COMMITTED_BYTES {
       return Err(io::Error::other("committed artifact bound exceeded"));
   }
   fs::rename(&transfer.temp_path, &final_path)?;
   if let Some(previous) = self.committed.remove(&d.content.content_id) {
       self.committed_bytes = self.committed_bytes.saturating_sub(previous.size);
       // final_path is derived from content_id, so the rename above already
       // replaced the old file; nothing else to delete.
   }
   self.committed_bytes = self.committed_bytes.saturating_add(d.content.size_bytes);
   ```

   Keep the `expires` computation and the `self.committed.insert(...)` that
   follow unchanged. The `usize::from(bool)` subtraction cannot underflow
   because the key is present whenever `previous_size` is `Some`.

3. Add two tests to the inline `mod tests`, modelled on
   `expired_artifact_is_removed_and_capacity_released`:
   - `commit_evicts_expired_artifacts_before_capacity_check`: call
     `s.set_now(0)`, then loop `i in 0..MAX_COMMITTED_ARTIFACTS`: `let bytes = [i as u8];`,
     `let sha = format!("{:x}", Sha256::digest(bytes));`,
     `let mut d = declaration(1, &sha);`, `d.transfer_id = format!("evict-{i}");`,
     `d.content.content_id = format!("evict-content-{i}");`,
     `d.content.expires_at_ms = Some(10);`, then `s.declare(d, 3).unwrap()`,
     `s.chunk(&chunk(&format!("evict-{i}"), 0, 0, 3, &bytes), 3).unwrap()`,
     `s.commit(ContentTransferCommit { transfer_id: format!("evict-{i}"), size_bytes: 1, sha256: sha, link_epoch: 3 }, 3).unwrap()`.
     After the loop assert `s.committed.len() == MAX_COMMITTED_ARTIFACTS`.
     Then `s.set_now(10)` and commit a fresh artifact (`transfer_id "fresh"`,
     `content_id "fresh-content"`, 3 bytes `b"abc"`, `expires_at_ms` left
     `None`) via declare/chunk/commit; assert the commit is `Ok`,
     `s.committed.len() == 1`, and `s.committed_bytes == 3`. (Distinct ids
     matter: with the duplicate-id fix in place, 32 commits under the
     fixture's default id would collapse into one entry and the test would
     pass without exercising eviction.)
   - `recommit_same_content_id_does_not_double_count`: first commit
     `transfer_id "dup-1"`, `content_id "dup"`, 3 bytes `b"abc"`, one chunk;
     second commit `transfer_id "dup-2"`, same `content_id "dup"`, 5 bytes
     `b"abcde"` with two chunks `chunk("dup-2", 0, 0, 3, b"abc")` and
     `chunk("dup-2", 1, 3, 3, b"de")` (the fixture's `chunk_bytes` is 3);
     assert `s.committed.len() == 1` and `s.committed_bytes == 5`.

**Verify**: `cargo nextest run -p sky-cua-service phone::direct::content_transfer`
→ all pass, including the 2 new tests (9 total in that module).

### Step 2: Add the regression test first (so you can watch it fail)

In `crates/sky-cua-service/src/phone/manager/tests.rs`, extend the existing
test `direct_observe_registers_its_appshot_as_an_actionable_phone_snapshot`
(attribute at line 772, fn at 773): after the observe completes and its
existing assertions pass, add

```rust
assert!(
    direct
        .describe_content_artifact("direct-device", 9, "<the appshot_id passed to direct_appshot_wire_result>-screen")
        .is_err(),
    "screenshot artifact must be released after the host copies it"
);
```

using the same `appshot_id` string literal the test passes to
`direct_appshot_wire_result` and the same device id / epoch it passes to
`register_fake`. Do not copy the test; extending it keeps the file smaller and
the assertion lands in the flow that actually produces the artifact.

**Verify**: `cargo nextest run -p sky-cua-service direct_observe_registers_its_appshot_as_an_actionable_phone_snapshot`
→ FAILS on the new assertion (the artifact is still described). If it passes
here, the test is not exercising the release path: STOP.

### Step 3: Release every referenced artifact on every exit path

In `appshot.rs`, add a drop guard so the release happens on success and on
every early `?` return (a failing tree write or screenshot digest mismatch
would otherwise still leak, and 32 failing observes inside the 15-minute
expiry window would still kill the link):

```rust
/// Releases every companion content artifact an AppShot response referenced,
/// on every exit path. The host copies what it needs into its own artifact
/// directory, so the per-link store copy is never read again.
struct ReleaseDirectContent {
    runtime: crate::phone::direct::DirectRuntimeHandle,
    device_id: String,
    epoch: u64,
    content_ids: Vec<String>,
}

impl Drop for ReleaseDirectContent {
    fn drop(&mut self) {
        for content_id in &self.content_ids {
            if let Err(error) =
                self.runtime
                    .release_content_artifact(&self.device_id, self.epoch, content_id)
            {
                tracing::debug!(%error, %content_id, "direct AppShot artifact release failed");
            }
        }
    }
}
```

Immediately after the `let value = provider.dispatch(...)...?;` statement
(ends line 61), insert:

```rust
let _release = ReleaseDirectContent {
    runtime: provider.runtime(),
    device_id: device_id.clone(),
    epoch,
    content_ids: [
        value.get("full_tree_content_ref"),
        value.get("screenshot").and_then(|v| v.get("content_ref")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|r| r.get("content_id").and_then(serde_json::Value::as_str).map(str::to_owned))
    .collect(),
};
```

The binding must be named `_release`, not `_`, so it lives to the end of the
function. Do not change `read_direct_content`. `tracing` is already a
dependency (`tracing.workspace = true` in `crates/sky-cua-service/Cargo.toml`).
Run `cargo fmt` after editing (the snippets above are not guaranteed to be
rustfmt-shaped).

**Verify**: `cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-service direct_observe_registers_its_appshot_as_an_actionable_phone_snapshot`
→ passes.

### Step 4: Full module runs

**Verify**: `cargo nextest run -p sky-cua-service phone::direct::content_transfer`
→ all pass (9 tests: 7 existing + 2 new);
`cargo nextest run -p sky-cua-service phone::manager` → all pass;
`cargo nextest run -p sky-cua-service` → all pass.

## Test plan

- `content_transfer.rs`: `commit_evicts_expired_artifacts_before_capacity_check`,
  `recommit_same_content_id_does_not_double_count` (Step 1).
- `manager/tests.rs`: the extended
  `direct_observe_registers_its_appshot_as_an_actionable_phone_snapshot`
  (Step 2), observed to fail before Step 3 and pass after.
- Structural pattern: existing tests in the same modules.
- Verification: `cargo nextest run -p sky-cua-service` → all pass.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-service --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-service` exits 0; `cargo nextest list -p sky-cua-service --message-format json | python3 -c "import json,sys; d=json.load(sys.stdin); print(sum(len(s['testcases']) for s in d['rust-suites'].values()))"` prints 788 (786 at `e928a40` plus 2)
- [ ] `grep -c 'release_content_artifact' crates/sky-cua-service/src/phone/manager/appshot.rs` ≥ 1 and `grep -n 'struct ReleaseDirectContent' crates/sky-cua-service/src/phone/manager/appshot.rs` matches
- [ ] `grep -n 'evict_expired' crates/sky-cua-service/src/phone/direct/content_transfer.rs` shows the definition and a call inside `commit`
- [ ] `git status --porcelain -- crates/` lists exactly `crates/sky-cua-service/src/phone/manager/appshot.rs`, `crates/sky-cua-service/src/phone/direct/content_transfer.rs`, `crates/sky-cua-service/src/phone/manager/tests.rs` (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code (drift).
- `release_content_artifact` or `describe_content_artifact` no longer exist
  with the signatures shown.
- The Step 2 assertion passes before Step 3 is implemented (then the test is
  not exercising the release path; report rather than weaken it).
- A fix seems to require changing `handle_inbound` in `direct/mod.rs`.
- Any previously-passing `phone::` test fails after your change.

## Maintenance notes

- `handle_inbound` still treats a failed `ContentCommit` as fatal to the
  link. After this plan the cap is reached only by more than 32 unreleased
  artifacts inside their 15-minute expiry window: camera or storage content
  the agent never releases through `phone_content` Release
  (`DirectDispatcher.kt:160-174` commits those into the same store), or
  AppShot responses that arrive after the 5 s `DIRECT_APPSHOT_RPC_TIMEOUT`
  (`appshot.rs:21`) and orphan their artifacts until expiry. A follow-up could
  answer an over-cap commit with an error frame instead of dropping the link;
  that is a protocol decision for the maintainer.
- If a future AppShot variant references more content refs (for example a
  separate region crop), add the new ref to `ReleaseDirectContent.content_ids`
  at construction. A reviewer should check that every `content_ref`-shaped
  field the producer reads is listed there.
- Live verification that was not possible during planning: run
  `python3 scripts/live_phone_use_smoke.py` against a direct-lane device and
  take 40 consecutive observes; the link must stay connected.
