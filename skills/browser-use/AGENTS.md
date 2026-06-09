# browser-use Skill Guide

## Package Identity

This directory is the host-portable browser-use workflow skill shipped with the runtime and bundled by host adapters.

## Patterns & Conventions

- Keep `SKILL.md` short and product-facing; put long smoke recipes in docs or scripts.
- Preserve the distinction between browser screenshot pixels and desktop screenshot pixels.
- Keep the target semantics aligned with `docs/features/browser-mcp-tools.md`.
- Do not turn native-host implementation details into default workflow advice unless they are needed for debugging.

## Touch Points / Key Files

- Skill entrypoint: `SKILL.md`
- Agent adapter: `agents/openai.yaml`
- Browser feature contract: `../../docs/features/browser-mcp-tools.md`
- MCP boundary: `../../docs/runtime/mcp-boundary.md`
- Browser runtime implementation: `../../crates/sky-cua-service/src/browser.rs`

## Pre-PR Checks

```bash
python3 scripts/build_plugin.py
```
