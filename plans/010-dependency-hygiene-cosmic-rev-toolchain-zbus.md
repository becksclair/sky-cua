# Plan 010: Move `cosmic-protocols` past its MIT relicense, pin the Rust toolchain, and stop Dependabot from offering zbus 6 alone

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- Cargo.toml Cargo.lock rust-toolchain.toml .github/dependabot.yml crates/sky-cua-cosmic-helper/Cargo.toml crates/sky-cua-cosmic-helper/src/main.rs`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live files before proceeding; on a semantic mismatch, treat it as a
> STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: migration
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

Three small dependency-hygiene defects, each one file:

1. The workspace pins `cosmic-protocols` to git rev `160b086`, whose
   `Cargo.toml` declares `license = "GPL-3.0-only"`. It is compiled into
   `sky-cua-cosmic-helper`, a binary shipped in the MIT-licensed standalone
   Linux archive (`scripts/_plugin_bundle.py:45` lists it among the runtime
   binaries). Upstream relicensed to MIT at commit `a2da481` (2026-08-13);
   the newest commit in the local cargo git database is `c0cff4d`
   (2026-09-11), also MIT. Between `160b086` and `c0cff4d` the only XML
   changes are to `cosmic-corner-radius`, `cosmic-keyboard-layout` and
   `cosmic-session-lock-layer`; the two protocols the helper uses
   (`toplevel_info`, `toplevel_management`) are untouched, so the bump is a
   license fix with no behaviour change. One side effect: upstream raised its
   `bitflags` requirement from `2.9` to `2.13` (commit `e95d895`), and
   `Cargo.lock` pins `bitflags 2.11.1`, so the lockfile diff is exactly two
   packages (`cosmic-protocols` and `bitflags`).
2. `rust-toolchain.toml` says `channel = "stable"`, so every six-week Rust
   release can turn `just verify` (which runs clippy with `-D warnings`) red
   with no code change. Commit `c30005b` ("satisfy rustc/clippy 1.99 lints",
   8 files) is the most recent instance. Release binaries are also built with
   whatever stable happens to be installed. Pinning the exact version the
   code was last validated against (1.99.1, which `stable` resolves to on the
   dev host today; no separately named `1.99.1` toolchain is installed yet)
   makes toolchain bumps deliberate commits. Because the pin names a toolchain
   that is not installed, the first `cargo` invocation after the change would
   make rustup download it; that install needs operator approval first.
3. Dependabot has an open PR (#22) bumping `zbus` 5.19 → 6.0. `ashpd`,
   `atspi` and `atspi-connection` still depend on zbus 5, so merging it would
   link two zbus stacks and two sets of `zvariant` types; a `zbus::Connection`
   from this crate could no longer be handed to ashpd or atspi. The cargo
   Dependabot block has no `ignore` rules. Adding one for zbus semver-major
   stops the PR from being re-offered until ashpd and atspi move together.

Closing PR #22 itself is an external action for the operator, not the
executor; this plan records the recommendation.

## Current state

- `Cargo.toml` line 49 (verbatim):

  ```toml
  cosmic-protocols = { git = "https://github.com/pop-os/cosmic-protocols", rev = "160b086", default-features = false, features = ["client"] }
  ```

  `Cargo.lock` lines 492-494 pin
  `source = "git+https://github.com/pop-os/cosmic-protocols?rev=160b086#160b086abe03cd34a8a375d7fbe47b24308d1f38"`.
  The only consumer is `crates/sky-cua-cosmic-helper/Cargo.toml:9`
  (`cosmic-protocols.workspace = true`); `crates/sky-cua-cosmic-helper/src/main.rs:10-13`
  imports `cosmic_protocols::toplevel_info::v1::client::{zcosmic_toplevel_handle_v1, zcosmic_toplevel_info_v1}`
  and `cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1`.
  `sky-cua-cosmic-helper` is NOT a workspace default member, so it must be
  built with an explicit `-p`.

  Local cargo git database (`~/.cargo/git/db/cosmic-protocols-*`) already
  contains `a2da481` and `c0cff4d`; both show `license = "MIT"` and
  `version = "0.2.0"` in `Cargo.toml`. `c0cff4d` is a direct child of the
  relicense commit `a2da481` and is the rev pop-os libcosmic itself pins for
  `cctk`; in the local database `origin/HEAD` is `a2da481` and `c0cff4d` is
  cached under `refs/commit/`, so "tip of the default branch" cannot be
  confirmed offline (either rev is acceptable; both need the same `bitflags`
  move). `git diff --stat 160b086..c0cff4d` touches `Cargo.toml` (including
  `bitflags = "2.13"`), `README.md`, `client-toolkit/*`, `src/LICENSE`,
  `src/lib.rs` (SPDX line, `doc_cfg`, two new modules `keyboard_layout` and
  `session_lock_layer`, a `corner_radius` dependency), and three
  `unstable/*.xml` files none of which the helper uses. The
  `toplevel_info::v1` / `toplevel_management::v1` paths and the `client`
  feature are unchanged. `Cargo.lock:292-293` pins `bitflags 2.11.1`;
  `bitflags-2.13.2.crate` is in the local registry cache. No crates.io
  `cosmic-protocols` release is cached.

- `rust-toolchain.toml` (verbatim):

  ```toml
  [toolchain]
  channel = "stable"
  components = ["rustfmt", "clippy"]
  ```

  `rustc --version` on the dev host: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`.
  `rustup toolchain list` shows `stable`, `nightly`, `1.92.0`, `1.93`,
  `1.93.0`, `1.97.0`; there is no named `1.99.1-x86_64-unknown-linux-gnu`.
  rustup 1.29.1 auto-installs a missing pinned toolchain on first use unless
  `RUSTUP_AUTO_INSTALL=0`. The `stable` toolchain also has cross targets
  installed (`x86_64-pc-windows-gnu`, `x86_64-pc-windows-msvc`,
  `x86_64-apple-darwin`, four Android targets); a fresh `1.99.1` install
  has only the host target. `docs/features/display-targeted-screenshots.md:157-158`
  documents `cargo check -p sky-cua-windows --target x86_64-pc-windows-gnu`.
  Root `Cargo.toml` has no `rust-version` field.

- `.github/dependabot.yml` lines 7-17 (verbatim):

  ```yaml
    - package-ecosystem: cargo
      directory: /
      schedule:
        interval: weekly
      groups:
        cargo-security:
          applies-to: security-updates
          patterns: ["*"]
        cargo-minor-patch:
          applies-to: version-updates
          update-types: [minor, patch]
  ```

  Open Dependabot cargo PRs on 2026-11-03 (`gh pr list`): #20 sctk 0.21.1,
  #21 reis 0.7.1, #22 zbus 6.0.0, #23 wgpu 30.0.0, #24 if-addrs 0.15.0.
  `cargo tree -i zbus@5.19.0 -e normal --depth 1` shows `ashpd 0.13.13`,
  `atspi 0.30.0`, `atspi-common/-connection/-proxies 0.14.0` all on
  `zbus 5.19.0`. 22 files under `crates/` mention `zbus` (18 in
  `sky-cua-linux`, including its example).

- CI (`.gitea/workflows/verify.yml`) and the release workflow
  (`.gitea/workflows/release-standalone.yml`, same `saga-sky-cua-build`
  runner, runs `just verify-rust` and `python3 install.py build`) both pick
  up the pin, as do native Windows builds of the Windows binaries. Whether
  the runner uses `rustup` (which honours `rust-toolchain.toml` and
  auto-installs the pinned version) or a system `cargo` is not recorded
  anywhere in the repo. See STOP conditions.

Conventions: Rust deps are workspace-managed in root `Cargo.toml`
(`AGENTS.md` Conventions). Commit style: conventional, e.g.
`build(deps): update atspi, base64, getrandom, and tokio-tungstenite majors`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Update the git pin in the lockfile (offline, the rev is cached) | `cargo update -p cosmic-protocols --offline` | `Cargo.lock` now references `c0cff4d`; `bitflags` moves to ≥ 2.13 |
| Is 1.99.1 installed as a named toolchain? | `rustup toolchain list \| grep '^1\.99\.1-'` | prints nothing at `e928a40` (not installed) |
| Build the only consumer | `cargo build -p sky-cua-cosmic-helper` | exit 0 |
| Confirm the rev and license | `cargo metadata --format-version 1 \| python3 -c "import json,sys; [print(p['source'], p['license']) for p in json.load(sys.stdin)['packages'] if p['name']=='cosmic-protocols']"` | one line, source contains `c0cff4d`, license `MIT` |
| Any GPL left? | `cargo metadata --format-version 1 \| python3 -c "import json,sys; print([p['name'] for p in json.load(sys.stdin)['packages'] if p.get('license') and 'GPL' in p['license'] and 'LGPL' not in p['license']])"` | `[]` |
| Toolchain in effect | `rustup show active-toolchain` | `1.99.1-x86_64-unknown-linux-gnu (overridden by '/home/bex/projects/sky-cua/rust-toolchain.toml')` |
| Full Rust gate | `just verify-rust` | exit 0 |
| YAML sanity | `python3 -c "import yaml; yaml.safe_load(open('.github/dependabot.yml'))"` | exit 0 |

## Scope

**In scope** (the only files you should modify):
- `Cargo.toml` (line 49 only)
- `Cargo.lock` (via `cargo update -p cosmic-protocols`; no hand edits)
- `rust-toolchain.toml`
- `.github/dependabot.yml` (add an `ignore` block to the cargo ecosystem entry)

**Out of scope** (do NOT touch):
- Closing, merging or commenting on any pull request. Report the
  recommendation to close #22; the operator does it.
- The other four cargo Dependabot PRs (#20, #21, #23, #24). Each is its own
  migration; #24 (if-addrs) is trivial and is noted in `plans/README.md`.
- `rust-version` in `[workspace.package]`. It has no effect unless every
  crate manifest adds `rust-version.workspace = true`; separate decision.
- A `deny.toml` / license allow-list. Useful, but a new tool in the gate is a
  separate decision.
- Any code in `crates/`.

## Git workflow

- Branch: `bex/deps-hygiene-cosmic-toolchain-zbus`
- Commit only if the operator authorised it; one commit per step is natural:
  `build(deps): move cosmic-protocols to the MIT-relicensed c0cff4d`,
  `build: pin the Rust toolchain to 1.99.1`,
  `build(deps): ignore zbus semver-major until ashpd and atspi move`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Bump the `cosmic-protocols` rev

Edit `Cargo.toml` line 49: change `rev = "160b086"` to `rev = "c0cff4d"`.
Then run `cargo update -p cosmic-protocols --offline`. If `--offline` fails
because the rev is not in the local git database, drop `--offline` (network
fetch of the pop-os repository); that is acceptable.

**Verify**:
- `grep -n 'cosmic-protocols' Cargo.lock` → the `source` line contains `rev=c0cff4d#c0cff4d`
- `cargo build -p sky-cua-cosmic-helper` → exit 0
- the "Confirm the rev and license" command → source contains `c0cff4d`, license `MIT`
- the "Any GPL left?" command → `[]`
- `git diff --stat` → only `Cargo.toml` and `Cargo.lock` changed, and
  `git diff Cargo.lock` touches exactly two `[[package]]` entries: the
  `cosmic-protocols` `source` line, and `bitflags` (version and checksum,
  2.11.1 → 2.13.2 offline; online resolution may pick a newer 2.x, which is
  also acceptable). Any other package changing is a STOP.

### Step 2: Pin the toolchain

Before editing, run `rustup toolchain list | grep '^1\.99\.1-'`. On the dev
host at `e928a40` it prints nothing: 1.99.1 exists only as what `stable`
currently resolves to. rustup auto-installs a missing pinned toolchain on the
first `cargo`/`rustc` call, which is a local environment change (a download
of about 200 MB plus `rustfmt` and `clippy`). Get the operator's approval
first; once approved run
`rustup toolchain install 1.99.1 --profile minimal -c rustfmt -c clippy`.
Without approval, STOP before editing the file. Run every Step 2
verification with `RUSTUP_AUTO_INSTALL=0` in the environment so nothing is
downloaded silently.

Then replace `rust-toolchain.toml` with:

```toml
[toolchain]
# Pinned on purpose: `just verify-rust` runs clippy with -D warnings, and a
# floating `stable` turned the gate red on each toolchain release (see
# c30005b). Bump deliberately, together with any lint fixes the new release
# needs.
channel = "1.99.1"
components = ["rustfmt", "clippy"]
```

**Verify**: `RUSTUP_AUTO_INSTALL=0 rustup show active-toolchain` → begins with
`1.99.1-`; `RUSTUP_AUTO_INSTALL=0 just verify-rust` → exit 0 (fmt, clippy,
nextest all pass on the pinned toolchain; this also rebuilds with the new
cosmic-protocols rev).

### Step 3: Ignore zbus semver-major in Dependabot

In `.github/dependabot.yml`, inside the `package-ecosystem: cargo` entry,
add after the `groups:` block (same indentation as `groups:`):

```yaml
    ignore:
      # zbus 6 cannot be adopted alone: ashpd and atspi still build on zbus 5,
      # and two zbus stacks cannot share a Connection. Lift this when both
      # publish zbus-6 releases and bump all three in one change.
      - dependency-name: zbus
        update-types: ["version-update:semver-major"]
```

**Verify**: the YAML sanity command → exit 0;
`grep -n 'dependency-name: zbus' .github/dependabot.yml` → one match.

### Step 4: Report the external action

In your completion report (and in the `plans/README.md` status row), state:
"PR #22 (zbus 6.0.0) should be closed by the operator; Dependabot will not
re-open it while the ignore rule stands." Do not close it yourself.

## Test plan

No new tests. Verification is the existing gate on the pinned toolchain plus
the metadata checks above.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `grep -c 'rev = "c0cff4d"' Cargo.toml` prints `1`, and `grep -q '160b086' Cargo.toml Cargo.lock` exits non-zero (no match in either file)
- [ ] the "Any GPL left?" command prints `[]`
- [ ] `cargo build -p sky-cua-cosmic-helper` exits 0
- [ ] `grep -n 'channel = "1.99.1"' rust-toolchain.toml` matches
- [ ] `rustup show active-toolchain` begins with `1.99.1-`
- [ ] `just verify-rust` exits 0
- [ ] `grep -n 'dependency-name: zbus' .github/dependabot.yml` matches and the YAML parses
- [ ] `git diff --name-only e928a40 -- Cargo.toml Cargo.lock rust-toolchain.toml .github/` lists exactly those four files (holds before and after committing; untracked `plans/*.md` files are not in the path filter)
- [ ] `plans/README.md` status row updated, including the PR #22 note

## STOP conditions

Stop and report back (do not improvise) if:

- `cargo update -p cosmic-protocols` changes any lockfile entry other than
  `cosmic-protocols` and `bitflags`.
- `cargo build -p sky-cua-cosmic-helper` fails after the bump (then the
  `toplevel_*` protocol bindings changed after all; report the error).
- The "Any GPL left?" command prints a non-empty list (another GPL dependency
  exists; report it, do not try to replace it).
- `rustup` is not installed, or 1.99.1 is not an installed toolchain
  (`rustup toolchain list`) and the operator has not approved installing it.
- You have any reason to believe the Gitea runner builds with a system
  `cargo` rather than `rustup` (for example a runner setup script in the
  repo or documentation says so). A pinned channel would then be ignored or
  fail there; report before committing Step 2.

## Maintenance notes

- Bumping the toolchain is now a deliberate change: edit `channel`, run
  `just verify-rust`, fix lints, commit together. Consider doing it on the
  Rust release cadence (every six weeks) so lint drift stays small.
- The CI runner must have `rustup` with the pinned toolchain available (or
  network access to install it on first use). If the first CI run after
  this change fails at toolchain resolution, that is the runner's setup, not
  the pin. The pin applies equally to `release-standalone.yml` (same runner)
  and to native Windows builds.
- The pinned toolchain is a separate rustup install. Cross targets installed
  on `stable` (for example `x86_64-pc-windows-gnu`, used by
  `docs/features/display-targeted-screenshots.md:157`) must be re-added with
  `rustup target add --toolchain 1.99.1 <target>` where they are needed.
- When ashpd and atspi publish releases built on zbus 6, remove the
  Dependabot ignore and bump `zbus`, `ashpd`, `atspi`, `atspi-connection` in
  one commit; 22 Rust files import `zbus` (15 in `sky-cua-linux`).
- `cosmic-protocols` still has no crates.io release; if one appears with the
  `toplevel_*` protocols, switching from the git pin to a version is the next
  tidy-up. The reviewer should check that the lockfile diff is exactly
  `cosmic-protocols` + `bitflags`.
