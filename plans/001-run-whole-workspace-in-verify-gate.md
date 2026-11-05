# Plan 001: Make `just verify` and CI test the whole Rust workspace and the bun packages

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- justfile .gitea/workflows/verify.yml .config/nextest.toml Cargo.toml AGENTS.md README.md scripts/test_release_pipeline.py packages/browser-use/package.json packages/sky-cua-js/package.json runtime/cua-node/package.json`
> (no `..HEAD`, so uncommitted edits are included). If any of these files
> changed since this plan was written, compare the "Current state" excerpts
> against the live files before proceeding; on a semantic mismatch, treat it
> as a STOP condition. Excerpts are abridged where marked; formatting-only
> differences are not drift.

## Executor report (2026-11-03, run in a clean worktree at `e928a40`)

The executor stopped at Step 0, as the plan instructs, because the bun
baseline is red before any change. Exact results with the plan's commands:
browser-use 35 pass / 2 fail; sky-cua-js 58 pass / 1 fail (flaky under a
load average of ~51; three re-runs gave 1, 9 and 0 failures); cua-node
200 pass / 5 fail. Causes, each verified by the executor:

1. `packages/browser-use` tests import `runtime/cua-node` source, so
   `runtime/cua-node/node_modules` must be installed BEFORE browser-use's
   tests run. The Step 2 recipe installs browser-use first. (Fix: install all
   three packages before testing any of them.)
2. `runtime/cua-node`'s `test/source-lock-truth.test.ts` reads
   `runtime/cua-node/dist/cli.js`, so `bun run build` must precede
   `bun run test` there (its `verify` script does build, typecheck, test).
3. `packages/browser-use` `test/runtime.test.ts:922-929` ("generated locator
   programs execute against a real Chrome DOM") requires the release output
   `out/components/cua-node-linux-x64-glibc/lib/node_modules/playwright/index.js`
   and `/usr/bin/google-chrome-stable`. Neither exists in a clean or CI
   checkout. Gating or skipping it headlessly means editing TypeScript, which
   this plan forbids. **Maintainer decision needed**: either allow a TS gate
   for that one test, or exclude browser-use's test step from CI and keep
   only its build and typecheck.
4. `packages/sky-cua-js` transport, pack and phone suites time out under
   host load; treat them as flakes to investigate before they gate CI.

The Rust half (Steps 1, 3.1 and 4: `--workspace`, `NEXTEST_PROFILE=ci`, the
docs) does not depend on the bun suites and can be landed on its own. All
other plans (002-017) are on `bex/improve-plans`; until this plan lands, the
gate there still skips `sky-cua-linux`'s tests (plans 007, 009, 012 added
tests in that crate, verified by a manual `cargo nextest run --workspace`:
1965/1965).

Recommended rework of Step 2 once the decision on item 3 is made:

```
verify-js:
    #!/usr/bin/env bash
    set -euo pipefail
    for dir in packages/browser-use packages/sky-cua-js runtime/cua-node; do
        (cd "$dir" && bun install --frozen-lockfile)
    done
    (cd packages/browser-use && bun run build && bun run typecheck && bun run test)
    (cd packages/sky-cua-js && bun run typecheck && bun run test)
    (cd runtime/cua-node && bun run build && bun run typecheck && bun run test)
```

with the Step 0 baseline re-run in that order (expected: only the
real-Chrome DOM test fails until item 3 is decided).

Re-run on the development host on 2026-11-04 (main at `a87d706`, host idle,
load ~4), in exactly that order: browser-use 37/37 pass (the real-Chrome DOM
test passes in ~0.8 s because this host has `/usr/bin/google-chrome-stable`
and an assembled `out/components/cua-node-linux-x64-glibc`); sky-cua-js
59/59 pass on two consecutive runs (no flakes at normal load); cua-node
205 pass / 7 skip / 0 fail after `bun run build`. So items 1, 2 and 4 are
confirmed as ordering/build/load effects, and item 3 is the only genuine CI
gap: the verify workflow never assembles the cua-node component and the
runner's Chrome availability is unrecorded. Options for item 3: (a) gate
that one test with `test.skipIf(!(playwrightExists && chromeExists))` and a
loud skip message (one TypeScript edit; real-Chrome acceptance then runs
wherever the artifacts exist, which includes the dev host and the release
pipeline); (b) have the CI `js` job run `python3 scripts/assemble_cua_node.py`
and require Google Chrome on the runner; (c) run only browser-use's build and
typecheck in CI. Recommendation: (a).

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

The root `justfile` runs `cargo nextest run` with no `--workspace` flag. In a
virtual workspace cargo selects only the `default-members` listed in the root
`Cargo.toml`, and that list omits `sky-cua-linux`, `sky-cua-capture`,
`sky-cua-cosmic-helper`, and `sky-cua-windows`. Measured with
`cargo nextest list` on 2026-11-03: the default selection lists 1,527 tests,
`--workspace` lists 1,927. The Linux desktop backend, the main product path,
carries 385 of the 400 unrun tests (capture 7, cosmic-helper 8, windows 0 on
Linux). `just verify` and the Gitea CI workflow both use this recipe, so a
regression in `crates/sky-cua-linux` merges green.

The same gate never runs the 44 bun test files under `packages/` and
`runtime/cua-node`, even though CI builds one of those packages and the
standalone release ships the cua-node runtime. And the `[profile.ci]` block in
`.config/nextest.toml` (fail-fast disabled) is never selected, so CI stops at
the first failure, the opposite of what the profile's comment promises.

All 400 currently-unrun Rust tests were executed on 2026-11-03 on the
development host with
`cargo nextest run -p sky-cua-linux -p sky-cua-capture -p sky-cua-cosmic-helper`
and passed (400/400 in 65 s; two of them spawn throwaway X servers). This plan
adds no known-failing Rust tests to the gate. The bun suites were NOT run
during planning; Step 0 establishes their baseline in a clean worktree, which
matters because the development host has stale ignored `dist/`, `build/` and
`node_modules/` directories that can mask failures CI would see.

## Current state

- `justfile` lines 1-11 (verbatim):

  ```
  release: verify
    python3 install.py release --patch
    python3 install.py install

  # Fast headless verification: Rust + Python. Kotlin runs when a JDK is available.
  verify: verify-rust verify-python verify-kotlin

  verify-rust:
      cargo fmt --check
      cargo clippy --workspace --all-targets -- -D warnings
      cargo nextest run
  ```

  Lines 13-17 (`verify-python`) run ruff, basedpyright and
  `uv run pytest --ignore=scripts/test_browser_control_acceptance.py`. There is
  no recipe that runs any `bun` command. Note `release: verify` on line 1:
  after this plan `just release` also runs the new JS recipe.

- `Cargo.toml` lines 20-27:

  ```toml
  default-members = [
    "crates/sky-cua-platform",
    "crates/sky-cua-service",
    "crates/sky-cua-client",
    "crates/sky-cua-chrome-host",
    "crates/sky-cua-input-helper",
    "crates/sky-cua-overlay-host",
  ]
  ```

  Leave this list alone. It exists so that `cargo build` on a non-Linux host
  does not try to build Linux-only crates. The fix is in the test command.

- `.config/nextest.toml` lines 40-44:

  ```toml
  [profile.ci]
  # CI keeps the same serialization but surfaces flakes loudly instead of hiding
  # them behind a retry, and fails fast is disabled so one race does not mask the
  # rest of the run.
  fail-fast = false
  ```

  Nothing selects it. nextest reads the `NEXTEST_PROFILE` environment variable
  natively (`cargo nextest run --help` shows `-P, --profile <PROFILE> [env: NEXTEST_PROFILE=]`),
  so CI can select it with an env var and no `justfile` change. The
  serial-group overrides at lines 25-38 filter on `package(sky-cua-linux)`;
  `cargo nextest show-config test-groups --profile ci --workspace` confirms the
  `ci` profile inherits them.

- `.gitea/workflows/verify.yml` — two jobs, `rust` and `python`, each
  `runs-on: saga-sky-cua-build` with `actions/checkout@v4` (`clean: true`,
  which runs `git clean -ffdx`, wiping ignored `dist/`, `build/`,
  `node_modules/`). The `rust` job runs `just verify-rust`. The `python` job
  runs `bun install --frozen-lockfile && bun run build` inside
  `packages/browser-use`, then `uv sync --dev`, then `just verify-python`.
  No job runs `bun test` or `bun run typecheck`.

- `scripts/test_release_pipeline.py:414` (verbatim):

  ```python
      assert verify_workflow.count("runs-on: saga-sky-cua-build") == 2
  ```

  Adding a third job makes this `3`; the test must be updated in the same
  change or `just verify-python` fails.

- bun packages, `package.json` scripts (verified 2026-11-03), and what their
  tests need:
  - `packages/browser-use`: `"test": "bun test"`, `"typecheck": "bunx tsc -p tsconfig.json --noEmit"`,
    `"build": "bun run src/build.ts"`. Its `verify` script also runs
    `fixtures`, which regenerates files; do not use `verify`. Its
    `test/build.test.ts:88` spawns `node`.
  - `packages/sky-cua-js`: `"test": "bun run build && bun test test"`,
    `"typecheck": "tsc --noEmit -p tsconfig.json"`. Its tests import
    `./dist/index.js` (`test/packaging.test.ts:105,112`,
    `test/sky-cua-contract.test.ts:965`); `dist/` is gitignored, so the
    build inside the `test` script is required. Plain `bun test` would skip
    it: `bun test` always invokes bun's built-in runner, never the
    `package.json` script. Use `bun run test`.
  - `runtime/cua-node`: `"test": "bun test contracts test production compliance"`,
    `"typecheck": "tsc --noEmit"`. Its tests copy
    `../../../packages/browser-use/build/browser-client.mjs`
    (`test/browser-control-ingress.test.ts:33`, `test/browser-native-pipe.test.ts:56`)
    without checking it exists, so browser-use must be built first. Eight of
    its test files import `test/test-node-path.ts`, which runs
    `execFileSync("node", ...)` at import time, so `node` must be on PATH.
  - Each directory has its own `bun.lock`. `bun` 1.4.4 is on the dev host and
    the CI runner (the existing workflow calls it). Whether `node` is on the
    runner's PATH is unknown; on the dev host it comes from a mise shim.

- `AGENTS.md` line 13 (`cargo build && cargo nextest run`), lines 21-23
  (describes `just verify` as "Rust fmt/clippy/nextest, Python
  ruff/basedpyright/pytest, and the Kotlin companion unit tests when a JDK is
  present"), line 25 (the paragraph "Run Rust tests with `cargo nextest run`,
  not `cargo test`..."), line 109 (Definition of Done: `cargo fmt --check && cargo clippy --workspace --all-targets && cargo nextest run`).
  `README.md` line 114: `cargo nextest run   # install once: cargo install cargo-nextest`.
  `docs/operations/*.md` contain no `cargo nextest run` today.

Conventions: the repository mandates `cargo nextest run`, never `cargo test`
(`AGENTS.md` lines 25-33 explain why). Commit messages are conventional, for
example `fix(service): hard-cap daemon log generations and back off accept errors`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Count tests, default members | `cargo nextest list --message-format json \| python3 -c "import json,sys; d=json.load(sys.stdin); print(sum(len(s['testcases']) for s in d['rust-suites'].values()))"` | `1527` at `e928a40` |
| Count tests, whole workspace | same command with `--workspace` after `list` | `1927` at `e928a40` (or more if tests were added since) |
| Rust gate | `just verify-rust` | exit 0 |
| JS gate | `just verify-js` | exit 0 |
| Python gate | `just verify-python` | exit 0 |
| Release-pipeline test only | `uv run pytest scripts/test_release_pipeline.py -q` | all pass |
| Whole gate | `just verify` | exit 0 |
| YAML sanity | `python3 -c "import yaml; yaml.safe_load(open('.gitea/workflows/verify.yml'))"` | no output, exit 0 (system `python3` has PyYAML; if `import yaml` fails use `uv run --with pyyaml python -c ...`) |

## Scope

**In scope** (the only files you should modify):
- `justfile`
- `.gitea/workflows/verify.yml`
- `scripts/test_release_pipeline.py` (line 414 only)
- `AGENTS.md` (lines 13, 21-23, 109, plus one sentence appended to the paragraph at lines 25-33)
- `README.md` (line 114 only)

**Out of scope** (do NOT touch):
- `Cargo.toml` `default-members`.
- `.config/nextest.toml`. The `ci` profile already exists; this plan selects
  it via the environment.
- Any test source other than the single assertion in
  `scripts/test_release_pipeline.py`. If a newly-included test fails, that is
  a STOP condition, not a test to delete, `#[ignore]`, or weaken.
- `scripts/test_browser_control_acceptance.py` and the `--ignore` that skips
  it. It needs a built `target/debug/sky-cua-service`; wiring that into the
  Python CI job is a separate decision.
- `verify-kotlin`. Making it fail without a JDK is a release-flow policy change.
- `.gitea/workflows/release-standalone.yml` and
  `scripts/_standalone_release_command.py`: both call `just verify-rust` and
  `just verify-python` directly. They inherit `--workspace` automatically but
  will not run `verify-js` or the `ci` profile. Adding JS to the release gate
  is a separate decision; note that `just release` (which depends on
  `verify`) does pick up `verify-js`.
- Installing toolchains (`node`, `bun`) in the workflow.

## Git workflow

- Branch: `bex/verify-whole-workspace`
- Commit only if the operator authorised it; style:
  `build(verify): run the whole cargo workspace and the bun packages in just verify`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 0: Baseline the bun suites in a clean worktree

The dev host has stale ignored build output that can hide failures. Before
editing anything:

```bash
git worktree add /tmp/sky-cua-001 e928a40
cd /tmp/sky-cua-001
(cd packages/browser-use && bun install --frozen-lockfile && bun run build && bun run typecheck && bun run test)
(cd packages/sky-cua-js && bun install --frozen-lockfile && bun run typecheck && bun run test)
(cd runtime/cua-node && bun install --frozen-lockfile && bun run typecheck && bun run test)
cd /home/bex/projects/sky-cua
git worktree remove --force /tmp/sky-cua-001
```

Record the exit status and any failing test names for each of the three
blocks.

**Verify**: all three blocks exit 0. If any fails at `e928a40`, see STOP
conditions; do not continue to Step 2 with a red baseline (Step 1 may still
proceed).

### Step 1: Test the whole workspace in `verify-rust`

Edit `justfile`: change the line `    cargo nextest run` (inside
`verify-rust`) to:

```
    cargo nextest run --workspace
```

No profile flag here: nextest reads `NEXTEST_PROFILE` from the environment,
and CI sets it in Step 3. Also change line 5 to:

```
# Fast headless verification: Rust + Python + JS. Kotlin runs when a JDK is available.
```

**Verify**: `just verify-rust` → exit 0, and the nextest summary line reports
`1927 tests run` or more (not 1527). A cargo warning
`output filename collision` for the `session_presence_probe` example in
`sky-cua-linux`/`sky-cua-windows` is expected with `--workspace` (clippy
already prints it) and is not a failure.

### Step 2: Add a `verify-js` recipe and include it in `verify`

Add to `justfile` after `verify-python-full`:

```
# Typechecks and tests the three bun projects in dependency order:
# runtime/cua-node tests read packages/browser-use/build, so browser-use is
# built first. Uses `bun run test` (the package script), never bare
# `bun test`, because sky-cua-js's script builds dist/ before testing.
verify-js:
    #!/usr/bin/env bash
    set -euo pipefail
    (cd packages/browser-use && bun install --frozen-lockfile && bun run build && bun run typecheck && bun run test)
    (cd packages/sky-cua-js && bun install --frozen-lockfile && bun run typecheck && bun run test)
    (cd runtime/cua-node && bun install --frozen-lockfile && bun run typecheck && bun run test)
```

Change the `verify:` recipe line to
`verify: verify-rust verify-python verify-js verify-kotlin`.

**Verify**: `just verify-js` → exit 0 on the dev host. Then repeat in a clean
worktree of your branch (same `git worktree add` / `remove` pattern as Step 0,
using your branch name instead of `e928a40`) and confirm `just verify-js`
exits 0 there too.

### Step 3: Add a `js` job to the Gitea workflow, select the CI profile, fix the job-count test

In `.gitea/workflows/verify.yml`:

1. In the `rust` job, set the profile through the environment:

   ```yaml
         - name: Verify Rust
           env:
             NEXTEST_PROFILE: ci
           run: just verify-rust
   ```

2. Add a third job, mirroring the shape of the existing two:

   ```yaml
     js:
       runs-on: saga-sky-cua-build
       steps:
         - uses: actions/checkout@v4
           with:
             clean: true

         - name: Verify bun packages
           run: just verify-js
   ```

   Keep the trailing comment about Kotlin as is.

3. In `scripts/test_release_pipeline.py` line 414 change `== 2` to `== 3`.

**Verify**: the YAML sanity command from the table → exit 0;
`grep -n 'NEXTEST_PROFILE: ci\|^  js:' .gitea/workflows/verify.yml` → two
matches; `uv run pytest scripts/test_release_pipeline.py -q` → all pass.

### Step 4: Fix the documented command

- `AGENTS.md` line 13: `cargo build && cargo nextest run` →
  `cargo build && cargo nextest run --workspace`
- `AGENTS.md` lines 21-23: replace the parenthetical with
  "(Rust fmt/clippy/nextest, Python ruff/basedpyright/pytest, bun
  typecheck/test for the three JS projects, and the Kotlin companion unit
  tests when a JDK is present)".
- `AGENTS.md` line 109: `cargo nextest run` → `cargo nextest run --workspace`
  inside the Definition of Done bullet.
- `AGENTS.md`, append to the paragraph that begins at line 25 ("Run Rust tests
  with `cargo nextest run`, not `cargo test`..."): "Always pass
  `--workspace`: the root `default-members` list excludes the Linux, capture,
  cosmic-helper and Windows crates, so a bare `cargo nextest run` silently
  skips their tests."
- `README.md` line 114: `cargo nextest run   # install once: cargo install cargo-nextest`
  → `cargo nextest run --workspace   # install once: cargo install cargo-nextest`

**Verify** (each must match):
- `grep -n 'cargo build && cargo nextest run --workspace' AGENTS.md`
- `grep -n 'bun typecheck/test for the three JS projects' AGENTS.md`
- `sed -n 109p AGENTS.md | grep -- '--workspace'` (if line numbers shifted, grep for `cargo nextest run --workspace` within the Definition of Done section instead)
- `grep -n 'cargo nextest run --workspace' README.md justfile`

## Test plan

No new tests are written; one assertion is updated. The deliverable is that
existing tests run.

- `just verify-rust` reports ≥ 1,927 tests run.
- `just verify-js` runs all three bun projects, in a clean worktree too.
- `uv run pytest scripts/test_release_pipeline.py -q` passes with `== 3`.
- `just verify` exits 0 on the development host.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `just verify-rust` exits 0 and its nextest summary shows ≥ 1927 tests run
- [ ] `just verify-js` exits 0, on the dev host and in a clean worktree of the branch
- [ ] `uv run pytest scripts/test_release_pipeline.py -q` exits 0
- [ ] `grep -n 'NEXTEST_PROFILE: ci' .gitea/workflows/verify.yml` matches
- [ ] `grep -n '^  js:' .gitea/workflows/verify.yml` matches
- [ ] the four Step 4 greps all match
- [ ] `git status --porcelain -- . ':!plans'` lists only the five in-scope files (the untracked `plans/*.md` files are expected and excluded)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- Any test in `sky-cua-linux`, `sky-cua-capture` or `sky-cua-cosmic-helper`
  fails under `just verify-rust`. Report the test names and output. They
  passed on the development host at `e928a40`; a failure means drift or an
  environment dependency (`Xvfb`, `xpra`, `xdpyinfo`, `xdotool`) the
  maintainer must decide how to gate.
- Any bun block fails in the Step 0 clean-worktree baseline. Report which
  package and which tests; do not edit TypeScript.
- The `js` CI job fails with `node: command not found` (or ENOENT spawning
  `node`) or `bun: command not found`. Report it; do not install toolchains in
  the workflow.
- `just` rejects the recipe (report `just --version`).
- The fix appears to require editing `Cargo.toml`, `.config/nextest.toml`, or
  any TypeScript.

## Maintenance notes

- `cargo nextest run --workspace` on Linux also compiles `sky-cua-windows`,
  which is an empty crate off Windows (every module is behind
  `cfg(target_os = "windows")`), so it adds no tests and little time. Getting
  its 39 tests to run somewhere automated is a separate finding (`TESTS-12`).
- Two newly-included tests (`isolated_x11_probe`, `isolated_app_launch_leak`)
  spawn throwaway X servers and take about 30 s each. They
  `eprintln!("skip: ...")` and return (reported as PASS) when
  `xpra`/`Xvfb`/`xdpyinfo`/`xdotool` are missing, so on a bare runner they are
  silently vacuous. Making that a visible skip is a follow-up (`DX-08`).
- `scripts/test_browser_control_acceptance.py` remains excluded from
  `verify-python`; it is the only headless test that drives the real
  `sky-cua-service` binary over its socket. Including it needs the Python CI
  job to build the service first (`TESTS-07`).
- The release workflow still runs only `verify-rust` and `verify-python`.
  Whether the shipped cua-node runtime's tests should gate a release is the
  maintainer's call.
- A reviewer should check that the `js` job's `bun install --frozen-lockfile`
  needs no network access beyond what the existing `python` job already uses,
  and that `node` is on the runner's PATH.
