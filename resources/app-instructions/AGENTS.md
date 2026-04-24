# App Instructions Guide

## Package Identity

`resources/app-instructions/` contains app-specific guidance and machine-readable action policy for the Linux backend and MCP client.
Markdown guides are packaged into the plugin and may appear in `get_app_state.app_guidance`.

## Setup & Run

```bash
cargo test -p sky-cua-platform app_instructions
cargo test -p sky-cua-linux app_policy
python3 scripts/build_plugin.py
```

## Patterns & Conventions

- Register every app guidance file in `index.json`.
- Use desktop-file IDs as canonical `key` values where possible.
- Keep aliases lowercase and practical, following `kate`, `kwrite`, `dolphin`, and `firefox`.
- Only add machine policy fields when backend code reads them.
- DO: Use `Kate.md` and `KWrite.md` as examples for editor guidance with `set_value_fallback`.
- DO: Use `Firefox.md` as the pattern for cautious web-content guidance.
- DO: Keep markdown short and behavioral; avoid broad documentation or transcripts.
- DO: Keep policy values aligned with `crates/sky-cua-linux/src/app_policy.rs`.
- DON'T: Claim semantic write support unless a live/backend proof exists.
- DON'T: Invent per-app policies in markdown only if code needs a structured knob.

## Touch Points / Key Files

- Registry and policy metadata: `index.json`
- Kate guidance: `Kate.md`
- KWrite guidance: `KWrite.md`
- Dolphin guidance: `Dolphin.md`
- Firefox guidance: `Firefox.md`
- Platform key normalization: `../../crates/sky-cua-platform/src/app_instructions.rs`
- Backend policy reader: `../../crates/sky-cua-linux/src/app_policy.rs`

## JIT Index Hints

- Find registered apps: `rg -n '"key"|"aliases"|"set_value_' index.json`
- Find guidance wording: `rg -n "semantic|physical|keyboard|tree|fallback" *.md`
- Find backend policy consumers: `rg -n "SetValueFallback|set_value_fallback|set_value_routing" ../../crates`
- Find app-guidance enrichment: `rg -n "app_guidance|HeuristicsRegistry" ../../crates/sky-cua-client/src`

## Common Gotchas

- `index.json` drives both markdown lookup and backend action policy; validate both readers after changes.
- Keep fallback descriptions honest: guidance should teach strategy, not pretend an app has better accessibility than it does.
- Build the plugin after resource edits so packaging failures show up early.

## Pre-PR Checks

```bash
cargo test -p sky-cua-platform app_instructions && cargo test -p sky-cua-linux app_policy && python3 scripts/build_plugin.py
```
