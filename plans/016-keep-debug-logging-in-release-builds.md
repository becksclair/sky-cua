# Plan 016: Keep `debug!` logging compiled into release builds and document how to turn it on

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- Cargo.toml crates/sky-cua-service/src/main.rs crates/sky-cua-client/src/main.rs docs/runtime/mcp-boundary.md docs/operations`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live files before proceeding; on a semantic mismatch, treat it as a
> STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: dx
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

The workspace `tracing` dependency enables `release_max_level_info`, which
compiles every `debug!` and `trace!` call out of release builds. The
installed runtime is always a release build (`scripts/build_plugin.py`
runs `cargo build --release`; the standalone archive ships release
binaries), so `RUST_LOG=debug` against an installed daemon silently does
nothing: the 65 `debug!` sites across the service (45), Linux backend (18),
client (1) and Windows backend (1) cannot be reached without a rebuild (the
workspace has no `trace!` sites of its own; only dependencies such as zbus
emit trace). Diagnosing a field failure therefore means rebuilding with a
different feature set. Meanwhile the daemon's log location
(`~/.local/state/sky-cua/daemon-<socket-stem>.log`) is documented only in a
runtime-contract doc, and `RUST_LOG` is documented nowhere.

The fix is one Cargo feature change (`release_max_level_debug`, which keeps
`debug!` and still compiles out `trace!`) plus a short operator runbook. The
runtime cost of a disabled `debug!` site is a filter check, not formatting;
all 65 sites were read on 2026-11-03 and none is in a per-element, per-frame
or per-pixel loop (the nearest are a 500 ms idle-overlay timer at
`daemon/mod.rs:306`, which logs only on a transition, and one line per
browser keepalive heartbeat at `browser/keepalive.rs:187`); Step 1 has you
confirm that.

## Current state

- `Cargo.toml` line 67 (verbatim):

  ```toml
  tracing = { version = "0.1.41", features = ["release_max_level_info"] }
  ```

  `tracing-subscriber = { version = "0.3.20", features = ["fmt", "env-filter"] }`
  is on line 69. `tracing` is specified as `0.1.41` and locked at `0.1.44`.
  Its static max-level features are
  `release_max_level_{off,error,warn,info,debug,trace}`, keyed on
  `not(debug_assertions)`; if several are set the most restrictive wins, so in
  practice one is chosen. `cargo tree -e features -i tracing` shows
  `release_max_level_info` coming only from the workspace table; every crate
  uses `tracing.workspace = true` and no crate or dependency sets a
  `max_level_*` feature.

- Subscriber initialisation, both binaries use the env filter with an `info`
  default:
  - `crates/sky-cua-service/src/main.rs:70-82` (`init_tracing`):
    `EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())`,
    writer is a `RotatingLog` when `SKY_CUA_DAEMON_LOG_PATH` is set, else
    stderr. The doc comment above it (lines 61-69) explains the per-endpoint
    self-rotating log.
  - `crates/sky-cua-client/src/main.rs:25-27`: same filter, stderr.
  `RUST_LOG` is the variable `EnvFilter::try_from_default_env` reads.

- How `RUST_LOG` reaches the daemon (verified 2026-11-03): the daemon
  `Command` built in `crates/sky-cua-client/src/service_launcher.rs:540-575`
  does not call `env_clear`; it adds the log path, launch-environment
  repairs and isolated-desktop overrides, and `env_remove`s graphical keys in
  detached mode. The daemon therefore inherits the client's full
  environment. `RUST_LOG` is not in `.mcp.json` `env_vars` nor in the
  installer forwarding lists, so under hosts that filter MCP server
  environments (Codex passes a fixed base set plus the configured
  `env`/`env_vars`) it must be set explicitly in that server's config. The
  daemon is long-lived and shared: a new value takes effect only after that
  daemon restarts.
- Headless `debug!` reachable for the proof: `crates/sky-cua-linux/src/env_probe.rs:16`
  `debug!("probing Linux desktop environment")` is the first statement of
  `probe_environment()`, reached unconditionally by `sky-cua-service doctor`
  (`run_doctor` → `desktop_backend.rs:21` → `probe_environment_base`,
  `backend/mod.rs:152`) on a cold cache, with or without a display. Do not
  use `sky-cua-client health` for the proof: it talks to whatever daemon owns
  the default socket (possibly the live installed one), and the client's only
  `debug!` (`service_launcher.rs:111`) fires only in isolated-desktop mode.
- Output format: `tracing_subscriber::fmt` colours output by default even
  when piped (the live daemon log on the dev host contains ANSI codes), so
  any grep for ` DEBUG ` must run with `NO_COLOR=1`.

- Documentation today: `docs/runtime/mcp-boundary.md:28-38` ("Daemon logs"
  paragraph, ending "...agent transcripts are the only other trace.")
  describes the daemon log path and rotation (8 MiB, one `.log.old`
  generation, `SKY_CUA_DAEMON_LOG_PATH`). `README.md`, `NOTES.md` and every file under
  `docs/operations/` (`browser-control-plane-migration.md`,
  `cua-node-dependency-bumps.md`, `gui-desktop-test-harness.md`,
  `isolated-daemon-smokes.md`, `plugin-release.md`,
  `testing-vm-desktop-smokes.md`) have no mention of `RUST_LOG`.
  `docs/AGENTS.md` defines `docs/operations/` as "operator-facing runbooks"
  and gives templates for feature and research docs but none for runbooks;
  the existing runbooks are short task-oriented Markdown.

- Site counts (non-test, `grep -rn 'tracing::debug!\|tracing::trace!\|\bdebug!(\|\btrace!(' --include='*.rs' crates | grep -v tests`):
  service 45, linux 18, client 1, windows 1; all are `debug!`.

Conventions: Rust deps are workspace-managed in root `Cargo.toml`; docs
hierarchy per `AGENTS.md` ("Document Hierarchy"); conventional commits, e.g.
`fix(service): hard-cap daemon log generations and back off accept errors`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Release build of the two binaries | `cargo build --release -p sky-cua-service -p sky-cua-client` | exit 0 |
| Probe, before and after the flip | `NO_COLOR=1 RUST_LOG=debug ./target/release/sky-cua-service doctor 2>&1 >/dev/null \| grep -c ' DEBUG '` | `0` before Step 2, ≥ 1 after |
| Rust gate (matches `just verify-rust`) | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run --workspace` | exit 0 |
| Hot-loop check | `grep -rn -B12 'debug!\|trace!' --include='*.rs' crates \| grep -v /tests \| grep -E '^\S+-[0-9]+-\s*(for \|while \|loop)'` | a short list to review by hand (see Step 1) |
| Lockfile unchanged | `git diff --quiet -- Cargo.lock` | exit 0 |

## Scope

**In scope** (the only files you should modify):
- `Cargo.toml` (line 67 only)
- `docs/operations/daemon-logs.md` (create)
- `docs/runtime/mcp-boundary.md` (one cross-reference sentence appended after line 38, the end of the "Daemon logs" paragraph)
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- Adding request spans (`info_span!(session_id, turn_id)`) around IPC
  dispatch. Valuable, but it touches `daemon/mod.rs` and is a separate change
  (`DX-03` second half).
- Changing any log level at any call site, or the `info` default filter.
- `tracing-log`, which `DEBT-20` reports as an unused dependency in the
  client crate.
- Log rotation parameters.

## Git workflow

- Branch: `bex/release-debug-logging`
- Commit only if the operator authorised it; style:
  `build: keep debug! compiled into release builds; document RUST_LOG and daemon logs`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Confirm no `debug!` sits in a hot loop

Run the hot-loop check command from the table. It prints the `for`/`while`/
`loop` lines that appear within 12 lines above a `debug!`/`trace!` site; open
each hit and judge whether the site is inside a per-element, per-frame or
per-pixel loop. Expected hits at `e928a40`: the bounded timer loops at
`daemon/mod.rs:306` and `:448` and the retry loop at `daemon/phone.rs:199`;
all are bounded or log only on a transition. Expected result: no
per-element, per-frame or per-pixel site.

**Verify**: write down the list and your judgement for each hit. If any site
is in a hot loop, see STOP conditions.

### Step 2: Record the "before", then flip the feature

First build the current release binary and run the probe from the table:
`cargo build --release -p sky-cua-service` then
`NO_COLOR=1 RUST_LOG=debug ./target/release/sky-cua-service doctor 2>&1 >/dev/null | grep -c ' DEBUG '`
→ `0` (debug is compiled out today). Record it; the after-flip result is only
meaningful as a delta.

Then edit `Cargo.toml` line 67 to:

```toml
tracing = { version = "0.1.41", features = ["release_max_level_debug"] }
```

**Verify**: `cargo build --release -p sky-cua-service -p sky-cua-client` → exit 0;
`git diff --quiet -- Cargo.lock` → exit 0 (features are not recorded in the lockfile).

### Step 3: Prove it, headlessly

Run `NO_COLOR=1 RUST_LOG=debug ./target/release/sky-cua-service doctor 2>&1 >/dev/null | grep -c ' DEBUG '`
→ ≥ 1. The line comes from `env_probe.rs:16` (`probing Linux desktop
environment`), which `doctor` reaches unconditionally. `doctor` may exit
non-zero on a headless box after printing its report; only the stderr grep
matters.

Then run the same with `RUST_LOG=trace` and `grep -c ' TRACE '` → `0`. This
half is meaningful only when `DBUS_SESSION_BUS_ADDRESS` is reachable, because
zbus is the only `trace!` emitter in the dependency graph `doctor` touches
(the workspace itself has no `trace!` sites); without a session bus, record
"trace check vacuous" and do not STOP.

**Verify**: before/after debug counts are `0` and ≥ 1; trace count is `0` (or
recorded as vacuous). Record the exact commands and one DEBUG line in the
plan's status row note.

### Step 4: Write the runbook

Create `docs/operations/daemon-logs.md` with exactly this content:

```markdown
# Daemon logs and log levels

Operator runbook for finding the sky-cua daemon's logs and raising the log
level on an installed (release) build.

## Where the logs are

When the MCP client spawns the daemon it captures stderr into a per-endpoint
file under the state directory and passes the same path to the daemon via
`SKY_CUA_DAEMON_LOG_PATH`, so tracing output goes through a self-rotating
writer:

- Linux: `~/.local/state/sky-cua/daemon-<socket-stem>.log` (the isolated
  desktop daemon gets its own file)
- Rotation: past 8 MiB, one `.log.old` generation is kept
- A daemon started by hand (`sky-cua-service daemon`) has no log path set and
  logs to stderr

See `docs/runtime/mcp-boundary.md` ("Daemon logs") for the contract.

## Raising the log level

Both binaries read `RUST_LOG` through `tracing_subscriber::EnvFilter`; the
default is `info`. Release builds compile `debug!` in and `trace!` out
(`tracing` feature `release_max_level_debug` in the root `Cargo.toml`), so:

- `RUST_LOG=debug` works on installed binaries.
- `RUST_LOG=trace` behaves like `debug` on release builds. For trace output
  use a dev build (`cargo build -p sky-cua-service`, which ignores the
  release cap) and point the client at it with `SKY_CUA_SERVICE_PATH`.
- Prefer target-scoped filters, e.g.
  `RUST_LOG=info,sky_cua_service=debug,sky_cua_linux=debug`. A bare
  `RUST_LOG=debug` also enables every dependency's debug output (zbus alone
  has a dozen sites), rotates the 8 MiB log faster, and only one `.log.old`
  is kept, so the forensic window shrinks. Restore the default after
  capturing.

How the variable reaches the daemon: the daemon inherits the MCP client's
full environment (the spawn in `crates/sky-cua-client/src/service_launcher.rs`
does not clear it), so `RUST_LOG` must reach the client. It is not in the
plugin's `.mcp.json` `env_vars`; hosts that filter MCP server environments
(Codex) need it set explicitly in that server's `env` configuration. The
daemon is long-lived and shared: a new value takes effect only when the
daemon restarts, so stop the targeted daemon (only that one) and let the
client respawn it. Alternatively run a daemon by hand on its own socket:
`RUST_LOG=debug SKY_CUA_SERVICE_SOCKET_PATH=<path> sky-cua-service daemon`;
it logs to stderr.

## Reading a log

Lines are `tracing` fmt output: timestamp, level, enclosing spans with their
fields, target, message, then event fields. Files contain ANSI colour codes
(view with `less -R`, or set `NO_COLOR=1` in the daemon's environment).
Desktop CUA actions and phone requests carry `session_id`/`turn_id` fields at
debug level; the browser lane tags `target`/`tab_id`. There is no per-request
span yet.
```

Then append one sentence after line 38 of `docs/runtime/mcp-boundary.md`
(the end of the "Daemon logs" paragraph): "Operator steps for raising the
level are in `docs/operations/daemon-logs.md`."

**Verify**: `test -e docs/operations/daemon-logs.md`;
`grep -n 'daemon-logs.md' docs/runtime/mcp-boundary.md` → one match;
`grep -c 'release_max_level_debug\|NO_COLOR' docs/operations/daemon-logs.md` → ≥ 2.

### Step 5: Gate

**Verify**: `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo nextest run --workspace`
→ exit 0 (the feature change affects only release builds; the test gate
runs debug builds and must be unaffected).

## Test plan

No new automated tests; the proof is the Step 3 probe on the release
binaries. Existing tests unchanged.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `grep -n 'release_max_level_debug' Cargo.toml` matches and `grep -n 'release_max_level_info' Cargo.toml` prints nothing
- [ ] `cargo build --release -p sky-cua-service -p sky-cua-client` exits 0; the `NO_COLOR=1` probe printed `0` before the flip and ≥ 1 after; the trace probe printed `0` or was recorded as vacuous
- [ ] `git diff --quiet -- Cargo.lock` exits 0
- [ ] `test -e docs/operations/daemon-logs.md` and `grep -c 'release_max_level_debug\|NO_COLOR' docs/operations/daemon-logs.md` ≥ 2
- [ ] `grep -n 'daemon-logs.md' docs/runtime/mcp-boundary.md` matches
- [ ] the Rust gate command exits 0
- [ ] `git diff --name-only e928a40 -- Cargo.toml docs/` lists exactly `Cargo.toml`, `docs/operations/daemon-logs.md`, `docs/runtime/mcp-boundary.md` (holds before and after an authorised commit)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- Step 1 finds a `debug!` inside a per-element, per-frame or per-pixel loop.
  Report the site; the maintainer decides whether to demote it to `trace!`
  before flipping the feature.
- The release build fails after the feature change.
- The after-flip probe prints `0` with `NO_COLOR=1 RUST_LOG=debug` (then
  `env_probe.rs:16` is no longer reached by `doctor`, or the feature did not
  take; report rather than switch to `sky-cua-client health`).
- `git diff --quiet -- Cargo.lock` exits non-zero.

## Maintenance notes

- `trace!` is still compiled out of release builds (the workspace emits
  none; dependencies such as zbus do). If a field debugging session needs
  trace-level output it needs a dev build pointed at via
  `SKY_CUA_SERVICE_PATH`; that is a deliberate choice to keep the hottest
  paths free of even filter checks.
- The static max level is global to the build, so this also compiles in
  dependencies' `debug!` sites. The runbook's target-scoped filter advice
  exists for that reason.
- The second half of `DX-03`, a per-request `info_span!` carrying
  `session_id`/`turn_id` across client → service → backend so logs can be
  correlated by turn, is not done here. When it is, update the runbook's
  "Reading a log" section.
- Adding `RUST_LOG` to `.mcp.json` `env_vars` (and the installer forwarding
  lists, with the `test_env_key_contract.py` consequences) would let Codex
  users set it without editing the server config by hand. That is a contract
  change, left as a follow-up.
