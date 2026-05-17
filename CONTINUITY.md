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

- Cleanup branch is `bex/docs-and-plans-cleanup`; all nine phases of
  [`plans/docs_and_plans_cleanup.md`](plans/docs_and_plans_cleanup.md)
  landed with one commit per phase boundary.
- Layered layout is in place: `ROADMAP.md` at the root,
  `docs/runtime/`, `docs/features/`, `docs/operations/`,
  `docs/research/`, and `plans/` for active forward-looking design only.
- Eight feature docs, eight research extracts, one runtime
  architecture doc, and a populated ROADMAP all in place. `goals/`
  removed entirely. Active plans now in `plans/`:
  [`cdul_linux_enhancements.md`](plans/cdul_linux_enhancements.md),
  [`wayland_fallback_vision_anchors.md`](plans/wayland_fallback_vision_anchors.md),
  [`windows_capture_ladder.md`](plans/windows_capture_ladder.md), and
  [`windows_app_shell_smokes.md`](plans/windows_app_shell_smokes.md).
  The cleanup plan itself remains in `plans/` only as a record of how
  the cleanup was done; it can be retired into `archive/plans/` or
  deleted at any time.
- AGENTS contract updated at root, `docs/`, and `plans/`. Naming rule
  bans timestamp-prefixed and auto-generated filenames; lifecycle rule
  requires retiring an ExecPlan into a feature doc plus research
  extract plus a ROADMAP update when it reaches "code complete".
- Pre-PR checks (`cargo fmt --check`, `cargo test`, `uv run ruff check`,
  `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`)
  all clean. All 79 Markdown links in the cleaned tree resolve.

## Working Set

- ExecPlan: `plans/docs_and_plans_cleanup.md`
- Roadmap: `ROADMAP.md`
- Feature docs: `docs/features/`
- Research extracts: `docs/research/`
- Active plans: `plans/`
- AGENTS contract: `AGENTS.md`, `docs/AGENTS.md`, `plans/AGENTS.md`

## Next Step

Review the cleanup branch and merge to `main` when satisfied, or
provide feedback on what to adjust. The branch is ready for review.

## Open Questions

- Whether to archive retired ExecPlans under `archive/plans/` for
  searchable history or rely on git log alone. Currently relying on
  git log; revisit only if grepping retired plans becomes a real
  need.
- Whether the agent cursor overlay feature doc should be split if
  visible-overlay and synthetic-screenshot-cursor paths grow further
  divergent. Currently combined; revisit if the doc passes ~200 lines.
- Whether to address the preexisting `scripts/live_desktop_smoke.py`
  ruff format drift in a separate small change. It was present on
  `main` before this cleanup and is unrelated, but it does flag in
  the pre-PR checks.
