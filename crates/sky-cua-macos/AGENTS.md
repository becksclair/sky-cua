# sky-cua-macos Guide

`sky-cua-macos` implements the native macOS desktop backend for the shared
Computer Use contract. Keep Accessibility (AX), ScreenCaptureKit, CoreGraphics
window-list, and CGEvent details inside this crate and expose only
`DesktopBackend` behavior upward.

## Layout

- `backend.rs` — `MacosDesktopBackend`, the `DesktopBackend` implementation
  that composes the modules below.
- `ax/` — Accessibility runtime: per-application worker lanes, tree reads,
  semantic actions, and retained element handles.
- `windows.rs`, `displays.rs` — CoreGraphics window and display enumeration
  and target resolution.
- `window_match.rs` — public-API correlation of an AX window with a
  `CGWindowID` (no private symbols).
- `capture.rs` — ScreenCaptureKit window and display capture, prepared through
  `sky-cua-capture`.
- `input.rs`, `keymap.rs` — CGEvent pointer and keyboard injection.
- `permissions.rs` — TCC status, System Settings guidance, doctor reporting.
- `coords.rs` — the single place that converts between global points and
  model-image pixels.

## Conventions

- The public MCP/service model lives in `sky-cua-platform`; do not duplicate
  contract types here.
- One coordinate rule: AX, CGEvent, and the CoreGraphics window list all use
  global points with a top-left origin, and that is this crate's
  `DesktopLogical` space. Never use AppKit (`NSScreen`, `NSWindow`) geometry,
  whose origin is bottom-left. Every point/pixel conversion goes through
  `coords.rs`.
- AX calls are synchronous IPC into the target application. They never run on
  a tokio runtime thread; they run on the `ax` worker lanes with a messaging
  timeout and an overall deadline, and a slow application yields a truncated
  tree with a diagnostic instead of a stalled daemon.
- Window identity uses public APIs only. Ambiguous correlation fails closed;
  it never substitutes a different window.
- Prefer AX semantic actions; CGEvent input is the physical fallback.
- Missing TCC permissions are reported as structured diagnostics that name
  the System Settings pane; nothing is inferred from prose.
- Keep fallback roles blunt: real window bounds are useful, fabricated widget
  semantics are not.
