# sky-cua Agent Guide

## Project Snapshot

`sky-cua` is a cross-platform Codex Computer Use plugin built as a Rust workspace plus Python harnesses.
The core runtime is Rust 2024: `sky-cua-client`, `sky-cua-service`, `sky-cua-platform`, and platform backends such as `sky-cua-linux` and `sky-cua-windows`.
Python under `scripts/` builds, installs, and live-smokes the plugin through `uv`, Ruff, basedpyright, and pytest.
Subdirectories have their own `AGENTS.md`; read the nearest one before editing files there.

## Root Setup Commands

```bash
cargo build
cargo test
uv sync --dev
uv run ruff format scripts
uv run ruff check scripts
uv run basedpyright
uv run pytest
python3 scripts/build_plugin.py
```

## Universal Conventions

- Prefer current source, `CONTINUITY.md`, and `NOTES.md` over memory or old artifacts.
- Keep runtime contracts explicit: structured diagnostics, concrete backend names, and honest fallback states.
- Prevent god files. When a source file grows past roughly 800 lines, do not keep adding unrelated responsibilities by default; first look for a cohesive boundary, such as contract families, transport adapters, planning policy, matching policy, or testable helpers. Keep public compatibility with re-exports when splitting shared contracts.
- Rust uses workspace-managed dependencies in `Cargo.toml`; do not add crate-local versions unless needed.
- Python harnesses are typed enough for basedpyright `standard`; do not weaken checks to hide real issues.
- Validate Python file changes with Ruff and basedpyright (`uv run ruff format --check scripts`, `uv run ruff check scripts`, and `uv run basedpyright`).
- Avoid speculative semantics in UI fallback code. Real bounds and blunt roles are better than fake widgets.
- Keep examples and docs professional. No persona, no private shorthand, no transcript paste.
- Preserve executable wrapper contracts in `bin/` and `.mcp.json`.
- Branch names normally use `bex/` unless the user asks otherwise.

## Document Hierarchy

The repository uses one curated layout for project knowledge. Do not invent
parallel structures.

- `README.md` — onboarding, quickstart, current state at a glance.
- `ROADMAP.md` — phased checklist of active workstreams. Links to feature docs
  and active ExecPlans. Slow-changing and curated.
- `CONTINUITY.md` — live working snapshot for the current session: goal,
  constraints, current state, working set, next step, open questions.
  Fast-changing, not a permanent record. Trim before adding more if it grows
  past ~80 lines.
- `NOTES.md` — durable tactical memory: proven commands, pitfalls, patterns,
  invariants, environment quirks. Not transcripts, not stale TODO lists, not
  artifact dumps. Trim before adding more if it grows past ~150 lines.
- `docs/` — durable project knowledge. Subdivided by purpose:
  - `docs/runtime/` — stable runtime contracts (MCP boundary, architecture).
  - `docs/features/` — descriptive docs for shipped features. One per
    subsystem, written from the feature doc template in `docs/AGENTS.md`.
  - `docs/operations/` — operator-facing harness, runbook, and procedure docs.
  - `docs/research/` — dated research findings (`YYYY-MM-<slug>.md`).
    Self-contained, not living documents.
- `plans/` — active forward-looking ExecPlans only. See `plans/AGENTS.md`
  for the lifecycle rule. Should be empty or near-empty when the team is
  caught up.

Do not create new top-level documentation directories. Do not add `goals/`,
`specs/`, `prds/`, `rfcs/`, or similar parallel hierarchies; they fragment the
knowledge tree.

## File Naming

- Use human-readable slugs in lowercase with underscores or hyphens, e.g.
  `linux_virtual_input.md`, `agent-cursor-overlay.md`.
- Do not use timestamp-prefixed or auto-generated names like
  `1778463694899-nimble-knight.md`.
- Research files are dated: `docs/research/YYYY-MM-<slug>.md`.

## Security & Secrets

- Never commit tokens, auth files, portal restore tokens, screenshots with sensitive UI, or live request payloads.
- Runtime secrets belong in local Codex config, environment variables, or ignored artifact homes.
- Treat `artifacts/**`, `dist/**`, `.venv/**`, and `target/**` as generated local state.
- Portal token state is per-user state, not repo material.

## JIT Index

### Package Structure

- Rust workspace: `crates/` -> [crates/AGENTS.md](crates/AGENTS.md)
- Shared Rust model/contracts: `crates/sky-cua-platform/` -> [crates/sky-cua-platform/AGENTS.md](crates/sky-cua-platform/AGENTS.md)
- Linux backend: `crates/sky-cua-linux/` -> [crates/sky-cua-linux/AGENTS.md](crates/sky-cua-linux/AGENTS.md)
- Long-lived daemon: `crates/sky-cua-service/` -> [crates/sky-cua-service/AGENTS.md](crates/sky-cua-service/AGENTS.md)
- MCP client: `crates/sky-cua-client/` -> [crates/sky-cua-client/AGENTS.md](crates/sky-cua-client/AGENTS.md)
- Python harnesses: `scripts/` -> [scripts/AGENTS.md](scripts/AGENTS.md)
- App-specific guidance: `resources/app-instructions/` -> [resources/app-instructions/AGENTS.md](resources/app-instructions/AGENTS.md)
- Bundled computer-use skill: `skills/computer-use/` -> [skills/computer-use/AGENTS.md](skills/computer-use/AGENTS.md)
- Bundled browser-use skill: `skills/browser-use/` -> [skills/browser-use/AGENTS.md](skills/browser-use/AGENTS.md)
- Local VM smoke skill: `.agents/skills/vm-tests/` -> [.agents/skills/vm-tests/SKILL.md](.agents/skills/vm-tests/SKILL.md)
- Docs: `docs/` -> [docs/AGENTS.md](docs/AGENTS.md)
  - Runtime contracts: `docs/runtime/`
  - Feature docs: `docs/features/`
  - Operator docs: `docs/operations/`
  - Research extracts: `docs/research/`
- Plans: `plans/` -> [plans/AGENTS.md](plans/AGENTS.md)
- Roadmap: `ROADMAP.md`

### Quick Find Commands

- Find Rust symbols: `rg -n "struct|enum|trait|impl|fn name" crates`
- Find tool definitions: `rg -n "tool_definitions|handle_tool_call|tools/list" crates/sky-cua-client/src`
- Find service IPC paths: `rg -n "ServiceRequest|ServiceResponse|service_socket" crates`
- Find backend routing: `rg -n "execute_action|ActionName|route_action" crates`
- Find diagnostics: `rg -n "PortalApprovalPending|CaptureBackendDowngraded|DiagnosticEntry" crates scripts`
- Find Python harness commands: `rg -n "subprocess|codex|app-server|exec" scripts`
- Find VM smoke workflow guidance: `rg -n "run_gui_testing_vm_smoke|testing-vm|wayland-display|desktop-env" .agents/skills docs scripts`
- Find app guidance entries: `rg -n "set_value_fallback|aliases|entries" resources/app-instructions`
- Find tests: `rg -n "#\\[test\\]|tokio::test|def test_" crates scripts`

## Definition of Done

- Run the narrowest relevant crate/package check first, then the root check if shared contracts changed.
- For Rust runtime changes: `cargo fmt --check && cargo test`.
- For Python harness changes: `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest`.
- For packaging changes: `python3 scripts/build_plugin.py` and inspect the staged bundle shape.
- State any live-smoke gates not run, especially desktop/portal/KDE/COSMIC/Hyprland/GNOME flows.
- When the user says "run all the smoke tests", run the `all` profile (`python3 scripts/run_gui_testing_vm_smoke.py --profile all`). This exercises every desktop environment and agent harness in sequence. Do not run a subset unless explicitly asked.
- For shipped features, update or create `docs/features/<slug>.md` and the
  matching `ROADMAP.md` entry. See `docs/AGENTS.md` for the feature doc
  template.
- For retired ExecPlans, follow the lifecycle rule in `plans/AGENTS.md`.
