# browser-use Skill Guide

This directory is the host-portable browser-use workflow skill shipped with
the runtime and bundled by host adapters.

## Conventions

- Keep `SKILL.md` short and product-facing; long smoke recipes belong in
  docs or scripts.
- Preserve the distinction between browser screenshot pixels and desktop
  screenshot pixels, and keep target semantics aligned with
  `docs/features/browser-mcp-tools.md`.
- Do not turn native-host implementation details into default workflow
  advice unless they are needed for debugging.
- Validate packaging after edits: `python3 scripts/build_plugin.py`.
