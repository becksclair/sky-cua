# Plan 012 (spike): Close the isolated-desktop live proofs, then design the disposable/headless CUA lane

> **Executor instructions**: Two-phase spike: first close the existing
> unchecked proofs (concrete), then produce a short design doc (analysis).
> Do not build the headless lane in this plan. Honor STOP conditions. When
> done, update the status row in `advisor-plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-client/src/isolated_desktop.rs crates/sky-cua-client/src/isolated_desktop docs/features/isolated-xpra-desktop.md`
> On drift, re-read the feature doc before proceeding.

## Status

- **Priority**: P3 (direction)
- **Effort**: M (coarse; phase 1 is execution, phase 2 is a design doc)
- **Risk**: MED — the host-leak proof is the whole point of the sandbox;
  treat any leak found as a release blocker finding
- **Depends on**: none. Phase 1 requires the Arch testing-vm
  (`docs/operations/gui-desktop-test-harness.md`) and/or the live host.
- **Category**: direction
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

The isolated xpra desktop (private X11 desktop the agent owns:
`[isolated_desktop]` config, `desktop_launch_app`, read-only viewer) is the
freshest shipped primitive in the repo, but its headline proofs are still
unchecked in ROADMAP: "Live host-leak and headline `desktop_launch_app`
end-to-end run plus the `isolated-xpra` VM smoke profile". Beyond closing
that, the primitive makes a bigger product cheap: CUA sessions that never
touch the operator's real desktop — CI agent runs, parallel sessions,
disposable eval environments. That second half is a design question the
maintainer needs framed, not built.

## Current state

- Feature doc: `docs/features/isolated-xpra-desktop.md` — READ FIRST; it
  documents config, env keys (`SKY_CUA_ISOLATED_DESKTOP_*`), the viewer, and
  the verification story including what remains unproven.
- Runtime: `crates/sky-cua-client/src/isolated_desktop.rs` +
  `isolated_desktop/{probe,owned_bus}.rs` (note: currently under blanket
  `#![allow(dead_code)]` — plan 008 addresses that; ignore here).
- Known hard-won constraint (operator memory, verified in the feature
  work): KDE single-instance containment needs a private D-Bus bus plus
  `CLIENT_CLEARED_SESSION_ENV_KEYS`, or launched apps re-hydrate
  `WAYLAND_DISPLAY` and leak onto the host session. The host-leak guards
  exist as tests (`ef13b80` "host-leak and X11 env-recipe guards"); the
  *live* end-to-end leak proof is the unchecked box.
- VM smoke profile: `isolated-xpra` exists (`9617036`) in
  `scripts/run_gui_testing_vm_smoke.py` — run
  `python3 scripts/run_gui_testing_vm_smoke.py --list-profiles` to confirm
  its name and requirements.
- ROADMAP entry: "Phase: Linux desktop parity" first item, sub-box unchecked.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Profile listing | `python3 scripts/run_gui_testing_vm_smoke.py --list-profiles` | `isolated-xpra` listed |
| VM smoke | `python3 scripts/run_gui_testing_vm_smoke.py --host testing-vm --profile isolated-xpra` | green, artifacts written |
| Live host run | per the feature doc's Verification section | `desktop_launch_app` lands in the sandbox; leak checks pass |

## Scope

**In scope**:
- Running the proofs; fixing *smoke-harness* bugs they surface (not runtime
  bugs — those are findings)
- `docs/features/isolated-xpra-desktop.md` Verification updates + ROADMAP
  box
- New design doc: `docs/research/2026-07-disposable-desktop-lane.md`

**Out of scope**:
- Building the headless/disposable lane (phase 2 output is a doc).
- Runtime changes to `isolated_desktop*` — leaks or bugs found are reported,
  not hot-fixed here.
- Wayland-native isolation (xpra lane is X11 by design; note it in the doc).

## Steps

### Step 1: Run the VM smoke profile

Run `isolated-xpra` against the testing-vm. Record artifacts per the
harness's evidence conventions.

**Verify**: profile green; artifacts referenced in the feature doc.

### Step 2: Live host-leak + headline end-to-end

Per the feature doc: bring up the isolated desktop on the live host, run
`desktop_launch_app` for at least one KDE single-instance app (the hard
case) and one plain X11 app, verify (a) windows appear ONLY in the sandbox
(host `list_windows`/KWin shows nothing new), (b) no host-session env
leakage (the documented leak checks), (c) capture/input route to the
sandbox. Check the ROADMAP box only when both this and step 1 are green.

**Verify**: documented leak checks pass; feature doc Verification section
updated with dates + artifacts.

### Step 3: Design doc — the disposable/headless lane

Write `docs/research/2026-07-disposable-desktop-lane.md` (dated-research
format per `docs/AGENTS.md`), answering:

1. **Session lifecycle**: what "create → use → destroy" looks like as a
   first-class surface (config-scoped today; would it become an MCP tool,
   an installer mode, or an operator CLI?). Enumerate what state persists
   (xpra sockets, private bus, launched processes) and what teardown must
   reap.
2. **Concurrency**: can N isolated desktops coexist (socket/display-number
   allocation, per-session capture routing)? What breaks first?
3. **Headless viability**: what the lane needs when NO host session exists
   (CI): xpra's own virtual display vs Xvfb, portal-free capture path
   implications, session-presence interactions.
4. **Non-goals**: Wayland-native isolation, multi-tenant security hardening
   (per the project's explicit perf-over-security stance).
5. **Cost estimate** per option, and a recommendation.

Ground every claim in the current code/config (cite paths); where behavior
is unknown, run a small probe rather than speculating, and say what was
probed.

## Done criteria

- [ ] `isolated-xpra` VM profile green with artifacts
- [ ] Live host-leak + `desktop_launch_app` end-to-end proven and documented
- [ ] ROADMAP sub-box checked (only if both proofs green)
- [ ] `docs/research/2026-07-disposable-desktop-lane.md` exists, answers the five questions with cited evidence, and ends with a recommendation
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- The live leak check FAILS (a sandbox-launched app reaches the host
  session) — this is a release-blocking runtime finding; report immediately
  with the exact app + env evidence, do not patch around it.
- The testing-vm is unavailable/broken (see `docs/operations/` +
  memory notes about VM codex auth going stale) — do phase 2 with the
  design doc flagging phase 1 as blocked.

## Open questions for the maintainer (carry into the design doc)

- Is the disposable lane a sky-cua feature or an operator recipe
  (docs/operations runbook) — i.e., does it deserve MCP tool surface?
- Should parallel sessions be in scope at all, or is one-sandbox-at-a-time
  the honest v1?
