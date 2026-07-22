# Ship and consume one standalone sky-cua installation

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` current while executing it.

This document is governed by `~/.agents/PLANS.md` and `plans/AGENTS.md`. It is
self-contained for execution from the three current working trees. Planning is
read-only beyond this file; implementation, commits, pushes, installation, and
deployment still require Bex's approval.

## Purpose / Big Picture

sky-cua will ship one target-specific standalone archive and install one mutable
tree at `${XDG_DATA_HOME:-~/.local/share}/sky-cua`. There will be no generation
store, `current` symlink, retained rollback, release pin, manifest-hash argument,
consumer checksum, or consumer trust allowlist.

The public workflow is exactly:

    python3 install.py build
    python3 install.py install

`build` assembles every required Rust, Browser Use, Node, extension-host, plugin,
skill, and installer input through durable reusable outputs under `out/` and
`dist/`, then writes one archive under `dist/`. `install` from an extracted
archive installs that archive; `install` from a checkout builds or refreshes the
same durable staged payload and installs it. The install also creates
stable launchers, registers the external Chrome/Chromium extension native host,
projects the three sky-cua routing skills, and configures the supported local
hosts detected on that machine. There are no separate build, plugin, skill, MCP,
activation, or verification commands in the normal path.

Codex Desktop and OpenClaw use the fixed installed paths. They do not pin a
release, commit, SHA, component tree, Browser client, marketplace manifest, or
plugin bytes. They do not copy a sky-cua release into their own package. Native
Codex `plugin/install` remains the mechanism that copies and enables the two
plugins in each Codex home; consumers treat a successful native install request
as authoritative instead of re-reading and hashing the result.

Observable success is:

- a fresh checkout needs only `python3 install.py build` to produce the archive,
  including when `packages/browser-use/build` and all other generated inputs are
  initially absent;
- `python3 install.py install` leaves one complete tree at
  `~/.local/share/sky-cua`, plus working stable launchers, native-host registration,
  plugins, and skills;
- installing a second artifact replaces that tree, with no retained generations;
- unchanged Codex Desktop and OpenClaw builds consume the replacement without a
  repin or source edit; and
- live Computer Use and external-extension Browser Use calls succeed through
  Codex Desktop and through an OpenClaw Codex-harness agent with model fallback
  disabled or explicitly reported as unused.

## Progress

- [x] Read `~/.agents/PLANS.md`, this repository's root and `plans/AGENTS.md`,
  Codex Desktop's `AGENTS.md`, and OpenClaw's root rules and `bex-fork.md` seam.
- [x] Confirmed the current producer has two overlapping installer families,
  requires prebuilt component inputs, content-addresses releases, promotes them
  through `GenerationStore`, writes activation receipts, and resolves `current`.
- [x] Confirmed Codex Desktop's `scripts/sky-cua-release.cjs` embeds release,
  manifest, producer, Browser, component, marketplace, and plugin-tree hashes,
  and that packaging/install scripts copy and activate the pinned release.
- [x] Confirmed OpenClaw's
  `extensions/codex/src/app-server/managed-native-plugins.ts` validates a
  hash-heavy producer schema and hashes each installed plugin tree.
- [x] Inspected sibling Codex source. `plugin/install` accepts an absolute local
  `marketplacePath` plus `pluginName`, installs the local plugin, updates config,
  and enables it. No caller-provided checksum is part of the protocol.
- [x] Implement and prove the sky-cua artifact and fixed-root installer.
- [x] Re-read the governing instructions in the delegated implementation task,
  confirmed the producer worktree is otherwise clean, and selected the local
  package-build lane without authorizing live installation or Git writes.
- [x] Replace the producer build/runtime/install contract and focused tests.
- [x] Changed core plugin staging to carry only the highest bundled Chrome
  extension manifest version; the standalone assembler rejects any core bundle
  that contains zero or multiple extension trees, flattens that one tree to
  `browser/extension`, and removes the inherited duplicate resource tree.
- [x] Ran `python3 install.py build` through the durable outputs. It reused the
  existing release-profile Rust build, verified the 3,804-file cua-node tree,
  staged one Chrome extension, and produced
  `dist/sky-cua-linux-x64-glibc.tar.gz`.
- [x] Extracted that archive and ran its own `install.py install` with isolated
  HOME/XDG/PATH roots. It installed one `data/sky-cua` tree, six stable
  launchers, four browser native-host manifests, and three projected skills
  without detecting or changing live Codex/OpenClaw state.
- [x] The standalone builder/replacement/host-detection and plugin-bundle focused
  suite passes (49 tests, including 4 standalone installer tests and the latest
  extension-selection cases).
- [x] Removed the Browser-client hash field from the cua-node manifest/schema,
  host module policy, runtime manager, verifier, Browser package tests, and
  production live-acceptance harness. The harness now binds the installed client
  to `components.browser_use.entrypoint`; its focused suite passes (21 passed)
  and cua-node typecheck passes. Only a negative contract assertion and frozen
  upstream-5307 evidence retain the retired variable name.
- [x] Prove the producer from a durable clean clone and isolated HOME/XDG
  roots, then run the convergence gate.
- [ ] Simplify Codex Desktop in dedicated task
  `019f89f0-9b87-7c33-9d82-b1524fe8d370`. Implementation is active; its
  current focused convergence suite passes 95 tests while it closes remaining
  generated-patch and full-build gates.
- [x] Simplified OpenClaw in dedicated task
  `019f89f0-975b-7fe1-8293-52bee0d04aa1` and updated `bex-fork.md`; 28 focused
  tests pass. Its remote convergence build is explicitly blocked by the
  unavailable Crabbox/Testbox backend.
- [x] Deleted the obsolete release-generation verifier, OpenCode release helper,
  OpenClaw release-only installer path, their focused tests, and the retired
  complete-release activation runbook. Moved the four ordinary cua-node
  artifact helpers into `scripts/_artifact_helpers.py`.
- [x] Completed broad Ultra Review and fixed its producer findings: validate the
  overlay host before replacement, select only a canonical keyed Codex/ChatGPT
  extension, avoid per-module Browser trust hashing in the embedded kernel,
  remove stale hash-contract prose, and delete the remaining generation-era
  adapters and runbook.
- [ ] Complete fresh-install, replacement-install, and live consumer acceptance.

## Surprises & Discoveries

- The release artifact is already close to complete, but
  `scripts/build_complete_release.py::_prepare_inputs` copies
  `packages/browser-use/build` instead of owning that build. This caused the
  observed clean-checkout failure and must be fixed at the orchestration owner,
  not with an operator pre-build instruction.

- `scripts/release_generation.py` contained both useful artifact assembly helpers
  and the unwanted content-addressed store. Four ordinary helpers were extracted
  to `scripts/_artifact_helpers.py`; the generation store, verifier, journals,
  rollback, and release-only OpenCode/OpenClaw consumers were deleted.

- Codex Desktop is not merely pinning one value. Its pin fans out through
  `scripts/sky-cua-release.cjs`, Browser cache projection, AST patching,
  packaging, Arch packaging, installed-artifact verification, and tests. The
  consumer task is deletion-first and should remove the whole pinning path.

- Native Codex already owns local marketplace parsing, plugin copying, config
  enablement, and installed cache naming. OpenClaw's byte-for-byte post-install
  verifier duplicates that owner rather than supplying a missing guarantee.
  Evidence is in sibling Codex
  `codex-rs/app-server/src/request_processors/plugins.rs::plugin_install_response`
  and `codex-rs/core-plugins/src/manager.rs::install_plugin`.

- The Browser trust allowlist is not only an installer projection. The bundled
  cua-node host derives `NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S` from its
  runtime manifest and the isolated module policy grants Browser code by that
  digest. The producer change therefore must remove the hash field from the
  runtime manifest/schema and trust the co-installed Browser package by its
  fixed runtime path; changing only consumer environment variables would leave
  the old contract intact.

- The delegated task explicitly authorizes local implementation and validation
  but forbids live installation. All installer acceptance in this producer task
  therefore uses isolated temporary HOME/XDG roots; user launchers, browser
  manifests, Codex homes, OpenClaw state, and running services remain untouched.

- The repository intentionally retains three extracted upstream extension
  versions as source/reference inputs (`1.1.4`, `1.1.5`, and
  `1.2.27203.26575`), and the old generic `resources/` staging copied all three
  into the core plugin. They share the same extension identity. Packaging now
  selects the numerically highest manifest version at the producer boundary, so
  neither the core plugin nor standalone artifact carries historical versions.

- A real clean clone exposed three implicit generated prerequisites: Bun
  dependencies for `runtime/cua-node`, `packages/browser-use`, and
  `packages/sky-cua-js`. The canonical build now runs frozen-lockfile installs
  for all three before their builds. These directories are durable workspace
  caches; reruns reported `no changes` rather than rebuilding dependencies.

- `scripts/build_plugin.py` previously ignored `CARGO_TARGET_DIR` when copying
  freshly built binaries, so a shared durable Cargo cache built successfully but
  staging searched the clone-local `target/`. It now resolves both build stamps
  and runtime binaries from Cargo's configured target root.

- Fresh review found the first standalone tree had merged the core repository
  docs over raw model-documentation source, omitted generated inventories, and
  retained `resources/release/RELEASE.schema.json` with the retired generation
  and Browser-hash contract. The builder now creates a durable
  `out/components/model-documentation` component, replaces `docs/` with it, and
  prunes both retired resource trees. Payload validation requires the four
  inventories and rejects retired contract terms in model/Codex-facing files.

- Cargo fingerprints include checkout paths, so alternating two distinct clones
  against one target directory can still trigger recompilation. Durable
  per-checkout target directories are the preferred steady-state cache;
  `CARGO_TARGET_DIR` support remains correct for callers deliberately sharing a
  compatible cache.

- Ultra Review found that replacement validation did not require
  `bin/sky-cua-overlay-host`, even though launcher installation did. An invalid
  payload could therefore replace the good tree before failing. Validation now
  rejects that payload before the first rename, and a focused regression proves
  the prior installed tree remains intact.

- The same review found that choosing the numerically highest extension
  directory alone could select unrelated or malformed extracted Chrome data.
  Selection now reuses the established identity conditions: a `Codex` or
  `ChatGPT` manifest name, a non-empty key, and a canonical `<version>_0`
  directory matching its numeric manifest version.

- Deleting the retired activation runbook exposed a development-staging edge:
  `git ls-files` includes paths deleted from the worktree. Plugin staging now
  snapshots only tracked paths that still exist; a file disappearing after that
  snapshot remains a hard error. This preserves intentional deletions without
  weakening the copy-time race check.

- The production MCP host imported its supported protocol list and instructions
  from a frozen upstream research fixture. That pulled the fixture's retired
  Browser trust-hash field into the compiled host even though no production code
  used it. Those two stable MCP constants are now production-owned; the frozen
  fixture remains test evidence only, and the compiled host no longer contains
  the retired variable.

- OpenClaw's deletion-first consumer implementation removed 1,238 lines while
  adding 144. Deep review caught two remaining ownership problems: configured
  plugin-owned MCP names could override native plugins, and module-local
  single-flight state could split across source/dist module copies. The consumer
  task fixed both with explicit plugin-owned MCP filtering and process-global
  per-client state, then reran its 28 focused tests. `bex-fork.md` now records
  the fixed-root native-install seam and honest external-extension transport.

## Decision Log

- Decision: use one fixed install root and no resolver command.
  Rationale: consumers can derive stable paths from XDG data home; a resolver,
  active receipt, release selector, and `current` link exist only to support the
  generation model being removed.

- Decision: remove release identity and cryptographic fields from the public
  consumer contract.
  Rationale: the installed tree is the contract. `RELEASE.json` may retain a
  human-readable product version and target for diagnostics, but no consumer
  branches on a commit, content digest, tree digest, or manifest digest.

- Decision: remove caller-supplied Browser client trust hashes.
  Rationale: Browser Use loads the client shipped in the same installed sky-cua
  tree. Codex Desktop and OpenClaw must not compute, inject, compare, or approve
  that file by SHA. Any package-manager integrity or compliance hashes unrelated
  to release selection remain out of scope.

- Decision: let native Codex installation be authoritative.
  Rationale: consumers call `plugin/install` with the fixed local marketplace
  path for `computer-use` and `browser-use`. They do not call `plugin/read` merely
  to attest the copy, hash cache trees, or maintain custom provenance state.

- Decision: preserve OpenClaw's required ownership behavior without preserving
  its verifier framework.
  Rationale: the Codex harness still installs both plugins before the first
  thread in every isolated agent Codex home, and still excludes OpenClaw's global
  `node_repl` MCP projection from Codex to prevent duplicate ownership. The
  global non-Codex `node_repl` MCP remains installed. Collision scans, release
  keys, rollover hashes, marketplace fallbacks, and post-install byte proof go.

- Decision: do not add migration, status, rollback, or compatibility machinery.
  Rationale: the next install replaces the old sky-cua store wholesale. Bex owns
  any manual cleanup of legacy config.

- Decision: keep producer build outputs durable rather than compiling or
  assembling in temporary directories.
  Rationale: the Rust/plugin bundle, cua-node runtime, and flattened standalone
  tree are large and expensive; stable outputs at `dist/plugin/sky-cua`,
  `out/components/cua-node-linux-x64-glibc`, and
  `dist/standalone/sky-cua-linux-x64-glibc` preserve incremental/precompiled
  work across `build` and checkout `install`. Installation uses no sibling
  staging path; the validated artifact is copied directly into the fixed root.

- Decision: use `/home/bex/projects/sky-cua-validation/standalone-release` as a
  durable shared clone for fresh-checkout proof rather than an ephemeral build
  directory.
  Rationale: its checkout-local `target/`, `out/`, `dist/`, and Bun installs
  remain reusable for convergence and replacement tests. Temporary directories
  are limited to isolated install fixtures and atomic archive-file replacement.

- Decision: ship the latest Chrome extension exactly once.
  Rationale: the three extracted versions remain repository evidence, but only
  `1.2.27203.26575` is a release input. The standalone payload exposes it at
  `browser/extension` and prunes the inherited versioned resource copy.

## Outcomes & Retrospective

The producer portion is implemented and proven locally. The canonical archive is
`/home/bex/projects/sky-cua/dist/sky-cua-linux-x64-glibc.tar.gz`; it installs to
`${XDG_DATA_HOME:-~/.local/share}/sky-cua`, carries semantic version `0.1.0` for
target `linux-x64-glibc`, contains exactly extension `1.2.27203.26575`, and has no
generation/current/rollback installer state or Browser trust hash. A durable
clean clone built the archive, both checkout and extracted-artifact installers
passed under isolated HOME/XDG roots, a second checkout install removed an
old-only marker, and `just verify` converged (1,340 Rust nextest cases, 752
Python tests, and Android unit tests). After Bex explicitly removed staging,
backup, rollback, and power-loss recovery from scope, the final focused direct
install/plugin suite passed 49 tests and the rebuilt archive installed directly
into an isolated fixed root.
Browser Use verification passed 36 tests and
cua-node passed 198 with 7 intentional skips. Consumer implementation and live
installed acceptance remain in the dedicated tasks above; no live install,
deployment, commit, push, or publication was performed from this task.

The final archive contains one `browser/extension/manifest.json` and zero
`resources/chrome-extension` entries. A fresh extracted-artifact install and a
second replacement install passed in isolated HOME/XDG roots; the second install
removed an injected old-only marker. A repeated producer build reused Cargo's
release artifacts (`Finished ... in 0.11s`) and all three Bun installs reported
`no changes`, confirming that build outputs and dependency work are durable.

The OpenClaw consumer portion is locally implemented and reviewed: it resolves
only the fixed XDG marketplace, performs exactly two native `plugin/install`
calls with retry/single-flight behavior before a thread, preserves global
non-Codex `node_repl`, and records `extension_native_host` / `isIab=false` in
`bex-fork.md`. Its focused 28-test suite and `git diff --check` pass. Remote
Testbox convergence and all live/Gateway acceptance remain unrun because the
backend is unavailable and live mutation was not authorized.

## Context and Orientation

The producer worktree is `/home/bex/projects/sky-cua`. The checkout and extracted
artifact now share `install.py` plus `scripts/standalone_release.py`. The prior
package, complete-release, activation, and installer entrypoints have been
deleted, along with the legacy release verifier and the local OpenCode/OpenClaw
release-only helpers that depended on it. The fixed-root path below is the only
distribution and installation contract.

The target installed layout is stable and intentionally boring:

    ~/.local/share/sky-cua/
      RELEASE.json
      bin/
        sky-cua-client
        sky-cua-service
        node
        node_repl
      browser/
        browser-client.mjs
        extension/
        native-host/
      codex/
        openai-bundled/
          .agents/plugins/marketplace.json
          plugins/computer-use/
          plugins/browser-use/
      skills/
        computer-use/
        browser-use/
        phone-use/
      docs/

Exact subordinate filenames may follow existing built component names when
changing them would add work, but the public roots above and the marketplace
manifest path are fixed. Stable launchers under `~/.local/bin` point directly
into this tree. Native messaging manifests point to the stable launcher, not a
generation directory. Runtime state, sockets, logs, and user configuration do
not live inside this replaceable tree.

The Codex Desktop worktree is `/home/bex/projects/codex-desktop`. The central
pin owner is `scripts/sky-cua-release.cjs`; callers include
`scripts/browser-use-cache-sync.cjs`, `scripts/install-linux.ts`,
`scripts/run-electron-forge.ts`, `scripts/build-arch-package.ts`,
`scripts/installed-cua-node-verification.ts`,
`scripts/patch-linux-browser-integrations.ts`, `forge.config.ts`, and their
tests. The task must read installed paths directly, stop bundling a release
under Electron resources, and remove Browser-client trust-hash injection.

The OpenClaw worktree is `/home/bex/projects/openclaw`. The implementation task
must start by reading `bex-fork.md` and must update its native Codex plugin fork
seam. `extensions/codex/src/app-server/managed-native-plugins.ts` is invoked by
`attempt-startup.ts` and `conversation-binding.ts`. It should become a small
pre-thread installer that calls native `plugin/install` twice against the fixed
marketplace manifest. `managed-native-plugins.test.ts` should test those two
calls and fresh-home behavior, not producer schema or cache bytes.

## Plan of Work

### Step 1 — Make sky-cua own every build input and emit one archive

Outcome: `python3 install.py build` works from a fresh checkout and writes one
target-specific archive under `dist/` containing the complete fixed layout.

Work:

- Add `build` and `install` as the only normal subcommands in checkout
  `install.py`; route both through `scripts/standalone_release.py`.
- Make `scripts/standalone_release.py` invoke the existing accepted build
  commands for the Rust core, `packages/browser-use`, the cua-node runtime, the
  browser extension/native host, Codex compatibility marketplace, documentation,
  skills, and launchers before assembly. Missing generated directories are
  outputs, never operator prerequisites.
- Flatten the assembled tree into the layout above. Keep a small semantic
  `RELEASE.json` with schema version, product version, target, and relative
  runtime/plugin/skill paths. Remove content-addressed release IDs, producer
  commit requirements, public manifest/tree hashes, checksum-selected install,
  source inventory gates, and clean-committed-tree requirements from this path.
- Retain deterministic archive metadata only if it remains simpler than removing
  it. Do not expose its digest as an install or consumer contract.
- Delete or fold the obsolete build/package entrypoints once both public commands
  use the canonical implementation. Do not keep wrappers for unshipped internal
  command names.

Validation: from the durable clean clone at
`/home/bex/projects/sky-cua-validation/standalone-release`, keep its reusable
`out/` and `dist/` outputs and run:

    python3 install.py build

Expect exactly one current-target archive under `dist/`. Extract it to a temp
directory and assert the fixed `bin`, `browser`, `codex/openai-bundled`, `skills`,
and `docs` roots exist; both exact plugin identities and their MCP declarations
are present; no `releases/`, `current`, promotion journal, or expected-hash input
exists. Run focused Python tests for the builder plus the existing Browser Use,
plugin-bundle, and release-layout tests rewritten around semantic layout.

### Step 2 — Replace the generation store with one fixed-root install

Outcome: `python3 install.py install` deploys the complete product and local host
integrations to one root, and a second install simply replaces it.

Work:

- Replace `GenerationStore`, activation receipts, promotion journals, rollback,
  `current`, candidate selection, and old-generation process validation with a
  straightforward direct copy into `${XDG_DATA_HOME:-~/.local/share}/sky-cua`.
  Remove the previous tree before copying; retain no old generation, backup, or
  staging tree.
- Make checkout `install` build or refresh the durable staged standalone tree and
  call the same installer carried inside the archive. Make extracted-artifact
  `install` install itself. Do not compile or assemble producer outputs in a
  temporary directory.
- Install stable launchers into `~/.local/bin`, pointing directly at fixed paths.
  Register the Chrome/Chromium `extension_native_host` manifest against the stable
  native-host launcher. Do not add IAB, host-provided IAB, Electron nativePipe,
  codex-desktop, selector environment, socket-selection, or marketplace-selection
  dependencies.
- Project `computer-use`, `browser-use`, and `phone-use` skills from the fixed
  installed `skills/` root into the normal detected Codex/OpenClaw skill roots.
  Their wrappers point to fixed paths and contain no release marker or digest.
- Install the default Codex-home plugins through native exact-path
  `plugin/install` when Codex is available. Configure OpenClaw's global non-Codex
  `node_repl` MCP and model skills when OpenClaw is detected; do not inject the
  standalone `sky_cua` MCP into its Codex harness.
- Remove `--manifest-sha256`, `verify`, `ensure`, `verify-activation`,
  `resolve-active`, `recover`, `rollback`, profiles, generation pruning, and the
  separate per-host setup command surface from the normal installer.
- Remove the Browser client hash allowlist from the shipped Browser/node_repl
  integration and its environment/config projection. The installed browser client
  path is used directly.

Validation: install into isolated temporary HOME/XDG roots and assert one fixed
tree, stable launcher targets, native-host manifest, two native Codex plugins,
three projected skills, and the retained global OpenClaw `node_repl` definition.
Install a fixture artifact containing a changed semantic version and marker file;
assert the new marker exists and an old-only marker is gone. Rerun the same
install and expect success. Then run the repository's narrow Python, Browser Use,
installer, and plugin-package suites followed by the applicable `just verify`
convergence gate.

### Step 3 — Make Codex Desktop a path-only consumer

Outcome: Codex Desktop launches against the fixed sky-cua installation and has no
release pin, checksum, byte verifier, embedded sky-cua payload, or Browser trust
allowlist.

Work, performed only in a dedicated task rooted at
`/home/bex/projects/codex-desktop`:

- Delete the pinned contract and release discovery/copy machinery in
  `scripts/sky-cua-release.cjs`. Replace it only with a small XDG-aware fixed-path
  helper if more than one caller needs the same paths.
- Remove `EXPECTED_RELEASE_ID`, `EXPECTED_MANIFEST_SHA256`,
  `EXPECTED_PRODUCER_COMMIT`, `EXPECTED_BROWSER_SHA256`, pinned components,
  Codex marketplace/plugin tree hashes, `SHA256SUMS` traversal, activation
  verification, candidate discovery, and generation environment exports.
- Remove `browser-use-cache-sync.cjs` behavior that copies or attests a release or
  plugin cache. Codex plugins are installed through native Codex from the fixed
  marketplace; the Browser runtime uses the fixed installed Node, node_repl, and
  browser-client paths.
- Remove trusted-Browser-hash injection from
  `scripts/patch-linux-browser-integrations.ts` and the generated Electron config.
  Preserve only patches genuinely needed to pass the installed runtime paths and
  external-browser behavior.
- Stop `forge.config.ts`, `run-electron-forge.ts`, `build-arch-package.ts`, and
  `install-linux.ts` from embedding, activating, repinning, or verifying sky-cua.
  Codex Desktop installation and packaging assume sky-cua was installed by its
  own installer.
- Delete `installed-cua-node-verification.ts` and release-specific tests/fixtures;
  replace them with focused path-resolution and missing-install error tests.

Validation: from `/home/bex/projects/codex-desktop`, run the focused resolver,
Browser patch, Forge config, Linux install, and package tests, then
`bun test ./scripts` and `bun run build:linux-x64`. Install once and prove the
installed application reads runtime paths under `~/.local/share/sky-cua`, exposes
no trusted Browser SHA list, and completes one Computer Use action and one Browser
Use action. A repository search over active source must find no sky-cua release,
manifest, component, plugin-tree, or Browser-client SHA constant.

### Step 4 — Reduce OpenClaw to two native install calls

Outcome: every isolated OpenClaw Codex home receives both plugins before its first
thread, without validating the producer manifest or installed bytes.

Work, performed only in a dedicated task rooted at
`/home/bex/projects/openclaw` after reading `bex-fork.md`:

- Replace the schema, hash-tree, release-key, collision-inventory, `plugin/read`,
  `plugin/installed`, and MCP-status proof in
  `extensions/codex/src/app-server/managed-native-plugins.ts` with a small
  single-flight function that derives the fixed marketplace manifest path and
  sends `plugin/install` for `computer-use` and `browser-use` before thread start.
- Keep exact compatibility identities in policy/tests, but take plugin versions
  from the marketplace and do not hard-code or compare them in OpenClaw.
- Preserve only the Codex projection rule that filters OpenClaw's global
  `node_repl` MCP from the Codex thread so the Browser Use plugin owns that name.
  Preserve the per-thread disable overlay when callers prohibit MCP use.
- Remove active-release resolution, release rollover hashing, installed cache
  hashing, managed provenance exceptions, trust-prompt hash exemptions, and tests
  for those deleted mechanisms. A replacement sky-cua install is consumed when a
  new app-server client performs the same two native installs.
- Rewrite the existing native sky-cua plugin section of `bex-fork.md` to record
  the simpler fork seam and its exact files. Do not add a new framework or status
  command.

Validation: from `/home/bex/projects/openclaw`, run
`pnpm test extensions/codex/src/app-server/managed-native-plugins.test.ts extensions/codex/src/app-server/attempt-startup.test.ts src/agents/cli-runner/bundle-mcp-codex.user-config.test.ts`,
then `pnpm build` and `git diff --check`. The focused tests must prove a fresh
agent Codex home causes exactly two successful native installs using the fixed
absolute marketplace path, concurrent first-thread starts share the install, and
the global `node_repl` projection is absent from Codex while remaining available
to non-Codex OpenClaw. The task must re-check the sibling Codex files cited above
before implementation, as required by OpenClaw's repository rules.

### Step 5 — Prove replacement without consumer repins

Outcome: one artifact replacement updates all consumers with no changes to either
consumer repository.

Work:

- Build artifact A, install it, and complete the fresh Codex Desktop and OpenClaw
  acceptance flows.
- Build artifact B with an observable semantic version/plugin marker change and
  install it over the same fixed root.
- Restart Codex Desktop and the OpenClaw Gateway/app-server clients as ordinary
  process lifecycle operations. Do not edit either consumer or run a repin.
- Repeat the Computer Use and Browser Use calls. For OpenClaw, require the native
  Codex provider/model and explicit `fallbackUsed: false` or equivalent run
  evidence. Browser evidence must show `extension_native_host`, the external
  Chrome/Chromium tab, and `isIab=false`.

Validation: preserve the two artifact paths, install command outputs, installed
`RELEASE.json` version after each install, native Codex plugin/install responses,
and live run IDs/log excerpts. Acceptance fails if a consumer source file changes,
a hash must be supplied, a permission prompt appears for the managed Browser or
Computer MCP, OpenClaw falls back to another model/backend, or Browser Use claims
IAB.

## Coordination

Target: two focused implementation days—one for the producer simplification and
one for both thin-consumer changes plus installed/live acceptance. Stop and
re-estimate only if the underlying cua-node package cannot run without its
Browser-client hash gate and therefore requires an upstream dependency change.

Execute in dependency order:

1. The sky-cua task owns the artifact layout, fixed-root installer, Browser trust
   removal, launchers, marketplace, and skills. Land and build this first.
2. After the artifact layout is stable, create a dedicated Codex Desktop task
   rooted at `/home/bex/projects/codex-desktop`. It consumes the installed layout
   and does not edit sky-cua.
3. In parallel with Codex Desktop only after step 1, create a dedicated OpenClaw
   task rooted at `/home/bex/projects/openclaw`. It must read and update
   `bex-fork.md`; it does not edit sky-cua or Codex Desktop.
4. Integrate only through the installed artifact. Do not copy uncommitted source
   files between repositories or temporarily repin hashes to bridge the work.

If sky-cua changes the fixed public paths after either consumer starts, pause both
consumer tasks, update this plan's Decision Log, and refresh their contract before
continuing. Commits and pushes remain separate explicit approvals per repository.

## Validation and Acceptance

The work is complete only when all of these are true:

- Build: a clean checkout runs `python3 install.py build` without manual component
  preparation and produces one complete target archive.
- Install: checkout and extracted-archive `python3 install.py install` both deploy
  the same fixed layout, plugins, skills, launchers, native host, and detected host
  integration without a hash argument.
- Shape: `~/.local/share/sky-cua` contains no `current`, `releases`, retained
  generation, activation receipt, or rollback state.
- Desktop: active Codex Desktop source and installed resources contain no sky-cua
  pin or Browser trust hash and do not bundle a private release copy.
- OpenClaw: a fresh agent home receives both exact plugin identities through
  native install before its first thread; OpenClaw does not read or hash installed
  plugin bytes.
- Replacement: artifact B overwrites artifact A; after ordinary process restarts,
  both consumers use B without source changes.
- Live: Computer Use and external-extension Browser Use succeed in both consumers;
  OpenClaw uses the requested Codex model with no fallback, and Browser Use reports
  `extension_native_host` and `isIab=false`.

Do not substitute broad unit-test success for installed/live proof. Conversely,
do not run unrelated sprawling test matrices: use each repository's focused tests,
one convergence build, and the two live flows above.

## Idempotence

`build` may be rerun and incrementally refreshes only its producer-owned outputs
at `dist/plugin/sky-cua`, `out/components/cua-node-linux-x64-glibc`,
`dist/standalone/sky-cua-linux-x64-glibc`, and the current-target archive under
`dist/`. `install` may be rerun and replaces the fixed installation tree and
managed projections. There is no rollback command and no retained generation.

Tests use temporary HOME/XDG roots. Live deployment must stop or restart the
affected sky-cua service, Codex Desktop process, and OpenClaw Gateway through
their normal lifecycle so no process keeps an obsolete executable or plugin
cache open. Do not preserve obsolete files for compatibility.

## Artifacts and Notes

Required closeout artifacts are intentionally small:

- the single `dist/sky-cua-<target>.tar.gz` path and extracted top-level listing;
- the installed `~/.local/share/sky-cua/RELEASE.json` semantic version/target;
- focused test/build command summaries for each repository;
- one fresh-home OpenClaw native plugin-install trace;
- one Codex Desktop live Computer/Browser trace;
- one OpenClaw live Computer/Browser trace with no fallback; and
- the second-install proof showing no consumer repository diff or repin.

Do not add a release status framework, migration ledger, hash report, trust report,
or generated architecture document.

## Interfaces and Dependencies

The only filesystem contract exposed to consumers is the fixed XDG data root and
the relative paths recorded above. The only Codex protocol dependency is native
`plugin/install` with an absolute `marketplacePath` and `pluginName`, confirmed in
sibling Codex `codex-rs/app-server/src/request_processors/plugins.rs` and
`codex-rs/core-plugins/src/manager.rs`.

The exact plugin identities remain:

    computer-use@openai-bundled  -> MCP computer-use
    browser-use@openai-bundled   -> MCP node_repl

Browser Use remains the OpenClaw-specific external Chrome/Chromium extension
transport using `extension_native_host`. It is not IAB and has no
`host_provided_iab`, Electron nativePipe, Playwright-launched browser, or
codex-desktop dependency. OpenClaw's separately configured global `node_repl`
MCP remains available outside the Codex harness.

Checksums that are intrinsic to third-party package managers, lockfiles, extension
IDs, APK signing, SBOMs, or unrelated compliance records are not part of this
plan. The removal target is release content-addressing, consumer pinning,
consumer byte verification, Browser-client trust allowlisting, and every command
or environment variable created to support those mechanisms.
