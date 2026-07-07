# Plan 002: Unit-test the untested pure logic (adb parsers, cursor geometry, phone response classification)

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `advisor-plans/README.md` — unless a reviewer dispatched you and told you
> they maintain the index.
>
> **Drift check (run first)**: `git diff --stat ed3aef3..HEAD -- crates/sky-cua-service/src/phone/adb crates/sky-cua-service/src/overlay/cursor_geometry.rs crates/sky-cua-client/src/mcp_tools/phone/response.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M (three small clusters, S each)
- **Risk**: LOW (test-only; no production code changes)
- **Depends on**: none (001 recommended first for the `just verify` gate)
- **Category**: tests
- **Planned at**: commit `ed3aef3`, 2026-07-07

> **ERRATUM (2026-07-07, post-execution)**: Step 1's premise was wrong — the
> adb parsers listed as UNTESTED already had coverage in
> `phone/adb/tests.rs` (reached via `use super::*;`, which the audit's
> import-line check missed). The executor verified this and correctly
> skipped Step 1. Steps 2 and 3 were real gaps and are done.

## Why this matters

Three clusters of pure, deterministic logic are currently proven only by
opt-in live smokes that need a real phone or desktop:

1. ~9 adb stdout parsers — `parse_wm_size` feeds device dimensions used by
   coordinate mapping, so a parse miss silently produces wrong tap
   coordinates on the phone.
2. `overlay/cursor_geometry.rs` — 302 lines of coordinate math mapping action
   points to native overlay points. Zero tests, while the *phone* equivalent
   mapping is thoroughly tested — an asymmetric gap.
3. `phone/response.rs` — a 25+-arm `matches!` deciding whether a phone
   diagnostic is an error. A new `Phone*` code added to the enum but missed
   here silently downgrades a failure to "ok".

All three are table-test material: string/struct in, value out. This also
builds the safety net plans 004–006 rely on.

## Current state

- `crates/sky-cua-service/src/phone/adb/parse.rs` — the parsers. Signatures
  (all `pub(in crate::phone)`):
  - `parse_version(&str) -> Option<String>` (:42)
  - `parse_server_status(&str) -> Option<usize>` (:57) — tested
  - `parse_devices_l(&str) -> Vec<AdbDeviceLine>` (:78) — UNTESTED
  - `classify_device_state(&str) -> PhoneDeviceState` (:153) — tested
  - `classify_connection_kind(&str) -> PhoneConnectionKind` (:173) — UNTESTED
  - `parse_mdns_services(&str) -> Vec<(String, String, String)>` (:195) — UNTESTED
  - `parse_wm_size(&str) -> Option<(u32, u32)>` (:220) — UNTESTED
  - `parse_wm_density(&str) -> Option<u32>` (:247) — UNTESTED
  - plus `parse_current_focus`/`extract_component` (~:356/:374),
    `parse_dimensions` (~:240), `parse_install_failure` (tested),
    `parse_rotation` (tested).
- `crates/sky-cua-service/src/phone/adb/tests.rs` — the existing test file and
  your structural pattern. Its header documents the intent: "Parsers are
  exercised directly over representative `adb` output (normal, unauthorized,
  offline, malformed, empty, multi-device)." Imports today:

  ```rust
  use super::parse::{classify_device_state, parse_install_failure, parse_server_status};
  use super::*;
  ```

- `crates/sky-cua-service/src/overlay/cursor_geometry.rs` — pure functions
  (selection): `state_from_action_request` (:14), `native_drag_start_point`
  (:60), `native_point_for_action` (:105), `point_to_stream_pixels` (~:196),
  `stream_pixels_to_native_point` (~:216), `point_to_pixels_through_rect`
  (~:285), `rect_center` (~:189). No `#[cfg(test)]` module in the file and no
  references from any test file (verified by grep at planning time).
- `crates/sky-cua-client/src/mcp_tools/phone/response.rs` —
  `phone_diagnostic_is_error_code` (:43, the big `matches!`),
  `phone_diagnostics_are_error` (:130), `phone_status_summary` (:146), plus
  ~10 summary builders. No references from `phone_tests.rs`.
- Conventions: tests are inline `#[cfg(test)] mod tests` or sibling
  `tests.rs` modules; run with `cargo nextest run`, NEVER `cargo test`
  (process-global env mutation races — see AGENTS.md). Match the existing
  fixture style: paste realistic raw adb output as string literals.

## Commands you will need

| Purpose | Command | Expected on success |
|---|---|---|
| Run the service crate tests | `cargo nextest run -p sky-cua-service` | all pass |
| Run the client crate tests | `cargo nextest run -p sky-cua-client` | all pass |
| Filter to new tests | `cargo nextest run -p sky-cua-service -E 'test(parse_wm_size)'` | new tests listed and pass |
| Format | `cargo fmt --check` | exit 0 |

## Scope

**In scope**:
- `crates/sky-cua-service/src/phone/adb/tests.rs` (extend)
- `crates/sky-cua-service/src/overlay/cursor_geometry.rs` (append a
  `#[cfg(test)] mod tests` only)
- `crates/sky-cua-client/src/mcp_tools/phone/response.rs` (append a
  `#[cfg(test)] mod tests` only)

**Out of scope** (do NOT touch):
- Any production code path. If a test you write exposes a real parser bug,
  STOP and report it with the failing case — do not fix the parser in this
  plan.
- Visibility changes beyond the minimum: the parse fns are
  `pub(in crate::phone)` and tests.rs is inside that scope already;
  `cursor_geometry` fns are `pub(super)`/private, which an inline
  `mod tests` can see. Do not widen any visibility.
- The ROADMAP.md "Add targeted unit tests" list (capture_screen retry,
  XTest scroll, crop_capture, etc.) — separately tracked, different owner.

## Git workflow

- Branch: `bex/advisor-002-pure-logic-tests`
- One commit per cluster, style: `test(phone): cover adb output parsers`,
  `test(overlay): cover cursor geometry mapping`,
  `test(client): cover phone response classification`.
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: adb parser tests

In `crates/sky-cua-service/src/phone/adb/tests.rs`, add table-driven tests
for each untested parser. Read each parser's body first to derive the exact
input format it expects, then write fixtures as realistic raw adb output.
Cover at minimum:

- `parse_devices_l`: normal USB device line with model/product/device/
  transport_id fields; a TCP serial (`192.168.x.x:5555`); `unauthorized` and
  `offline` states; empty input; a malformed line (must be skipped, not
  panic); multiple devices.
- `parse_wm_size`: standard `Physical size: 1080x2400`; override present
  (`Override size:` — read the parser to learn precedence); garbage; empty.
- `parse_wm_density`: `Physical density: 420`; override; garbage.
- `parse_mdns_services`: representative `adb mdns services` output; empty.
- `classify_connection_kind`: USB serial, `ip:port` serial, emulator serial
  (`emulator-5554`).
- `parse_version`, `parse_current_focus`/`extract_component`: one happy case +
  one malformed each.

**Verify**: `cargo nextest run -p sky-cua-service -E 'binary(sky_cua_service)'`
→ all pass, including your new tests (count them in the output).

### Step 2: cursor geometry tests

Append `#[cfg(test)] mod tests` at the bottom of
`crates/sky-cua-service/src/overlay/cursor_geometry.rs`. Build the input
structs (`ActionRequest`, `CaptureInfo`-derived types) the same way
neighboring overlay tests do — find the construction pattern with
`grep -rn "ActionRequest {" crates/sky-cua-service/src/overlay/` and reuse it.
Cover:

- `point_to_stream_pixels` / `stream_pixels_to_native_point`: identity at
  scale 1; a 2× logical→pixel scale; asymmetric width/height scaling;
  round-trip (to stream pixels and back lands within 1px).
- Degenerate inputs: zero-size pixel_size or logical_rect → `None` (read the
  code to confirm the expected behavior first; if it does NOT return `None`
  for zero sizes, test the actual behavior and flag it in your report).
- `native_point_for_action` / `native_drag_start_point`: an action with an
  explicit coordinate; an element-index action resolving through element
  bounds; an action with no point → `None`.
- The `CaptureBackendKind::PortalPipeWire` branch vs the non-PipeWire branch
  of `point_to_stream_pixels`.

**Verify**: `cargo nextest run -p sky-cua-service -E 'test(cursor_geometry)'`
→ new tests pass.

### Step 3: phone response classification tests

Append `#[cfg(test)] mod tests` in
`crates/sky-cua-client/src/mcp_tools/phone/response.rs`. Cover:

- `phone_diagnostic_is_error_code`: at least 5 codes that must classify as
  error and 5 that must not (pick them by reading the `matches!` arms).
- A **completeness guard**: if the `Phone*` diagnostic codes are a Rust enum
  (find where the codes originate — grep `PhoneScreencapDecodeFailed`), write
  a test that matches over every variant and asserts
  `phone_diagnostic_is_error_code` returns a deliberate value for each, so a
  new variant forces a compile error or test failure here. If the codes are
  plain strings (not an enum), instead assert the classifier's full known-set
  behavior in a table and add a comment telling maintainers to extend it.
- `phone_diagnostics_are_error`: empty slice; mixed error+info; info-only.
- Two golden-string tests for `phone_status_summary`.

**Verify**: `cargo nextest run -p sky-cua-client -E 'test(response)'` → pass.

## Test plan

This plan IS the test plan. Expected totals: ≥12 new adb parser tests, ≥8
geometry tests, ≥6 classification tests. Structural patterns:
`phone/adb/tests.rs` (step 1), any existing overlay test module (step 2),
`mcp_tools/phone_tests.rs` (step 3 — for assertion style, not location).

## Done criteria

- [ ] `cargo nextest run` exits 0 (whole workspace)
- [ ] `cargo fmt --check` exits 0
- [ ] Every parser listed in Step 1 has ≥2 test cases (happy + malformed/empty)
- [ ] `cursor_geometry.rs` and `response.rs` each contain a `#[cfg(test)]` module
- [ ] `git status` shows only the three in-scope files modified
- [ ] `advisor-plans/README.md` status row updated

## STOP conditions

- A new test exposes an actual parser/geometry bug (expected value per the
  code's documented intent ≠ actual). Report the failing input and observed
  output; do not change production code.
- Constructing `ActionRequest`/capture fixtures for step 2 requires more than
  ~30 lines of setup per test — the types may need a test builder that
  belongs in production code; report instead of building an ad-hoc one.
- The phone diagnostic codes turn out to be generated or defined outside the
  two crates in scope.

## Maintenance notes

- The step-3 completeness guard is the durable payoff: future `Phone*` codes
  must be classified deliberately. Reviewers should check the guard actually
  fails on an unhandled variant (try commenting one arm out locally).
- Plans 004 (retry idempotency) and 006 (phone capture) touch adjacent code;
  these tests are their regression net.
