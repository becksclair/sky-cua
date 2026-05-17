# CONTINUITY

## Goal

Land the documentation and knowledge-tree consolidation
(`plans/docs_and_plans_cleanup.md`) and keep the cleanup contract
enforced for future work.

## Constraints

- Documentation only; no runtime or test code changes.
- Each phase committable independently so the work can pause cleanly.
- Honor the new lifecycle: when an ExecPlan reaches "code complete",
  retire it into `docs/features/` plus `docs/research/` and update
  `ROADMAP.md`.

## Current State

- Cleanup branch is `bex/docs-and-plans-cleanup`.
- Layered layout is in place: `ROADMAP.md` at the root,
  `docs/runtime/`, `docs/features/`, `docs/operations/`,
  `docs/research/`, and `plans/` for active forward-looking design only.
- All previously shipped Linux plans are retired into feature docs and
  research extracts; `goals/` is removed. Active plans now in
  `plans/`: [`docs_and_plans_cleanup.md`](plans/docs_and_plans_cleanup.md),
  [`cdul_linux_enhancements.md`](plans/cdul_linux_enhancements.md),
  [`wayland_fallback_vision_anchors.md`](plans/wayland_fallback_vision_anchors.md),
  [`windows_capture_ladder.md`](plans/windows_capture_ladder.md), and
  [`windows_app_shell_smokes.md`](plans/windows_app_shell_smokes.md).
- AGENTS contract updated at root, `docs/`, and `plans/`. Naming rule
  bans timestamp-prefixed and auto-generated filenames.

## Working Set

- ExecPlan: `plans/docs_and_plans_cleanup.md`
- Roadmap: `ROADMAP.md`
- Feature docs: `docs/features/`
- Research extracts: `docs/research/`
- Active plans: `plans/`
- AGENTS contract: `AGENTS.md`, `docs/AGENTS.md`, `plans/AGENTS.md`

## Next Step

Phase 9 of `plans/docs_and_plans_cleanup.md`: cross-link validation
and run the project's pre-PR checks (`cargo fmt --check`,
`cargo test`, `uv run ruff format --check scripts`,
`uv run ruff check scripts`, `uv run basedpyright`, `uv run pytest`,
`python3 scripts/build_plugin.py`). No code changed, so all should
pass; verify there is no incidental breakage from path moves.

## Open Questions

- Whether to archive retired ExecPlans under `archive/plans/` for
  searchable history or rely on git log alone. Currently relying on
  git log; revisit only if grepping retired plans becomes a real
  need.
- Whether the agent cursor overlay feature doc should be split if
  visible-overlay and synthetic-screenshot-cursor paths grow further
  divergent. Currently combined; revisit if the doc passes ~200 lines.
