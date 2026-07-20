# Complete CUA stack ownership in sky-cua

This ExecPlan is governed by `/home/bex/.agents/PLANS.md` and `plans/AGENTS.md`. `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must stay synchronized with the work. The current source, installed artifacts, live process state, and recorded acceptance evidence override stale plan assumptions.

## Purpose and Success Criteria

Make `sky-cua` the sole producer and runtime owner of the complete Linux x86-64 glibc CUA stack. One immutable sky-cua release must install and verify two MCP servers: `sky_cua` for the existing direct low-latency Browser/Computer/Phone surface, and `node_repl` for the persistent Node 24 JavaScript workbench. Codex Desktop becomes a consumer adapter; OpenClaw and OpenCode obtain both servers from the same standalone installation.

Success is observable only when all of the following are true:

- A componentized immutable release contains `RELEASE.json`, `SHA256SUMS`, core Linux x64, canonical first-party Browser JS, `cua-node-linux-x64-glibc`, a generated Codex compatibility projection, provenance/licenses/SBOM, and an optional fat offline archive. Every component hash, size, tree hash, dependency, target, lock, trusted Browser SHA, and compliance artifact is bound into one release identity.
- `~/.local/share/sky-cua/releases/<id>` holds complete verified generations. Installation never mixes generations, and a durable journal makes atomic `current` promotion, recovery, idempotent reinstall, one-generation rollback, and one retained prior generation demonstrably correct. Full install is the default on supported x64 glibc; core-only is explicit.
- `node_repl` preserves the exact `js`, `js_reset`, and `js_add_node_module_dir` MCP contract, persistent Node 24.14.0 VM semantics, supplied Codex `_meta` byte-for-byte, stable synthetic process/session identity for metadata-free generic clients, one turn id per `tools/call`, and explicit provenance (`codex_desktop`, `openclaw`, `opencode`, `direct_mcp`).
- `@heliasar/browser-use` is the canonical first-party TypeScript/Bun-built, Node-24-run Browser implementation. `setupBrowserRuntime({globals})` installs `agent` and `display` and implements the full documented `agent.browsers.*` API, declarations, and command fixtures. Codex projections are generated from those exact bytes; no copied OpenAI Browser implementation or Skynet acceptance path remains.
- `@heliasar/sky-cua` connects directly to the already-running shared daemon, never manages daemon or MCP lifecycle, retains WebP as its screenshot default, and passes ordinary Computer Use operations from `node_repl`.
- Direct `sky_cua` still reaches the Rust daemon without a Node hop. IAB remains host-provided for Codex; Chrome/Chromium/Brave share the one daemon-owned extension bridge while separate callers retain truthful provenance and distinct groups/tabs.
- Clean/neutral-directory OpenClaw and OpenCode installs list and actually invoke both MCP servers after host reload. Ordinary live model tasks in Codex Desktop, OpenClaw, and OpenCode use node_repl Browser, Computer Use JS, image/PDF/OCR/file operations, with transcript evidence proving tool execution rather than only readiness.
- Correct Browser hashes work and incorrect hashes reject before socket connection. Tamper, missing component, mixed generation, interrupted journal, recovery, reinstall, rollback, and packaged-fallback gates all pass against installed artifacts.
- Baseline performance is recorded before cutover. Warm median is at most 110% of baseline and p95 at most 125%; WebP remains the default and direct `sky_cua` has no Node hop.
- After core producer/consumer cutover and initial installed acceptance, sky-cua ships one canonical model-facing documentation component usable without checkout paths by Codex Desktop, OpenClaw, and OpenCode. Compact Browser, Computer Use, and Phone Use routing skills progressively disclose shared installed references/recipes, generated API inventories, capability/version inventories, runnable examples, troubleshooting, and explicit unsupported behavior. Codex compatibility projections carry routing references but no separate docs implementation.
- Phone Use has a real first-party persistent JavaScript facade through `node_repl` over the normal daemon/service path, not merely direct Phone MCP tools. Its installed contract covers lifecycle, metadata/provenance, capability discovery, screenshots/image emission, local-file handling, structured errors, disconnect behavior, and direct-MCP-versus-REPL routing.
- Every installed Node 24 example passes against the immutable release. Model acceptance proves each compact top-level skill discovers and uses the right recipe; full Phone JS acceptance and direct-versus-REPL routing tests pass. Package presence and dependency README files are not acceptance.
- Frozen-scope exhaustive sky-cua review/fix/retest, separate Codex consumer review, and a cross-repository duplicate-ownership/mixed-generation review find no unresolved blocker. Documentation and `ROADMAP.md` describe the shipped state. Semantic commits are created only after validation; no push or PR occurs without fresh explicit authorization.
- The ExecPlan is retired only after all required live gates pass. Until then it remains present and honest about blockers.

The v1 target is Linux x86-64 glibc. macOS receives a truthful placeholder only. Arm64, musl, Windows `node_repl`, and `@heliasar/sky-cua/advanced` are explicit follow-ups. There is no npm publication, Node proxy for direct `sky_cua`, codex sandbox, approval system, auth token, or new authorization layer. Same-user owner-only sockets are the trust boundary. No mutation is retried when success is ambiguous.

## Progress

- [x] (2026-07-20 03:12Z) WP-00: Read all repository `AGENTS.md` files and `/home/bex/.agents/PLANS.md`, created the full end-to-end product goal, and captured both repository baselines.
- [x] (2026-07-20 03:12Z) WP-01A: Created this authoritative ExecPlan with dependency-first packages, locked architecture, acceptance, recovery, and approval boundaries.
- [x] (2026-07-20 04:46Z) WP-01B: Implemented the versioned release schema, content-derived release id, deterministic tree/archive hashes, full/core dependency selection, exact manifest/checksum/artifact verification, required component set, canonical Browser/projection equivalence, owner-only generation store, interprocess transaction lock, durable journal, atomic current/previous links, crash recovery, idempotent reinstall, one-prior retention, rollback, and verify/install/recover/rollback CLI. The schema reserves the post-cutover documentation component and per-inventory/example hashes. Focused release/package, transaction, assembler, and complete-release builder tests pass; final immutable candidate creation remains WP-05.
- [x] (2026-07-20 07:39Z) WP-02: Migrated the complete first-party `cua_node` source/runtime contract and locked assets from the preserved Codex Desktop input into `runtime/cua-node/**`, committed as `530deb5`, and closed its installed benchmark lock seam in `613c2d4`. Final clean component `01ebb6ba6bff636d6361c532d1d25e85b0374b56eaa536b765dc9591dbe479cc` is release-eligible, independently checked, and contains 3,798 files / 324,854,602 bytes. It passes exact installed MCP transcripts, 12-cell REPL media, full offline Canvas/Sharp/PDF.js/Tesseract/pixelmatch/system-Chrome Playwright, two-cycle web lifecycle, packaged Browser trust negatives, and the 100-cell benchmark.
- [x] (2026-07-20 07:27Z) WP-03: Implemented canonical `@heliasar/browser-use` with all 72 documented commands supported: 70 decompose onto existing raw/CDP/notification primitives and the final two use explicit daemon-local raw seams. The package preserves exact generated declarations/API fixtures, deterministic byte-identical projections, real Chrome DOM locator execution, WebP annotation, local content/assets, notification-driven events/dialogs/logs/file chooser/downloads, and wrong-hash rejection before connect. Frozen-scope review drove regression-backed fixes for per-tab marks, capability reachability, pre-armed main-frame navigation waits, caller-owned downloads over one browser-wide same-user destination, load-state semantics, modifier chords, CDP clip scale, and asset collisions. The one focused follow-up closed six findings and identified the final iframe-navigation and per-tab-download-root races; both are fixed deterministically with main-frame identity matching and one stable browser-wide path. `bun run verify` passes 24/24 with zero unsupported commands; canonical `browser-client.mjs` SHA-256 is `085ba347a047473272cafc9f024b59c35dca4b29e44dab8b22eaa80e81e7c60d`. The initial complete Browser implementation is committed as `742697f`, review fixes as `d2b904e`, aggregate capability acceptance as `0c47940`, and final focused corrections as `07aacf1`. Installed live Browser acceptance remains WP-09.
- [x] (2026-07-20 06:49Z) WP-04: Implemented connection-locked structured provenance/client surface/clientInfo, caller-separated Browser principals, truthful Codex host-IAB versus extension-native identity, no ambiguous mutation replay, and old/new protocol-v1 read/write compatibility. The two final Browser raw methods remain inside the existing scheduler: caller-owned tab enforcement, terminal mutation settlement, provenance-correlated bot reports, and credential-free live-origin/selector auth preflight with truthful Linux `unavailable`. Full `cargo fmt --check`, workspace clippy with warnings denied, and `cargo nextest run` pass 1,331/1,331; the 15-case browser-control acceptance also passes. The provenance slice is committed as `c06a76b` and the final host seams as part of `742697f`. Installed concurrent-host acceptance remains WP-09.
- [x] (2026-07-20 07:39Z) WP-05: Built and verified immutable complete release `2529ee922462c73d8ac26d1776bb067c82a2aecbfdc59e62815762774e74fc86` from producer commit `42dd29e5834c88f04cf0df2482439089950a8e7f`. Manifest SHA-256 is `1ab27d30b4503d7e8c3066591ddde26aac9523b7c4601498c55b1c94b264ed15`; full-profile verification binds five components, exact Browser/projection bytes, core Git-archive provenance, cua_node provenance/locks/licenses/SBOM, capabilities, and the optional fat archive. The verified release root is `/home/bex/projects/sky-cua/dist/complete-release/2529ee922462c73d8ac26d1776bb067c82a2aecbfdc59e62815762774e74fc86`.
- [ ] (2026-07-20 06:49Z, partial) WP-06: Implemented verified-generation OpenClaw and OpenCode two-server adapters, committed as `16b566a`. OpenClaw transactionally snapshots/sets/verifies/restores only `sky_cua` and `node_repl`, preserves unrelated servers, reports honest Gateway watcher/restart state, and never treats process-local `mcp reload` as Gateway proof. OpenCode performs targeted JSONC-preserving edits, detects higher-precedence config hazards, creates durable content-addressed backups, supports stale-safe rollback, and requires a full process restart. Both pin one exact generation, trust set, bundled Node/module/Playwright paths, browser socket, and host provenance; focused host/controller tests pass 26/26 and the full Python suite passes 847/847. Standalone promotion, real host reload/restart, neutral-directory listing/invocation, and successful model tool execution remain live gates.
- [ ] (2026-07-20 08:06Z, partial) WP-07: Ported exact runtime/package/native/data-asset locks, notices, provenance, CycloneDX/SPDX, and recorded the hash-bound current Codex migration-input baseline (`65c69a3`, runtime SHA-256 `eed2bb02daf9ed79add6e96b2b759897e04bde4b393d3049b7fdde4cb93e6b95`). The final clean component passes exact MCP transcripts, persistence/reset/timeout/cancel/meta, native/local-file/buffer/URL/image operations, OCR/PDF/Sharp/Canvas/pixelmatch, WebP/PNG, standalone Playwright, web lifecycle, and all three packaged Browser trust negatives before connection. Its 100-cell benchmark passes: warm median 0.292412 ms (limit 0.52381725), p95 0.57602 ms (limit 1.0988625), and idle RSS 85,905,408 bytes (limit 93,974,528). `@heliasar/sky-cua` typecheck/build and 47 tests pass. Direct topology is now proven from the immutable release: `components/core-linux-x64/bin/sky-cua-client mcp` initialized and listed 36 tools, resolves to the native x86-64 client linked only against glibc/libm/libgcc, while `node_repl` is a separate static ELF in the cua-node component. Installed IAB/Brave Browser action setup correctly remains unavailable against the pre-cutover daemon and is a WP-09 post-promotion gate; remaining WP-07 work is the live Computer action matrix.
- [ ] (2026-07-20 07:40Z, active) WP-08: Created the single gated Codex Desktop consumer task `019f7e76-256d-7973-bd87-4561f0b5b173` from its preserved dirty working-tree state, limited to resolver precedence, environment hydration, packaged fallback, compatibility materialization, installed acceptance, and producer-code removal. It received the exact release id, manifest hash, root, Browser hash, producer commit, and cua_node tree hash; monitor until it returns installed proof and semantic commits.
- [ ] WP-09: Promote one standalone generation atomically, reload hosts, install/restart Codex, run the full installed/live/concurrent acceptance matrix, and prove rollback on failure.
- [ ] WP-10: After WP-09 initial live acceptance, produce the cohesive model-facing documentation/Phone-JS design deliverable, implement the persistent Phone facade and canonical installed documentation component, and generate hashed API/example/capability inventories.
- [ ] WP-11: Run installed documentation/example/model-discoverability, direct-vs-REPL, composed-workflow, Phone-JS parity, and documentation performance acceptance.
- [ ] WP-12: Run sky-cua exhaustive review/fix/retest, Codex consumer review, cross-repo ownership review, documentation/roadmap closeout, semantic commits, stale-producer deletion, and ExecPlan retirement.

## Surprises & Discoveries

- Observation: sky-cua already contains two large local commits implementing the daemon-owned browser bridge control plane.
  Evidence: `main` is clean at `dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a`, two commits ahead of `origin/main`; `78ac69c` and `dcd4f30` add the raw Codex compatibility listener, scheduler, per-caller groups, persistence, installer integration, and acceptance harnesses.
  Impact: WP-04 extends and verifies an existing control plane rather than creating a second bridge.

- Observation: the Codex Desktop migration input is intentionally substantial and dirty after the already-landed first-party base commit.
  Evidence: `/home/bex/projects/codex-desktop` is `main` at `65c69a3f1afc9f81274189901bc72e80682ea03a`, four commits ahead of `origin/main`, with 22 modified tracked files and 22 untracked files. The dirty set centers on `runtime/cua-node/**`, runtime locks/capabilities, production acceptance/benchmarks, deployment/release gates, and Linux installation.
  Impact: WP-02 must copy by explicit source inventory and hashes, never clean/reset/rewrite the Codex tree, and must distinguish the committed base (`6de13114`) from later dirty enhancements.

- Observation: sky-cua already owns `packages/sky-cua-js`, direct host installers, release-package scripts/docs, and OpenClaw/OpenCode wiring.
  Evidence: `packages/sky-cua-js/**`, `scripts/package.py`, `scripts/installer.py`, `scripts/install_mcp_server.py`, `scripts/_openclaw_install.py`, and `docs/features/release-package.md` exist.
  Impact: the migration evolves accepted seams and avoids a parallel installer or JS facade.

- Observation: the raw browser ingress is a generic low-level transport over `SKY_CUA_CODEX_BROWSER_SOCKET_PATH`; the canonical Browser implementation decomposes its complete API locally onto `getInfo`, tab/CDP, and unhandled-capability methods instead of adding a Browser-specific Node proxy or Rust `executeAgentCommand` surface.
  Evidence: WP-03's 72-command fixtures and runtime tests exercise local decomposition; WP-04 keeps the raw listener generic, connection-locks provenance, and accepts identical caller context projected at the top level, `_meta`, and `request_meta` for compatibility.
  Impact: `@heliasar/browser-use` owns Browser semantics, must not label OpenClaw/OpenCode traffic as Codex ingress, and projects metadata identically across the three accepted shapes. Browser-byte trust remains a pre-connect Node/browser-runtime check because the Rust raw listener never sees JS module bytes.

- Observation: a source-clean commit alone does not prove ignored generated runtime bytes were built from that commit.
  Evidence: after the runtime schema gained `source.migration_input`, a stale ignored `runtime/cua-node/dist/cli.js` still embedded the old schema; static component verification passed but installed persistent REPL acceptance rejected the manifest. Rebuilding changed the host bundle hash and made both REPL/full subsystem acceptance pass.
  Impact: release eligibility now requires an immediate deterministic rebuild of the host bundle, Browser build, and `@heliasar/sky-cua` tarball, with commands, toolchain, and output hashes stored in the component attestation and rechecked by the outer producer.

- Observation: package-manager output may omit a license file even when `package.json` declares an SPDX license, and non-package data assets need explicit SBOM representation.
  Evidence: the exhaustive inventory found 194 package roots, including third-party packages represented only by declared-license records; PDF.js cmaps/fonts and the Node bundled license were lock-bound but underrepresented in the initial SBOM.
  Impact: exact publisher/SPDX texts and explicit Node/PDF.js data-asset hashes and license associations are now generated; WP-02/WP-07 remain open only until a clean commit-bound component rebuild repeats compliance and real-artifact verification.

- Observation: an internally consistent outer release was not sufficient proof when its inner component could self-assert eligibility or be reread across an atomic producer promotion.
  Evidence: the frozen producer review demonstrated five blockers: false PDF asset license expressions, uncorrelated canonical/embedded Browser bytes, insufficient inner attestation validation, live-source mixed-generation assembly, and temporary-workspace/output collision. Focused negative tests now cover each reproduced failure, and the producer snapshots under the assembler lock before validating the inner verifier and commit-bound source inventory.
  Impact: WP-05 may proceed only when the independent re-review closes all five findings and a post-commit release-eligible component passes the same checks.

- Observation: fixture-level command enumeration did not prove Browser semantic parity.
  Evidence: frozen-scope review found the first `@heliasar/browser-use` implementation routed unsupported operations through an invented raw command, discarded finalize status, ignored capability overrides, and implemented locator/wait semantics too narrowly even though its inventory named all 72 documented commands.
  Impact: WP-03 is reopened until every documented command has a concrete Node/CDP or daemon implementation and full declaration/fixture coverage; unsupported capability negotiation may describe a genuinely unavailable host capability but cannot be used as a universal parity escape hatch.

- Observation: release trust must start before any inspection of imported native bytes and must bind the core staging tree to the same clean producer commit.
  Evidence: structured autoreview reproduced self-authenticating migration input, nested symlink dereference, unbound caller-selected core input, native inspection before seed authentication, and dangling-current retention. The producer now pins the exact migration seed, rejects nested links/special entries, rebuilds core from the clean current producer commit, and validates installed generations before current/previous transitions.
  Impact: final WP-05 review and candidate production must run after non-release scopes are committed so the remaining frozen scope exactly matches the producer and transaction paths.

- Observation: the existing production acceptance suite is strong for subsystem bytes but does not yet orchestrate the complete installed three-host contract.
  Evidence: the audit mapped real REPL/file/media/Browser/VM tests, but found no installed full golden-transcript replay, no concurrent ordinary-model Codex/OpenClaw/OpenCode runner for both MCP servers, no active-generation `/proc` proof, no disposable real-candidate crash/recovery drill, no self-contained Browser fixture, and no complete live `@heliasar/sky-cua` action runner. The public Linux facade also lacked a window action required by the locked matrix.
  Impact: these are implementation gates, not manual closeout notes. Independent runtime-transcript and Computer-JS lanes are now active; the controller retains the three-host, generation-proof, failure-injection, Browser fixture, and cutover orchestration seams.

- Observation: a clean Git status did not make ignored producer inputs commit-derived, and a content-addressed cache marker did not make the marker an independent trust root.
  Evidence: focused release re-review forged a self-consistent cache pointer/marker and demonstrated that `build_plugin.py` could reuse ignored runtime/APK bytes or dereference a nested ignored symlink before the outer component scan.
  Impact: cache validation now compares every identity dimension to the compiled migration seed before composition, while release-core staging consumes validated `git archive` regular-file bytes in an isolated output and explicitly records excluded/external inputs.

## Decision Log

- Decision: Keep this file as the only repository forward plan; the product `/goal` tracks execution state but no `goals/<name>/` directory is created.
  Rationale: `plans/AGENTS.md` explicitly forbids the Plannotator goal-package layout, while the user separately requested an end-to-end `/goal` and this ExecPlan.
  Date/Author: 2026-07-20, Codex.

- Decision: The primary controller owns `RELEASE.json`, component manifests, shared lockfiles, installation transaction/promotion, host cutover, Codex task creation/monitoring, and final integration.
  Rationale: these are contested shared seams; concurrent writers would create mixed contracts and non-reproducible release identities.
  Date/Author: 2026-07-20, Codex.

- Decision: Treat Codex Desktop `HEAD` plus its dirty working tree as read-only migration input until the gated consumer task is created.
  Rationale: it contains complete first-party runtime work and unrelated active changes. Resetting, cleaning, or broad rewriting would destroy provenance and violate scope.
  Date/Author: 2026-07-20, Codex.

- Decision: Generate compatibility projections from canonical sky-cua Browser bytes and bind their exact hashes in the producer manifest.
  Rationale: projections must be consumers of one implementation, not independent copied code or a second trust authority.
  Date/Author: 2026-07-20, Codex.

- Decision: Reject a wrong trusted Browser hash before any socket connection, and never retry mutation after ambiguous transport failure.
  Rationale: this makes the trust and at-most-once mutation boundaries observable and testable.
  Date/Author: 2026-07-20, Codex.

- Decision: Reserve documentation ownership in the producer manifest now, but defer substantive model-facing docs and Phone JS implementation until after core cutover and initial installed acceptance.
  Rationale: the docs and facade must describe stable installed interfaces and execute against immutable artifacts; piecemeal Wave 1 prose would drift and duplicate contracts.
  Date/Author: 2026-07-20, Codex.

- Decision: Browser, Computer Use, and Phone Use stay compact routing skills; shared node_repl toolbox material is split into installed task-oriented recipes and generated references.
  Rationale: upstream behavior confirms persistent node_repl bootstrapping and progressive disclosure are useful, but broad toolbox coverage is first-party work and proprietary prose must not be copied.
  Date/Author: 2026-07-20, Codex.

- Decision: Development assemblies are useful for parity testing but are cryptographically ineligible for an immutable release.
  Rationale: a producer commit cannot bind uncommitted source. Development builds carry `release_eligible=false`; the outer release builder rejects them, and only a post-validation semantic commit followed by a clean deterministic rebuild may produce the WP-05 candidate.
  Date/Author: 2026-07-20, Codex.

## Outcomes & Retrospective

Wave 1 has converged into ownership-aligned semantic commits for provenance (`c06a76b`), canonical Browser plus final host seams (`742697f`), Computer JS window actions (`83dd279`), first-party node_repl runtime (`530deb5`), immutable release production (`103c1b1`), two-server host installation (`16b566a`), Browser review fixes (`d2b904e`, `0c47940`, `07aacf1`), and producer input trust (`2681ff3`). Source and aggregate gates are green, but no production outcome is claimed until the same clean commit produces an immutable component and release that pass installed-artifact verification. Update this section again at candidate-release acceptance, Codex consumer handoff, installed cutover, and final retirement.

Candidate integration is accepted at release `2529ee922462c73d8ac26d1776bb067c82a2aecbfdc59e62815762774e74fc86` / manifest `1ab27d30b4503d7e8c3066591ddde26aac9523b7c4601498c55b1c94b264ed15`. Installed node_repl/toolbox and benchmark gates pass, and the single Codex consumer task is active. This is not yet a production outcome: standalone promotion, host reloads, live Browser/Computer/model tasks, rollback, post-cutover Phone/docs, and final ownership review remain.

## Context and Constraints

The working directory for sky-cua commands is `/home/bex/projects/sky-cua`. The Codex migration-input repository is `/home/bex/projects/codex-desktop`.

Starting repository state:

- sky-cua: branch `main`, HEAD `dcd4f30f3d0246e9ff1e9450bdb25b3656a5510a`, clean working tree, two commits ahead of `origin/main` at `fd763fb`. No push is authorized.
- Codex Desktop: branch `main`, HEAD `65c69a3f1afc9f81274189901bc72e80682ea03a`, four commits ahead of `origin/main` at `31535b45`; 44 dirty entries (22 modified, 22 untracked). The committed first-party runtime base is `6de1311431393ca661d90f3c66c2d84b853fc1fb`. No reset, clean, checkout-overwrite, push, or PR is authorized.
- Current sky-cua ownership seams include Rust daemon/client/platform contracts under `crates/`, the shared browser control plane under `crates/sky-cua-service/src/browser/control_plane/`, the Codex compatibility ingress under `crates/sky-cua-service/src/codex_browser_compat/`, the JS Computer Use facade under `packages/sky-cua-js/`, release/install scripts under `scripts/`, and host skills under `skills/`.
- Current Codex producer seams include `runtime/cua-node/**`, `resources/node_repl`, `scripts/browser-use-cache-sync.cjs`, Linux packaging/install scripts, and the dirty runtime capability/release/deploy/acceptance work. WP-02 must inventory exact byte sources before migration.

Locked runtime dependencies are Node 24.14.0; Playwright 1.57.0 against system Chrome-family browsers; PDF.js 5.4.624 with fonts/cmaps; Tesseract.js 7.0.0 with language data; Sharp 0.34.5; `@img/sharp-linux-x64` 0.34.5; `@img/sharp-libvips-linux-x64` 1.2.4; `@napi-rs/canvas-linux-x64-gnu` 0.1.91; pixelmatch and the current codecs; no `fsevents`. Dependency versions, lock hashes, license data, and native binary targets must be verified from the preserved Codex artifacts and authoritative upstream metadata before the release manifest is frozen.

The release root contract is:

    ~/.local/share/sky-cua/
      releases/<release-id>/
      current -> releases/<release-id>
      previous -> releases/<prior-id>        # implementation may journal this instead of a public link
      install-journal.json

Every complete generation is immutable after verification. A candidate is staged in a sibling temporary directory, fsynced where durability matters, verified as a whole, renamed into `releases/<id>`, and only then atomically promoted. Recovery determines intent from the journal plus complete-generation verification; it never assembles a generation from individually valid components belonging to different releases.

The two MCP processes share the already-running sky-cua daemon and browser scheduler. `sky_cua` remains the Rust direct path. `node_repl` is a direct bundled Node host spawning a private kernel child. The kernel has no codex sandbox, approval, auth-token, or new authorization layer. Owner-only same-user sockets and verified release bytes form the trust boundary.

## Interfaces and Release Artifacts

`RELEASE.json` is the producer source of truth. Its schema must bind at least:

- schema and compatibility versions, release id, producer commit, creation metadata, target triple, architecture, OS, and libc;
- component names, relative paths, dependencies, SHA-256, byte size, canonical tree hash, executable entries, and required/optional status;
- Node, node_repl, package, Playwright, PDF.js, Tesseract.js, Sharp, libvips, Canvas, pixelmatch, and codec versions;
- exact trusted canonical Browser client SHA values and every generated projection's equivalence/hash relationship;
- source/runtime/dependency lock hashes plus SBOM, provenance, and license manifest hashes;
- structured supported and unsupported capabilities, including explicit v1 target limitations;
- resolver compatibility required by Codex Desktop and generic hosts.

The immutable component set is:

- `core-linux-x64`: Rust binaries, extension bridge/native host, direct MCP wrapper, resources and host-portable skills.
- `browser-js`: canonical `@heliasar/browser-use`, declarations, command fixtures, and build metadata.
- `cua-node-linux-x64-glibc`: Node 24.14.0, node_repl host/kernel, `@heliasar/sky-cua`, locked JS/native dependencies and data assets.
- `codex-compat`: generated `browser-use@openai-bundled` and `chrome@openai-bundled` projections plus resolver metadata, backed by exact canonical Browser bytes.
- `compliance`: provenance, license inventory/texts, CycloneDX or SPDX SBOM, dependency locks, and their hashes.
- `documentation`: compact routing skills, shared node_repl toolbox references/recipes, generated Browser/Computer/Phone/package API inventories, capability/version inventory, runnable examples, troubleshooting, and unsupported/follow-up inventory. Its tree/archive hashes and the hashes of generated inventories/examples are bound in `RELEASE.json` and provenance.
- Optional fat offline archive containing the exact component archives and root metadata without changing their hashes.

The Codex resolver preserves this precedence and environment contract:

1. verified `SKY_CUA_RELEASE_ROOT`;
2. verified standalone `~/.local/share/sky-cua/current`;
3. verified packaged fallback.

It preserves `CODEX_NODE_REPL_PATH`, `NODE_REPL_NODE_PATH`, `NODE_REPL_NODE_MODULE_DIRS`, `NODE_REPL_TRUSTED_BROWSER_CLIENT_SHA256S`, `SKY_CUA_CODEX_BROWSER_SOCKET_PATH`, and `SKY_CUA_MCP_CALLER_PROVENANCE`. An override that is present but invalid fails honestly rather than silently choosing unrelated bytes unless the manifest explicitly defines safe fallback behavior and tests pin it.

## Execution Strategy

One executor can run WP-01B through WP-10 in dependency order. Optional agents accelerate only disjoint Wave 1 scopes. The primary controller owns shared manifests, root package/config files, lockfile integration, scripts that assemble/promote releases, deployment/cutover, the Codex consumer handoff, final validation, and plan updates.

Wave 1 begins after this baseline:

- WP-01B (controller): release schema and transaction. This defines fixed manifest/projection inputs consumed by later packages.
- WP-02 (bounded worker): cua_node migration into new sky-cua-owned directories, excluding root/shared manifests and final lock integration.
- WP-03 (bounded worker): canonical Browser package and full conformance fixtures in a disjoint package/test scope.
- WP-04 (bounded worker): daemon raw-ingress provenance and identity in Rust contract/service scopes, coordinated against already-local browser-control commits.

The controller accepts each handoff only after its focused tests pass and the shared interface matches WP-01B. Contract changes invalidate dependent handoffs until refreshed.

Wave 2 starts after Wave 1 integration:

- WP-05 builds and verifies a complete candidate release.
- WP-06 adds OpenClaw/OpenCode two-server installers and reload/invocation enforcement.
- WP-07 completes parity, compliance, and benchmarks. Independent pure-test lanes may run concurrently, but host reloads, browser sessions, daemon sockets, and installed release roots are serialized.

Wave 3 is the gated Codex consumer task. It is created only after `RELEASE.json`, hashes, projections, and a candidate release root pass WP-05. The new task receives the exact release id/hash/root and the preserved current Codex working-tree state. It is the sole Codex implementation controller and may not alter sky-cua producer contracts.

Wave 4 serializes real installation and live acceptance. Promote standalone, reload generic hosts, install/restart Codex, prove active process generation, run ordinary and concurrent model tasks, then exercise rollback. Any failure with installed-state risk triggers journal-based rollback before further cleanup.

After the initial Wave 4 cutover passes, the final product phase creates one cohesive design and then implements persistent Phone JS plus canonical installed model documentation. Only after its installed examples, routing, discoverability, parity, and performance gates pass does final closeout run reviews on frozen scopes, fix verified findings, rerun impacted and broad gates, create semantic commits in each repository, update durable repository docs, prove duplicate producer paths are gone, and retire this plan. Push and PR remain separate approval gates.

## Work Packages

### WP-01B — Release schema and transactional generation manager

Outcome: a tested producer contract can build, verify, stage, promote, recover, reinstall, and roll back complete component generations.

Depends on: WP-00 and WP-01A.

Owned scope: controller-owned root/shared manifests; `scripts/package.py`, `scripts/installer.py`, focused new release modules/tests, generated schema fixtures, release documentation. Workers must not edit these shared files without coordination.

Work: inventory current package/install formats; define versioned `RELEASE.json`; implement deterministic file/tree hashing and archive metadata; define component dependencies; stage and fsync a candidate; verify before rename/promotion; journal every durable phase; retain one prior complete generation; implement core-only/full modes, idempotent reinstall, rollback, and crash recovery; bind compliance hashes and capability declarations; create tamper/missing/mixed-generation negative fixtures.

Validation: run focused Python tests through `uv run pytest scripts/test_<release modules>.py`; build twice from identical inputs and compare canonical manifests/tree hashes; simulate interruption at each journal phase; prove wrong/missing/mixed components never become `current`; run Ruff and basedpyright for touched scripts.

Handoff and stopping condition: publish the accepted schema/version, component directory contract, builder/verifier APIs, and focused passing evidence in this plan; stop when downstream packages can target a stable contract.

### WP-02 — Migrate and first-party cua_node in sky-cua

Outcome: sky-cua owns the complete Node 24 host/private-kernel runtime, exact MCP semantics, dependencies/data assets, source/build locks, production gates, and `@heliasar/sky-cua` integration.

Depends on: WP-00; consume WP-01B contract before final packaging.

Owned scope: a new sky-cua-owned `runtime/cua-node/**` (or evidence-backed equivalent), its focused tests and production acceptance, and package-local build configuration. Read `/home/bex/projects/codex-desktop/runtime/cua-node/**` and related dirty assets; never edit or clean the source tree. Root release manifests and shared installer files remain controller-owned.

Work: inventory committed plus dirty Codex inputs with hashes; copy source rather than old generated OpenAI Browser JS; preserve host/kernel lifecycle and termination bounds; port runtime asset discovery, module loader, protocol, capabilities, workbench/media fixtures, release/deploy gates, benchmark baseline, locks and compliance inputs; implement stable synthetic session identity per initialized metadata-free process, turn id per call, `initialize.clientInfo` capture, and identity-synthetic marker; preserve supplied Codex metadata exactly; enforce provenance values; connect `@heliasar/sky-cua` directly to the shared daemon and forbid lifecycle management.

Validation: exact JSON-RPC transcripts for initialize/list/call/cancel; persistent bindings/top-level await/reset/timeout/cancel/output/meta; module/local-file/native-addon/data-URL/buffer/path cases; real OCR/PDF/Sharp/Canvas/pixelmatch/WebP/PNG/JPEG/Playwright reads/transforms/emitted images/written outputs; lifecycle/termination regressions; wrong Browser hash rejection before connect; package verification under bundled Node 24.14.0.

Handoff and stopping condition: report source inventory and hashes, sky-cua paths, build commands, focused passing tests, unresolved external gates, and no Codex writes; stop when the runtime is ready for controller packaging.

### WP-03 — Canonical first-party Browser JavaScript

Outcome: `@heliasar/browser-use` is a TypeScript/Bun-built, Node-24-run canonical Browser runtime with complete documented API parity and deterministic projections.

Depends on: WP-00; consume WP-01B projection fields before final output.

Owned scope: new `packages/browser-use/**` (or evidence-backed package name), package-local fixtures/generator/tests/declarations. Do not edit shared release manifests, root locks, or Codex sources.

Work: derive the complete documented `agent.browsers.*` API/command surface from current Codex declarations, fixtures, and sky-cua browser contracts; implement `setupBrowserRuntime({globals})` installing `agent` and `display`; use the daemon/browser scheduler for both host-provided IAB and extension-backed browsers; preserve caller tab/group ownership and real provenance; generate declarations and exhaustive command fixtures; produce canonical bytes and a projection recipe that copies/links those exact bytes into Codex compatibility layouts without an independent implementation.

Validation: full generated API/declaration/command fixture comparison, not a narrow smoke; Node-24 runtime tests; IAB fixture behavior; Brave/Chrome bridge fixture behavior; correct/wrong trust hash pre-connect tests; canonical/projection byte equality and stable hashes; no references to Skynet acceptance or copied OpenAI implementation.

Handoff and stopping condition: report API inventory, canonical build path/hash, generated declaration/fixture coverage, focused passing evidence, and projection inputs; stop when controller can package exact bytes.

### WP-04 — Truthful ingress identity and shared bridge isolation

Outcome: raw Codex ingress and normalized direct/OpenClaw/OpenCode ingress enter one scheduler with correct provenance, IAB/extension identity, stable caller isolation, and no ambiguous mutation retry.

Depends on: WP-00; integrate against the local `78ac69c`/`dcd4f30` control-plane base.

Owned scope: focused Rust contracts/tests in `crates/sky-cua-platform`, raw ingress and scheduling in `crates/sky-cua-service`, and necessary client provenance plumbing. Shared packaging/install files remain controller-owned.

Work: audit the existing bridge control plane before editing; carry `SKY_CUA_MCP_CALLER_PROVENANCE` and initialize client identity into structured daemon events/status; synthesize identity only at the node_repl boundary; distinguish host-provided IAB from extension actors truthfully; ensure each caller/session owns separate groups/tabs through one bridge actor; reject invalid Browser trust before socket connect; classify reads versus mutations and prohibit mutation replay after ambiguous completion; keep direct sky_cua Node-free.

Validation: `cargo nextest run` focused to changed crates/tests; exact structured provenance assertions; IAB ordinary-Codex and Brave-Origin fixtures with `CODEX_BROWSER_PROVIDER` unset; concurrent Codex/direct/OpenClaw/OpenCode group/tab isolation fixture; ambiguous mutation transport failure proves no retry; malformed ingress leaves daemon serving.

Handoff and stopping condition: report contract changes, compatibility implications, passing nextest evidence, and any required WP-01B manifest field; stop when candidate integration has truthful structured identity.

### WP-05 — Candidate release integration and projection proof

Outcome: one immutable candidate release passes whole-generation verification and exposes both MCP servers plus canonical compatibility projections.

Depends on: WP-01B, WP-02, WP-03, WP-04.

Owned scope: controller integration of shared manifests, locks, builder scripts, compatibility materialization, release docs, and `dist/release` evidence.

Work: reconcile package-local locks into producer locks; generate component archives and compliance outputs; assemble `RELEASE.json`/`SHA256SUMS`; verify tree hashes and dependency closure; build optional fat archive from exact components; materialize Codex projections from canonical Browser bytes; run the release verifier from a neutral directory; record release id, manifest hash, root, and producer commit.

Validation: deterministic rebuild/hash comparison; extracted/fat parity; `sky_cua tools/list` and `node_repl tools/list`; direct invocations through release-root wrappers; correct projection hash accepted, modified byte rejected before connect; packaged bytes and declared tree hashes match.

Handoff and stopping condition: freeze one candidate release id/hash/root. Only then create WP-08's Codex task.

### WP-06 — OpenClaw/OpenCode two-server installation and invocation

Outcome: standalone installation configures, reloads, lists, and invokes both servers in OpenClaw and OpenCode from clean neutral directories.

Depends on: WP-05.

Owned scope: controller-owned host installer adapters and focused tests under `scripts/`; host config mutations only through existing accepted installer seams with backups/rollback.

Work: extend existing OpenClaw/OpenCode install logic to resolve one verified current generation; emit both MCP definitions and required provenance/env; preserve unrelated host config; add reload/status/list plus actual tool-invocation enforcement; add model harness assertions equivalent to current Pi evidence rules.

Validation: fixture tests for merge/idempotence/rollback; clean temporary-home install tests; real OpenClaw and OpenCode reload; neutral-directory `tools/list`; successful `sky_cua` action and persistent `node_repl` multi-call execution; tool-evidence parsing rejects readiness-only false positives.

Handoff and stopping condition: record config backups, exact installed generation, host reload evidence, and successful model tool calls.

### WP-07 — Full parity, compliance, and performance gates

Outcome: runtime capabilities, real file/media behavior, legal/provenance inventory, and latency meet the locked contract.

Depends on: WP-05; may overlap WP-06 where resources do not contend.

Owned scope: production acceptance fixtures, compliance generation/verification, benchmark harness and artifacts; shared release changes integrate through controller.

Work: port and expand Codex production acceptance; ensure real file reads/writes and image emissions are content-verified; verify native addon architecture and data files; create SBOM/provenance/license outputs; capture pre-cutover baseline using the current installed path; benchmark the candidate on representative warm workloads; identify direct sky_cua request path and prove absence of Node process/hop.

Validation: all mandatory media/file/module cases; independent hash/content checks of written outputs; license/SBOM/provenance hashes agree with `RELEASE.json`; repeated warm samples with raw results; median <=110% and p95 <=125% of baseline; WebP default assertion.

Handoff and stopping condition: attach machine-readable artifacts under `artifacts/cua-stack-ownership/` and summarize accepted ratios/remaining variance in this plan.

### WP-08 — Codex Desktop verified consumer task

Outcome: one separate Codex Desktop task turns the preserved current tree into a verified consumer of the accepted sky-cua release and removes producer ownership only after installed acceptance.

Depends on: WP-05 candidate release accepted.

Owned scope: `/home/bex/projects/codex-desktop` only; resolver precedence, environment hydration, packaged fallback, compatibility materialization, installed acceptance, eventual producer deletion. It must not modify sky-cua.

Work: create exactly one new Codex project task from the current working-tree state; give it a dedicated full `/goal`; supply release id, manifest hash, release root, resolver/env contract, dirty-tree preservation rules, acceptance, review, and semantic-commit authorization; monitor it from this controller; require exact runtime root/hash/process proof before accepting; defer producer deletion until standalone/current and packaged fallback both pass.

Validation: resolver precedence fixtures; invalid/tampered roots fail honestly; env values preserved; packaged fallback verified; generated compatibility bytes match canonical hash; installed `/opt/...` artifact and process environment point to exact accepted generation; ordinary Codex IAB and Brave-origin node_repl tasks succeed; separate Codex consumer review passes.

Handoff and stopping condition: task reports exact commit/status plus release id/hash/root and installed live evidence. Do not accept source-only tests or readiness logs.

### WP-09 — Atomic promotion and full live acceptance

Outcome: the standalone current generation and all three hosts run the accepted release, concurrency/provenance is proven, and rollback is operational.

Depends on: WP-06, WP-07, WP-08.

Owned scope: controller-only real release root, host reloads, daemon/browser state, Codex install/restart, live evidence and rollback.

Work: stop only processes required by accepted installer seams; promote standalone generation; verify journal/current/prior; reload OpenClaw/OpenCode; install/restart Codex consumer; prove active binaries/modules via paths, hashes, process environment/maps where applicable; run ordinary live model tasks in each host; run simultaneous Codex/OpenClaw/OpenCode Browser tasks against separate tabs/groups through one bridge actor; run `@heliasar/sky-cua` screenshot/move/click/drag/scroll/keyboard/type/window operations; test IAB with ordinary Codex and Brave Origin with provider unset; trigger rollback and re-promote.

Validation: exact MCP/model transcripts and daemon structured provenance; real screenshots/files/PDF/OCR outputs; group/tab inventory; process generation proof; current/prior/journal hashes; post-rollback host invocations; required VM `all` profile and applicable live desktop gates per repository instructions.

Handoff and stopping condition: all live gates pass on installed artifacts, or rollback restores the prior generation and the plan records the exact external blocker plus preserved candidate.

### WP-10 — Canonical installed documentation and persistent Phone JS

Outcome: one designed-as-a-whole post-cutover component makes the complete CUA stack discoverable and runnable by ordinary models, and Phone Use reaches parity as a persistent first-party JavaScript facade through node_repl.

Depends on: WP-09 initial installed/live acceptance. Only release-schema field reservation occurs earlier.

Owned scope: a dedicated design deliverable under `docs/research/` or the evidence-backed design location selected after cutover; `packages/` Phone facade/export; compact top-level skills; shared installed references/recipes; documentation/API/example generators and focused tests. Existing stable skill/runtime contracts remain authoritative. Shared release integration stays controller-owned.

Work: first write and review one cohesive design covering information architecture, routing decisions, Phone JS public API/lifecycle/errors/provenance, installed paths, generators, test harness, performance budget, and compatibility projection behavior. Preserve upstream evidence only as behavioral inspiration: persistent `node_repl js`, installed Browser bootstrap, `nodeRepl.write`/`emitImage`, `agent.browsers.*`, required `browser.documentation()`, `tab.playwright`, persistent Computer Use wrapper/state, and local screenshot reads via `node:fs/promises`; copy no proprietary prose. Implement the Phone JS facade over the normal sky-cua daemon/service path with capability discovery, screenshots/image emission, local files, structured disconnect/errors, supplied/synthetic metadata fidelity, explicit provenance, and direct-vs-REPL routing. Keep Browser/Computer/Phone top-level skills compact and route progressively into shared canonical recipes/references rather than duplicating one giant guide.

The shared node_repl toolbox must cover persistent `js`/`js_reset`/`js_add_node_module_dir`; `nodeRepl.write`, `emitImage`, response metadata and local files; Node globals/imports; Buffer/ArrayBuffer/Blob/streams/paths/file URLs/data URLs; Sharp WebP/PNG/JPEG; Canvas/Skia; pixelmatch; PDF.js extraction/rendering with fonts/cmaps; Tesseract language data; standalone Playwright using system Chrome-family browsers, explicitly distinguished from `tab.playwright`; and composed Browser + Computer + Phone + OCR/PDF/image/file workflows. Generate API/contract and package/version capability inventories from source/manifest truth. Make examples copy-safe and installed-path-only, with troubleshooting and explicit unsupported/follow-up behavior.

Validation: package-local Phone unit/integration tests; generated-output drift tests; routing/link/progressive-disclosure validation; no checkout paths or stale versions; all examples execute under bundled Node 24 against installed release artifacts; import/capability tests; supplied and synthetic metadata/provenance checks; screenshot/image/local-file/error/disconnect cases; projection equality proves no separate implementation.

Performance considerations: measure Phone facade and recipe bootstrap cold/warm overhead separately from direct MCP; reuse the shared daemon and persistent node_repl state; do not add a daemon/MCP lifecycle hop; keep compact skill token cost bounded and record generated reference size. Any material latency or context-cost regression must be resolved or recorded against an explicit budget before acceptance.

Handoff and stopping condition: the reviewed design, implementation, generated inventories, exact installed paths/hashes, focused passing tests, and remaining model/live gates are recorded. Stop only when WP-11 can test immutable installed artifacts; source-only or dependency-README proof is insufficient.

### WP-11 — Installed docs, discoverability, Phone parity, and composed acceptance

Outcome: Codex Desktop, OpenClaw, and OpenCode ordinary models discover the compact skills, route to the right installed recipe, execute every example, and use persistent Phone JS correctly alongside Browser/Computer/toolbox workflows.

Depends on: WP-10 integrated into a new immutable candidate generation; rerun applicable WP-05 verification and promote through the WP-09 transaction path.

Owned scope: controller-owned installed release, three host reloads, ordinary-model acceptance harnesses, Phone device/emulator lane, documentation/example artifacts, and performance evidence. Host/device state is serialized.

Work: verify documentation component tree/archive and per-inventory/example hashes in `RELEASE.json` and provenance; run link/routing/progressive-disclosure checks from an extracted/fat release with no checkout; execute every example under bundled Node 24; prove imports and capability inventory; ask ordinary models in each host to start from Browser, Computer Use, and Phone Use skills and observe correct direct-MCP versus persistent-node_repl choices; run full Phone JS lifecycle/capability/screenshot/image/local-file/error/disconnect acceptance; run composed Browser + Computer + Phone + OCR/PDF/image/file tasks; confirm compatibility projections route to canonical docs without copied implementation.

Parity/acceptance matrix: each of Codex Desktop/OpenClaw/OpenCode x Browser/Computer/Phone main skill must discover at least one relevant installed recipe and execute it; direct actions remain direct when persistence/composition is unnecessary; persistent/reusable/composed tasks use node_repl; Browser `tab.playwright` and standalone Playwright are selected correctly; every generated example has an execution result and content assertion; Phone direct MCP and Phone JS results agree on capability/identity semantics; disconnect and unsupported behavior are structured and truthful.

Validation: documentation tests plus installed ordinary-model transcripts with enforced tool evidence; full Phone JS acceptance on an available connected device and emulator-supported subset; output hashes/content checks; bootstrap and warm-operation latency plus compact-skill token/reference-size measurements; tamper one docs/example/API inventory byte and prove generation verification fails.

Handoff and stopping condition: all cells pass against the same installed release id/hash/root and active process generation, or rollback restores the prior generation and the exact external device/model blocker remains active in this plan.

### WP-12 — Frozen-scope review, documentation, commits, and retirement

Outcome: implementation is reviewed and simplified, ownership is singular, durable docs match reality, validated work is committed semantically, and this plan is retired only after live proof.

Depends on: WP-11.

Owned scope: controller integration; review fixes remain within frozen changed/ownership scopes. Codex consumer fixes stay in its task/repository.

Work: freeze sky-cua diff and run exhaustive review/fix/retest; run separate Codex consumer review; inspect both repositories and installed release for duplicate producer paths, mixed-generation fallbacks, copied Browser/docs bytes, Skynet acceptance, stale trust hashes, and checkout-only examples; remove stale producer paths only after fallback/live proof; update/create `docs/features/complete-cua-stack-ownership.md`, relevant operations/runtime docs, and `ROADMAP.md`; create semantic commits per repository; verify working trees and installed generation; follow `plans/AGENTS.md` retirement by deleting this file after docs and roadmap are complete.

Validation: focused tests for every review fix; root `cargo fmt --check`, `cargo clippy --workspace --all-targets`, `cargo nextest run`, `cargo test --doc` when applicable; Python Ruff format/check, basedpyright, pytest; JS/Bun build/test/typecheck/format gates; `python3 scripts/build_plugin.py`; packaging/release verifier; VM `all`; final installed live acceptance; `git diff --check`; cross-repo ownership searches and byte/hash comparison.

Handoff and stopping condition: semantic commits exist locally in each changed repo, no push/PR occurs, docs/roadmap are shipped, this ExecPlan is deleted, and the active `/goal` is marked complete only after all live evidence is recorded.

## Validation and Recovery

Package validation is narrow first, then integration-wide. Rust tests use `cargo nextest run`, never `cargo test`, except the separately required `cargo test --doc`. Python commands use the repository `uv` environment. Node runtime tests execute under the bundled Node 24.14.0; Bun builds/tests the first-party TypeScript sources.

Mandatory transcript and runtime matrix:

- MCP framing: initialize, `tools/list`, `js`, `js_reset`, `js_add_node_module_dir`, cancellation, timeouts, malformed calls, supplied metadata fidelity, synthetic metadata stability, `initialize.clientInfo`, and per-call turn ids.
- Persistent VM: bindings across calls, top-level await, reset erasure, output ordering/limits, exception formatting, cancellation recovery, and private child teardown.
- Modules/files: installed module dirs, local ESM/CJS files, native addons, data URLs, buffers, absolute/relative paths, neutral working directories, and verified outputs.
- Browser: every documented API/declaration/command fixture; correct/wrong hash behavior; IAB ordinary Codex task; separate Brave Origin task with `CODEX_BROWSER_PROVIDER` unset; concurrent three-host tabs/groups and structured provenance; no ambiguous mutation retry.
- Computer Use JS: screenshot, move, click, drag, scroll, keyboard, type, and window action through `@heliasar/sky-cua`; WebP default plus explicit PNG/JPEG.
- Workbench: OCR, PDF.js fonts/cmaps, Sharp/libvips, Canvas, pixelmatch, codecs, Playwright with system browser, real file reads/transforms, emitted images, and content-verified written outputs.
- Installation: full default and core-only; tamper/missing/mixed component rejection; journal interruption at each phase; recovery; reinstall; rollback; resolver fallback; active process generation.
- Hosts: clean/neutral OpenClaw/OpenCode install, reload, list, actual invocation, and ordinary model task; installed Codex task with IAB and Origin; readiness-only output is not acceptance.
- Performance: record baseline raw samples first, use the same warmed workload/hardware/session for candidate comparison, report median/p95 ratios, and fail the locked thresholds.
- Documentation/Phone final phase: verify routing links and progressive disclosure; reject stale versions/checkout paths; execute every example under bundled Node 24; test imports/capabilities; enforce ordinary-model recipe discovery in all three hosts; prove direct-vs-REPL choices; run full Phone JS lifecycle/image/file/error/disconnect acceptance; bind and tamper-test docs/API/example hashes; measure bootstrap/warm latency and compact-skill/reference size.

All generated/live artifacts go under ignored `artifacts/cua-stack-ownership/<release-id>/` with a small index containing commands, UTC timestamps, target, release id, hashes, outcomes, and paths. Never store credentials, tokens, private browser payloads, or sensitive screenshots.

Installation and cutover are idempotent. Re-running the builder with unchanged inputs produces the same component content/tree hashes. Reinstalling an already-complete release verifies it and leaves `current` stable. A failed pre-promotion build removes only its temporary staging directory. A failure after journal creation is recovered by reading the journal and verifying complete generations; no heuristic file mixing is allowed. A live acceptance failure rolls `current` back to the retained prior generation and reloads affected hosts before debugging continues. Stale producer deletion occurs only after a successful rollback/re-promotion drill.

External blockers do not justify a completion claim. If a model host, browser session, device, portal, VM, credential, or package-manager state blocks live proof, exhaust safe local alternatives, retain the verified candidate and prior generation, record the exact command/error/missing state here, and keep the goal active. Deployment-like local host reload/install actions are authorized by the objective; pushes, PRs, releases to remote systems, and credential use for a new purpose are not.

## Artifacts

Expected durable or ignored evidence:

- `plans/complete-cua-stack-ownership.md` while active; deleted on fully proven retirement.
- `docs/features/complete-cua-stack-ownership.md` plus relevant runtime/operations docs and `ROADMAP.md` at closeout.
- Versioned release schema and fixtures under an evidence-backed source path selected by WP-01B.
- `dist/release/<release-id>/RELEASE.json`, `SHA256SUMS`, component archives/directories, Codex projection, compliance data, and optional fat archive.
- The post-cutover `documentation` component plus generated routing/API/capability/example inventories, all individually hashed by `RELEASE.json`/provenance and consumable without repository paths.
- `artifacts/cua-stack-ownership/<release-id>/` transcript, benchmark, install/recovery, process-generation, concurrency/provenance, VM, and live-model evidence.
- Local semantic sky-cua and Codex Desktop commits after validation. No push/PR directive or remote publication without fresh authorization.
