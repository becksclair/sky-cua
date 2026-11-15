# sky-cua-macos Guide

`sky-cua-macos` implements the native macOS desktop backend for the shared
Computer Use contract. Keep Accessibility (AX), ScreenCaptureKit, CoreGraphics
window-list, and CGEvent details inside this crate and expose only
`DesktopBackend` behavior upward.

## Layout

- `backend.rs` — `MacosDesktopBackend`, the `DesktopBackend` implementation
  that composes the modules below. `backend/delivery.rs` chooses how one
  physical input reaches its window: posted to the window's process, or
  through the HID tap after the window is brought to the front.
- `ax/` — Accessibility runtime: per-application worker lanes, tree reads,
  semantic actions, and retained element handles.
- `windows.rs`, `displays.rs` — CoreGraphics window and display enumeration
  and target resolution.
- `window_match.rs` — public-API correlation of an AX window with a
  `CGWindowID` (no private symbols).
- `capture.rs` — ScreenCaptureKit window and display capture, prepared through
  `sky-cua-capture`.
- `input.rs`, `keymap.rs` — CGEvent pointer and keyboard injection, on the
  HID tap or to one process. `input/window_events.rs` builds the left
  clicks addressed to one window of an inactive application.
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
- One private symbol is used, `CGEventSetWindowLocation`, in
  `input/window_events.rs`. It is resolved at run time, and its absence
  makes that route unavailable, never an error at start-up. Do not add
  another without the same shape and a reason in the module docs.
- Prefer AX semantic actions; CGEvent input is the physical fallback.
- Leave the user's frontmost application and pointer alone whenever the
  input still reaches its target. A capture never activates a window.
  Keyboard input goes to the target process when its window is the
  application's key window, and a left click is addressed to the window;
  only input that would not take effect there brings the window forward. A fallback to the foreground path is always
  reported as a diagnostic.
- Missing TCC permissions are reported as structured diagnostics that name
  the System Settings pane; nothing is inferred from prose.
- Keep fallback roles blunt: real window bounds are useful, fabricated widget
  semantics are not.
