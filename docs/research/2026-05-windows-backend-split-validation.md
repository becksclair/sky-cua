# Windows backend split validation

## Context

ICA-005 asked whether `crates/sky-cua-windows/src/backend.rs` should be
refactored now, deferred, or limited to mechanical module splits. The current
Windows backend is a v1 implementation that owns Win32 window discovery, GDI
capture, UIA fallback wiring, SendInput, RDP-safe window-message input,
coordinate conversion, and Win32 error handling behind the shared
`DesktopBackend` contract.

The validation question is deliberately narrower than a refactor plan: decide
what can move without changing Windows behavior or importing Linux-shaped
abstractions.

## Investigation

Current responsibility map:

- UIA-first semantic path: `WindowsDesktopBackend::execute_action` calls
  `uia::try_execute_semantic_action` before selecting a physical fallback.
  `crates/sky-cua-windows/src/uia.rs` owns UIA backend references, element
  traversal, action dispatch, and UIA desktop-to-stream bounds conversion.
- SendInput transport: `execute_send_input_action` handles physical click,
  secondary click, scroll, drag, text, key, and `set_value` fallback results
  with user-facing `SendInput ... completed` messages.
- WindowsMessages/RDP transport: `execute_window_message_action` mirrors the
  physical action surface through per-window cursor/message helpers and keeps
  the `Windows RDP ... completed` / `Windows v1 used RDP message ...`
  messages stable.
- GDI capture: `capture_desktop`, `capture_source`,
  `capture_desktop_blocking`, blank-frame detection, and `empty_capture` build
  the `CaptureInfo` shape with `CaptureBackendKind::WindowsGdi` and the blank
  capture diagnostic.
- Window enumeration/selection: `enumerate_windows`, `window_info`,
  `window_title`, `executable_for_pid`, `select_window`, and `window_element`
  bridge Win32 HWND metadata into `AppInfo`, `FocusedApp`, and fallback
  `ElementNode` values.
- Stream-pixel-to-desktop coordinate mapping: `desktop_action_point`,
  `desktop_drag_from_point`, `desktop_target_point`, and
  `stream_to_desktop_point` translate model-visible screenshot coordinates
  through `CaptureInfo.logical_rect` and `logical_to_pixel_scale` only at the
  native input boundary.

Existing characterization coverage already protects the riskiest seams:

- Stream-pixel coordinate conversion for explicit points, element centers, and
  drag start points.
- Window fallback element bounds staying screenshot-local.
- UIA fallback diagnostic target detection.
- RDP/window-message backend wire value.
- Blank black/white GDI frame detection.
- UIA semantic action routing and bounds mapping in `uia.rs` tests.

Devbox verification on 2026-05-21 used a clean archive of the current checkout
under `C:\Users\bex\projects\sky-cua` and a repo-local Cargo config to disable
the host's broken global `sccache` wrapper and force LLVM codegen instead of
Cranelift:

```powershell
cargo +nightly test -p sky-cua-windows
```

Result: 28 Windows crate tests passed.

## Conclusion

Defer any behavior-changing Windows backend refactor. The Windows v1 backend is
stable enough to accept small Windows-local mechanical splits, but not stable
enough to justify introducing an action/capture/windowing architecture modeled
after Linux.

The safe next split, when needed, is mechanical:

1. Move GDI capture helpers and blank-frame detection into a Windows-local
   `capture` module.
2. Move HWND enumeration, selection, and fallback window elements into a
   Windows-local `windowing` module.
3. Move physical input transports into a Windows-local `input` module only
   after preserving the current SendInput and WindowsMessages outcome strings
   in tests.

Do not add a shared Linux/Windows action abstraction for this slice. The shared
contract already exists at `sky_cua_platform::backend::DesktopBackend` and the
public model types; the lower-level Windows dependencies remain Win32/UIA/GDI
specific.

## Implications

- ICA-005 is validated rather than queued as an immediate code movement task.
- Future Windows refactors must preserve current wire shapes, UIA-first
  fallback diagnostics, SendInput outcome messages, WindowsMessages outcome
  messages, GDI blank-frame diagnostics, and stream-pixel coordinate tests.
- The coordinate-conversion parking-lot audit should use the existing Windows
  stream-pixel tests as golden cases before proposing any shared coordinate
  abstraction.
