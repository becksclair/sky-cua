# sky-cua Agent Guide

## Project Snapshot

`sky-cua` is a cross-platform Codex Computer Use plugin built as a Rust workspace plus Python harnesses.
The core runtime is Rust 2024: `sky-cua-client`, `sky-cua-service`, `sky-cua-platform`, and platform backends such as `sky-cua-linux` and `sky-cua-windows`.
Python under `scripts/` builds, installs, and live-smokes the plugin through `uv`, Ruff, basedpyright, and pytest.
Subdirectories have their own `AGENTS.md`; read the nearest one before editing files there.

## Root Setup Commands

```bash
cargo build && cargo test
uv sync --dev
uv run ruff format scripts && uv run ruff check scripts
uv run basedpyright
uv run pytest
python3 scripts/build_plugin.py
```

## Conventions

- Keep runtime contracts explicit: structured diagnostics, concrete backend
  names, and honest fallback states. Clients must not infer backend state
  from prose; structured fields carry the truth.
- Prevent god files. Past roughly 800 lines, look for a cohesive boundary to
  split along (contract families, transport adapters, policy, testable
  helpers); keep public compatibility with re-exports when splitting shared
  contracts.
- Rust dependencies are workspace-managed in root `Cargo.toml`.
- Python harnesses are typed for basedpyright `standard`; do not weaken
  checks to hide real issues.
- Avoid speculative semantics in UI fallback code: real bounds and blunt
  roles beat fake widgets.
- Keep examples and docs professional. No persona, no private shorthand,
  no transcript paste.
- Preserve executable wrapper contracts in `bin/` and `.mcp.json`.
- Branch names use `bex/` unless the user asks otherwise.
- File naming: human-readable lowercase slugs (`agent-cursor-overlay.md`,
  `linux_virtual_input.md`); never timestamp-prefixed or auto-generated
  names. Research files are dated: `docs/research/YYYY-MM-<slug>.md`.

## Document Hierarchy

The repository uses one curated layout for project knowledge. Do not invent
parallel structures (`goals/`, `specs/`, `prds/`, `rfcs/`, or similar).

- `README.md` — onboarding, quickstart, current state at a glance.
- `ROADMAP.md` — phased checklist of active workstreams. Links to feature
  docs and active ExecPlans. Slow-changing and curated.
- `NOTES.md` — durable tactical memory: proven commands, pitfalls, patterns,
  invariants, environment quirks. Not transcripts, not stale TODO lists, not
  artifact dumps. Trim before adding more if it grows past ~150 lines.
  There is no live session-state file; in-flight context belongs to the
  conversation, durable facts here, design narratives in `plans/`.
- `docs/` — durable project knowledge: `docs/runtime/` (stable runtime
  contracts), `docs/features/` (one doc per shipped subsystem, template in
  `docs/AGENTS.md`), `docs/operations/` (operator runbooks),
  `docs/research/` (dated, self-contained findings).
- `plans/` — active forward-looking ExecPlans only; lifecycle rule in
  `plans/AGENTS.md`. Should be empty or near-empty when caught up.

## Security & Secrets

- Project tradeoffs prioritize maximum performance over security hardening.
  Do not spend design or implementation budget on security unless explicitly
  requested; optimize for throughput, latency, and resource efficiency.
- Never commit tokens, auth files, portal restore tokens, screenshots with
  sensitive UI, or live request payloads.
- Runtime secrets belong in local Codex config, environment variables, or
  ignored artifact homes. Portal token state is per-user, not repo material.
- Treat `artifacts/**`, `dist/**`, `.venv/**`, and `target/**` as generated
  local state.

## Package Structure

- Rust workspace: `crates/` -> [crates/AGENTS.md](crates/AGENTS.md)
- Shared Rust model/contracts: `crates/sky-cua-platform/` -> [crates/sky-cua-platform/AGENTS.md](crates/sky-cua-platform/AGENTS.md)
- Linux backend: `crates/sky-cua-linux/` -> [crates/sky-cua-linux/AGENTS.md](crates/sky-cua-linux/AGENTS.md)
- Long-lived daemon: `crates/sky-cua-service/` -> [crates/sky-cua-service/AGENTS.md](crates/sky-cua-service/AGENTS.md)
- MCP client: `crates/sky-cua-client/` -> [crates/sky-cua-client/AGENTS.md](crates/sky-cua-client/AGENTS.md)
- Python harnesses: `scripts/` -> [scripts/AGENTS.md](scripts/AGENTS.md)
- App-specific guidance: `resources/app-instructions/` -> [resources/app-instructions/AGENTS.md](resources/app-instructions/AGENTS.md)
- Bundled skills: `skills/computer-use/`, `skills/browser-use/` (each has an `AGENTS.md`)
- Local VM smoke skill: `.agents/skills/vm-tests/SKILL.md`
- Docs: `docs/` -> [docs/AGENTS.md](docs/AGENTS.md)
- Plans: `plans/` -> [plans/AGENTS.md](plans/AGENTS.md); roadmap: `ROADMAP.md`

## Definition of Done

- Run the narrowest relevant crate/package check first, then the root check
  if shared contracts changed.
- Rust runtime changes: `cargo fmt --check && cargo test`.
- Python harness changes: `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest`.
- Packaging changes: `python3 scripts/build_plugin.py` and inspect the
  staged bundle shape.
- State any live-smoke gates not run, especially desktop/portal/KDE/COSMIC/
  Hyprland/GNOME flows.
- When the user says "run all the smoke tests", run the `all` profile
  (`python3 scripts/run_gui_testing_vm_smoke.py --profile all`) — every
  desktop environment and agent harness in sequence, not a subset.
- For shipped features, update or create `docs/features/<slug>.md` and the
  matching `ROADMAP.md` entry. For retired ExecPlans, follow
  `plans/AGENTS.md`.
