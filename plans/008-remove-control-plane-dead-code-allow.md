# Plan 008: Remove the blanket dead-code suppressions on the browser control plane and fix what they hid

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-service/src/browser/control_plane crates/sky-cua-service/src/browser/transport.rs crates/sky-cua-service/src/browser/protocol.rs crates/sky-cua-service/src/phone/direct/lan.rs`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, re-run the measurement in Step 0 and compare
> it with the list in "Current state" before proceeding; a different list is
> not drift (fix what the compiler reports), but a missing file or renamed
> module is a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tech-debt
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

`crates/sky-cua-service/src/browser/control_plane/mod.rs` opens with
`#![allow(dead_code, unused_imports)] // Staged behind the fake executor until WP-04 routes ingress.`
That attribute covers a 16,191-line subtree (29 files) and was meant to be
temporary. WP-04 has shipped: the control plane is called from production
code (`browser/transport.rs:336`, `browser/probe.rs:31,158`,
`browser/executor.rs:46-49`, `daemon/mod.rs:188`). Three item-level
`#[allow(dead_code)] // Forwarded and consumed when WP-04 wires control ingress.`
in `browser/transport.rs` carry the same stale justification for functions
that are now called. A fourth module-level `#![allow(dead_code)]` sits on
`phone/direct/lan.rs` next to an `#[allow(unused_imports)]`.

While these stay, the compiler cannot report dead code in the largest and
fastest-changing service subsystem, and every other cleanup there is
guesswork. A forced-warning build on 2026-11-03
(`cargo rustc -p sky-cua-service --bin sky-cua-service -- --force-warn dead_code --force-warn unused_imports`
into a scratch target dir) shows exactly what the control-plane allow hides:
9 unused-import warnings and 5 dead-item warnings, listed below. The fix is
mechanical: delete the suppressions, delete the unused imports, and for each
dead item either delete it or, if tests use it, gate it with the repository's
own idiom `#[cfg_attr(not(test), expect(dead_code))]` so the attribute
self-removes once production uses the item.

The much larger `#![allow(dead_code)]` on `phone/protocol/mod.rs:21` (65
hidden warnings, mostly method-name constants in `protocol/mod.rs` and
`protocol/dto.rs`) is deliberately NOT in this plan; see Maintenance notes.

## Current state

- `crates/sky-cua-service/src/browser/control_plane/mod.rs` lines 1-7:

  ```rust
  //! Daemon-local browser scheduling and ownership core.
  //!
  //! This module deliberately has no dependency on the current browser transport.
  //! A later integration layer can implement [`Executor`] for the persistent bridge
  //! actor without moving scheduling or lease policy into that actor.

  #![allow(dead_code, unused_imports)] // Staged behind the fake executor until WP-04 routes ingress.
  ```

- `crates/sky-cua-service/src/browser/transport.rs` lines 24, 32, 43: three
  `#[allow(dead_code)] // Forwarded and consumed when WP-04 wires control ingress.`
  on `BrowserControlMode::uses_persistent_actor` (line 25), the free fn
  `browser_control_mode()` (line 33) and `browser_control_mode_error` (line 44).
  All three are used in production: `daemon/mod.rs:188,193,285`,
  `daemon/browser.rs:81`, `browser/executor.rs:47`, and
  `browser_control_mode_error` is called inside `browser_control_mode`.

- `crates/sky-cua-service/src/phone/direct/lan.rs` lines 1 and 7-8:

  ```rust
  #![allow(dead_code)]
  ...
  #[allow(unused_imports)]
  use super::{is_private_ip, validate_bind_addr};
  ```

  `is_private_ip` is used at line 45; `validate_bind_addr` only appears in a
  doc comment (147) and inside the unused `#[cfg(test)]` free function
  `candidates_are_bindable` (lines 148-154, doc comment 145-147), so the
  non-test build reports it as an unused import. `lan.rs` also has a second
  unused `#[cfg(test)]` helper, `clear_lan_cache_for_tests` (lines 84-91).
  `grep -rn 'clear_lan_cache_for_tests\|candidates_are_bindable' crates/`
  prints only the two definitions; the test module `mod tests` (line 157,
  `use super::*;`) calls neither. Both become dead-code errors in the test
  target once the blanket allow is gone.

- `crates/sky-cua-service/src/browser/protocol.rs:29-32`: two constants each
  carry their own `#[allow(dead_code)]`:
  `RESOLVE_ELEMENT_REQUEST_ID` (line 30, attribute line 29) and
  `ELEMENT_FOCUS_REQUEST_ID` (line 32, attribute line 31). At `e928a40`
  `RESOLVE_ELEMENT_REQUEST_ID` IS used (`browser/resolve.rs:62,108,559`), so
  its attribute is stale and only the attribute goes; `ELEMENT_FOCUS_REQUEST_ID`
  has no reference outside its definition (the forced-warning build reports
  it), so the constant and its attribute both go. Step 0 re-checks both.

- What the allows hide. Two measurements are needed because the gate runs
  `clippy --all-targets`, which builds the bin target (where test-only items
  show as dead) and the test target (where only truly dead items show).

  Non-test build (`--bin sky-cua-service`), 16 locations:

  Unused imports:
  - `bridge_actor.rs:31:74` `Handshake`
  - `scheduler/state.rs:9:78` `OperationClass`, `Principal`
  - `integration.rs:17:75` `BrowserProvenanceSource`
  - `integration.rs:22:26` `mpsc`
  - `integration.rs:53:5` `settle_actor_message`, `settle_late_response`
  - `mod.rs:29:73` `BridgeActorHealth`, `BridgeRequestSize`
  - `mod.rs:38:24` `IDLE_LEASE_MS`, `LeaseState`
  - `mod.rs:43:5` `SettlementState`
  - `mod.rs:45:30` `RecoveryGroupHint`, `RecoveryJournal`
  - `phone/direct/lan.rs:8:28` `validate_bind_addr`

  Dead items:
  - `control.rs:89` enum `Command` variants `SettlementState`, `Offer`,
    `Accept`, `Force`, `ResumeRecovered` never constructed
  - `control.rs:159` struct `ControlPlane` fields `generation`, `persistence`
    never read
  - `control.rs:173` `impl ControlPlane`: `recover`, `generation`, `cancel`,
    `settlement_state`, `offer_handoff`, `accept_handoff`, `force_handoff`,
    `resume_recovered` never used
  - `operation.rs:109` enum `SettlementOutcome` variants
    `ProvenPreDispatchFailure`, `TargetLost` never constructed
  - `integration/runtime.rs:29` `BrowserControlRuntime::new` never used
  - `browser/protocol.rs:32` `ELEMENT_FOCUS_REQUEST_ID` never used

  Test build (`--profile test`), 10 in-scope locations:
  - `integration.rs:31:82` unused import `persistence` (its only user is the
    `#[cfg(not(test))] fn default_recovery_path` in `integration/support.rs:25`,
    via `super::persistence::RECOVERY_JOURNAL_FILE`)
  - `integration_tests.rs:6:20` unused import `mpsc`
  - `integration_tests.rs:1894:29` unused import `GroupId`
  - `control.rs:291:19` method `generation` never used
  - `lan.rs` `clear_lan_cache_for_tests` and `candidates_are_bindable` never used
  - plus the imports above that are unused in both builds

  Resolved worklist at `e928a40` (an item in the non-test list but NOT the
  test list is test-only; an item in both is dead):
  - **Delete**: imports `Handshake`, `OperationClass`, `Principal`,
    `BrowserProvenanceSource`, `mpsc` (integration.rs), `BridgeActorHealth`,
    `mpsc` and `GroupId` (integration_tests.rs); the `ControlPlane.generation`
    field (remove only the `generation,` struct-literal lines around
    control.rs 205, 250, 285; keep the `generation` constructor parameters,
    `EventRecorder::new(generation.clone())` at 202/248 still uses them) and
    the `generation()` method (control.rs:291); `ELEMENT_FOCUS_REQUEST_ID`
    with its attribute; the two `lan.rs` helpers.
  - **Gate re-exports with `#[cfg(test)]`** (split the `use` list if needed):
    `settle_actor_message`, `settle_late_response` (integration.rs:53; keep
    `spawn_actor_events` ungated); from mod.rs `BridgeRequestSize`,
    `IDLE_LEASE_MS`, `LeaseState`, `SettlementState`, `RecoveryGroupHint`,
    `RecoveryJournal`.
  - **Move the `persistence` import**: drop `persistence` from
    `integration.rs:31`'s `use super::{...}` and change
    `integration/support.rs:25` to `super::super::persistence::RECOVERY_JOURNAL_FILE`
    (the const is `pub(super)` in `control_plane::persistence`, so the path
    resolves).
  - **`#[cfg_attr(not(test), expect(dead_code))]`** (test-only): variants
    `Command::{SettlementState, Offer, Accept, Force, ResumeRecovered}` and
    `SettlementOutcome::{ProvenPreDispatchFailure, TargetLost}`; field
    `ControlPlane.persistence` (read only by `#[cfg(test)] fn flush_persistence`,
    control.rs:505-509); methods `ControlPlane::{recover, cancel, settlement_state, offer_handoff, accept_handoff, force_handoff, resume_recovered}`;
    `BrowserControlRuntime::new` (integration/runtime.rs:29). Several of these
    are used only from in-file `#[cfg(test)]` code, not from the test files,
    which is why grepping test files is not the rule.

  Line numbers in `mod.rs` shift up by one after deleting line 7; match by
  identifier, not line.

- Repository idiom for test-only items (several existing uses, e.g.
  `phone/command.rs:74`, `phone/direct/mod.rs:2143`):
  `#[cfg_attr(not(test), expect(dead_code))]`. Using `expect` rather than
  `allow` means the attribute itself becomes a warning once the item is used
  in production, so it cannot go stale the way these did.

- Gate: `cargo clippy --workspace --all-targets -- -D warnings` is part of
  `just verify-rust` (`justfile:10`), so any surviving warning fails the gate.
  Commit style: conventional, e.g. `fix(service): hard-cap daemon log generations and back off accept errors`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Measure, non-test build (read-only, scratch target) | `CARGO_TARGET_DIR=/tmp/sky-cua-deadcode-target cargo rustc -p sky-cua-service --bin sky-cua-service -- --force-warn dead_code --force-warn unused_imports 2>&1 \| grep -E '^\s+--> ' \| grep -E 'control_plane\|transport.rs\|direct/lan.rs\|browser/protocol.rs'` | 16 lines (14 control-plane, 1 lan.rs, 1 protocol.rs) |
| Measure, test build | same command with `--profile test` inserted after `--bin sky-cua-service` | 10 lines (8 control-plane, 2 lan.rs) |
| Format | `cargo fmt --check` | exit 0 |
| Lint (the real gate) | `cargo clippy -p sky-cua-service --all-targets -- -D warnings` | exit 0 |
| Service tests | `cargo nextest run -p sky-cua-service` | all pass (786 at `e928a40`) |
| Remaining allows in scope | `grep -rn 'allow(dead_code\|allow(unused_imports' crates/sky-cua-service/src/browser/control_plane crates/sky-cua-service/src/browser/transport.rs crates/sky-cua-service/src/browser/protocol.rs crates/sky-cua-service/src/phone/direct/lan.rs` | prints nothing (only `clippy::too_many_arguments` allows remain in transport.rs) |

## Scope

**In scope** (the only files you should modify):
- `crates/sky-cua-service/src/browser/control_plane/**/*.rs`, including the
  test files (`integration_tests.rs`, `tests.rs`, `bridge_actor_tests.rs`,
  `integration_tests/`) but only to delete unused `use` items
- `crates/sky-cua-service/src/browser/transport.rs` (remove three attributes)
- `crates/sky-cua-service/src/browser/protocol.rs` (lines 29-32 only: two allow-suppressed constants, deleted only if Step 0 shows them unreferenced)
- `crates/sky-cua-service/src/phone/direct/lan.rs` (remove two attributes, fix the import, delete two unused test helpers)

**Out of scope** (do NOT touch):
- `crates/sky-cua-service/src/phone/protocol/**` and its
  `#![allow(dead_code)]`. 65 warnings, mostly protocol constants whose
  fate (delete vs keep as documented wire vocabulary) is a maintainer
  decision. Named as a follow-up below.
- Any behavioural change. Deleting an unused enum variant is allowed only if
  nothing constructs or matches it; do not rewrite logic to "use" a dead item.
- Public serde wire shapes in `sky-cua-platform`.
- Test files' logic. Removing a now-dead `use` in a test file is fine; do not
  delete tests.

## Git workflow

- Branch: `bex/control-plane-dead-code`
- Commit only if the operator authorised it; style:
  `refactor(service): drop the stale control-plane dead-code allow`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 0: Confirm the measurement

Run both measurement commands from the table (non-test and `--profile test`).
Record both lists. They should match the 16 + 10 locations in "Current
state". Classify each item with the rule: in non-test list only → test-only
(gate); in both → dead (delete). If there are additional in-scope locations,
add them to your worklist with the same rule. Also
run `grep -rn 'RESOLVE_ELEMENT_REQUEST_ID\|ELEMENT_FOCUS_REQUEST_ID' crates/ --include='*.rs'`
and note, for each, whether it has any reference besides its definition
(at `e928a40`: `RESOLVE_ELEMENT_REQUEST_ID` is used in `browser/resolve.rs`;
`ELEMENT_FOCUS_REQUEST_ID` is not).

**Verify**: the command prints the expected locations; `cargo clippy -p sky-cua-service --all-targets -- -D warnings` exits 0 at this point (baseline).

### Step 1: Remove the suppressions

1. Delete line 7 of `browser/control_plane/mod.rs` (the `#![allow(...)]`).
2. Delete the three `#[allow(dead_code)] // Forwarded and consumed when WP-04 wires control ingress.`
   lines in `browser/transport.rs` (24, 32, 43).
3. In `phone/direct/lan.rs` delete line 1 (`#![allow(dead_code)]`) and line 7
   (`#[allow(unused_imports)]`); change line 8 to `use super::is_private_ip;`.
   Then delete the two unused `#[cfg(test)]` helpers:
   `clear_lan_cache_for_tests` (lines 84-91) and `candidates_are_bindable`
   together with its doc comment (lines 145-154). Nothing references either.
   With both gone, `validate_bind_addr` needs no import anywhere in `lan.rs`.

**Verify**: `cargo clippy -p sky-cua-service --all-targets -- -D warnings` now
FAILS. Expected at `e928a40`: about 26 lint errors across two targets (bin:
14; bin test: 12), all at locations in the two Step 0 lists. Record the exact
list. If any location is outside the in-scope files, STOP.

### Step 2: Resolve each warning with the decision rule

For each location reported in Step 1:

- **Unused import reported in both builds** → delete the named item(s) from
  the `use` list.
- **Unused re-export reported only in the non-test build** → it is used by
  tests: gate it with `#[cfg(test)]`, splitting the `use` list if needed.
- **Dead item reported only in the non-test build** → test-only: add
  `#[cfg_attr(not(test), expect(dead_code))]` directly above the item, with
  a one-line `//` reason naming where tests use it. Do not rely on grepping
  the test files: several of these items are used only from in-file
  `#[cfg(test)]` code (`control.rs:505-509`, test-only `ControlPlane`
  methods).
- **Dead item reported in both builds** → delete it. For an enum variant,
  also delete any `match` arms for it. For a struct field never read, delete
  the field and its struct-literal initialisers (but keep constructor
  parameters that other code still reads; see the `generation` note in
  "Current state").
- The resolved worklist in "Current state" already applies this rule at
  `e928a40`; follow it unless Step 0 showed a difference.
- **`RESOLVE_ELEMENT_REQUEST_ID` / `ELEMENT_FOCUS_REQUEST_ID`** in
  `browser/protocol.rs:29-32`: for a constant Step 0 found referenced
  elsewhere, delete only its `#[allow(dead_code)]` line (the attribute is
  stale); for one with zero references outside its definition, delete the
  constant and the attribute. At `e928a40` that means: keep
  `RESOLVE_ELEMENT_REQUEST_ID` minus its attribute; delete
  `ELEMENT_FOCUS_REQUEST_ID` and its attribute.

Work one file at a time and re-run clippy after each file so the error list
shrinks monotonically. Run `cargo fmt` at the end.

**Verify**: `cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0.
`cargo nextest run -p sky-cua-service` → all pass (count unchanged: 786 at `e928a40`).

### Step 3: Confirm nothing else was suppressed by accident

**Verify**:
- The "Remaining allows in scope" command from the table → prints nothing.
- `grep -rn '^#!\[allow(dead_code' crates/sky-cua-service/src` → exactly two
  lines, `phone/protocol/mod.rs:21` and `phone/manager/tests.rs:6` (both out
  of scope).
- `grep -rn 'allow(unused_imports' crates/sky-cua-service/src` → exactly two
  lines, `phone/protocol/mod.rs:30` and `phone/manager/mod.rs:54` (both out
  of scope).
- `grep -rn 'WP-04' crates/sky-cua-service/src` → no matches on attribute
  lines (doc prose mentioning WP-04 may remain).
- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0 (the full gate).

## Test plan

No new tests. The existing control-plane suites
(`browser::control_plane::tests`, `integration_tests`, `bridge_actor_tests`)
are the regression net; they must pass unchanged and the test count must not
drop.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-service` exits 0 with the same test count as before (786 at `e928a40`)
- [ ] `grep -rn 'allow(dead_code\|allow(unused_imports' crates/sky-cua-service/src/browser/control_plane crates/sky-cua-service/src/browser/transport.rs crates/sky-cua-service/src/browser/protocol.rs crates/sky-cua-service/src/phone/direct/lan.rs` prints nothing
- [ ] `grep -rn '^#!\[allow(dead_code' crates/sky-cua-service/src` prints exactly the `phone/protocol/mod.rs:21` and `phone/manager/tests.rs:6` lines
- [ ] `grep -c 'allow(dead_code)' crates/sky-cua-service/src/browser/transport.rs` is `0`
- [ ] `git status --porcelain -- crates/` lists only files under the in-scope list (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The combined count of distinct file:line locations in the two Step 0
  measurements exceeds 35, or any location is outside the in-scope files
  (at `e928a40`: 16 + 10, all in scope).
- The decision rule says delete (not gate) for any item whose name contains
  `Journal`, `Settlement`, `Recovery`, `Recover` or `Handoff`. At `e928a40`
  no such item is a deletion; all of them are test-gated.
- Deleting an enum variant would require changing a `match` in a way that
  alters behaviour for other variants.
- Any test fails or the test count drops after your change.
- You find yourself editing anything under `phone/protocol/`.

## Maintenance notes

- Follow-up (`DEBT-01` remainder): `phone/protocol/mod.rs:21`
  `#![allow(dead_code)]` hides 65 warnings: 43 in `protocol/mod.rs`,
  18 in `protocol/dto.rs`, 2 each in `protocol/identity.rs` and
  `protocol/client.rs`. Most are method-name constants and DTO structs the
  doc comment says exist "until the integrator wires the client into
  `manager.rs`". Whether they are dead or documented wire vocabulary needs a
  maintainer's call before an executor touches them. Measure with the Step 0
  command filtered on `phone/protocol`.
- The forced-warning measurement is the right tool for the remaining
  item-level allows too (about 60 `allow`/`expect(dead_code)` sites in the
  service crate); `--force-warn` overrides every allow, so filter its output
  by file to audit any of them without editing.
- A reviewer should check that every new `cfg_attr(not(test), expect(dead_code))`
  names the test that uses the item, and that no `match` arm was removed for
  a variant that is still constructed in a test.
