# Plan 010 (spike): Native Windows UIA readback — populate text, numeric_value, supports_editable_text

> **Executor instructions**: This is a design/build spike, not a mechanical
> fix. Follow the steps; where the plan says "investigate", produce written
> findings in the deliverable doc rather than improvising code. Honor STOP
> conditions. When done, update the status row in `advisor-plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-windows/src/uia.rs crates/sky-cua-windows/src/backend.rs`
> On drift, re-verify the excerpts; on mismatch, STOP.

## Status

- **Priority**: P2 (direction)
- **Effort**: M (coarse — direction estimate)
- **Risk**: LOW — additive fields; shipped semantic actions untouched
- **Depends on**: none. NOTE: requires a Windows build/test environment
  (the repo's Windows lane builds `sky-cua-windows`; live proof uses the
  operator's Windows devbox VM — see `~/.claude` memory hints and
  `docs/features/windows-uia-automation.md` for the established verification
  path). If you have no Windows target available, do the code + unit-test
  work behind `cargo check --target x86_64-pc-windows-msvc` (or the repo's
  established cross-check — see how CI-less Windows changes were verified in
  `docs/features/windows-uia-automation.md`) and mark live proof as the
  remaining gate.
- **Category**: direction
- **Planned at**: commit `ed3aef3`, 2026-07-07

## Why this matters

Windows can *act* semantically (invoke/select/expand/toggle shipped —
`ROADMAP.md` "[x] Windows UIA inspection and semantic actions") but cannot
*read back* state: every element hardcodes the three readback fields. An
agent can click a slider or type into a field but cannot confirm the
resulting value without a screenshot round trip. Linux populates these
fields via AT-SPI (README "AT-SPI rich readback"), so this is the last gap
to genuine cross-platform readback parity, and the ROADMAP explicitly lists
it unchecked: "Native Windows/UIA readback (`text`, `numeric_value`,
`supports_editable_text`)".

## Current state

- `crates/sky-cua-windows/src/uia.rs:628-637` — element assembly:

  ```rust
      name: info.name,
      description,
      value: info.value,
      text: None,
      numeric_value: None,
      supports_editable_text: false,
      state_flags,
      semantic_actions,
      bounds: ...
  ```

- `uia.rs` already detects patterns for actions (grep `has_invoke`,
  `try_execute_semantic_action`, `collect_elements_for_hwnd`) — the COM
  plumbing, element walk, and pattern-availability checks exist; readback
  is a matter of querying three more patterns during collection.
- `crates/sky-cua-windows/src/backend.rs:979-980, 2305-2306, 2336-2337` —
  additional sites constructing elements with the same hardcoded values
  (fallback/window-level elements; some may legitimately stay `None`).
- The shared element model lives in `crates/sky-cua-platform/src/model.rs`
  (fields already exist — Linux populates them; NO platform changes needed).
- UIA mapping (design input, verify against the `windows` crate 0.62 API):
  - `text` ← TextPattern (`UIA_TextPatternId`) document range text, capped;
    or ValuePattern string for simple controls.
  - `numeric_value` ← RangeValuePattern (`UIA_RangeValuePatternId`) `Value`.
  - `supports_editable_text` ← ValuePattern present AND `IsReadOnly == false`,
    or TextPattern + `UIA_IsTextEditTransformPatternAvailablePropertyId`.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Windows-target check | `cargo check -p sky-cua-windows --target x86_64-pc-windows-msvc` (if toolchain target installed; else the repo's documented Windows build path) | exit 0 |
| Tests (host) | `cargo nextest run` | all pass (windows crate tests are cfg-gated) |
| Live proof (Windows box) | per `docs/features/windows-uia-automation.md` Verification section | fields populated for Notepad/Calculator fixtures |

## Scope

**In scope**:
- `crates/sky-cua-windows/src/uia.rs` — pattern queries during element
  collection + the three fields
- `crates/sky-cua-windows/src/backend.rs` — thread real values where the
  element context has an HWND/UIA element; leave pure-fallback sites `None`
  with a comment
- `docs/features/windows-uia-automation.md` — update the readback section
- `ROADMAP.md` — check the readback box when live-proven

**Out of scope**:
- `crates/sky-cua-platform` model changes (fields exist).
- Windows capture ladder, overlay (plans/roadmap items of their own).
- Perf tuning of the UIA walk (measure first; TextPattern queries can be
  slow — see Open Questions).

## Git workflow

- Branch: `bex/advisor-010-uia-readback`
- Style: `feat(windows): populate UIA readback fields`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Investigate (write findings into the PR/report)

Read `uia.rs`'s existing pattern-availability code. Answer in writing:
(a) does collection already cache `IUIAutomationElement` pattern pointers,
or query per property? (b) what is the cost model — is the walk using
`CacheRequest` batching? (c) which of the three fields can come from the
cache request vs. needing a live pattern call? This determines whether
readback adds per-element round trips (the Linux AT-SPI lesson: per-element
DBus calls dominate walk time; UIA has the same shape via cross-process
COM).

### Step 2: Implement

Populate the three fields per the mapping above, using the cheapest source
identified in step 1. Cap `text` extraction (suggest 4KB per element,
matching whatever cap the Linux AT-SPI readback uses — grep
`supports_editable_text` in `crates/sky-cua-linux/` and mirror its
conventions). Guard every pattern call: absence of a pattern is normal, not
an error.

**Verify**: Windows-target check exits 0; host `cargo nextest run` green.

### Step 3: Unit tests + live proof

Add cfg-gated unit tests for the pure mapping logic (pattern-presence →
field decisions) with fake pattern results if the code structure allows;
then run the documented live verification on the Windows environment
(Notepad: `supports_editable_text=true`, text populated; a slider fixture:
`numeric_value` populated). Record artifacts per the feature doc's format.

## Done criteria

- [ ] The three fields are populated from UIA patterns where available (grep shows no unconditional `numeric_value: None` in `uia.rs` element assembly)
- [ ] Fallback sites that keep `None` carry a one-line reason comment
- [ ] Windows-target check + host `cargo nextest run` green
- [ ] Live proof recorded, or explicitly reported as the remaining gate
- [ ] `docs/features/windows-uia-automation.md` updated; ROADMAP box updated only with live proof
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- Step 1 reveals the element walk has no pattern caching and adding three
  live pattern calls per element measurably degrades snapshot latency —
  report with numbers; the design may need a readback-on-demand tool
  parameter instead of always-on collection.
- The `windows` crate 0.62 API surface for TextPattern differs materially
  from the mapping above — document the actual API and proceed only if the
  mapping is still 1:1 obvious.

## Open questions for the maintainer

- Should `text` readback be always-on in snapshots or opt-in per request
  (payload size vs. convenience)? Linux behavior is the tiebreak — match it.
- Is Calculator/Notepad sufficient as the live fixture set, or should the
  Sumwall/Edge app-shell smokes (planned in `plans/windows_app_shell_smokes.md`)
  absorb the proof?
