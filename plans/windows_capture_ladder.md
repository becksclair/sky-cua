# Windows capture ladder (WGC / DXGI before GDI)

This ExecPlan is a living document. Keep `Progress`, `Surprises &
Discoveries`, `Decision Log`, and `Outcomes & Retrospective` current as
work proceeds.

## Purpose / Big Picture

After this work, the Windows backend has a stronger capture lane for
browser-like GPU windows than the current GDI-only path. The agent will
get reliable pixels for Microsoft Edge and other Chromium-based shells
without silently accepting black screenshots. Capture metadata reports
which backend produced each image, so blank or protected frames are
diagnosable rather than mysterious.

## Progress

- [x] (2026-05-08) Established that GDI / `PrintWindow` returns black
  images for Edge while keyboard input still works and window titles
  change. Added a blank-frame diagnostic so the failure is visible
  through `get_app_state` instead of silent.
- [ ] Decide between Windows Graphics Capture (WGC) and DXGI Desktop
  Duplication as the first additional capture lane.
- [ ] Implement the chosen lane behind a Windows-only feature flag,
  preserving GDI as the fallback.
- [ ] Wire `capture.image_backend` so the snapshot reports which lane
  produced the image.
- [ ] Add a Windows host smoke that proves non-black Edge capture or
  records the structured diagnostic.

## Surprises & Discoveries

- The blank-frame diagnostic landed in the first Windows UIA milestone
  but the underlying capture limitation was not addressed. See
  `docs/research/2026-05-windows-uia-investigation.md` for the original
  investigation.

## Decision Log

- Decision: Capture upgrades are scoped to a separate ExecPlan from UIA
  inspection and semantic actions.
  Rationale: better pixels without UIA is still partial; UIA without
  better pixels still misses the Edge case. Splitting the work avoids
  bundling unrelated risk in one slice.
  Date/Author: 2026-05-17 / Codex (extracted from the original goal
  package).

## Outcomes & Retrospective

Pending implementation. At completion, record:

- Which capture lane was selected and why.
- Whether the GDI fallback path is preserved unchanged.
- Live evidence for at least one browser-like GPU window.

## Context and Orientation

The Windows backend lives in `crates/sky-cua-windows/src/backend.rs`. It
already records screenshot pixel metadata in `CaptureInfo`, maps stream
pixels back to desktop coordinates in `stream_to_desktop_point`, and
emits the blank-frame diagnostic added during the UIA work. The blank-
frame check looks for low-variance frames; the diagnostic carries the
HWND and capture lane so an operator can see exactly which lane went
black.

Existing capture metadata fields: `capture.backend`,
`capture.image_backend`, `capture.pixel_size`, `capture.logical_rect`.
The split between `backend` (selected primary lane) and `image_backend`
(actual lane that produced the image) already exists; this work adds a
new value to the second field.

Open question from the original goal package's `blockers.md`: whether
the `windows-sys` crate is enough for WGC / DXGI traversal or whether a
typed `windows` crate dependency is justified. Investigate at
implementation time; record the conclusion in the Decision Log here.

## Plan of Work

1. Investigate WGC vs DXGI Desktop Duplication on the available Windows
   host. Choose the path that produces non-black pixels for Edge with
   the smallest per-call overhead.
2. Implement the selected lane in a new module under
   `crates/sky-cua-windows/src/`. Keep the dependency footprint small;
   pull in `windows` crate features only as needed and explicitly.
3. Update the Windows backend to try the new lane first when
   conditions permit, falling back to GDI / `PrintWindow` and finally
   to the blank-frame diagnostic.
4. Update `capture.image_backend` reporting to include the new lane
   value.
5. Add a host-only smoke (`scripts/live_windows_capture_smoke.py` or
   similar) that captures Edge through the new lane, asserts the
   resulting image is not blank, and records the artifact.

## Validation and Acceptance

- `cargo +nightly fmt --check`
- `cargo +nightly --config 'profile.dev.codegen-backend="llvm"' --config 'profile.test.codegen-backend="llvm"' test -p sky-cua-windows`
- `cargo +nightly --config 'profile.dev.codegen-backend="llvm"' --config 'profile.test.codegen-backend="llvm"' test`
- A live Windows smoke captures a non-black Edge frame through the new
  lane, with the artifact path recorded.
- Existing GDI fallback behavior remains unchanged for non-browser apps.

## Idempotence and Recovery

The change is additive: the new lane sits in front of GDI, not in
place of it. If the new lane fails to produce a frame, the backend
falls through to the existing path. Reverting is a single-file unwire.

## Interfaces and Dependencies

- `crates/sky-cua-platform/src/model.rs` — extend
  `CaptureBackendKind` with the new lane name.
- `crates/sky-cua-windows/Cargo.toml` — pull in only the `windows`
  crate features needed.
- `Cargo.toml` (workspace) — coordinate any new dependency through
  workspace-managed versions per the universal conventions.

## Revision Notes

- 2026-05-17 / Codex: Extracted from `goals/windows-app-automation/`
  (Plan slice 3, "Improve capture diagnostics and ladder") during the
  documentation cleanup. The blank-frame diagnostic part of that slice
  shipped with the UIA milestone and is now in
  `docs/features/windows-uia-automation.md`. This plan covers only the
  capture-lane upgrade.
