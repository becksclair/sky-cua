# Cross-Surface AppShots

This ExecPlan is maintained while implementation is active. Follow
`plans/AGENTS.md` when updating or retiring it.

## Purpose

Promote `observe(surface=desktop|browser|phone)` into sky-cua's canonical
AppShot producer. The returned envelope binds pixels, semantic state, exact
subject identity, and the existing action snapshot to one capture generation.
State-changing MCP tools require a fresh `appshot_id`; validation and recovery
remain server-side so Codex, OpenCode, Pi, OpenClaw, and direct MCP clients all
receive the same behavior without AppServer or host-composer hooks.

## Progress

- [x] (2026-08-02) Created an isolated `bex/phone-companion-direct` worktree
  from `origin/main`; the original dirty checkout remains untouched.
- [x] (2026-08-02) Defined the shared AppShot envelope, tagged subject identity,
  consistency/truncation states, recovery error, and negative serde gates. The
  Phase 1 broad and blocker-focused ultra-review findings were fixed; the
  terminal deterministic marker is 115 platform tests plus clippy.
- [ ] Adapt desktop, browser, and phone observation producers.
- [ ] Fence state-changing actions and return recovery AppShots without
  executing rejected side effects.
- [ ] Update all shipped skills and installed-host harnesses.
- [ ] Complete deterministic, installed-host, and physical acceptance gates.

## Surprises & Discoveries

- The existing Linux AppShot is a service/Node API with private artifacts; it
  is not yet the universal MCP `observe` response.
- Browser observation and screenshot are separate operations and do not share
  an explicit navigation/document generation.
- Phone observation currently combines sequential reads rather than one
  consistency-fenced accessibility capture.

## Decision Log

- Decision: extend `observe`, not introduce a host-specific pre-turn hook or a
  parallel AppShot tool. Rationale: MCP-server enforcement is portable to every
  supported host, including OpenCode.
- Decision: ordinary dispatch actions do not automatically recapture afterward.
  Explicit open/navigation/activation transitions return a destination AppShot.
- Decision: a missing or invalid AppShot fence fails before dispatch and
  includes a fresh recovery AppShot.

## Outcomes & Retrospective

Pending.

## Context

The current exact-window producer lives under `crates/sky-cua-service/src/appshot.rs`.
MCP observation/action routing lives under `crates/sky-cua-client/src/mcp_tools/`.
Browser contracts are in `crates/sky-cua-platform/src/model/browser.rs`; phone
contracts are in `crates/sky-cua-platform/src/model/phone.rs`.

## Plan of Work

1. Add one surface-neutral serialized AppShot contract and shared validation
   vocabulary in `sky-cua-platform`.
2. Make each producer capture pixels and semantics against an exact subject and
   generation, retrying once if the subject changes.
3. Store full phone trees as bounded private semantic artifacts while returning
   a token-bounded projection with explicit truncation metadata.
4. Add server-side AppShot fencing at state-changing dispatch boundaries.
5. Update skill guidance and installed-host traces.

## Validation

- Platform serialization and fixture round trips.
- Surface-specific unit/integration tests.
- Missing, stale, wrong-target, wrong-session, and wrong-epoch rejection with
  proof that no requested side effect executed.
- Installed direct MCP, Codex, OpenCode, Pi, and OpenClaw traces.
- Physical phone screenshot/tree congruence, rotation, overlays, sparse trees,
  and secure-window behavior.

## Idempotence and Recovery

Observation is idempotent. Rejected mutations never dispatch. Ordinary
mutations are never replayed automatically. A fresh observation supersedes the
prior action fence for that exact subject/session/epoch.

## Artifacts

Ignored evidence belongs under `artifacts/phone-companion-direct/appshots/`.
No captured personal content is committed.

## Interfaces and Dependencies

- `AppShotEnvelope` and surface-specific subject/semantic payloads.
- Existing desktop action snapshot, browser tab/document identity, and phone
  device/link epoch.
- Existing MCP `observe` tool; no AppServer dependency.
