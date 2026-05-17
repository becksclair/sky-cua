# Documentation and Knowledge Tree Consolidation

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`,
`Decision Log`, and `Outcomes & Retrospective` current as work proceeds.

This document must be maintained in accordance with `~/.codex/PLANS.md`
(or `~/.agents/PLANS.md` when running under Devin). It is self-contained on
purpose: a future contributor should be able to start with this file, the
current repository, and the commands below, without needing the chat that
created the plan.

## Purpose / Big Picture

After this work, project knowledge in `sky-cua` lives in one curated layout
instead of five overlapping ones. A contributor or agent will have a single
top-level index (`ROADMAP.md`) that names every active workstream, links each
shipped subsystem to a descriptive feature doc, and links each in-flight piece
of work to one ExecPlan. Durable behavior lives in `docs/`, dated research
findings live in `docs/research/`, live session state stays compact in
`CONTINUITY.md`, and `NOTES.md` is once again durable tactical memory rather
than an artifact ledger.

The goal is not cosmetic cleanup. The current sprawl — `plans/` accumulating
postmortems, the vestigial `goals/` Plannotator package, two parallel "active
plan" formats, embedded research buried inside ExecPlan Decision Logs, and
`CONTINUITY.md`/`NOTES.md` both at ~48 KB — already makes it hard to answer
"what is the project doing next, and where does that decision live?" Without
this consolidation, the next round of features will fragment further.

## Progress

- [x] (2026-05-17) Surveyed the current state of `docs/`, `plans/`, `goals/`,
  `CONTINUITY.md`, `NOTES.md`, and root onboarding. Confirmed five distinct
  planning shapes coexist and that most plans in `plans/` are postmortems.
- [x] (2026-05-17) Settled the high-level design: layered structure, single
  `ROADMAP.md` as phased checklist, `docs/features/` for shipped feature docs,
  `docs/research/` for dated research, `goals/` retired, ExecPlans as the
  single forward-looking format, "feature docs" rather than PRD/FTDD as the
  label.
- [x] (2026-05-17) Phase 1: Locked the contract — AGENTS.md rule updates at
  root, `docs/`, and `plans/`, including Document Hierarchy, File Naming,
  Feature Doc Template, Research Doc Scope, Pre-PR Checks, and Lifecycle.
- [x] (2026-05-17) Phase 2: Created the skeleton — `ROADMAP.md`,
  `docs/features/`, `docs/research/`, `docs/runtime/`, `docs/operations/`.
  Moved existing `docs/` files into the new subdirectories and updated
  cross-references.
- [x] (2026-05-17) Phase 3: Pilot extraction — `plans/native_agent_cursor_overlay.md`
  retired into `docs/features/agent-cursor-overlay.md` plus
  `docs/research/2026-05-kwin-effect-discovery.md` and
  `docs/research/2026-05-x11-shaped-window-vs-layer-shell.md`.
- [x] (2026-05-17) Phase 4: Migrated five shipped plans into feature docs
  and extracted three research findings. CDUL comparison research extracted
  while the forward-looking CDUL plan stayed active.
- [x] (2026-05-17) Phase 5: Dismantled `goals/windows-app-automation/` into
  `docs/features/windows-uia-automation.md`,
  `plans/windows_capture_ladder.md`, `plans/windows_app_shell_smokes.md`,
  and `docs/research/2026-05-windows-uia-investigation.md`. Deleted the
  `goals/` directory.
- [x] (2026-05-17) Phase 6: Refactored `docs/computer-use-linux-plan.md`
  into `docs/runtime/linux-architecture.md` and
  `docs/research/2026-04-pipewire-vs-screenshot-portal.md`. Distilled
  `plans/1778463694899-nimble-knight.md` into ROADMAP entries plus a new
  `docs/features/codex-desktop-compat.md`, then deleted the plan.
- [x] (2026-05-17) Phase 7: Trimmed `CONTINUITY.md` from 284 lines and
  ~48 KB to ~60 lines under the canonical session-snapshot contract.
- [x] (2026-05-17) Phase 8: Trimmed `NOTES.md` from 201 lines of one flat
  list to ~190 lines under eight named section headers, with artifact
  paths routed into feature doc Verification sections and resolved
  investigations collapsed to research extract pointers.
- [x] (2026-05-17) Phase 9: Cross-link validation — all 79 Markdown
  `[text](path)` links in `ROADMAP.md`, the four `docs/` subdirectories,
  all `plans/`, root AGENTS.md, CONTINUITY.md, NOTES.md, and README.md
  resolve. Pre-PR checks: `cargo fmt --check`, `cargo test` (321 tests
  passed), `uv run ruff check scripts`, `uv run basedpyright`,
  `uv run pytest` (113 tests passed), `python3 scripts/build_plugin.py`
  all clean. The preexisting ruff-format issue with
  `scripts/live_desktop_smoke.py` was present on `main` before this
  cleanup and is unrelated.

## Surprises & Discoveries

- The current `plans/AGENTS.md` already says "Move durable, completed
  investigation narrative into `../docs/` when it stops being a plan" and
  "DON'T: Let plans become stale TODO dumps; update or delete them when code
  lands." The bug is procedural, not structural: the rule exists but is not
  enforced. Five of the eight files in `plans/` are explicitly marked "code
  complete" or carry 60+ checked progress entries against fully shipped work.
- Two distinct active-plan formats coexist without a rule for when to use
  which: the ExecPlan format (Progress / Surprises / Decision Log) and the
  shorter "Plan: …" status-ledger format with timestamped Codex auto-names
  like `1778463694899-nimble-knight.md`. The latter format is just a
  postmortem and should not have a separate identity.
- The `goals/` directory is a Plannotator artifact, not part of the project's
  preferred workflow. It contains exactly one occupant
  (`goals/windows-app-automation/`) which is partly shipped and partly open.
- `docs/computer-use-linux-plan.md` is structurally an ExecPlan that landed
  in `docs/`. It is half durable architecture doc, half historical Progress
  ledger, and confuses both audiences.
- `CONTINUITY.md` (284 lines, ~48 KB) is acting as a permanent project ledger
  rather than a session snapshot. Its `Current State` section alone is ~255
  lines, with content from many sessions and many shipped features. The home
  `AGENTS.md` rule says it should be "short, current, and professional."
- `NOTES.md` (201 lines, ~48 KB) is one flat unstructured list mixing genuine
  durable tactical memory, specific run artifact paths, and resolved Codex
  plugin debugging investigations. The home `AGENTS.md` rule says it is for
  "proven commands, pitfalls, patterns, invariants" and "Do not store
  transcripts, speculation, or stale TODO lists."

## Decision Log

- Decision: Do not consolidate into a single master ExecPlan / master spec.
  Rationale: ExecPlan format does not scale to project scope (`Progress`
  becomes unbounded; `Decision Log` mixes seam-level and product-level
  decisions). The product is a runtime contract, already documented in
  `docs/mcp-runtime.md`. The existing layered shape is correct; what's
  missing is one curated top-level index plus retirement discipline.
  Date/Author: 2026-05-17 / Codex

- Decision: Add a single `ROADMAP.md` at the repo root as the curated phased
  checklist. No separate `TODO.md`. A "Backlog / Ideas" section lives at the
  bottom of `ROADMAP.md`.
  Rationale: One index, one place to look. A second tracker just splits
  attention.
  Date/Author: 2026-05-17 / Codex

- Decision: Call descriptive, backward-looking documents "feature docs", not
  PRDs and not FTDDs. They live in `docs/features/<slug>.md` under a fixed
  template (Status, Summary, Contract surface, Behavior, Source paths,
  Verification, Known limitations, Related).
  Rationale: PRD and FTDD both imply forward-looking design intent. Feature
  docs are descriptive of current behavior. Acronyms add jargon without
  precision; location plus template communicates the intent cleanly.
  Date/Author: 2026-05-17 / Codex

- Decision: Retire `goals/`. ExecPlans in `plans/` are the single
  forward-looking format. Forward-looking PRD-style content stays inside the
  ExecPlan rather than in a separate brief / verification / blockers
  package.
  Rationale: `goals/` is a vestigial Plannotator artifact. The user's working
  pattern is ExecPlans. One format reduces format-switching cost and removes
  the routing question of "which format does this work need?"
  Date/Author: 2026-05-17 / Codex

- Decision: Add `docs/research/YYYY-MM-<slug>.md` for dated research findings,
  with a scope rule: one research question per file, self-contained, not a
  living document, not for transcripts or raw artifact dumps.
  Rationale: Research is currently buried inside ExecPlan `Surprises &
  Discoveries` and `Decision Log` sections, which makes it invisible the
  moment the plan is retired. Extracting it preserves the durable findings
  separately from the seam-specific design narrative.
  Date/Author: 2026-05-17 / Codex

- Decision: Add an explicit lifecycle rule to `plans/AGENTS.md`. When an
  ExecPlan reaches "code complete" with live proof, it must be retired by
  creating a feature doc, extracting research, updating ROADMAP, and deleting
  the plan (or moving to `archive/plans/` if the long-form ledger has
  archaeological value beyond git history).
  Rationale: The "don't let plans become stale TODO dumps" intent is already
  in `plans/AGENTS.md` but not codified as a step-by-step retirement
  procedure. Without a procedure, decay continues.
  Date/Author: 2026-05-17 / Codex

- Decision: Trim `CONTINUITY.md` aggressively to its original contract
  (~30–60 lines target). Trim `NOTES.md` to durable tactical memory under
  named section headers (~80–120 lines target). Move artifact-path bullets
  into the relevant feature doc's `Verification` section, not into the new
  files.
  Rationale: Both files have decayed past their stated purpose. The signal
  for needing another trim is when `Current State` accumulates entries from
  more than one or two active workstreams.
  Date/Author: 2026-05-17 / Codex

## Outcomes & Retrospective

Complete. All nine phases landed in one branch (`bex/docs-and-plans-cleanup`)
with one commit per phase boundary.

Plans retired into feature docs:

- `plans/native_agent_cursor_overlay.md` → `docs/features/agent-cursor-overlay.md`
- `plans/compositor_cursor_hiding.md` → `docs/features/compositor-cursor-hiding.md`
- `plans/linux_virtual_input_backend.md` → `docs/features/linux-virtual-input.md`
- `plans/rich_atspi_readback.md` → `docs/features/atspi-rich-readback.md`
- `plans/detached_session_env_repair.md` → `docs/features/session-env-repair.md`
- `plans/1778571910929-proud-mountain.md` → `docs/features/kwin-x11-workspace-metadata.md`
- `plans/1778463694899-nimble-knight.md` → `docs/features/codex-desktop-compat.md` (plus ROADMAP entries for the rest)

Plans kept in `plans/` (active forward-looking):

- `plans/docs_and_plans_cleanup.md` — this plan
- `plans/cdul_linux_enhancements.md` — open CDUL adoption slices
- `plans/wayland_fallback_vision_anchors.md` — fallback anchor work
- `plans/windows_capture_ladder.md` — new, extracted from the Windows goal
- `plans/windows_app_shell_smokes.md` — new, extracted from the Windows goal

Research extracts landed (eight total):

- `docs/research/2026-04-codex-plugin-chatgpt-auth-expedition.md` (was in `docs/`)
- `docs/research/2026-04-pipewire-vs-screenshot-portal.md` (from `docs/computer-use-linux-plan.md`)
- `docs/research/2026-05-kwin-effect-discovery.md` (from native cursor overlay)
- `docs/research/2026-05-x11-shaped-window-vs-layer-shell.md` (from native cursor overlay)
- `docs/research/2026-05-cosmic-cursor-hiding-options.md` (from compositor cursor hiding)
- `docs/research/2026-05-ydotool-vs-direct-uinput.md` (from Linux virtual input)
- `docs/research/2026-05-cdul-comparison.md` (from CDUL plan)
- `docs/research/2026-05-windows-uia-investigation.md` (from the Windows goal)

Other shipped artifacts:

- `ROADMAP.md` populated with active workstreams across Linux desktop
  parity, Windows parity, host portability, performance, and operator UX
  phases plus a backlog.
- `docs/runtime/linux-architecture.md` as the durable Linux backend
  architecture description, replacing the half-architecture, half-ledger
  `docs/computer-use-linux-plan.md`.
- `goals/` directory removed entirely.
- `CONTINUITY.md` trimmed from 284 lines (~48 KB) to ~60 lines under
  the canonical Goal / Constraints / Current State / Working Set /
  Next Step / Open Questions sections.
- `NOTES.md` trimmed from 201 lines (~48 KB) of one flat list to ~190
  lines under eight section headers, with artifact paths routed into
  feature doc Verification sections.

Workstreams intentionally not given a ROADMAP entry:

- The cleanup work itself (this plan); it is the work that produced the
  ROADMAP, not a thing to track on it.
- Internal refactor / cleanup items that are not user-facing features.

Retrospective notes:

- The Markdown link validator was useful late in the process and caught
  several dead source-path references and retired-plan link pointers
  that I had written reflexively. Future cleanup-style migrations
  should run the link checker after each phase, not only at the end.
- The "Originating ExecPlan retired" line in feature docs originally
  linked to the retired plan path inside a backtick code span. Those
  paths no longer exist, so the phrasing was rewritten to "see git
  history for ..." to avoid grep-and-link confusion. Worth carrying
  into the feature doc template if this comes up again.

## Context and Orientation

`sky-cua` is a Rust workspace plus Python harnesses. Project-level
documentation today spans:

- `README.md`, `AGENTS.md` at the repo root.
- `docs/`: 6 files mixing durable runtime contracts (`mcp-runtime.md`),
  operator-facing harness docs (`gui-desktop-test-harness.md`), historical
  investigations (`codex-plugin-e2e-expedition.md`), an ExecPlan that
  landed in `docs/` (`computer-use-linux-plan.md`), and a small performance
  doc (`image-size-performance.md`).
- `plans/`: 8 plan files in two formats. Most are postmortems for shipped
  work (`native_agent_cursor_overlay.md`, `compositor_cursor_hiding.md`,
  `linux_virtual_input_backend.md`, `rich_atspi_readback.md`,
  `detached_session_env_repair.md`, `1778571910929-proud-mountain.md`).
  Active forward-looking content lives in `wayland_fallback_vision_anchors.md`
  and `cdul_linux_enhancements.md`.
- `goals/windows-app-automation/`: one Plannotator goal package
  (brief / plan / verification / blockers / goal-prompt / progress.jsonl)
  for partly-shipped Windows UIA work.
- `CONTINUITY.md`, `NOTES.md`: live state and tactical memory, both ~48 KB
  and well past their stated purpose.

The target shape after this plan completes is documented in the "Plan of
Work" section below. The contract for what each layer is for lives in the
new AGENTS.md rule text added in Phase 1.

## Plan of Work

The work is broken into nine phases. Each phase is independently committable
and the project remains coherent if the work pauses between phases.

### Phase 1 — Lock the contract

Update AGENTS.md files so the new layout is documented as the rule before any
content moves. Three files change:

- Root `AGENTS.md`: add a "Document Hierarchy" section naming `README.md`,
  `ROADMAP.md`, `CONTINUITY.md`, `NOTES.md`, and the four `docs/`
  subdirectories (`runtime/`, `features/`, `operations/`, `research/`).
  Add a "File Naming" rule banning timestamp-prefixed and auto-generated
  names.
- `docs/AGENTS.md`: replace "Patterns & Conventions" with the per-subdirectory
  purpose, the feature doc template, the research doc scope rule, and a
  "Pre-PR Checks" entry that requires updating feature doc + ROADMAP when a
  feature ships.
- `plans/AGENTS.md`: tighten "Patterns & Conventions" so it states plans are
  forward-looking only, and add a "Lifecycle" section with the retirement
  procedure.

No files outside AGENTS.md change in this phase.

### Phase 2 — Create the skeleton

Create empty placeholders so subsequent phases can populate without ordering
issues:

- `ROADMAP.md` at the repo root, populated with active workstream entries
  but with most checkboxes pointing at not-yet written feature doc paths.
- `docs/features/`, `docs/research/`, `docs/runtime/`, `docs/operations/` —
  create as empty directories with a placeholder `.gitkeep` if needed.

Move existing `docs/` files into their target subdirectories without
rewriting yet:

- `docs/mcp-runtime.md` → `docs/runtime/mcp-boundary.md`
- `docs/computer-use-linux-plan.md` → stays at `docs/` for Phase 6 to
  refactor
- `docs/gui-desktop-test-harness.md` → `docs/operations/gui-desktop-test-harness.md`
- `docs/codex-plugin-e2e-expedition.md` → `docs/research/2026-04-codex-plugin-chatgpt-auth-expedition.md`
- `docs/image-size-performance.md` → `docs/features/image-size-performance.md`
  (it already reads as a feature doc; verify and slot into the template
  during Phase 4)

Update internal cross-links and any `README.md` / `AGENTS.md` references to
moved paths in a single grep+rewrite pass.

### Phase 3 — Pilot extraction (agent cursor overlay)

Distil `plans/native_agent_cursor_overlay.md` into:

- `docs/features/agent-cursor-overlay.md` using the feature doc template.
- 1–2 research extracts into `docs/research/`. Candidates:
  `2026-05-kwin-effect-discovery.md` (user-level vs system install),
  `2026-05-x11-shaped-window-vs-layer-shell.md` (XWayland visible-overlay
  failure under portal capture, accepted real-X11 acceptance path).
- ROADMAP entry for the agent cursor overlay updated with `[x]` and a link
  to the new feature doc.

After this lands, the pilot is the template every later extraction copies
from.

### Phase 4 — Migrate the remaining implemented plans

For each plan, produce one feature doc plus any research extracts, then
delete the plan. Order by smallest first so the muscle memory builds:

- `plans/1778571910929-proud-mountain.md` → `docs/features/kwin-x11-workspace-metadata.md`
- `plans/rich_atspi_readback.md` → `docs/features/atspi-rich-readback.md`
- `plans/detached_session_env_repair.md` → `docs/features/session-env-repair.md`
- `plans/compositor_cursor_hiding.md` →
  `docs/features/compositor-cursor-hiding.md` plus
  `docs/research/2026-05-cosmic-cursor-hiding-options.md`
- `plans/linux_virtual_input_backend.md` →
  `docs/features/linux-virtual-input.md` plus
  `docs/research/2026-05-ydotool-vs-direct-uinput.md`

Active plans are not retired:

- `plans/wayland_fallback_vision_anchors.md` — keep, still forward-looking.
- `plans/cdul_linux_enhancements.md` — keep the unfinished slices, but split
  off a `docs/research/2026-05-cdul-comparison.md` for the comparison
  research that's already done.

### Phase 5 — Dismantle `goals/windows-app-automation/`

Split the goal package into the new shape:

- Shipped → `docs/features/windows-uia-automation.md`. Covers UIA inspection,
  UIA semantic actions (focus / activate / select / expand / collapse /
  toggle), GDI blank-frame diagnostics, release plugin install validation,
  Edge live smoke evidence with ValuePattern address bar, tab switching, and
  Settings menu activation.
- Open → ExecPlans in `plans/`:
  - `plans/windows_capture_ladder.md` for the WGC / DXGI capture upgrade
    above GDI.
  - `plans/windows_app_shell_smokes.md` for broader Edge / Sumwall live
    smoke coverage.
- Research → `docs/research/2026-05-windows-uia-investigation.md`. Captures
  the windows-sys vs typed windows crate question, the Edge GDI black-capture
  investigation, and the Sumwall accessibility-flag launch question.

After extraction, delete `goals/` outright (git history preserves the
original package).

### Phase 6 — Refactor the structural odd-ones

- `docs/computer-use-linux-plan.md`: split into
  `docs/runtime/linux-architecture.md` (durable architecture description, no
  Progress section, no Decision Log ledger) plus
  `docs/research/2026-04-pipewire-vs-screenshot-portal.md` if the capture
  lane investigation deserves its own research doc.
- `plans/1778463694899-nimble-knight.md`: distil its `Pending` block into
  ROADMAP entries. The rest is duplication of feature docs that just landed
  in Phase 4. Delete after distillation.

### Phase 7 — Trim `CONTINUITY.md`

Bring `CONTINUITY.md` back under its original contract (~30–60 lines target).

- `## Goal`: one sentence about the active goal of the current work.
- `## Constraints`: 0–3 items if any apply.
- `## Current State`: 5–15 bullets, current work only. When a slice
  completes, its bullets leave the section.
- `## Working Set`: files, commands, artifacts the next session needs to
  pick up cleanly.
- `## Next Step`: one focused next move.
- `## Open Questions`: only questions actively blocking the next step.

For each existing entry, classify and route:

- "Latest `<feature>` proof artifact: …" → that feature's doc
  `Verification` section (already covered by Phases 4–5), then delete
  from `CONTINUITY.md`.
- Multi-line milestone summaries → already covered by feature docs; delete.
- Image-size A/B and Codex plugin / app-server / ChatGPT-auth investigation
  prose → covered by `docs/features/image-size-performance.md` and
  `docs/research/2026-04-codex-plugin-chatgpt-auth-expedition.md`; delete.
- Open questions about retired features (TIDAL, nested-X11) → delete.
- Open questions about real future direction → ROADMAP backlog.
- Open questions that are research questions → `docs/research/` if worth
  keeping, otherwise close.

### Phase 8 — Trim `NOTES.md`

Bring `NOTES.md` back under its original contract (~80–120 lines target),
sectioned so the file is scannable.

Add the following section headers and route existing bullets into them:

- `## Environment quirks (host)`
- `## VM session management`
- `## Compositor and capture gotchas`
- `## Input adapters`
- `## AT-SPI and selectors`
- `## Plugin packaging and Codex loading`
- `## Smoke harnesses`
- `## Portal state`

Cull rules:

- Delete every bullet that says "Accepted ... artifact: `/workspace/...`".
  Artifact paths are evidence and belong in feature docs.
- Delete resolved investigations. Replace 20 codex-exec / app-server /
  ChatGPT-auth bullets with a single "Plugin loading recipe and historical
  investigation: see `docs/research/2026-04-codex-plugin-chatgpt-auth-expedition.md`".
- Delete feature-behavior summaries duplicated in feature docs. Keep only
  the gotcha that bit you.
- Keep what's actually reusable across sessions.

### Phase 9 — Cross-link, validate, lock

- Walk every `ROADMAP.md` entry and verify each link resolves to a real
  feature doc or active ExecPlan. Open boxes link to ExecPlans; closed boxes
  link to feature docs.
- Walk every feature doc and verify it links back to source paths and any
  research it consumed.
- Run repository link validation.
- Run the project's own pre-PR checks: `cargo fmt --check`, `cargo test`,
  `uv run ruff format --check scripts`, `uv run ruff check scripts`,
  `uv run basedpyright`, `uv run pytest`,
  `python3 scripts/build_plugin.py`. No code changed, so all should pass.

## Validation and Acceptance

Acceptance is structural and behavioral, not cosmetic.

- `goals/` is removed from the live tree.
- `plans/` contains only forward-looking ExecPlans. Every file has at least
  one unchecked progress item or an explicit forward-looking scope. No file
  is marked "code complete" without forward-looking work.
- `docs/features/` contains one document per shipped subsystem named in
  ROADMAP, each conforming to the template.
- `docs/research/` contains dated research extracts named
  `YYYY-MM-<slug>.md`. None are living documents.
- `ROADMAP.md` exists at the repo root. Every closed box links to a feature
  doc; every open box links to an ExecPlan.
- `CONTINUITY.md` is under ~80 lines and contains current-session content
  only. `NOTES.md` is under ~150 lines, sectioned.
- AGENTS.md files at root, `docs/`, and `plans/` reflect the new contract.
- Project pre-PR checks pass: `cargo fmt --check`, `cargo test`,
  `uv run ruff format --check scripts`, `uv run ruff check scripts`,
  `uv run basedpyright`, `uv run pytest`,
  `python3 scripts/build_plugin.py`.

## Idempotence and Recovery

Every phase is independently committable. If a phase is interrupted:

- Phase 1 (rules): partial AGENTS.md edits are safe to leave; the rest of
  the repo continues to work under the old shape.
- Phase 2 (skeleton): empty directories and moved files do not break
  anything that doesn't reference docs by path. Cross-link rewrite at end
  of phase closes the gap.
- Phases 3–6 (extractions): each plan retirement is an atomic move of one
  plan into one feature doc plus any research extracts plus a ROADMAP entry
  edit. If interrupted halfway, the remaining plans still render correctly
  in their original location.
- Phases 7–8 (trim): destructive on prose, but git history preserves the old
  versions.
- Phase 9 (validation): non-destructive.

## Interfaces and Dependencies

This work is documentation and process only. It does not change runtime
contracts, MCP tool surfaces, source code, or test code. The only
"interface" is the documentation contract:

- `docs/AGENTS.md` defines the feature doc template and research doc scope
  rule. New feature docs and research docs must conform to it.
- `plans/AGENTS.md` defines the ExecPlan lifecycle. Future plans must follow
  the retirement procedure.
- `ROADMAP.md` is the single index. New workstreams must add an entry; new
  shipped features must update an entry and link to the feature doc.

External dependencies: none beyond existing project tooling.

## Revision Notes

- 2026-05-17 / Codex: Initial ExecPlan created from a planning conversation
  that surveyed the current state of `docs/`, `plans/`, `goals/`,
  `CONTINUITY.md`, and `NOTES.md`, settled the consolidated layout, and
  drafted the AGENTS rule text and ROADMAP skeleton.
