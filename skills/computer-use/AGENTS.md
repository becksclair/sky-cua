# computer-use Skill Guide

This directory is the host-portable computer-use workflow skill shipped with
the runtime and bundled by host adapters. It teaches agents how to combine
control trees, screenshots, and physical actions when operating desktop
apps. The skill is packaged for agents: bloated wording directly costs
runtime tokens.

## Conventions

- `SKILL.md` is the entrypoint and stays concise; deeper examples live in
  `references/*.md` (e.g. `references/hybrid-patterns.md`), agent-specific
  adapters under `agents/`.
- Preserve the "tree as structure, screenshot as visual truth" stance, and
  keep compact-state guidance aligned with the actual tool output contract
  in `crates/sky-cua-client/src/mcp_server.rs`.
- Use cross-platform shortcut wording (`Cmd + A / Ctrl + A`); do not
  hardcode Linux-only tool names in generic workflow rules unless the
  section is explicitly Linux-specific.
- `references/apps/` mirrors `resources/app-instructions/` for hosts that
  cannot reach the runtime `resources/` tree; keep them in sync.
- Keep workflow guidance compatible with sparse/fallback-only UIs as well as
  richer semantic trees.

## Acceptance

After skill edits: `python3 scripts/build_plugin.py`, install, and run
`python3 scripts/live_app_server_smoke.py` for real proof.
