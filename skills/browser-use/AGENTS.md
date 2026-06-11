# browser-use Skill Guide

This directory is the host-portable browser-use workflow skill shipped with
the runtime and bundled by host adapters. It is read by frontier models at
runtime: every line costs tokens and competes with task context.

## Conventions

- `SKILL.md` carries only what a capable model cannot infer from the tool
  schemas: tab ownership/claiming rules, the CSS-pixel coordinate contract,
  output-size levers, and scroll semantics. No generic agent hygiene or
  numbered action loops.
- Preserve the distinction between browser CSS pixels and desktop screenshot
  pixels, and keep target semantics aligned with
  `docs/features/browser-mcp-tools.md` (repo-side reference; do not link
  repo paths from `SKILL.md` — installed copies cannot reach them).
- Do not turn native-host implementation details into workflow advice unless
  the model needs them to act correctly.
- Validate packaging after edits: `python3 scripts/build_plugin.py`.
