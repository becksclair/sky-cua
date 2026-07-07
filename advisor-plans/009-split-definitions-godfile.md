# Plan 009: Split mcp_tools/definitions.rs (4,395 lines) into per-tool-family schema modules

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-client/src/mcp_tools/definitions.rs crates/sky-cua-client/tests/fixtures/`
> On any in-scope drift, re-verify the excerpts below; on mismatch, STOP.

## Status

- **Priority**: P3
- **Effort**: M
- **Risk**: LOW — pure code motion with a byte-identical output gate
  (the contract fixtures make this one of the safest splits possible)
- **Depends on**: 008 (adds the ROADMAP tracking entry; not a hard blocker)
- **Category**: tech-debt
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

`crates/sky-cua-client/src/mcp_tools/definitions.rs` is 4,395 lines and ~160
functions of JSON-Schema builders — every tool-schema change lands in one
flat file. It was carved out of the tracked god file `mcp_tools.rs` and is
itself ~5× the repo's ~800-line split threshold (AGENTS.md: "Past roughly
800 lines, look for a cohesive boundary to split along"). The function names
already encode the boundary: desktop / browser / phone / status / common
schema helpers. Critically, the MCP tool contract is pinned by golden
fixtures (`tests/fixtures/tool_contract.json` etc.), so a pure-motion split
is machine-verifiable: if the fixtures still match without regeneration, the
split changed nothing.

## Current state

- `crates/sky-cua-client/src/mcp_tools/definitions.rs` — 4,395 lines, ~160
  fns: schema builders (`observe_properties`, `desktop_pointer_constraints`,
  `exact_branch_schema_with_constraints`, per-tool definition fns), the
  grouped-definition assembler, and at the bottom (~:3670-3700) the fixture
  tests:
  - `assert_fixture_matches` (:3693) compares
    `crates/sky-cua-client/tests/fixtures/{tool_contract,call_cases,mcp_tool_surface_matrix}.json`
    against generated output; setting `SKY_CUA_UPDATE_MCP_FIXTURES=1`
    regenerates them. **You must NOT set that variable in this plan** — an
    unchanged fixture passing is the proof of pure motion.
  - `call_cases_match_grouped_dispatcher` (:3672) cross-checks call cases
    against the real dispatcher.
- The parent module is `crates/sky-cua-client/src/mcp_tools.rs` (declares
  `mod definitions;`) with siblings under `crates/sky-cua-client/src/mcp_tools/`
  (`browser_tests.rs`, `phone_tests.rs`, `phone/` etc.) — the split follows
  the existing directory-module convention.
- Tool families on the surface (from the canonical tool list —
  `docs/features/mcp-tool-surface.md`): desktop (`observe`,
  `desktop_action`, `desktop_pointer`, `desktop_keyboard`, `desktop_scroll`,
  `desktop_semantic`, `desktop_set_value`, `desktop_toggle`,
  `desktop_launch_app`, `capture_*`, `activate_window`, `setup_desktop`),
  browser (`browser_*`), phone (`phone_*`), status/meta (`status`, `doctor`,
  `list_resources`, `session_presence`).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| The gate | `cargo nextest run -p sky-cua-client` | all pass, fixtures byte-unchanged |
| Fixture untouched proof | `git status crates/sky-cua-client/tests/fixtures/` | clean |
| Format | `cargo fmt --check` | exit 0 |
| Line counts after | `wc -l crates/sky-cua-client/src/mcp_tools/definitions/*.rs` | each file < 1200 |

## Scope

**In scope**:
- `crates/sky-cua-client/src/mcp_tools/definitions.rs` → becomes
  `definitions/` directory module:
  `definitions/mod.rs` (assembler + re-exports),
  `definitions/common.rs` (shared schema helpers),
  `definitions/desktop.rs`, `definitions/browser.rs`,
  `definitions/phone.rs`, `definitions/status.rs`,
  `definitions/fixture_tests.rs` (the test module, `#[cfg(test)]`)

**Out of scope**:
- `crates/sky-cua-client/tests/fixtures/*` — MUST remain byte-identical.
- Any schema content change: no renamed fields, no reordered tools, no
  "while I'm here" cleanups. Pure motion + `use` adjustments only.
- `mcp_tools.rs` itself (its split is a separately tracked ROADMAP item).
- Function visibility widening beyond what the module split forces
  (prefer `pub(super)`/`pub(in crate::mcp_tools)`).

## Git workflow

- Branch: `bex/advisor-009-split-definitions`
- Commits: one preparatory commit if you first convert to
  `definitions/mod.rs` wholesale, then one commit per extracted family —
  each commit must pass the gate. Style:
  `refactor(client): split tool schema definitions by family`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Convert to a directory module

`git mv crates/sky-cua-client/src/mcp_tools/definitions.rs crates/sky-cua-client/src/mcp_tools/definitions/mod.rs`.

**Verify**: `cargo nextest run -p sky-cua-client` → all pass;
`git status crates/sky-cua-client/tests/fixtures/` → clean.

### Step 2: Extract families one at a time

For each family in order (common → status → phone → browser → desktop):

1. Identify the family's functions by name prefix and by reading the
   assembler (`build_grouped_tool_definitions` or equivalent — find it with
   `grep -n "fn.*grouped" definitions/mod.rs`) to see which definition fns
   feed which tools.
2. Move them verbatim into `definitions/<family>.rs`; add
   `mod <family>; use <family>::*;` (or explicit imports) in `mod.rs`.
3. Shared helpers used by ≥2 families go to `common.rs`; helpers used by
   one family move with that family.
4. Run the gate after EACH family.

**Verify** (after each family): `cargo nextest run -p sky-cua-client` → all
pass; `git status ...fixtures/` → clean; `cargo fmt --check` → exit 0.

### Step 3: Move the test module

Move the `#[cfg(test)]` fixture-test module into
`definitions/fixture_tests.rs` (declared `#[cfg(test)] mod fixture_tests;`
in `mod.rs`). Keep `SKY_CUA_UPDATE_MCP_FIXTURES` handling exactly as-is.

**Verify**: `cargo nextest run -p sky-cua-client -E 'test(fixture)'` → the
fixture tests still run (nonzero test count) and pass.

### Step 4: Size check

**Verify**: `wc -l crates/sky-cua-client/src/mcp_tools/definitions/*.rs` —
no file over ~1,200 lines (desktop is the biggest family; if it exceeds,
split `desktop.rs` into `desktop.rs` + `desktop_actions.rs` along the
action-tool boundary).

## Test plan

No new tests — the existing fixture suite IS the gate. The invariant to
report: zero fixture diffs across the entire split
(`git diff --stat ed3aef3..HEAD -- crates/sky-cua-client/tests/fixtures/` → empty).

## Done criteria

- [ ] `definitions.rs` no longer exists as a single file; family modules in place
- [ ] `cargo nextest run` (workspace) exits 0
- [ ] `git diff --stat ed3aef3..HEAD -- crates/sky-cua-client/tests/fixtures/` → empty (fixtures never regenerated)
- [ ] `grep -rn "SKY_CUA_UPDATE_MCP_FIXTURES" crates/sky-cua-client/` still hits exactly the moved test module
- [ ] No file in `definitions/` exceeds ~1,200 lines
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- Any fixture file shows a diff at any point — you changed behavior, not
  location. Revert the offending move and report.
- A function can't move without widening visibility beyond
  `pub(in crate::mcp_tools)` — report rather than making it `pub`.
- The assembler's structure doesn't decompose along tool families (e.g.
  heavy cross-family schema sharing beyond simple helpers) — report the
  actual coupling.

## Maintenance notes

- Future tool additions now have an obvious home; reviewers should push
  back on new schema fns landing in `mod.rs`.
- The `mcp_tools.rs` (5,146-line) split remains tracked in ROADMAP — this
  plan's family layout is the template for it.
- Fixture regeneration discipline is unchanged: `SKY_CUA_UPDATE_MCP_FIXTURES=1`
  requires human diff review of `tool_contract.json` (273KB) — noted from
  the audit as a process risk; consider structural invariant tests later.
