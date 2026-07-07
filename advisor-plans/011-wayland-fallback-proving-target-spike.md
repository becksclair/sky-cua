# Plan 011 (spike): Choose and prove a live target for the Wayland fallback vision anchors

> **Executor instructions**: This is a validation spike: the deliverable is a
> chosen target app, a green live proof, and updated docs — minimal or no
> runtime code. Honor STOP conditions. When done, update the status row in
> `advisor-plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- plans/wayland_fallback_vision_anchors.md ROADMAP.md`
> On drift, re-read the ExecPlan before proceeding.

## Status

- **Priority**: P3 (direction)
- **Effort**: S-M (coarse)
- **Risk**: LOW — validation work; if the anchors prove weak, that's a
  finding, not a break
- **Depends on**: none. Requires a live Linux Wayland desktop session (the
  operator's KDE host or the Arch testing-vm — see
  `docs/operations/gui-desktop-test-harness.md`).
- **Category**: direction
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

The richer native-Wayland fallback region tree (vision-anchor roles like
`wayland_header_band` for apps that expose no AT-SPI tree) shipped, but its
proving workflow (TIDAL) was retired, and both follow-up boxes in
`ROADMAP.md` ("Choose a current fallback-only target app to replace the
retired TIDAL flow" and "Live agent-loop or app-server proof on the new
target") are unchecked. A shipped capability with zero live proof is at
silent-regression risk — every capture/coordinate change since the TIDAL
retirement has landed unverified against the fallback lane.

## Current state

- Owning ExecPlan: `plans/wayland_fallback_vision_anchors.md` — READ IT
  FIRST; it holds the design, prior TIDAL findings, and validation
  expectations. This spike exists to complete that plan's open work, and its
  results feed back into it (the repo's ExecPlan lifecycle in
  `plans/AGENTS.md` then retires it into a feature doc).
- The fallback tree implementation lives in the Linux backend
  (`crates/sky-cua-linux/src/backend.rs` fallback-snapshot path; grep
  `vision_anchor` and `wayland_header_band` across `crates/sky-cua-linux/`).
- Agent-loop harness: `scripts/live_agentic_loop_smoke.py` (README: the
  agent-loop acceptance tool). App-server probe:
  `scripts/_app_server_harness.py` / `live_app_server_*` family.
- Candidate constraints (from the ROADMAP item and the ExecPlan): the app
  must be (a) native Wayland, (b) AT-SPI-poor (fallback-only — verify with
  `observe`/`get_app_state` showing no semantic tree), (c) installed/
  installable on the proving host, (d) resettable to a known state for
  repeatable smokes.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Candidate AT-SPI check | drive `get_app_state` via the installed MCP surface or `scripts/live_desktop_smoke.py` patterns | fallback tree with vision anchors, no AT-SPI children |
| Agent-loop proof | `python3 scripts/live_agentic_loop_smoke.py` (read its CLI for target selection) | green run |
| Python suite | `uv run pytest` | all pass |

## Scope

**In scope**:
- Candidate evaluation notes (2-4 candidates, one chosen) →
  `plans/wayland_fallback_vision_anchors.md` (Progress / Decision Log
  sections per the ExecPlan format)
- A live smoke or agent-loop scenario for the chosen target (extend
  `live_agentic_loop_smoke.py`'s scenario set or add a
  `live_<app>_smoke.py` following the existing `live_krita_smoke.py`
  pattern — match whichever the ExecPlan prescribes)
- `ROADMAP.md` — check the two boxes when proven

**Out of scope**:
- Changing the fallback-tree runtime code. If the proof exposes weak
  anchors, file findings in the ExecPlan; fixes are follow-up work.
- The retired TIDAL flow.

## Steps

### Step 1: Candidate survey

Pick 2-4 candidates. Grounded starting suggestions (verify, don't assume):
Electron/Chromium apps with accessibility disabled by default, Flutter
desktop apps (classically AT-SPI-poor on Wayland), games/launchers, or media
apps similar to the retired TIDAL profile. For each: launch on the proving
host, run a fallback-tree observation, record (a) AT-SPI coverage, (b)
anchor quality (does `wayland_header_band` etc. line up with real UI), (c)
resettability. Write the comparison into the ExecPlan's Decision Log with
artifacts.

### Step 2: Wire the proof

For the chosen target, build the smallest repeatable scenario: launch →
observe (fallback tree) → one or two anchor-guided physical actions → a
verifiable outcome (window state, dialog, or pixel evidence per the repo's
smoke conventions). Follow an existing live smoke as the structural pattern.

**Verify**: the smoke runs green twice consecutively on the proving host.

### Step 3: Close the loop

Update the ExecPlan (Progress, Outcomes), check the two ROADMAP boxes, and
if the ExecPlan is now code-complete-with-proof, follow `plans/AGENTS.md`
lifecycle (feature doc + research extraction + delete the ExecPlan) — ask
the operator before the deletion step.

## Done criteria

- [ ] Decision Log records the candidate comparison and the choice rationale
- [ ] A repeatable live proof exists and ran green ≥2× (artifacts referenced)
- [ ] ROADMAP boxes updated
- [ ] `uv run pytest && uv run ruff check scripts && uv run basedpyright` green if any Python was added
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- No surveyed candidate is genuinely fallback-only (everything has usable
  AT-SPI) — report; the capability may want a synthetic fixture instead.
- The anchors are wrong on every candidate (misaligned bands) — that's a
  runtime bug report for the maintainer, not smoke-side fudging.
- No live Wayland session is available to you.

## Open questions for the maintainer

- Should the proof live in the VM matrix (`run_gui_testing_vm_smoke.py`
  profile) or stay a host-side live smoke? The VM gives repeatability; the
  host gives realism. Recommend: host-side first, VM profile as follow-up.
