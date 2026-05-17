# Broader Windows app-shell live smokes

This ExecPlan is a living document. Keep `Progress`, `Surprises &
Discoveries`, `Decision Log`, and `Outcomes & Retrospective` current as
work proceeds.

## Purpose / Big Picture

After this work, the Windows backend has live-smoke coverage for the
real app-shell flows that exercise UIA inspection, semantic actions,
and capture diagnostics across more than just Microsoft Edge. The first
shipped UIA work proved Edge's address bar, tab switching, and Settings
menu via UIA, and recorded that Sumwall Browser was minimized /
off-screen with only a root UIA node. This plan completes the
Sumwall path (or any equivalent native-controls browser app) and adds a
small matrix of additional native Windows apps so coverage is not
Edge-only.

## Progress

- [x] (2026-05-08) Edge live smoke: address bar via `ValuePattern`, tab
  switch and restore via UIA, Settings menu activation via the widened
  UIA click path.
- [ ] Sumwall Browser live smoke that does not depend on a minimized /
  off-screen window state.
- [ ] At least one additional native-controls Windows app exercised
  through `focus_element`, `activate_element`, `select_element`,
  `expand_element`, `collapse_element`, and `toggle_element`.
- [ ] Document the smoke harness and acceptance criteria in
  `docs/operations/` so future Windows runs are reproducible.

## Surprises & Discoveries

- Sumwall Browser was observable but reported minimized / off-screen in
  the original installed-cache MCP smoke, with only a root UIA node and
  a blank GDI capture diagnostic. Whether this is a Sumwall window-
  state issue, a missing accessibility-friendly launch flag, or
  something the UIA backend should handle differently is an open
  question.
- The browser-app capture problem is partly orthogonal to live smokes;
  see `plans/windows_capture_ladder.md` for the capture-lane work.

## Decision Log

- Decision: Live smokes for Sumwall and broader app coverage are scoped
  separately from the capture ladder.
  Rationale: smoke coverage can land on top of the existing GDI lane
  with the blank-frame diagnostic; capture lane work should not block
  smoke matrix expansion.
  Date/Author: 2026-05-17 / Codex (extracted from the original goal
  package).

- Decision: Do not kill, relaunch, or mutate persistent browser
  profiles for Edge, Sumwall, or any user app outside an explicit smoke
  harness.
  Rationale: protect user data. Stop and ask before changing startup
  flags for user-owned browser processes.
  Date/Author: 2026-05-17 / Codex (carried forward from
  `goals/windows-app-automation/blockers.md`).

## Outcomes & Retrospective

Pending implementation. At completion, record:

- Sumwall launch path (disposable profile, accessibility flags, etc.).
- Which additional native apps were exercised.
- Whether the existing semantic action lanes were sufficient or
  whether new patterns had to be wired.

## Context and Orientation

The Windows UIA backend, semantic action routing, and the canonical
semantic action tool set are all shipped and described in
`docs/features/windows-uia-automation.md`. The blank-frame diagnostic
fires today when GDI capture returns a black image, so smokes must
treat that as evidence of a known limitation rather than a regression
in the UIA path.

The Windows release plugin install path is
`%CODEX_HOME%/plugins/cache/sky-cua-local/sky-cua/<version>/`, set up
by `scripts/deploy_release_plugin.py --no-build` after
`scripts/build_plugin.py`. The first UIA smoke ran from that installed
cache via a direct stdio probe.

Sumwall app-shell launch question: can Sumwall be launched in a
disposable smoke profile with accessibility-friendly flags, without
touching the user's active browsing state? Investigate before
implementing the Sumwall smoke.

## Plan of Work

1. Resolve the Sumwall launch question. If a disposable profile and
   accessibility-friendly flags are available, document them in this
   plan and use them in the smoke. If not, record the blocker.
2. Add a Windows live-smoke harness under `scripts/` that drives one
   target app per run, uses the installed plugin cache, and produces a
   timestamped artifact directory under `artifacts/windows-app-shell/`.
3. Add per-app smoke variants: Edge, Sumwall, and at least one
   additional native-controls app (Notepad, File Explorer, Settings, or
   similar).
4. Each smoke proves: window selection, at least one focus / activate /
   select action via UIA, a `set_value` action where applicable, and
   honest reporting of capture state.
5. Update `docs/operations/` with the smoke harness invocation and the
   artifact expectations.

## Validation and Acceptance

- A Sumwall smoke completes either with full UIA coverage or with a
  documented launch / state blocker that is not "the window is
  minimized."
- At least one additional native Windows app smoke completes with the
  full canonical semantic action set exercised.
- The smoke harness honors the blockers.md rule: no destructive
  mutation of persistent browser profiles outside the smoke.
- `cargo +nightly fmt --check`, `cargo test`, and the Python harness
  checks (`uv run ruff format --check scripts`, `uv run ruff check
  scripts`, `uv run basedpyright`, `uv run pytest`) pass on the
  Windows host.

## Idempotence and Recovery

The smokes use disposable profiles where possible and avoid mutating
persistent state. Each run produces a new timestamped artifact
directory. If a smoke leaves behind processes (orphan browser, stale
plugin host), the harness must clean them up before the next run.

## Interfaces and Dependencies

- `crates/sky-cua-client/src/mcp_server.rs` — canonical semantic action
  tool set used by the smokes.
- `scripts/build_plugin.py` and `scripts/deploy_release_plugin.py` —
  installed-plugin cache used by the smokes.
- `docs/operations/` — destination for the harness documentation.
- No new public model fields are expected; if the smokes uncover the
  need for new diagnostics, route them through the existing
  `DiagnosticEntry` shape.

## Revision Notes

- 2026-05-17 / Codex: Extracted from `goals/windows-app-automation/`
  (Plan slice 4, "Add app-shell live smokes") during the documentation
  cleanup. The Edge slice of that work shipped with the UIA milestone
  and is now in `docs/features/windows-uia-automation.md`. This plan
  covers only the broader live-smoke matrix.
