---
name: cua-deploy
description: >-
  Build, package, install, or refresh the standalone fixed-root sky-cua
  distribution; restart its standalone MCP runtime; or perform an explicitly
  requested git closeout. Use for checkout installs, clean-target tarball
  handoffs, and deployment verification. Do not use for tests or smokes alone,
  reviews, docs-only work, Gradle-only Android builds, or unrelated code changes.
---

# cua-deploy

Use the smallest requested operational lane. The deployment surface has one
mutable install tree and two canonical commands:

```bash
python3 install.py build
python3 install.py install
```

`build` refreshes durable outputs under `target/`, `out/`, and `dist/`, producing
`dist/sky-cua-linux-x64-glibc.tar.gz`. `install` from a checkout builds or
refreshes those outputs and then installs them. The same `install.py` inside an
extracted archive installs that artifact without a checkout build.

`python3 install.py release` is a separate checkout-only Git publication
operation, not a deploy lane. It requires explicit release wording and triggers
the Gitea-to-Saga pipeline after pushing its commit and tag.

## Hard boundaries

- Install only into `${XDG_DATA_HOME:-~/.local/share}/sky-cua`. Do not invent
  release IDs, generations, `releases/`, a `current` symlink, manifest-hash
  arguments, activation receipts, staging trees, rollback, or verification
  subcommands.
- Do not prebuild Browser Use manually or build in a temporary checkout. Reuse
  the canonical durable build outputs.
- Installation validates the artifact, replaces the fixed tree directly, and
  projects integrations. Do not add backup or power-loss machinery.
- Build portability overrides such as `RUSTFLAGS=-Ctarget-cpu=x86-64-v3` are
  allowed when a machine-wide Cargo configuration selects unsupported native
  CPU instructions. Do not make them part of the artifact contract.
- Deploy/build/install/restart do not authorize `git commit` or `git push`.
  Each Git action requires explicit wording. Preserve unrelated work.

## Lanes

### Build a transferable archive

From the checkout:

```bash
python3 install.py build
```

Inspect `dist/sky-cua-linux-x64-glibc.tar.gz`. Load
`references/release-package.md` for the required artifact shape and target
handoff.

### Install from a checkout

Run exactly:

```bash
python3 install.py install
```

This is the normal checkout deployment command; do not precede it with a manual
Browser Use build or the archive build unless the user separately requested an
archive. Load `references/local-deploy.md` for projected integrations and live
evidence.

### Install an extracted archive

On the target:

```bash
tar xzf sky-cua-linux-x64-glibc.tar.gz
cd sky-cua-linux-x64-glibc
python3 install.py install
```

There is no hash argument or follow-up activation command. Validate the fixed
installed paths and, when requested, the actual host integration.

### Restart a standalone MCP runtime

For a configuration refresh without a full distribution rebuild or deploy. This
refreshes the standalone MCP installation before restarting its runtime:

```bash
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime
```

Do not add `--refresh-accessibility` unless AT-SPI is proven wedged. Load
`references/rare-operations.md` for accessibility or OpenClaw reload cases.

### Explicit Git closeout

Only when explicitly requested, use scoped pathspecs and preserve unrelated
changes. If Git closeout depends on a successful deployment, stop on the first
failed prerequisite. Load `references/git-closeout.md`.

## Completion evidence

Report the exact commands and outcomes. For packaging, report the archive path
and inspected shape. For installation, verify the fixed root, stable launchers,
three projected skills, native messaging manifests, and both Codex plugins in
the local marketplace. When OpenClaw is in scope, verify global `node_repl` uses
the fixed tree; OpenClaw itself owns per-agent native plugin reconciliation.

For a live OpenClaw acceptance, require the requested provider/model with
`fallbackUsed=false`, Computer Use on the intended desktop session, and Browser
Use reporting `extension_native_host` with `isIab=false`. State any live gates
not run. Load `references/troubleshooting.md` after a failure.

Relevant source checks remain proportional to changed code: Rust `cargo fmt
--check && cargo clippy --workspace --all-targets && cargo nextest run`; Python
`uv run ruff format --check scripts && uv run ruff check scripts && uv run
basedpyright && uv run pytest`. A pure install or restart does not imply broad
source validation.
