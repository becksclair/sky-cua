# computer-use-workflows Skill Guide

## Package Identity

This directory is the host-portable workflow skill shipped with the runtime and bundled by the Codex plugin adapter.
It teaches agents how to combine control trees, screenshots, and physical actions when operating desktop apps.

## Setup & Run

```bash
python3 scripts/build_plugin.py
python3 scripts/install_plugin.py --bundle-root dist/plugin/sky-cua
python3 scripts/live_app_server_smoke.py
```

## Patterns & Conventions

- `SKILL.md` is the entrypoint and must stay concise enough for frequent loading.
- Put deeper examples in `references/*.md` and link them from `SKILL.md`.
- Agent-specific adapter files live under `agents/`.
- Use cross-platform shortcut wording like `Cmd + A / Ctrl + A` from `SKILL.md`.
- DO: Preserve the "tree as structure, screenshot as visual truth" stance in `SKILL.md`.
- DO: Keep compact-state guidance aligned with `crates/sky-cua-client/src/mcp_server.rs`.
- DO: Use `references/hybrid-patterns.md` for longer workflow examples.
- DON'T: Hardcode Linux-only tool names in generic workflow rules unless the section is explicitly Linux-specific.
- DON'T: Encourage typing placeholder junk or acting from stale screenshots.
- DON'T: Duplicate app-specific guidance already living in `resources/app-instructions/*.md`.

## Touch Points / Key Files

- Skill entrypoint: `SKILL.md`
- Agent adapter: `agents/openai.yaml`
- Longer workflow reference: `references/hybrid-patterns.md`
- App-specific runtime guidance: `../../resources/app-instructions/*.md`
- MCP tool output contract: `../../crates/sky-cua-client/src/mcp_server.rs`

## JIT Index Hints

- Find compact-state guidance: `rg -n "compact|detail|get_app_state" . ../../crates/sky-cua-client/src`
- Find screenshot-first wording: `rg -n "screenshot|visual source|tree" SKILL.md references`
- Find stale text-entry guidance: `rg -n "stale|typing|text entry|clear" .`
- Find app-specific overlap: `rg -n "Kate|KWrite|Firefox|Dolphin|TIDAL" . ../../resources/app-instructions`

## Common Gotchas

- This skill is packaged for agents, not just local docs; bloated wording directly costs runtime tokens.
- Keep workflow guidance compatible with sparse/fallback-only UIs and richer semantic trees.
- After skill edits, build/install and use the app-server smoke for real acceptance proof.

## Pre-PR Checks

```bash
python3 scripts/build_plugin.py && python3 scripts/live_app_server_smoke.py
```
