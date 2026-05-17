# Plans Guide

## Package Identity

`plans/` contains active forward-looking ExecPlans only. Each plan covers
one seam or feature area and is retired the moment the work it describes
lands and is proven.

## Setup & Run

```bash
rg -n "TODO|FIXME|blocked|open question|artifact" plans
```

## Patterns & Conventions

- One ExecPlan per seam or feature area. Do not bundle unrelated work.
- Plans are forward-looking. If a plan is "code complete", retire it via the
  Lifecycle section below; do not leave it as a postmortem in `plans/`.
- Use the standard ExecPlan shape: Purpose, Progress, Surprises & Discoveries,
  Decision Log, Outcomes & Retrospective, plus Context, Plan of Work,
  Validation, Idempotence, Artifacts, and Interfaces sections.
- Use human-readable slug filenames (`wayland_fallback_vision_anchors.md`).
  No timestamps, no auto-generated names like `1778463694899-nimble-knight.md`.
- Include exact source paths, commands, and artifacts that prove current
  state. Separate confirmed behavior from proposed work.
- DO: Use `wayland_fallback_vision_anchors.md` as the pattern for a focused
  forward-looking ExecPlan.
- DO: Move durable, completed investigation narrative into `../docs/features/`
  and `../docs/research/` when it stops being a plan.
- DON'T: Let plans become stale TODO dumps; retire them when code lands.
- DON'T: Store transcripts or raw JSON; point to artifacts and summarize.
- DON'T: Use the `goals/<name>/` Plannotator package layout; ExecPlans are
  the single forward-looking format.

## Lifecycle

When an ExecPlan reaches "code complete" with live proof, retire it:

1. Create or update `docs/features/<slug>.md` from the feature doc template
   in `docs/AGENTS.md`. The feature doc is descriptive of current behavior
   and does not include `Progress`, `Surprises & Discoveries`, or
   `Decision Log` sections.
2. Extract any durable research findings into
   `docs/research/YYYY-MM-<slug>.md`. One research question per file.
3. Update the matching `ROADMAP.md` entry: check the box, link the feature
   doc, add follow-up sub-items if partial work remains.
4. Delete the ExecPlan, or move it to `archive/plans/` if the long-form
   ledger has archaeological value beyond what git history preserves.

Do not let `plans/` accumulate postmortems. If you are adding a "Status: code
complete" line to a plan, you are retiring it, not annotating it.

## Touch Points / Key Files

- Roadmap index: `../ROADMAP.md`
- Feature docs (where retired plans land): `../docs/features/`
- Research extracts (where retired research lands): `../docs/research/`
- Live handoff snapshot: `../CONTINUITY.md`
- Tactical facts: `../NOTES.md`

## JIT Index Hints

- Find target source paths: `rg -n "crates/|scripts/|resources/" .`
- Find proof artifacts: `rg -n "artifacts/|last-message|summary.json|jsonl" .`
- Find unresolved work: `rg -n "next|blocked|open|remaining|TODO|FIXME" .`

## Common Gotchas

- Plans are guidance, not authority. Current source and live runtime proof
  win.
- Do not duplicate `CONTINUITY.md`; plans should explain a design seam, not
  track every session turn.
- Do not duplicate `ROADMAP.md` entries; the plan is the design narrative,
  the roadmap is the index.

## Pre-PR Checks

```bash
rg -n "TODO|FIXME|PLACEHOLDER" plans && false || true
```
