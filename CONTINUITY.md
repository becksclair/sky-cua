# CONTINUITY

## Goal

Land the documentation and knowledge-tree consolidation and keep the
cleanup contract enforced for future work.

## Constraints

- Documentation only; no runtime or test code changes.
- Each phase committable independently so the work can pause cleanly.
- Honor the new lifecycle: when an ExecPlan reaches "code complete",
  retire it into `docs/features/` plus `docs/research/` and update
  `ROADMAP.md`.

## Current State

- Documentation and knowledge-tree cleanup landed on
  `bex/docs-and-plans-cleanup` with one commit per phase boundary.
  Original ExecPlan retired per the lifecycle rule; see git history
  for the per-phase record.
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
- AGENTS contract updated at root, `docs/`, and `plans/`. Naming rule
  bans timestamp-prefixed and auto-generated filenames; lifecycle rule
  requires retiring an ExecPlan into a feature doc plus research
  extract plus a ROADMAP update when it reaches "code complete", then
  deleting the plan (git history is the canonical archive).
- Pre-PR checks (`cargo fmt --check`, `cargo test`, `uv run ruff check`,
  `uv run basedpyright`, `uv run pytest`, `python3 scripts/build_plugin.py`)
  all clean. All Markdown links in the cleaned tree resolve.

## Working Set

- Roadmap: `ROADMAP.md`
- Feature docs: `docs/features/`
- Research extracts: `docs/research/`
- Active plans: `plans/`
- AGENTS contract: `AGENTS.md`, `docs/AGENTS.md`, `plans/AGENTS.md`

## Next Step

Review the cleanup branch and merge to `main` when satisfied, or
provide feedback on what to adjust. The branch is ready for review.

## Open Questions

None blocking. The preexisting `scripts/live_desktop_smoke.py` ruff
format drift will be addressed as a separate small change on its own
branch.
