# computer-use Skill Guide

This directory is the host-portable computer-use workflow skill shipped with
the runtime and bundled by host adapters. It is read by frontier models at
runtime: every line costs tokens and competes with task context.

## Conventions

- `SKILL.md` carries only what a capable model cannot infer from the tool
  schemas: coordinate-space contracts, trust ordering between tree and
  screenshot, fallback-tree semantics, flag usage policy, and platform
  quirks. No generic agent hygiene ("verify your work", step-by-step
  ladders, example dialogues) — capable models do that unprompted, and
  filler dilutes the contracts.
- Keep guidance aligned with the actual tool output contract in
  `crates/sky-cua-client/src/mcp_server.rs` and the tool definitions in
  `crates/sky-cua-client/src/mcp_tools.rs`.
- Use cross-platform shortcut wording (`Cmd + A / Ctrl + A`); Linux-only
  facts live in an explicitly Linux-scoped section.
- `references/apps/` mirrors `resources/app-instructions/` for hosts that
  cannot reach the runtime `resources/` tree; keep them in sync. App files
  hold app-specific facts only.
- Skill-relative paths only: installed copies cannot reach repo `docs/`.

## Acceptance

After skill edits: `python3 scripts/build_plugin.py`, install, and run
`python3 scripts/live_app_server_smoke.py` for real proof.
