# Docs Guide

`docs/` holds durable project knowledge: stable runtime contracts
(`runtime/`), one descriptive document per shipped subsystem (`features/`),
operator-facing runbooks (`operations/`), and dated research findings
(`research/`, filenames `YYYY-MM-<slug>.md`). It is not a place for
in-flight design (that lives in `plans/`) or live session state (which is
not recorded in repo files).

Docs are durable narrative, not live task state. `NOTES.md` stores reusable
tactical facts; do not duplicate large sections between the two. Feature doc
`Verification` sections own artifact paths; do not also list them in
`NOTES.md`.

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
feature docs; those are ExecPlan concerns and stay with the plan.

## Research Doc Scope

A research doc answers ONE research question with evidence and a conclusion:
self-contained, dated in the filename, not a living document. Use for
experiment outcomes, comparisons, third-party investigations, and
postmortems of one decision — not for transcripts, raw artifact dumps, or
per-session notes. Shape: `## Context` (what prompted it), `## Investigation`
(evidence), `## Conclusion` (what we now believe, concrete enough to act
on), `## Implications` (what it changed).

## When a Feature Ships

1. Add or update `docs/features/<slug>.md` using the template above.
2. Extract any durable research into `docs/research/YYYY-MM-<slug>.md`.
3. Update the relevant `ROADMAP.md` entry (check the box, link the feature
   doc, add follow-up sub-items if partial work remains).
4. Retire the originating ExecPlan per `plans/AGENTS.md`.
