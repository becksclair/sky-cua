# phone-use Skill Guide

This directory is the host-portable phone-use workflow skill shipped with the
runtime and bundled by host adapters. It is read by frontier models at runtime:
every line costs tokens and competes with task context.

## Conventions

- `SKILL.md` carries only what a capable model cannot infer from the tool
  schemas: the connect-before-act ordering, the capability-profile and
  available/unavailable-action contract, the fresh-`phone_snapshot_id`
  coordinate rule, the three-cursor-plane distinction, the explicit
  notification/action-id requirement, and the prefer-companion routing note. No
  generic agent hygiene, numbered loops, or example dialogues.
- Keep guidance aligned with the tool definitions in
  `crates/sky-cua-client/src/mcp_tools/` and the service phone backends in
  `crates/sky-cua-service/src/phone/`. The companion wire contract is
  authoritative in `docs/runtime/phone-companion-protocol.md` (repo-side
  reference; do not link repo paths from `SKILL.md` — installed copies cannot
  reach them).
- Do not turn native-host or Android implementation details into workflow
  advice unless the model needs them to act correctly.
- Skill-relative paths only: installed copies cannot reach repo `docs/`.
- Never put pairing codes, RPC tokens, notification content, or accessibility
  dumps into this skill or its examples.

## Acceptance

Validate packaging after edits: `python3 scripts/build_plugin.py`, then inspect
the staged bundle for `skills/phone-use`. Live phone proof runs through
`scripts/live_phone_use_smoke.py` against a connected device.
