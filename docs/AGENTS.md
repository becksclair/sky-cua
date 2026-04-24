# Docs and Plans Guide

## Package Identity

`docs/` and `plans/` hold durable investigation writeups, implementation plans, and operator-facing context.
They should explain proven behavior and current seams without becoming transcripts.

## Setup & Run

```bash
rg -n "TODO|FIXME|blocked|proof|artifact" docs plans
python3 scripts/build_plugin.py
```

## Patterns & Conventions

- Use `docs/` for durable writeups like `docs/mcp-runtime.md` and
  `docs/codex-plugin-e2e-expedition.md`.
- Use `plans/` for focused design/implementation notes like `plans/wayland_fallback_vision_anchors.md`.
- Keep artifact references path-specific and summarize only the proof that matters.
- Update docs only when behavior, commands, or operational truth changed.
- DO: Follow the evidence-heavy style in `docs/codex-plugin-e2e-expedition.md` for E2E investigations.
- DO: Follow the scoped-plan style in `plans/wayland_fallback_vision_anchors.md` for design work.
- DON'T: Paste raw transcripts or giant JSON blobs; point to artifact paths and summarize.
- DON'T: Document speculative future behavior as if it already works.

## Touch Points / Key Files

- Full installed-plugin E2E investigation: `docs/codex-plugin-e2e-expedition.md`
- Portable runtime boundary: `docs/mcp-runtime.md`
- Linux implementation plan: `docs/computer-use-linux-plan.md`
- Wayland fallback anchors plan: `plans/wayland_fallback_vision_anchors.md`
- Live state snapshot: `../CONTINUITY.md`
- Durable tactical notes: `../NOTES.md`

## JIT Index Hints

- Find artifact references: `rg -n "artifacts/|last-message|summary.json|jsonl" docs plans`
- Find current blockers: `rg -n "blocker|blocked|remaining|next" docs plans`
- Find command snippets: `rg -n "python3 scripts|cargo|uv run|codex" docs plans`
- Find docs that mention a seam: `rg -n "PipeWire|KWin|TIDAL|app-server|ChatGPT-auth" docs plans`

## Common Gotchas

- `CONTINUITY.md` is the live handoff snapshot; docs are durable narrative, not live task state.
- `NOTES.md` stores reusable facts; do not duplicate large sections into docs.
- If a doc says a live proof exists, include the exact command or artifact path.

## Pre-PR Checks

```bash
rg -n "TODO|FIXME|PLACEHOLDER" docs plans && false || true
```
