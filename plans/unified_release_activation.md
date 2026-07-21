# Unify sky-cua release activation across sky-cua and Codex Desktop

This ExecPlan is a living document governed by `/home/bex/.agents/PLANS.md`. Keep `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` current while implementation proceeds.

## Purpose / Big Picture

Installing a new Codex Desktop package must leave one coherent sky-cua release active everywhere without requiring an operator to remember a second generic installer or repair native-host manifests by hand. After this work, sky-cua owns one idempotent activation transaction. Codex Desktop invokes that transaction, projects the active release into its cache and skills, and verifies the complete installed state. Replacing Electron's ASAR still requires an Electron relaunch; every other activation and verification step is automated.

The user-visible proof is a fresh Codex task that asks only to use the Browser Use plugin and can immediately open and control the in-app browser. The machine-level proof is stricter: the active generation, activation receipt, native-messaging manifests, stable command links, Codex package projection, Codex cache and skills, and running sky-cua processes must all agree on one release. A verifier must reject deleted generations, stale manifests, legacy mutable binary copies, and processes running an obsolete release.

## Progress

- [x] (2026-07-21 09:39Z) Captured the clean Codex Desktop baseline at `dbe97caf42b5dbb80eec15838d93800e0af82506` on `main`.
- [x] (2026-07-21 09:39Z) Captured the sky-cua baseline at `31e7cfb338706e753cdf6b9988dd85940672f206` on `main` and recorded the pre-existing modified files that must remain untouched: `docs/features/unified-browser-bridge-control-plane.md`, `docs/operations/browser-control-plane-migration.md`, `scripts/deploy_plugin.py`, and `scripts/test_deploy_plugin.py`.
- [x] (2026-07-21 09:39Z) Confirmed the ownership split: sky-cua owns activation and cleanup; Codex Desktop owns invocation, consumer projections, and end-to-end installed verification.
- [x] (2026-07-21 10:16Z) Defined and tested the schema-1 activation receipt plus idempotent `ensure` and read-only `verify-activation` interfaces.
- [x] (2026-07-21 10:16Z) Made complete release installation the only public activation path and guarded raw generation promotion behind `--internal-generation-only`.
- [x] (2026-07-21 10:16Z) Replaced known mutable compatibility binaries in both user and store bin directories with transactional links through `current`, preserving unknown entries.
- [x] (2026-07-21 10:16Z) Made Codex Desktop invoke producer activation during user-run install and first-start self-healing, then resolve/cache/accept against the exact standalone active generation.
- [x] (2026-07-21 10:16Z) Extended installed verification to reject cross-surface identity/path skew, exact manifest/link/receipt drift, obsolete/deleted runtime processes, and stale persistent Node REPL processes.
- [x] (2026-07-21 10:16Z) Passed sky-cua's 897-test suite, Codex Desktop's 1,015-test suite with 1,005 passes and 10 fixture skips, focused post-review tests, Ruff, basedpyright, TypeScript typecheck, Oxlint, skill validation, and diff checks.
- [ ] Build/package/install the committed artifacts and run the isolated live Browser Use acceptance task.
- [ ] Record installed proof, reconcile the final outcome, and remove this ExecPlan only after all acceptance criteria pass.

## Surprises & Discoveries

- Observation: The failed deployment invoked `scripts/release_generation.py install`, which only verifies, promotes, switches `current`, and prunes generations. It does not install native-messaging manifests or complete the other machine-facing activation work.
  Evidence: `_GenerationTransaction.install` in `scripts/release_generation.py` ends after generation promotion and pruning, while `install_complete_release` in `scripts/install_complete_release.py` separately invokes `install_native_messaging_manifests` and owns rollback.

- Observation: The release-root `install.py install` entry point already routes through the complete installer, so the producer has the correct public ownership seam; the problem is that the lower-level command remained easy to invoke as if it were complete.
  Evidence: The generated release-root `install.py` dispatches `install` to `scripts/install_complete_release.py`.

- Observation: The installed Codex package and its packaged sky-cua projection were current, but the machine was not coherent.
  Evidence: `/opt/chatgpt-desktop` and the Codex cache/skills selected release `edcc4cd4...`; native-host manifests selected `531239...`; legacy mutable binaries selected `82c3ace...`; live sky-cua processes referenced deleted generations.

- Observation: A package post-install hook cannot safely activate per-user state because Arch package hooks execute as root without a trustworthy target user home.
  Evidence: The existing package hook confines itself to system integration. User-state activation belongs in the user-run installer and an idempotent first-start ensure path.

- Observation: No production caller in either repository currently invokes `scripts/release_generation.py install` directly.
  Evidence: Repository-wide call-site mapping found only the public CLI dispatch and its CLI tests. Internal `GenerationStore.install` calls are transaction primitives. The remaining confusing operator paths are legacy installer/package documentation and the local `cua-deploy` workflow.

- Observation: Persistent CUA Node REPL processes do not use a `sky-cua-*` executable name after startup.
  Evidence: `node_repl` re-execs the bundled `bin/node` with the matching `lib/node_repl/cli.js`. Whole-machine verification now recognizes only that paired path inside an immutable standalone or packaged complete release, and tests reject stale packaged/store REPLs while ignoring unrelated Node processes.

- Observation: Codex installation originally activated standalone `current` and then projected `/opt` paths back into config during acceptance.
  Evidence: Cross-repository review found the verifier forcing the packaged release root. The final flow uses `/opt` only as the immutable candidate and package-integrity proof, then resolves, synchronizes, and accepts against `~/.local/share/sky-cua/releases/<release-id>`.

## Decision Log

- Decision: Use one cross-repository ExecPlan, stored in sky-cua because sky-cua defines the producer contract and owns activation.
  Rationale: A single contract prevents each repository from inventing a different meaning of “installed,” while work packages still preserve repository ownership.
  Date/Author: 2026-07-21 / Codex

- Decision: Define activation as an idempotent producer transaction, not as a Codex-specific deployment procedure.
  Rationale: Native manifests, compatibility commands, process handoff, pruning, rollback, and the activation receipt are producer state. Every consumer should get identical behavior by calling the same interface.
  Date/Author: 2026-07-21 / Codex

- Decision: Keep generation promotion as an internal library primitive and reject operator-facing CLI use unless an explicit internal flag is supplied.
  Rationale: The previous failure was caused by a valid-looking lower-level command that silently stopped before activation was complete.
  Date/Author: 2026-07-21 / Codex

- Decision: Persist an activation receipt in the standalone store and make verification derive state from artifacts rather than trusting the receipt alone.
  Rationale: The receipt provides a cheap idempotency key and diagnostics; manifests, links, release metadata, projections, and live processes remain the source of proof.
  Date/Author: 2026-07-21 / Codex

- Decision: Point human-facing compatibility commands through the stable `current` link rather than copying mutable executables into a separate bin store.
  Rationale: Copies create a third release identity that can silently drift. Links preserve stable command names while retaining one active artifact identity.
  Date/Author: 2026-07-21 / Codex

- Decision: Codex Desktop invokes the release-root producer interface both after a user-run install and through a cheap first-start ensure operation.
  Rationale: The installer handles the common case, while first start repairs stale or externally changed state without duplicating producer logic. Electron must still relaunch to load a replaced ASAR.
  Date/Author: 2026-07-21 / Codex

- Decision: Preserve `install.py verify` as immutable release-integrity verification and add a distinct `verify-activation` operation for installed machine state.
  Rationale: Existing packaging and release verification depend on checking an extracted release before it is active. A distinct command makes the mutability boundary explicit without breaking that contract.
  Date/Author: 2026-07-21 / Codex

## Context and Orientation

`/home/bex/projects/sky-cua` builds immutable release generations. `scripts/release_generation.py` implements the generation store and the `current`/`previous` links. `scripts/install_complete_release.py` composes generation promotion with native-host and optional integration installation. `scripts/_native_messaging_install.py` writes browser manifests that point at exact release-host executables. `scripts/install_mcp_server.py` contains the older mutable compatibility-bin behavior. Release-root `install.py` is the public self-contained entry point included in every generated release.

`/home/bex/projects/codex-desktop` packages a verified sky-cua release under Electron resources. `forge.config.ts` installs immutable Browser Use resources while packaging. `scripts/install-linux.ts` installs the Arch package and runs installed CUA verification. `scripts/installed-cua-node-verification.ts` currently proves package projection, cache synchronization, and CUA acceptance, but it does not activate or verify all producer-owned machine state. `scripts/build-arch-package.ts` renders the user-facing launcher, which is the appropriate first-start repair seam.

An “activation receipt” is a small JSON record written atomically under the standalone sky-cua store after every producer-owned activation surface has succeeded. It records the release identity, manifest identity, selected profile/platform, and the producer-owned artifacts written by the transaction. It is not authoritative by itself: `verify` and `ensure` compare it with the filesystem and process state.

## Work Packages

### WP-01: Shared activation contract and transaction boundaries

Owner: primary agent. Repository: sky-cua. Dependencies: none.

Define the public CLI and data contract before parallel implementation. The release-root interface must support `install`, `ensure`, and `verify-activation` with a shared store-root/profile/manifest identity model. Preserve the existing `verify` operation as immutable release-integrity verification for build compatibility. `install` performs the full transaction. `ensure` performs a cheap proof and invokes the same transaction only when repair is required. `verify-activation` is read-only and reports actionable machine-state skew. Define an atomically written activation receipt with an explicit schema version. Define rollback snapshots for the prior `current` target, native manifests, stable links, and prior receipt. Define process handling so obsolete processes are drained before obsolete generations can be pruned.

Acceptance: tests can construct a release fixture, install it, run ensure twice without changing state, verify the receipt and artifact-derived state, inject skew into each producer-owned surface, and observe a precise failure. A failed activation restores the prior coherent state.

### WP-02: sky-cua activation, cleanup, and public entry points

Owner: sky-cua implementation lane. Repository: sky-cua. Dependencies: WP-01.

Implement the contract using the existing complete installer and native-manifest transaction. Move pruning to the end of complete activation, after stale-process draining and all producer-owned state is committed. Replace known mutable compatibility binaries with stable links through `current`; remove only the explicitly known legacy artifacts and keep rollback information until success. Guard the `release_generation.py install` CLI behind an unmistakable internal-only option and print the correct release-root command when it is invoked incorrectly. Do not edit the four pre-existing modified sky-cua files.

Acceptance: focused installer/generation/native-manifest tests pass; a raw operator-style generation install fails with corrective guidance; complete install, ensure, verify, rollback, stable-link repair, stale-process detection, and safe pruning have direct tests.

### WP-03: Codex Desktop invocation and installed-state proof

Owner: Codex Desktop implementation lane. Repository: codex-desktop. Dependencies: WP-01.

Teach the user-run Linux installer to invoke the packaged release-root activation interface before Codex cache synchronization and acceptance checks. Teach the generated launcher to run the cheap ensure path before resolving browser environment variables; the producer remains solely responsible for deciding and applying repairs. Add the producer runtime required by this launcher to package dependencies if it is not already guaranteed. Extend installed verification to compare the packaged release, standalone active release and receipt, native manifests, stable compatibility links, Browser Use cache/skills/trust projections, and live process executables. Keep the immutable packaged fallback and existing trusted-hash generation intact.

Acceptance: installer and launcher rendering tests prove the invocation order and arguments; installed verification fixtures pass for one coherent release and fail for stale receipt, manifest, link, projection, deleted process, and obsolete release process cases.

### WP-04: Cross-repository integration and legacy-path retirement

Owner: primary agent. Repositories: both. Dependencies: WP-02 and WP-03.

Integrate both lanes, resolve contract drift, and remove remaining internal callers of the raw generation CLI. Keep `scripts/deploy_plugin.py` outside this change because it contains pre-existing user work; if its behavior remains relevant, leave a compatibility note or separate follow-up rather than overwriting it. Ensure no generic Codex-side installer reimplements producer mutation. Update untouched durable documentation with the single-command workflow and mark superseded commands as internal.

Acceptance: repository searches find no production/operator path that uses raw generation promotion as complete activation, and documentation names only the release-root activation interface for normal installation.

### WP-05: Packaging, local deployment, and live acceptance

Owner: primary agent. Repositories: both. Dependencies: WP-04.

Build a fresh sky-cua release and Codex Desktop package. Install through the normal Codex Desktop user-run installer, allowing it to activate sky-cua automatically. Run the strict whole-machine verifier. Launch a second isolated Codex Desktop instance when needed to avoid stale Electron state and create a fresh uncontaminated task with the simple instruction to use the Browser Use plugin. Confirm the task discovers `host_provided_iab`, opens the in-app browser, navigates, captures a screenshot, and views it without environment overrides or manually supplied module roots. Do not terminate the Electron process that owns this task. Record the existing app's required relaunch if it is still using the replaced ASAR.

Acceptance: the verifier reports one release identity across all surfaces, no deleted/obsolete sky-cua process remains, no known legacy mutable binary remains, and the isolated live task completes Browser Use control through the native transport.

## Concrete Steps

Run commands from the stated repository directory. Prefer the narrowest tests during implementation, then broaden.

1. In `/home/bex/projects/sky-cua`, add the activation contract and focused tests. Exercise release fixtures using temporary store, home, config, and process roots; never mutate the real home in unit tests.

2. In `/home/bex/projects/sky-cua`, run focused tests for the new activation module and changed complete/generation installers, followed by:

       uv run pytest scripts/test_release_generation.py scripts/test_install_complete_release.py scripts/test_native_messaging_install.py

   Use the repository's established broader test command discovered from `pyproject.toml`, CI, or contributor documentation before declaring the producer complete.

3. In `/home/bex/projects/codex-desktop`, add activation invocation and installed-state fixtures. Run focused Bun tests for each changed script, then:

       bun test ./scripts

4. Build a fresh producer release using the repository's canonical release builder and verify its manifest before consuming it.

5. In `/home/bex/projects/codex-desktop`, synchronize/pin the verified release using the existing upstream metadata flow, patch the package, and run:

       bun run build:linux-x64

   Then install with the normal local installer command from this repository. Do not call raw `release_generation.py install`.

6. Run installed verification and inspect exact release IDs in the active receipt, native manifests, stable links, Codex packaged resources, Browser Use cache/skills, and `/proc` process executable paths.

7. Launch the isolated app instance and perform the fresh Browser Use task. Save concise terminal/log evidence and the resulting screenshot path or task identifier.

## Validation and Acceptance

The change is accepted only if all of the following are true:

- One normal Codex Desktop installation automatically installs or repairs the selected sky-cua release; no generic installer command is required.
- Repeating producer `ensure` is fast and idempotent when state is coherent.
- A failure during activation restores the prior coherent release and its manifests, links, and receipt.
- Raw generation promotion cannot be mistaken for public complete installation.
- Native-host manifests resolve to the active release and never to a pruned generation.
- Stable compatibility commands resolve through `current`; known mutable legacy copies are absent.
- Obsolete processes are stopped before their generation is pruned, and verification rejects deleted or obsolete live executables.
- Codex packaged resources, Browser Use cache, skills, and trusted metadata agree with the active release.
- A freshly launched isolated Codex task discovers and uses `host_provided_iab` through Browser Use without manual environment setup.
- Existing pre-change modifications in sky-cua remain byte-for-byte preserved unless the user separately authorizes integrating them.

## Idempotence and Recovery

All activation writes must use temporary paths followed by atomic replacement. Before the first mutation, capture enough prior state to restore the prior `current` target, native manifests, stable links, and receipt. Keep the new generation unpruned until all producer-owned writes and process handoff succeed. On failure, restore the captured state and leave both generations available for diagnosis. `ensure` must make no writes when artifact-derived verification succeeds. It may repair by running the exact same transaction as `install`; it must not contain a second partial installer.

Unit and integration tests use isolated temporary roots. Local deployment may be rerun safely because the generated release is immutable and activation is idempotent. If the installed Electron app is already running, complete activation and verification first, then relaunch Electron or use an isolated second instance to consume the new ASAR.

## Artifacts and Notes

Baseline release identities observed before implementation:

- Codex packaged/cache/skills and standalone `current`: `edcc4cd4...`
- Native-host manifests: `531239...`
- Legacy mutable compatibility binaries: `82c3ace...`
- Live sky-cua processes: executable paths under already deleted generations

These abbreviated IDs are diagnostic breadcrumbs only. Validation must read and compare full IDs from current artifacts.

## Interfaces and Dependencies

The producer interface lives in the generated release-root `install.py` and must remain self-contained inside each immutable release. Its public operations are:

    install.py install [release selection and integration options]
    install.py ensure [same selection options, repair only when verification fails]
    install.py verify-activation [same selection options, read-only machine-state proof]

The existing `install.py verify` continues to verify the immutable release tree without requiring prior activation. The exact new option spelling may follow existing parser conventions, but all activation operations must use the same internal activation model and emit machine-readable diagnostics suitable for Codex Desktop verification. The activation receipt schema must be versioned and parsed defensively. Codex Desktop may execute this interface and interpret its result; it must not import sky-cua source modules or reproduce their mutation logic.

Python is the producer release installer's runtime dependency. If the Codex Desktop Linux package relies on `install.py` at first start, its package metadata must explicitly depend on the appropriate system Python runtime.

## Outcomes & Retrospective

Implementation is in progress. Record final behavioral proof, remaining limitations, and any follow-up here before deleting this ExecPlan as required by the repository plan lifecycle.
