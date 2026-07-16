---
name: cua-deploy
description: >-
  Use for sky-cua operational requests in one of four lanes: local plugin
  deploy/rebuild/install, release package build or target bundle install,
  standalone MCP runtime restart, or explicitly requested git commit and/or
  push closeout. Deploy/build/package/install/restart never authorize git
  actions; commit and push each require explicit wording, and push is an
  external write. Do not use for Gradle-only Android companion builds,
  config/skill-sync-only tasks, tests or smokes, reviews, docs, or code changes
  outside these lanes.
---

# cua-deploy

Route the request into the smallest requested sky-cua operational lane. A bare
“deploy” means local deploy; choose release packaging only when a clean target,
tarball, or bundle install is requested.

## Mandatory plan contract

Every plan and report must copy the selected lane's exact commands, order, and stop dependency rather than paraphrasing them:

- Local: `python3 scripts/deploy_plugin.py`, then only on success `python3 scripts/sync_agent_skills.py`. The first command already builds, bundles, installs, and refreshes the runtime; never split it.
- Release: `python3 scripts/package.py`, inspect the tarball; target handoff is `tar xzf sky-cua-<version>-<platform>.tar.gz`, `cd sky-cua-<version>`, `python3 install.py`, using the bundled runtime without Cargo. Do not mutate the local runtime or global skill links.
- Standalone restart: exactly `python3 scripts/install_mcp_server.py --host claude-code --restart-runtime`, without build/deploy/package/sync. Do not add `--refresh-accessibility` unless AT-SPI is proven wedged; report that decision.
- Git: only when explicitly requested, with explicit pathspecs; stop before git writes when the prerequisite lane fails, and never push after a failed deploy or commit.

Always report each exact command's result, unrelated-work preservation, skipped validation/live-smoke gates, and whether commit or push was intentionally not authorized.

## Contract

- Deploy, build, package, install, and restart are local operational actions.
  None authorizes `git commit` or `git push`.
- `git commit` requires an explicit request to commit. `git push` requires an
  explicit request to push. If both are explicitly named, commit first and push
  only after it succeeds; push is an external write. Never infer either action
  from “ship”, “release”, “deploy”, or similar wording, and never add automatic
  git behavior.
- Preserve unrelated worktree changes. The repository has no marketplace or
  publish flow.

## Lane router

### Local deploy

For unreleased local iteration, the mandatory sequence is:

```bash
# Step 1
python3 scripts/deploy_plugin.py
# Step 2, only when step 1 exits successfully
python3 scripts/sync_agent_skills.py
```

This order is normative: `deploy_plugin.py` is the build, bundle, install, and
runtime-refresh entrypoint. Do not split it into `build_plugin.py` plus a
manual installer. No build, install, or sync command may precede step 1, and
step 2 must never run before it succeeds. Plans and reports must name both
exact commands in this order. The sync only replaces the sky-cua-owned global
skill links. Load
`references/local-deploy.md` for link ownership, worktree caveats, or local
deploy behavior; load `references/command-and-flag-catalog.md` for variants;
load `references/android-phone-companion.md` when the deploy reports companion
status; load `references/rare-operations.md` for KWin, AT-SPI, freshness, or
live-smoke details.

### Release package

For a clean machine, use this exact order:

```bash
python3 scripts/package.py
# On the target:
tar xzf sky-cua-<version>-<platform>.tar.gz
cd sky-cua-<version>
python3 install.py
```

Inspect the tarball before target handoff. The target install uses the
bundled runtime and does not run Cargo. Load
`references/release-package.md` for package evidence and target install;
`references/command-and-flag-catalog.md` for flags.

### Standalone MCP restart

For an MCP configuration refresh without a full deploy, run:

```bash
python3 scripts/install_mcp_server.py --host claude-code --restart-runtime
```

This combined command is the standalone lane; do not replace it with a full
deploy or split it into unrelated helpers. Do not run skill sync. Load
`references/command-and-flag-catalog.md` for host/flag variants and
`references/rare-operations.md` for AT-SPI or OpenClaw reload cases.

### Explicit git closeout

Use this lane only for explicitly requested commit and/or push work, either
alone or after a named operational lane. Respect any user-stated dependency
such as “after deploy succeeds”. Stop if deploy or commit fails; never push
after a failed prerequisite. Load `references/git-closeout.md`.

## Stop and evidence

Execute only the selected lane and its required local substeps. If a required
step fails, stop dependent steps and report the exact failed command; do not
claim downstream success. Load `references/troubleshooting.md` for a lane
failure or unexpected result. After success, capture the lane’s concrete evidence:
installed runtime/result and skill-sync result for local deploy, tarball path
and shape for release packaging, restart result for MCP refresh, or commit ID
and push result for git closeout. For restart, report whether accessibility
refresh was used and which live-smoke gates were not run. Run only narrow checks relevant to the
request, and state skipped checks and live-smoke gates. Before any live test,
prove the deployed binary is fresh; load `references/rare-operations.md`.

## Relevant checks

Select only checks relevant to the changed seam: Rust `cargo fmt --check &&
cargo nextest run`; Python `uv run ruff format --check scripts && uv run ruff
check scripts && uv run basedpyright && uv run pytest`. A standalone restart
does not imply source checks. Run broader checks only when shared contracts
changed or the user asks for them. Packaging also requires inspecting the
staged bundle shape.
