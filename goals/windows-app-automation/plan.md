# Plan: First-class Windows app automation

## Solution Overview

Build a first-class Windows app-shell automation lane inside `sky-cua`. The work has three cooperating parts. The first part is a real Windows UI Automation inspector that can turn a selected HWND into a flattened tree of buttons, tabs, menus, edit boxes, panes, and dialogs. The second part is a semantic action router that invokes UI Automation patterns before falling back to SendInput. The third part is a stronger capture ladder that can handle browser-like GPU windows better than the current GDI-only path and reports when capture is blank or protected.

The result should let Codex inspect and operate the actual application window rather than pretending every Windows app is one opaque rectangle. Browser page automation is not part of this goal; browser-like apps matter because their shell is a Windows desktop application with tabs, address bars, menus, dialogs, and settings surfaces.

## Why This Approach

The shared platform model already separates capture, input, and semantic backends, so this work can deepen the Windows backend without replacing the MCP interface. UI Automation is the native Windows semantics layer for desktop app automation and testing. It is the right first-class path for app chrome and dialogs. SendInput remains valuable, but only as a physical fallback when semantics are missing or unreliable.

The live Edge smoke proved why capture must be upgraded separately: keyboard input worked and window titles changed, but GDI capture returned a black image. A richer UI tree without reliable pixels is not enough, and better pixels without semantics still leaves agents guessing.

## How It Will Work

`WindowsDesktopBackend::get_app_state` will still select a top-level window through the existing Win32 enumeration. After selection it will try to collect UI Automation data for that HWND. If UIA succeeds, the backend will flatten the UIA subtree into `ElementNode` values with stable roles, names, values, state flags, bounds, backend references, and semantic actions. If UIA fails or returns no useful children, the backend will keep the current top-level fallback and add a precise diagnostic.

Actions that target an element will first try the element's semantic backend reference. If the reference points to a UIA element and the needed pattern is available, the backend will invoke that pattern. If not, the backend will resolve the element bounds through the existing coordinate path and use SendInput or Windows messages.

Capture will keep the existing `CaptureInfo` shape. The backend will add a better Windows capture lane before GDI when feasible. Each capture result must identify which backend produced the image and whether the image looks blank.

## Slices

| Slice | Purpose | Main files or systems | Done when | Risks |
| --- | --- | --- | --- | --- |
| 1 | Add UIA inspection | `crates/sky-cua-windows/src/backend.rs`, optional new `uia.rs`, `Cargo.toml` features | `get_app_state` reports `semantic_backend = uia` and returns real child nodes for at least one native Windows app | COM and UIA raw bindings can be noisy; providers vary by app |
| 2 | Add semantic UIA actions | Windows backend action router | `click(element_index)` and `set_value(element_index)` prefer UIA patterns where available and fall back cleanly | Incorrect pattern use can click the wrong app control |
| 3 | Improve capture diagnostics and ladder | Windows capture code and platform enum if needed | Edge no longer silently returns an apparently valid black screenshot without a diagnostic; better capture is used where available | Windows capture APIs have session/security constraints |
| 4 | Add app-shell live smokes | `scripts/`, docs, release plugin validation | Edge/Sumwall smoke exercises window selection, tab/address/menu interactions, and records evidence | Live UI state is environment-sensitive |
| 5 | Document and package | README, `resources/app-instructions`, goal progress | The release plugin installs and exposes the improved Windows behavior | Packaging can accidentally ship stale binaries |

## Sequencing

Start with UIA inspection because it is the root of first-class app-shell automation. Then wire semantic actions against those returned nodes. Improve capture in parallel only after the UI tree path is proven, because capture changes are easier to validate when there are known app targets. Add live smokes after the backend can report observable differences. Finish with packaging and release install validation.

## Phase Boundaries

This goal ends when UIA-backed inspection and actions work for at least one native Windows app and provide materially better state for Edge or Sumwall, with honest diagnostics for any remaining black-capture or sparse-provider cases. A later goal should handle deeper browser-specific side channels such as CDP/WebView2 metadata if UIA and capture are already first-class.

## Steering Notes

- Prefer honest sparse state over fake widgets. A fallback rectangle with a clear diagnostic is better than invented button semantics.
- Browser-like apps are test subjects for app-shell automation, not permission to drift into website automation.
- Keep current physical input behavior working throughout the migration.

## Acceptance Criteria

- [ ] `get_app_state` on a UIA-capable Windows app returns more than the top-level window fallback and marks `semantic_backend` as `uia`.
- [ ] UIA node bounds, names, roles, values, and available actions are represented through existing `ElementNode` fields without breaking existing clients.
- [ ] `click(element_index)` uses UIA `InvokePattern` or equivalent when available and reports which action lane was used.
- [ ] `set_value(element_index)` uses UIA `ValuePattern` when available and falls back to focus/select/type with a diagnostic when not.
- [ ] Browser-like app capture no longer silently accepts black screenshots as normal state.
- [ ] Edge and Sumwall live smokes record whether UIA, capture, and input succeeded, with screenshot paths or diagnostics.
- [ ] `cargo fmt --check`, targeted Windows Rust tests, root Rust tests that are practical on the host, Python harness checks, and release plugin build/install validation pass or have documented external blockers.

## Required Evidence

| Requirement | Evidence to inspect | Where evidence is recorded |
| --- | --- | --- |
| UIA inspection | JSON snapshot showing multiple UIA-derived elements and `semantic_backend = uia` | `goals/windows-app-automation/progress.jsonl` |
| UIA actions | Test output and live smoke transcript showing semantic click/value lane | `progress.jsonl` and terminal transcript |
| Capture diagnostics | Snapshot diagnostic for blank/protected/fallback capture, or non-black WGC/DXGI capture artifact | `progress.jsonl` plus capture path |
| Edge/Sumwall app-shell behavior | Live smoke result with app title, selected window, actions attempted, and observed result | `progress.jsonl` |
| Packaging | Deploy script output and app-server status showing installed release plugin exposes `computer-use` | `progress.jsonl` |

## Completion Audit

Before marking the goal complete, Codex must map every explicit requirement, file, command, check, and deliverable to real evidence. If any item is missing, incomplete, weakly verified, or uncertain, the goal is not complete.
