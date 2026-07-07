# Plan 008: Housekeeping sweep (retire root TODO files, drop unused x11rb, fix the README Windows contradiction, targeted dead_code allows, track untracked god files)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- Cargo.toml README.md ROADMAP.md TODO_IMPROVE_CODEBASE.md TODO_PERFORMANCE.md IMPROVE_PERFORMANCE.md crates/sky-cua-client/src/isolated_desktop.rs crates/sky-cua-overlay-host/src/renderer/mod.rs`
> On any in-scope drift, re-verify the excerpts below; on mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: S-M (five independent S items)
- **Risk**: LOW (docs/manifest/lint hygiene; one compile-visible change)
- **Depends on**: none
- **Category**: tech-debt / docs
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

Five small pieces of accumulated drift: 129KB of *completed* backlog files
sit at repo root in violation of the repo's own Document Hierarchy rules;
an unused dependency pin misleads upgrades; the README's headline Windows
limitation describes a state that shipped months ago as still missing;
blanket `#![allow(dead_code)]` turns off the compiler's dead-code signal for
whole modules; and five god files past the repo's ~800-line threshold aren't
on the tracked backlog.

## Current state

1. **Stale root files**: `TODO_IMPROVE_CODEBASE.md` (all items ICA-001..018
   marked complete), `TODO_PERFORMANCE.md` (header: "13 of 13 critical/high
   implemented... 0 remain"), `IMPROVE_PERFORMANCE.md` (a security-measure
   performance inventory — research, not a TODO). `CLAUDE.md`/`AGENTS.md`
   "Document Hierarchy" says: README/ROADMAP/NOTES/docs//plans only, "Do not
   invent parallel structures", NOTES is "not stale TODO lists"; research
   belongs in `docs/research/YYYY-MM-<slug>.md`. `ROADMAP.md` already links
   `TODO_PERFORMANCE.md` from the "Performance and runtime tuning" phase —
   that link must be updated when the file moves.
2. **Unused dep**: `Cargo.toml:73` —
   `x11rb = { version = "0.13.2", features = ["shape", "xfixes"] }`.
   Verified: zero references in any `crates/*/Cargo.toml`, zero in source,
   zero in `Cargo.lock`.
3. **README contradiction**: `README.md:337-339`:
   ```
   - Windows v1 is intentionally conservative: it exposes real top-level window
     bounds and physical actions, but does not yet provide rich UI Automation
     child trees or semantic invoke/value routing.
   ```
   Reality: `ROADMAP.md` marks "[x] Windows UIA inspection and semantic
   actions" shipped; `crates/sky-cua-windows/src/backend.rs:355-533` wires
   UIA child-tree collection and semantic actions. The true residual gap is
   readback: `uia.rs:628-637` hardcodes `text: None, numeric_value: None,
   supports_editable_text: false` (tracked as unchecked ROADMAP item
   "Native Windows/UIA readback"). README "What Works" (:55-56) likewise
   understates Windows.
4. **Blanket allows**: `#![allow(dead_code)]` at module scope in
   `crates/sky-cua-client/src/isolated_desktop.rs:26`,
   `crates/sky-cua-client/src/isolated_desktop/probe.rs:13`,
   `crates/sky-cua-client/src/isolated_desktop/owned_bus.rs:18`,
   `crates/sky-cua-overlay-host/src/renderer/mod.rs:1` (with
   `renderer/cursor_texture.rs:9` re-overriding locally to get warnings
   back — proof the blanket is too coarse).
5. **Untracked god files** (non-test line estimates): `daemon.rs` ~1,415 in
   one `impl ServiceDaemon`; `windows/backend.rs` ~2,225;
   `virtual_input.rs` ~1,616; `chrome-host/host.rs` ~1,148; `uia.rs` ~995;
   plus `mcp_tools/definitions.rs` 4,395 (that one gets its own plan 009).
   ROADMAP's "Code quality / Ultra-review backlog" lists only
   `linux/backend.rs`, `capture_plan.rs`, `mcp_tools.rs`, and a `displays/`
   split.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Build | `cargo build` | exit 0 |
| Tests | `cargo nextest run` | all pass |
| Format | `cargo fmt --check` | exit 0 |
| Lock check | `cargo build 2>&1 \| grep -i x11rb` | empty |

## Scope

**In scope**:
- Delete/move: `TODO_IMPROVE_CODEBASE.md`, `TODO_PERFORMANCE.md`,
  `IMPROVE_PERFORMANCE.md`; create up to two files under `docs/research/`
- `Cargo.toml` (one line removal)
- `README.md` (Windows sections), `ROADMAP.md` (link fix + backlog
  additions)
- `crates/sky-cua-client/src/isolated_desktop.rs`, `.../probe.rs`,
  `.../owned_bus.rs`, `crates/sky-cua-overlay-host/src/renderer/mod.rs`,
  `.../renderer/cursor_texture.rs` — allow-attribute changes only
- Deleting items the compiler proves dead inside those modules (see Step 4
  boundary)

**Out of scope**:
- Splitting any god file (tracking only; `definitions.rs` split is plan 009).
- Any behavior change. Step 4 may delete *provably unused* items only —
  if an item is referenced by cfg-gated or platform-specific code, keep it
  with a targeted allow.
- Rewriting README beyond the Windows lines; NOTES.md.

## Git workflow

- Branch: `bex/advisor-008-housekeeping`
- One commit per numbered item, style: `docs(hierarchy): retire completed root backlogs`,
  `build(deps): drop unused x11rb pin`, `docs(readme): correct the Windows capability status`,
  `refactor(lint): replace blanket dead_code allows with targeted ones`,
  `docs(roadmap): track remaining god files`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Retire the root backlog files

- `git rm TODO_IMPROVE_CODEBASE.md TODO_PERFORMANCE.md` — both are fully
  closed; git history is the archive (matches `plans/AGENTS.md` lifecycle
  philosophy: no shadow archives).
- `IMPROVE_PERFORMANCE.md` is research, not a TODO: move its content to
  `docs/research/2026-06-performance-affecting-security-inventory.md`
  (dated per the repo's research naming rule; June 2026 is when it was
  produced per git log — confirm with `git log --follow --format=%as -- IMPROVE_PERFORMANCE.md | tail -1`
  and use that date), then `git rm IMPROVE_PERFORMANCE.md`.
- Update `ROADMAP.md`'s link to `TODO_PERFORMANCE.md` ("Deep performance
  review backlog closed — [TODO_PERFORMANCE.md]") to point at the git
  history instead: replace the link with plain text naming the file and its
  closing commit (`git log --oneline -1 -- TODO_PERFORMANCE.md`).
- Grep for any other references: `grep -rn "TODO_PERFORMANCE\|TODO_IMPROVE_CODEBASE\|IMPROVE_PERFORMANCE" --include='*.md' --include='*.py' .`
  (excluding `advisor-plans/`) and fix each.

**Verify**: the grep above (excluding `advisor-plans/` and `.git`) → no hits;
`ls TODO_*.md IMPROVE_PERFORMANCE.md 2>/dev/null` → nothing.

### Step 2: Drop x11rb

Delete `Cargo.toml:73` (the `x11rb` workspace dependency line).

**Verify**: `cargo build` → exit 0; `grep -rn x11rb Cargo.toml Cargo.lock crates/` → empty.

### Step 3: Fix the README Windows status

- Replace `README.md:337-339` with the true residual gap, e.g.:
  "Windows exposes UIA child trees and semantic actions (invoke, select,
  expand/collapse, toggle, focus), but element readback is not yet native:
  `text`, `numeric_value`, and `supports_editable_text` are not populated
  from UIA patterns (tracked in ROADMAP)."
- Update the "What Works" Windows bullet (:55-56) to mention UIA inspection
  and semantic actions alongside window discovery/GDI/SendInput.
- Cross-check wording against `docs/features/windows-uia-automation.md` so
  the three documents agree.

**Verify**: `grep -n "does not yet provide rich UI Automation" README.md` → empty.

### Step 4: Targeted dead_code allows

For each of the four modules: remove the module-level `#![allow(dead_code)]`,
run `cargo build -p sky-cua-client -p sky-cua-overlay-host 2>&1 | grep -A2 dead_code`,
then for every warned item either (a) delete it if nothing in the workspace
references it and it isn't part of a documented protocol/contract surface,
or (b) annotate it with item-level `#[allow(dead_code)]` plus a one-line
reason comment (the pattern already exists at
`renderer/cursor_texture.rs:472`). Prefer (b) when unsure — this step must
not change behavior. Also remove `cursor_texture.rs:9`'s re-override once
the parent blanket is gone (check what :9 actually does first — it may be a
`#![warn]`/local allow that becomes redundant).

**Verify**: `grep -rn '#!\[allow(dead_code)\]' crates/` → empty;
`cargo build` → exit 0 with no dead_code warnings; `cargo nextest run` → all pass.

### Step 5: Track the god files

In `ROADMAP.md` under "Code quality / Ultra-review backlog", extend the
split list with: `crates/sky-cua-service/src/daemon.rs`,
`crates/sky-cua-windows/src/backend.rs`,
`crates/sky-cua-linux/src/virtual_input.rs`,
`crates/sky-cua-chrome-host/src/host.rs`,
`crates/sky-cua-client/src/mcp_tools/definitions.rs` (note: split planned in
`advisor-plans/009`). One line each, matching the existing list's format.

**Verify**: `grep -n "definitions.rs" ROADMAP.md` → ≥1 hit.

## Test plan

No new tests. Gates: full `cargo fmt --check && cargo nextest run` plus the
Python suite untouched (`uv run pytest` → all pass, proves no script
referenced the deleted files).

## Done criteria

- [ ] Root contains no TODO_*/IMPROVE_* files; research content preserved under `docs/research/`
- [ ] `grep -rn x11rb Cargo.toml Cargo.lock crates/` → empty
- [ ] README Windows sections match shipped reality (step 3 grep empty)
- [ ] No module-level `#![allow(dead_code)]` remains; build has no dead_code warnings
- [ ] ROADMAP backlog lists the five files
- [ ] `cargo fmt --check && cargo nextest run` and `uv run pytest` all green
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- Removing a blanket allow reveals >25 dead-code warnings in one module —
  that module needs a real dead-code triage, not inline annotation spam;
  report the list.
- Any deleted "dead" item turns out to be referenced by cfg-gated
  (`#[cfg(windows)]` / test-only) code — restore and annotate instead.
- A Python script or doc references the root TODO files in a way that isn't
  a simple link fix.

## Maintenance notes

- The Document Hierarchy rule now holds at root; future audit outputs should
  land in `docs/research/` or ROADMAP, never new root files
  (`advisor-plans/` is the sanctioned exception while plans are live).
- Reviewer scrutiny: step 4's deletions — each should be provably
  unreferenced (`grep` the symbol across the workspace in the PR review).
