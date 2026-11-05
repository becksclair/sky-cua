# Plan 005: Bound the desktop and browser AppShot registries so daemon memory stops growing per observe

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40..HEAD -- crates/sky-cua-service/src/snapshot_manager.rs crates/sky-cua-service/src/browser/control_plane/integration/executor.rs crates/sky-cua-service/src/daemon/mod.rs crates/sky-cua-service/src/daemon/desktop.rs crates/sky-cua-service/src/daemon/appshot.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

Every desktop observe, destination capture and recovery stores a full
`AppShotEnvelope` (including its `semantic_projection`, often hundreds of KB
for a large AT-SPI tree) in `SnapshotManager::appshots`, a `HashMap` with no
eviction. The sibling `snapshots` map in the same struct is capped at 8 with
an insertion-order ring (a re-store refreshes position; reads do not). The browser control plane keeps a second, process-global
`HashMap` of AppShot envelopes (`REGISTRY` in `integration/executor.rs`) that
also only inserts. The phone manager prunes its map by link epoch; desktop and
browser never prune. Memory therefore grows linearly with the number of
observes for the daemon's lifetime, which for a long-lived service is
unbounded.

Safety of eviction differs between the two maps, so the caps differ:

- Desktop validation accepts an AppShot only when its `action_snapshot` is
  the latest snapshot, and every desktop AppShot capture stores a fresh
  snapshot first (`daemon/appshot.rs:150`) and binds the envelope to it
  (`:615`). The only AppShot that can be accepted is therefore the most
  recently inserted one, so a cap of 64 never changes an accepted desktop
  action. For an evicted desktop id the rejection reason becomes `Stale`
  instead of `Stale`/`WrongSession`, and the recovery AppShot is captured
  from the frontmost window instead of the original AppShot's window
  (`daemon/mod.rs:931-935`). This only affects ids more than 64 captures
  old, which are already stale.
- Browser AppShots are validated by target, tab, session and document
  generation (`executor.rs:42-60`), not by snapshot recency, and the
  registry is one process-wide map shared by every session and tab. A
  browser AppShot stays valid while its tab's document generation is
  unchanged, so evicting one that is still valid is possible if many other
  captures intervene; the result is an `AppShotRequired` rejection with
  reason `Missing` (`executor.rs:104-106`) carrying a fresh AppShot, one
  extra round trip, never a wrong mutation. The browser cap is therefore
  deliberately generous (256) so that this needs hundreds of intervening
  browser captures.

## Current state

- `crates/sky-cua-service/src/snapshot_manager.rs` (full file is 220 lines;
  production is lines 1-69):

  ```rust
  #[derive(Debug, Default)]
  pub struct SnapshotManager {
      snapshots: HashMap<String, AppStateSnapshot>,
      order: VecDeque<String>,
      max_snapshots: usize,
      appshots: HashMap<String, AppShotEnvelope>,
  }

  impl SnapshotManager {
      pub fn new(max_snapshots: usize) -> Self { ... max_snapshots: max_snapshots.max(1), appshots: HashMap::new() }

      pub fn store(&mut self, snapshot: AppStateSnapshot) {
          let snapshot_id = snapshot.snapshot_id.clone();
          self.snapshots.insert(snapshot_id.clone(), snapshot);
          self.order.retain(|id| id != &snapshot_id);
          self.order.push_back(snapshot_id);
          while self.order.len() > self.max_snapshots {
              if let Some(oldest) = self.order.pop_front() {
                  self.snapshots.remove(&oldest);
              }
          }
      }
      ...
      pub fn store_appshot(&mut self, appshot: AppShotEnvelope) {
          self.appshots.insert(appshot.appshot_id.clone(), appshot);
      }

      pub fn appshot(&self, appshot_id: &str) -> Option<&AppShotEnvelope> {
          self.appshots.get(appshot_id)
      }
  }
  ```

  Inline tests: `evicts_old_snapshots` and
  `registers_canonical_appshot_by_id_and_replaces_previous_capture`, with a
  fixture fn `appshot(id, window_id, snapshot_id) -> AppShotEnvelope`.

- Construction: `crates/sky-cua-service/src/daemon/mod.rs:205`
  `snapshots: tokio::sync::Mutex::new(SnapshotManager::new(8))`.

- Insert sites: `daemon/desktop.rs:396`, `:659`, `:698`; `daemon/mod.rs:950`.
  Lookup sites: `daemon/desktop.rs:434`, `:545`, `:600`; `daemon/mod.rs:873`
  (inside `validate_cua_context_appshot`, lines 864-955). That validator's
  logic:

  ```rust
  match context.appshot_id.as_deref().and_then(|id| snapshots.appshot(id)) {
      None if context.appshot_id.is_none() => (AppShotRejectionReason::Missing, None, Vec::new()),
      None => (AppShotRejectionReason::Stale, None, Vec::new()),
      Some(appshot) => {
          ...
          if snapshots.is_latest(&appshot.action_snapshot.snapshot_id) && session_ok {
              (AppShotRejectionReason::WrongTarget, target, focus_window_ids)
          } else if !session_ok { (WrongSession, ...) } else { (Stale, ...) }
      }
  }
  ```

  So only AppShots whose `action_snapshot.snapshot_id` is still the latest of
  the 8 snapshots can ever be accepted; everything older is already `Stale`.
  All four lookup sites require latest-snapshot membership: `desktop.rs:434`
  only infers a `snapshot_id` and then runs `validate_action_appshot`;
  `desktop.rs:545` is gated by `get_if_latest`; `desktop.rs:600` requires
  `appshot_snapshot_is_latest`; `mod.rs:901` requires `is_latest`.

- `crates/sky-cua-service/src/daemon/appshot.rs`: `handle_appshot_capture`
  calls `snapshots.store(snapshot)` at line 150 before returning an envelope
  whose `appshot_id = request_id` (line 602) and
  `action_snapshot.snapshot_id` is that snapshot's id (line 615). Its callers
  (`desktop.rs:396, 659, 698`; `mod.rs:950`) store the envelope immediately
  afterwards, so each capture makes its own snapshot the latest. Phone and
  browser AppShots never enter `SnapshotManager::appshots`; `store_appshot`
  has only those four callers.

- `crates/sky-cua-service/src/browser/control_plane/integration/executor.rs`
  lines 13-31:

  ```rust
  #[derive(Clone)]
  struct RegisteredAppShot { envelope: AppShotEnvelope, target: BrowserTargetKind }

  fn appshot_registry() -> &'static Mutex<HashMap<String, RegisteredAppShot>> {
      static REGISTRY: OnceLock<Mutex<HashMap<String, RegisteredAppShot>>> = OnceLock::new();
      REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
  }

  fn register_appshot(envelope: AppShotEnvelope, target: BrowserTargetKind) {
      if let Ok(mut registry) = appshot_registry().lock() {
          registry.insert(envelope.appshot_id.clone(), RegisteredAppShot { envelope, target });
      }
  }
  ```

  Register sites: lines 121, 376, 441, 486. The single lookup is in
  `require_appshot` (lines 62-75): `registry.get(id).cloned()`. An unknown
  id falls through to `AppShotRejectionReason::Missing` (lines 104-106).
  Browser AppShot ids are `uuid::Uuid::new_v4()` (`browser/bridge.rs:794`,
  `:1175`), so there is no id reuse to worry about. The file has an inline
  test module `mod appshot_fence_tests` (the `#[cfg(test)]` attribute is on
  line 689, the module opens on line 690) with four `#[test]`s and a fixture
  `entry(target, tab, session, generation) -> RegisteredAppShot` that
  hard-codes `appshot_id: "shot"`.

Conventions: keep public contracts in `sky-cua-platform` unchanged; tests via
`cargo nextest run`; conventional commits, e.g.
`fix(service): hard-cap daemon log generations and back off accept errors`
(`a92ae3e`, which did the same kind of bounding for log files).

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-service --all-targets -- -D warnings` | exit 0 |
| Snapshot tests | `cargo nextest run -p sky-cua-service snapshot_manager` | all pass |
| Browser executor tests | `cargo nextest run -p sky-cua-service browser::control_plane::integration::executor` | all pass |
| Crate tests | `cargo nextest run -p sky-cua-service` | all pass |

## Scope

**In scope** (the only files you should modify):
- `crates/sky-cua-service/src/snapshot_manager.rs`
- `crates/sky-cua-service/src/browser/control_plane/integration/executor.rs`

**Out of scope** (do NOT touch):
- `daemon/mod.rs`, `daemon/desktop.rs` — the insert/lookup call sites keep
  their signatures; no change needed there.
- The phone AppShot map (`phone/manager/capture_screenshot.rs`, `host.rs`);
  it already prunes by epoch.
- On-disk capture file pruning under `$XDG_RUNTIME_DIR/sky-cua/captures`
  (audit finding `PERF-08` file half). Separate change.
- `sky-cua-platform` model types.

## Git workflow

- Branch: `bex/bound-appshot-registries`
- Commit style: `fix(service): bound the desktop and browser AppShot registries`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Give `SnapshotManager::appshots` an insertion-order cap

In `snapshot_manager.rs`:

1. Add a constant and two fields:

   ```rust
   /// Multiplier on `max_snapshots` giving the retained AppShot cap. Only the
   /// AppShot from the most recent capture can be accepted, so the retained
   /// tail only preserves `WrongSession` reasons and exact-window recovery
   /// targets for recent ids.
   const APPSHOTS_PER_SNAPSHOT: usize = 8;
   ```

   and in the struct: `appshot_order: VecDeque<String>, max_appshots: usize`.
   In `new`, set `max_appshots: max_snapshots.max(1) * APPSHOTS_PER_SNAPSHOT`
   (64 for the daemon's `new(8)`). Change `#[derive(Debug, Default)]` to
   `#[derive(Debug)]`: nothing constructs `SnapshotManager` via `Default`
   (`grep -rn 'SnapshotManager::default' crates/` prints nothing at
   `e928a40`; every construction is `SnapshotManager::new(..)`), so add no
   replacement impl. `Debug` stays derivable because `VecDeque<String>` is
   `Debug`.

2. Rewrite `store_appshot` to mirror `store` (insertion-order ring; a
   re-store of an existing id refreshes its position, reads do not):

   ```rust
   pub fn store_appshot(&mut self, appshot: AppShotEnvelope) {
       let appshot_id = appshot.appshot_id.clone();
       self.appshots.insert(appshot_id.clone(), appshot);
       self.appshot_order.retain(|id| id != &appshot_id);
       self.appshot_order.push_back(appshot_id);
       while self.appshot_order.len() > self.max_appshots {
           if let Some(oldest) = self.appshot_order.pop_front() {
               self.appshots.remove(&oldest);
           }
       }
   }
   ```

3. Add a test `evicts_old_appshots_beyond_cap`: `SnapshotManager::new(1)`
   (cap 8), store 9 distinct appshots via the existing `appshot(...)` fixture,
   assert the first id is gone, the ninth is present, and
   `manager.appshots.len() == 8` (tests are in the same module and can read
   the private field). Also assert re-storing an existing id does not grow
   the count (the existing replacement test covers the value; add the length
   assertion there).

**Verify**: `cargo nextest run -p sky-cua-service snapshot_manager` → all pass
(3 tests).

### Step 2: Bound the browser registry

In `executor.rs`:

1. Replace the registry value type with a small struct that carries order:

   ```rust
   /// Global cap across all sessions and tabs. Browser AppShots stay valid
   /// while their tab's document generation is unchanged, so this is
   /// deliberately generous; an evicted id is rejected as `Missing` with a
   /// fresh AppShot attached.
   const MAX_REGISTERED_APPSHOTS: usize = 256;

   #[derive(Default)]
   struct AppShotRegistry {
       entries: HashMap<String, RegisteredAppShot>,
       order: VecDeque<String>,
   }

   impl AppShotRegistry {
       fn insert(&mut self, envelope: AppShotEnvelope, target: BrowserTargetKind) {
           let id = envelope.appshot_id.clone();
           self.entries.insert(id.clone(), RegisteredAppShot { envelope, target });
           self.order.retain(|existing| existing != &id);
           self.order.push_back(id);
           while self.order.len() > MAX_REGISTERED_APPSHOTS {
               if let Some(oldest) = self.order.pop_front() {
                   self.entries.remove(&oldest);
               }
           }
       }
       fn get(&self, id: &str) -> Option<&RegisteredAppShot> { self.entries.get(id) }
   }
   ```

   Replace `appshot_registry()` with:

   ```rust
   fn appshot_registry() -> &'static Mutex<AppShotRegistry> {
       static REGISTRY: OnceLock<Mutex<AppShotRegistry>> = OnceLock::new();
       REGISTRY.get_or_init(|| Mutex::new(AppShotRegistry::default()))
   }
   ```

   Change `register_appshot` to call `registry.insert(envelope, target)`. The
   lookup in `require_appshot`,
   `registry.get(id).cloned()`, compiles unchanged (the guard derefs to
   `AppShotRegistry`, and `RegisteredAppShot` already derives `Clone`).
   Change the import on line 1 to `use std::collections::{HashMap, VecDeque};`.

2. Add a test inside the existing `mod appshot_fence_tests` (opens line 690).
   Its `entry(...)` fixture hard-codes `appshot_id: "shot"`, so override the
   id per iteration or every insert collapses into one entry:

   ```rust
   #[test]
   fn registry_evicts_oldest_beyond_cap() {
       let mut registry = AppShotRegistry::default();
       for index in 0..=MAX_REGISTERED_APPSHOTS {
           let mut shot = entry(BrowserTargetKind::UserChrome, "tab-1", "session-1", 7);
           shot.envelope.appshot_id = format!("shot-{index}");
           registry.insert(shot.envelope, shot.target);
       }
       assert!(registry.get("shot-0").is_none());
       assert!(registry.get(&format!("shot-{MAX_REGISTERED_APPSHOTS}")).is_some());
       assert_eq!(registry.entries.len(), MAX_REGISTERED_APPSHOTS);
       assert_eq!(registry.order.len(), MAX_REGISTERED_APPSHOTS);
   }
   ```

   Use a local `AppShotRegistry::default()`, not the global, so the test
   stays independent of the other fence tests.

**Verify**: `cargo nextest run -p sky-cua-service browser::control_plane::integration::executor` → all pass (5 tests).

### Step 3: Full crate run

**Verify**: `cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-service` → all pass. At `e928a40` neither
`daemon::tests` nor `browser::control_plane::integration_tests` stores more
than a handful of AppShots per test, so nothing approaches either cap; if a
test does fail with an unexpected `Stale`/`Missing` reason, that is a STOP
condition.

## Test plan

- `snapshot_manager.rs`: `evicts_old_appshots_beyond_cap`; extend
  `registers_canonical_appshot_by_id_and_replaces_previous_capture` with a
  length assertion.
- `executor.rs`: `registry_evicts_oldest_beyond_cap`.
- Verification: `cargo nextest run -p sky-cua-service` → all pass.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-service --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-service` exits 0 with the 2 new tests
- [ ] `grep -n 'appshot_order' crates/sky-cua-service/src/snapshot_manager.rs` matches
- [ ] `grep -n 'MAX_REGISTERED_APPSHOTS' crates/sky-cua-service/src/browser/control_plane/integration/executor.rs` matches
- [ ] `git status --porcelain -- crates/` lists exactly `crates/sky-cua-service/src/snapshot_manager.rs` and `crates/sky-cua-service/src/browser/control_plane/integration/executor.rs` (the untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- A `daemon` or `integration_tests` test fails because it relied on an
  AppShot older than the cap being found. Report the test; do not raise the
  cap silently.
- `grep -rn 'SnapshotManager::default' crates/` prints a match (the derive
  removal would then break a caller; report it).
- The fix appears to require touching `daemon/mod.rs` or `daemon/desktop.rs`.

## Maintenance notes

- `APPSHOTS_PER_SNAPSHOT = 8` is a heuristic: observe, destination and
  recovery captures can each create an AppShot per snapshot. If a future
  flow creates more per snapshot, raise the multiplier rather than removing
  the bound.
- `MAX_REGISTERED_APPSHOTS = 256` is a global cap across sessions. If many
  concurrent browser agents share one daemon and start seeing `Missing`
  rejections for AppShots they just captured, the next step is a
  per-`(target, tab_id, session_id)` cap rather than a larger global one.
- The companion half of audit finding `PERF-08`, orphaned full-frame PNGs in
  the tmpfs captures directory after targeted AppShots and no pruning for
  `capture_desktop`, is not addressed here and still costs RAM on tmpfs.
- A reviewer should confirm the browser registry's `OnceLock` global remains
  the only shared state and that tests use a local `AppShotRegistry`, not the
  global, so nextest's per-process isolation is not the only thing keeping
  them deterministic.
