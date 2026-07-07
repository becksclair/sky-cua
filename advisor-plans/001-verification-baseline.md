# Plan 001: Establish a one-command verification baseline (aggregate test target, toolchain pin, clippy, minimal CI)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- Cargo.toml scripts/_companion.py AGENTS.md README.md .config/nextest.toml`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

The repo has three well-built test suites (Rust via cargo-nextest, Python via
pytest, Kotlin via Gradle) but no single command that runs them, no CI, no
pinned Rust toolchain, and no clippy anywhere in the verify story. The
16-file Kotlin companion unit suite (JSON-RPC dispatch, token store,
notification PII redaction) is run by *nothing* automated — it can silently
rot. A pre-1.85 rustc fails on `edition = "2024"` with an opaque parse error
instead of "update your toolchain". This plan creates the baseline every
later plan's Done criteria lean on.

## Current state

- `Cargo.toml:31` (workspace root) — `edition = "2024"`; there is **no**
  `rust-toolchain.toml` anywhere in the repo.
- `.github/workflows` does not exist. No `Makefile`, `justfile`, or `xtask`.
- No `.pre-commit-config.yaml`; `.git/hooks` has only samples.
- `grep -i clippy README.md AGENTS.md` → no hits. Rust verify is documented
  as `cargo fmt --check && cargo nextest run` (AGENTS.md "Definition of Done").
- `scripts/_companion.py:211-221` — the only companion Gradle lane builds but
  never tests:

  ```python
  def gradle_assemble_command() -> list[str]:
      """`gradlew -p android/phone-companion :app:assembleDebug` ..."""
      return [
          str(COMPANION_GRADLEW),
          "-p",
          str(COMPANION_PROJECT_DIR),
          ":app:assembleDebug",
          "--console=plain",
      ]
  ```

- Kotlin unit tests live at
  `android/phone-companion/app/src/test/java/com/skycua/phonecompanion/**`
  (16 files) and run via
  `./gradlew -p android/phone-companion :app:testDebugUnitTest` (documented in
  `android/phone-companion/AGENTS.md`). They need JAVA_HOME/Android SDK; the
  aggregate target must treat that leg as *skippable when the toolchain is
  absent*, not as a hard failure.
- `.config/nextest.toml` serializes socket-heavy integration tests; nextest is
  mandatory (`cargo test` races env mutation — see AGENTS.md "Root Setup
  Commands").
- Rust tests link against system libs (gstreamer, wayland, xkbcommon, vulkan
  loaders per `Cargo.toml` workspace deps) — CI must install them.
- Repo convention: branch names use the `bex/` prefix; commit messages follow
  `type(scope): summary` (see `git log`: `fix(session-presence): only re-lock
  sessions we unlocked`).

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Rust fmt | `cargo fmt --check` | exit 0 |
| Rust tests | `cargo nextest run` | all pass |
| Rust lint (new) | `cargo clippy --workspace --all-targets` | exit 0 (warnings allowed initially) |
| Python fmt/lint | `uv run ruff format --check scripts && uv run ruff check scripts` | exit 0 |
| Python types | `uv run basedpyright` | 0 errors |
| Python tests | `uv run pytest` | all pass |
| Packaging | `python3 scripts/build_plugin.py` | exit 0 |
| Kotlin tests | `./android/phone-companion/gradlew -p android/phone-companion :app:testDebugUnitTest --console=plain` | BUILD SUCCESSFUL (only when JDK+SDK present) |

## Scope

**In scope** (the only files you should create/modify):
- `rust-toolchain.toml` (create)
- `justfile` (create; use `just` — check `command -v just`; if absent, create a
  `Makefile` with the same targets instead)
- `.github/workflows/verify.yml` (create)
- `AGENTS.md`, `README.md` — add the aggregate command and clippy to the
  documented verify flow (small edits to the existing "Root Setup Commands" /
  "Development" sections only)

**Out of scope** (do NOT touch):
- Any Rust/Python/Kotlin source file — this plan adds zero code changes. If
  clippy reports warnings, record the count in your report; do NOT fix them
  here.
- `scripts/_companion.py` — wiring the Kotlin tests into the Python build lane
  is deferred (see Maintenance notes); the justfile invokes gradle directly.
- Live smokes (`scripts/live_*.py`, `scripts/run_gui_testing_vm_smoke.py`) —
  never run in CI; they need a real desktop/VM/phone.
- `.pre-commit-config.yaml` — deferred; the operator may not want hooks.

## Git workflow

- Branch: `bex/advisor-001-verification-baseline`
- One commit per step, style `type(scope): summary`, e.g.
  `dx(toolchain): pin rust toolchain for edition 2024`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Pin the toolchain

Create `rust-toolchain.toml` at repo root:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

Use `channel = "stable"` (not a hard version) unless `rustc --version` on this
machine is a pinned non-stable; edition 2024 needs ≥1.85 and stable satisfies
it.

**Verify**: `cargo fmt --check` → exit 0 (proves rustup resolved the toolchain
and components).

### Step 2: Create the aggregate verify target

Create `justfile` at repo root with these recipes (adapt syntax if you fall
back to Make):

```just
# Fast headless verification: Rust + Python. Kotlin runs when a JDK is available.
verify: verify-rust verify-python verify-kotlin

verify-rust:
    cargo fmt --check
    cargo clippy --workspace --all-targets
    cargo nextest run

verify-python:
    uv run ruff format --check scripts
    uv run ruff check scripts
    uv run basedpyright
    uv run pytest

# Skips (exit 0 with a message) when no JDK is available.
verify-kotlin:
    #!/usr/bin/env bash
    if [ -z "${JAVA_HOME:-}" ] && ! command -v java >/dev/null; then
        echo "verify-kotlin: no JDK found, skipping companion unit tests"; exit 0
    fi
    ./android/phone-companion/gradlew -p android/phone-companion :app:testDebugUnitTest --console=plain
```

Note: `cargo clippy` here must NOT use `-D warnings` — the existing warning
backlog is unknown and fixing it is out of scope. If clippy *errors* (not
warns), STOP and report.

**Verify**: `just verify-rust` → exit 0; `just verify-python` → exit 0;
`just verify-kotlin` → either BUILD SUCCESSFUL or the skip message, exit 0.

### Step 3: Minimal CI workflow

Create `.github/workflows/verify.yml`:

- Trigger: `push` to `main`, `pull_request`.
- Job `rust` (ubuntu-latest): checkout; install system deps
  (`sudo apt-get update && sudo apt-get install -y libgstreamer1.0-dev
  libgstreamer-plugins-base1.0-dev libwayland-dev libxkbcommon-dev
  libdbus-1-dev pkg-config`); `dtolnay/rust-toolchain@stable` with
  `components: rustfmt, clippy`; `Swatinem/rust-cache@v2`;
  `cargo install cargo-nextest --locked` (or `taiki-e/install-action@nextest`);
  then `cargo fmt --check`, `cargo clippy --workspace --all-targets`,
  `cargo nextest run`.
- Job `python` (ubuntu-latest): checkout; `astral-sh/setup-uv@v5`;
  `uv sync --dev`; then the four verify-python commands plus
  `python3 scripts/build_plugin.py` — but note `build_plugin.py` requires
  built Rust binaries. Check its behavior first
  (`python3 scripts/build_plugin.py --help`); if it hard-requires release
  binaries, omit it from CI and note that in the workflow comment rather than
  building release in CI.
- Kotlin job: omit for now (SDK provisioning cost outweighs value for a solo
  repo); leave a comment in the workflow explaining the `just verify-kotlin`
  local path.

If the Rust job's system-dep list turns out insufficient (link errors on
gstreamer/wayland/vulkan), extend the apt list per the linker errors — that's
expected iteration, not a STOP. You cannot run GitHub CI locally; validate
the workflow file with `actionlint` if available, else by YAML parse.

**Verify**: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/verify.yml'))"` → exit 0. If `actionlint` is installed: `actionlint` → no errors.

### Step 4: Document

- In `AGENTS.md` "Root Setup Commands", add `just verify` as the aggregate
  entry point and add `cargo clippy --workspace --all-targets` to the Rust
  line of "Definition of Done".
- In `README.md` "Development", add one line: `just verify` runs the full
  headless suite (Rust + Python + Kotlin-when-JDK-present).

**Verify**: `grep -n "just verify" AGENTS.md README.md` → both hit.

## Test plan

No new tests — this plan wires existing suites. The verification is that all
three legs run through the single entry point:

- `just verify` → exit 0 with all legs green (Kotlin leg may print the skip
  message on a JDK-less machine; that still exits 0).

## Done criteria

- [ ] `rust-toolchain.toml` exists with rustfmt+clippy components
- [ ] `just verify` (or `make verify`) exits 0 locally
- [ ] `.github/workflows/verify.yml` exists and parses
- [ ] `grep -rn clippy AGENTS.md` → at least one hit
- [ ] No files outside the in-scope list modified (`git status`)
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- `cargo clippy --workspace --all-targets` fails with *errors* (not
  warnings) — report the errors; do not fix source files.
- `cargo nextest run` fails on a clean checkout before any of your changes —
  the baseline is broken; report which tests fail.
- The Kotlin test run fails with real test failures (not toolchain absence) —
  report them; fixing Kotlin tests is out of scope.
- `scripts/build_plugin.py` cannot run without artifacts you'd have to build
  in a way not described here.

## Maintenance notes

- Later advisor plans reference `just verify` in their Done criteria; if you
  rename the target, update `advisor-plans/*.md`.
- Follow-up deliberately deferred: wiring `:app:testDebugUnitTest` into
  `scripts/_companion.py` so companion *deploys* also test; a
  `.pre-commit-config.yaml`; tightening clippy to `-D warnings` once the
  backlog is cleared (record the initial warning count in the PR).
- Reviewer should scrutinize: the CI apt dependency list (likeliest breakage)
  and whether `build_plugin.py` was included or excluded from the python job.
