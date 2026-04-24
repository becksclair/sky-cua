# Plans Guide

## Package Identity

`plans/` contains focused design and implementation plans for future or in-progress runtime work.
Plans should be concise, evidence-backed, and easy to retire once the code or durable docs supersede them.

## Setup & Run

```bash
rg -n "TODO|FIXME|blocked|open question|artifact" plans
```

## Patterns & Conventions

- Keep plans scoped to one seam or feature area.
- Include exact source paths, commands, and artifacts that prove the current state.
- Separate confirmed behavior from proposed work.
- DO: Use `wayland_fallback_vision_anchors.md` as the pattern for a focused runtime seam plan.
- DO: Move durable, completed investigation narrative into `../docs/` when it stops being a plan.
- DON'T: Let plans become stale TODO dumps; update or delete them when code lands.
- DON'T: Store transcripts or raw JSON; point to artifacts and summarize the relevant evidence.

## Touch Points / Key Files

- Wayland fallback-anchor plan: `wayland_fallback_vision_anchors.md`
- Durable docs: `../docs/`
- Live handoff snapshot: `../CONTINUITY.md`
- Tactical facts: `../NOTES.md`

## JIT Index Hints

- Find target source paths: `rg -n "crates/|scripts/|resources/" .`
- Find proof artifacts: `rg -n "artifacts/|last-message|summary.json|jsonl" .`
- Find unresolved work: `rg -n "next|blocked|open|remaining|TODO|FIXME" .`

## Common Gotchas

- Plans are guidance, not authority. Current source and live runtime proof win.
- Do not duplicate `CONTINUITY.md`; plans should explain a design seam, not track every session turn.

## Pre-PR Checks

```bash
rg -n "TODO|FIXME|PLACEHOLDER" plans && false || true
```
