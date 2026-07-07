# Plan 007: Single source of truth for SKY_CUA_* env keys (Rust dedup + Rust↔Python drift guard)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-platform/src/config.rs scripts/install_mcp_server.py scripts/_agent_mcp_smoke.py`
> On any in-scope drift, re-verify the excerpts below; on mismatch, STOP.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED — touches installer env forwarding; a mistake silently drops
  a runtime toggle (exactly the failure this plan exists to prevent)
- **Depends on**: none (001 recommended: the drift test wants a CI to run in)
- **Category**: tech-debt
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

The `SKY_CUA_*` env-key strings are the actual runtime contract between the
client, service, helpers, installers, and smoke harnesses. Today:

- 8 keys are declared as independent Rust constants in two crates each
  (e.g. `SKY_CUA_ADB` in both `sky-cua-platform` and `sky-cua-service`;
  `SKY_CUA_OVERLAY_BACKEND` in both `sky-cua-service` and
  `sky-cua-overlay-host`).
- ~46 keys are re-hardcoded as Python string literals in the installer's
  forwarding allowlist and the smoke harnesses' passthrough sets.

A rename or addition on the Rust side that misses a Python allowlist means
the installer silently stops forwarding a toggle — invisible until a live
smoke fails. A prior cleanup (ICA-018) renamed the Rust constants but did
not deduplicate them, which is how the current drift surface survived.

## Current state

- Canonical table: `crates/sky-cua-platform/src/config.rs` owns ~64 `*_ENV`
  constants (e.g. `:33 pub const PHONE_ADB_ENV: &str = "SKY_CUA_ADB";`),
  plus a few in `crates/sky-cua-platform/src/paths.rs` and `lib.rs`.
- Verified duplicate Rust declarations (grep `= "SKY_CUA_` across crates,
  find repeated string values):
  - `"SKY_CUA_ADB"` — `platform/src/config.rs:33` AND
    `service/src/phone/command.rs:31` (`SKY_CUA_ADB_ENV`)
  - `"SKY_CUA_OVERLAY_BACKEND"` — `service/src/overlay/host/mod.rs:16` AND
    `overlay-host/src/lib.rs:36` (both `OVERLAY_BACKEND_ENV`)
  - `"SKY_CUA_MODEL_SCREENSHOT_FORMAT"` (+ JPEG/WebP quality keys) —
    `service/src/browser/model_image.rs:18` AND `capture/src/lib.rs:26`
  - `"SKY_CUA_INPUT_HELPER_SOCKET"` — `linux/src/virtual_input.rs:32` AND
    `overlay-host/src/pointer_tracking.rs:24`
  - `"SKY_CUA_BROWSER"` — `platform/src/config.rs:25` AND
    `service/src/browser/sockets.rs:15`
  - Run the discovery yourself for the full set:
    `grep -rhoP '= "(SKY_CUA_[A-Z0-9_]+)"' crates --include='*.rs' | sort | uniq -d`
- Python hardcodings:
  - `scripts/install_mcp_server.py:~375-410` — the forwarding allowlist
    (mix of literals like `"SKY_CUA_INPUT_BACKEND"` and constants imported
    from `_install_shared`).
  - `scripts/_agent_mcp_smoke.py:~79` — `SKY_CUA_RUNTIME_ENV_ALLOWLIST`
    with ~47 literals.
  - `scripts/run_gui_testing_vm_smoke.py` — ~23 more.
- Dependency direction: `sky-cua-platform` is the shared contracts crate
  every other crate already depends on (verify per-crate with
  `grep sky-cua-platform crates/*/Cargo.toml`).
- Python tooling: `uv run pytest`, tests in `scripts/test_*.py`;
  basedpyright at `standard`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Duplicate discovery | `grep -rhoP '= "(SKY_CUA_[A-Z0-9_]+)"' crates --include='*.rs' \| sort \| uniq -d` | empty AFTER step 1 |
| Rust tests | `cargo nextest run` | all pass |
| Python suite | `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest` | all green |

## Scope

**In scope**:
- `crates/sky-cua-platform/src/config.rs` (may add missing consts + the
  dump helper)
- The duplicate-declaring Rust files listed above (switch to platform consts)
- `crates/sky-cua-platform/` or `crates/sky-cua-client/` CLI surface for the
  key dump (step 2 — see design)
- `scripts/test_env_key_contract.py` (create — the drift test)
- `scripts/install_mcp_server.py`, `scripts/_agent_mcp_smoke.py` — comment
  breadcrumbs only (see step 3; NOT a rewrite of their lists)

**Out of scope** (do NOT touch):
- Changing any env key's *name or semantics*. This plan moves declarations;
  every string value stays identical.
- Rewriting the Python allowlists to load keys dynamically at install time
  (the installer must keep working from a release bundle without the Rust
  source tree; dynamic loading is a design change — deferred).
- `crates/sky-cua-overlay-host/src/main.rs:461`'s `set_var` test hook.
- The compat-enablement triplication tracked in ROADMAP.md (different item).

## Git workflow

- Branch: `bex/advisor-007-env-key-contract`
- Commits: `refactor(platform): single-source SKY_CUA env key constants`,
  `feat(client): dump the env-key table for contract checks`,
  `test(scripts): guard Rust↔Python env-key drift`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Deduplicate the Rust constants

For each duplicated key from the discovery grep: ensure a canonical const
exists in `sky-cua-platform` (add it there if the platform copy is the one
missing), then replace the other declaration with a re-export or a direct
use of the platform const. Keep local alias names where churn would be
large (`pub(crate) use sky_cua_platform::config::PHONE_ADB_ENV as SKY_CUA_ADB_ENV;`
is fine). `sky-cua-capture` and `sky-cua-overlay-host` both already depend
on `sky-cua-platform`? — verify; if one doesn't, adding the dependency is in
scope (it's the contracts crate; check for cycles with `cargo build` —
platform depends on nothing in-workspace).

**Verify**: the discovery grep → empty; `cargo nextest run` → all pass.

### Step 2: Machine-readable key dump

Add a hidden subcommand `sky-cua-client env-keys` (pattern-match the
existing CLI parsing in `crates/sky-cua-client/src/main.rs` /
`operator_cli.rs`) that prints every known `SKY_CUA_*` key, one per line,
sourced from a single `pub fn all_env_keys() -> &'static [&'static str]`
in `sky-cua-platform` (a static slice listing the consts — assemble it
manually next to the consts with a unit test asserting no duplicates and
that every entry starts with `SKY_CUA_`).

Completeness guard (Rust side): add a test in `sky-cua-platform` that greps
are not available to — instead, embed the check in the *Python* drift test
(step 3) which can grep the source tree.

**Verify**: `cargo run -p sky-cua-client -- env-keys | head` → prints keys;
`cargo run -p sky-cua-client -- env-keys | sort | uniq -d` → empty.

### Step 3: Python drift test

Create `scripts/test_env_key_contract.py` (pytest; follow the conventions of
existing `scripts/test_*.py` — imports from repo-relative paths, typed):

1. Collect the *source-of-truth* set: scan `crates/**/*.rs` with a regex for
   `"SKY_CUA_[A-Z0-9_]+"` string literals (reads files directly; no cargo
   needed — works in CI).
2. Collect every `SKY_CUA_*` literal from `scripts/**/*.py`.
3. Assert: every key referenced in Python exists in the Rust set
   (catches renames/typos — the actual observed failure mode).
4. Assert: every key in the Rust set that matches the installer's
   *forwarding-relevant* prefixes appears in `install_mcp_server.py`'s
   allowlist **or** in an explicit, commented exemption list inside the test
   (`KNOWN_NOT_FORWARDED: set[str] = {...}  # internal/test-only keys`).
   Build the initial exemption list empirically: run the test, move every
   current miss into the list with a one-line reason each, so the test is
   green at head but any *new* unforwarded key fails loudly.
5. Add a breadcrumb comment above the allowlist in `install_mcp_server.py`
   and `_agent_mcp_smoke.py`: `# Guarded by scripts/test_env_key_contract.py — new SKY_CUA_* keys must be added here or exempted there.`

**Verify**: `uv run pytest scripts/test_env_key_contract.py -v` → pass;
`uv run basedpyright` → 0 errors; `uv run ruff check scripts` → clean.

## Test plan

- Step 2's no-duplicates unit test in `sky-cua-platform`.
- Step 3's drift test (the deliverable). Negative check: temporarily add a
  fake `"SKY_CUA_BOGUS_KEY"` literal to a Python file → the test must fail;
  remove it. Mention performing this check in your report.

## Done criteria

- [ ] Duplicate-declaration grep returns empty
- [ ] `cargo run -p sky-cua-client -- env-keys` works and is duplicate-free
- [ ] `scripts/test_env_key_contract.py` passes, and fails on an injected bogus key (verified once, then reverted)
- [ ] Full gates: `cargo fmt --check && cargo nextest run` and the four Python commands all green
- [ ] No env key string *value* changed anywhere (`git diff -G'SKY_CUA_' --stat` reviewed: declarations moved, values identical)
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- A "duplicate" turns out to be deliberately divergent (same string, but one
  side documents different semantics) — report it; that's a real bug, not a
  mechanical dedup.
- Adding the platform dependency to a crate creates a dependency cycle.
- The exemption list in step 3.4 exceeds ~30 keys — the signal/noise is
  wrong and the prefix heuristic needs maintainer input.

## Maintenance notes

- Deferred: the installer loading keys from `sky-cua-client env-keys` at
  install time (needs bundle-mode thinking); collapsing
  `run_gui_testing_vm_smoke.py`'s list into `_agent_mcp_smoke.py`'s.
- New env keys now have a forced decision point (forward or exempt) —
  reviewers should reject PRs that grow the exemption list without a reason
  comment.
- Cross-language protocol constants (Kotlin companion vs Rust) were NOT
  covered here — a likely second instance of this pattern, noted for a
  future audit.
