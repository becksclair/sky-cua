# sky-cua Agent Guide

## Project Snapshot

`sky-cua` is a Linux-first Codex Computer Use plugin built as a Rust workspace plus Python harnesses.
The core runtime is Rust 2024: `sky-cua-client`, `sky-cua-service`, `sky-cua-linux`, and `sky-cua-platform`.
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
- Rust uses workspace-managed dependencies in `Cargo.toml`; do not add crate-local versions unless needed.
- Python harnesses are typed enough for basedpyright `standard`; do not weaken checks to hide real issues.
- Avoid speculative semantics in UI fallback code. Real bounds and blunt roles are better than fake widgets.
- Keep examples and docs professional. No persona, no private shorthand, no transcript paste.
- Preserve executable wrapper contracts in `bin/` and `.mcp.json`.
- Branch names normally use `bex/` unless the user asks otherwise.

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
- Bundled workflow skill: `skills/computer-use-workflows/` -> [skills/computer-use-workflows/AGENTS.md](skills/computer-use-workflows/AGENTS.md)
- Docs: `docs/` -> [docs/AGENTS.md](docs/AGENTS.md)
- Plans: `plans/` -> [plans/AGENTS.md](plans/AGENTS.md)

### Quick Find Commands

- Find Rust symbols: `rg -n "struct|enum|trait|impl|fn name" crates`
- Find tool definitions: `rg -n "tool_definitions|handle_tool_call|tools/list" crates/sky-cua-client/src`
- Find service IPC paths: `rg -n "ServiceRequest|ServiceResponse|service_socket" crates`
- Find backend routing: `rg -n "execute_action|ActionName|route_action" crates`
- Find diagnostics: `rg -n "PortalApprovalPending|CaptureBackendDowngraded|DiagnosticEntry" crates scripts`
- Find Python harness commands: `rg -n "subprocess|codex|app-server|exec" scripts`
- Find app guidance entries: `rg -n "set_value_fallback|aliases|entries" resources/app-instructions`
- Find tests: `rg -n "#\\[test\\]|tokio::test|def test_" crates scripts`

## Definition of Done

- Run the narrowest relevant crate/package check first, then the root check if shared contracts changed.
- For Rust runtime changes: `cargo fmt --check && cargo test`.
- For Python harness changes: `uv run ruff format --check scripts && uv run ruff check scripts && uv run basedpyright && uv run pytest`.
- For packaging changes: `python3 scripts/build_plugin.py` and inspect the staged bundle shape.
- State any live-smoke gates not run, especially desktop/portal/KDE/TIDAL flows.
