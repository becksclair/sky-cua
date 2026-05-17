# Docs Guide

## Package Identity

`docs/` holds durable project knowledge: stable runtime contracts, descriptive
documentation for shipped features, operator-facing harness and runbook docs,
and dated research findings. It is not a place for in-flight design (that
lives in `plans/`) or for live session state (that lives in `CONTINUITY.md`).

## Setup & Run

```bash
rg -n "TODO|FIXME|blocked|proof|artifact" docs
```

## Subdirectory Purposes

- `runtime/` — stable runtime contracts. The MCP boundary, the Linux backend
  architecture, the cross-platform model. Change these only when the actual
  contract changes.
- `features/` — one descriptive document per shipped subsystem. Use the
  feature doc template below. Update when behavior, contract, or limitations
  change.
- `operations/` — operator-facing harness, runbook, and procedure docs.
- `research/` — dated research findings. One question per file, self-contained,
  not living documents. Filenames are `YYYY-MM-<slug>.md`.

## Feature Doc Template

Every `docs/features/<slug>.md` follows this shape:

```markdown
# <Feature name>

## Status

Shipped / Partial / Deprecated. Last verified: <date / commit / artifact>.

## Summary

1–3 sentences. What this feature does for the user or agent.

## Contract surface

MCP tools, fields, env vars, IPC variants, file paths the feature exposes.
What callers can rely on. What is intentionally not stable.

## Behavior

How it actually works at runtime. Selection rules, fallback ladders,
diagnostics emitted, side effects.

## Source paths

Concrete `crates/...`, `scripts/...`, `resources/...` references.

## Verification

Unit/integration tests, live smokes, and the most recent accepted artifact.

## Known limitations

Honest list. Things deferred, environments not yet covered, partial paths.

## Related

Research extracts, ROADMAP entry, originating ExecPlan if archived.
```

Do not include `Progress`, `Surprises & Discoveries`, or `Decision Log` in
feature docs. Those are ExecPlan concerns and stay with the plan.

## Research Doc Scope

A research doc answers ONE research question with evidence and a conclusion.

- Self-contained. Not a living document. Dated in the filename.
- Use for: experiment outcomes, comparisons, third-party investigations,
  postmortems of one decision.
- Not for: transcripts, raw artifact dumps, or per-session notes (those live
  in `artifacts/` and are not committed unless small and curated).

A useful research doc shape:

```markdown
# <Research question or finding>

## Context

What prompted this investigation. Link to the seam or feature.

## Investigation

Evidence gathered, commands run, sources consulted.

## Conclusion

What we now believe and why. Concrete enough to act on.

## Implications

What this changed in the codebase, the plan, or the product.
```

## Touch Points / Key Files

- Roadmap index: `../ROADMAP.md`
- Feature docs: `features/`
- Research extracts: `research/`
- Active ExecPlans: `../plans/`
- Live state snapshot: `../CONTINUITY.md`
- Durable tactical notes: `../NOTES.md`

## JIT Index Hints

- Find feature docs that mention a seam: `rg -n "PipeWire|KWin|UIA|RemoteDesktop|portal" docs/features`
- Find research conclusions: `rg -n "## Conclusion" docs/research`
- Find runtime contract changes: `rg -n "tool|MCP|backend|capability" docs/runtime`
- Find command snippets: `rg -n "python3 scripts|cargo|uv run|codex" docs`

## Common Gotchas

- `CONTINUITY.md` is the live handoff snapshot; docs are durable narrative,
  not live task state.
- `NOTES.md` stores reusable facts; do not duplicate large sections into
  docs.
- Feature doc `Verification` sections own artifact paths; do not also list
  them in `NOTES.md` or `CONTINUITY.md`.

## Pre-PR Checks

When a feature ships:

1. Add or update `docs/features/<slug>.md` using the template above.
2. Extract any durable research into `docs/research/YYYY-MM-<slug>.md`.
3. Update the relevant `ROADMAP.md` entry (check the box, link the feature
   doc, add follow-up sub-items if any partial work remains).
4. Retire the originating ExecPlan per `plans/AGENTS.md`.

```bash
rg -n "TODO|FIXME|PLACEHOLDER" docs && false || true
```
