## Linux Platform Notes

Load this reference only for Linux, KDE/KWin, XWayland, or native Wayland behavior.

- `activate_window` success is focus-verified on Linux, including KDE/KWin; focused-window discovery works on KWin too.
- Errors from `activate_window` name the missing backend seam.
- On KDE/KWin Wayland, prefer `window_id` over `pid` when both are available.
- `window_id` identifies the exact window; `pid` can be ambiguous for multi-window apps and compositor-managed surfaces.
- XWayland editors may need keyboard input through the X11 lane rather than the portal keyboard lane.
- Native Wayland apps can expose good structure yet report wrong actionable bounds.
- Fallback-only Wayland windows need fresh screenshots after context menus, submenus, or inline rename steps.
- If semantic click wedges and the visible target is clear, click coordinates.
- Treat `SessionEnvRepaired` as context, not error.
- On Linux, check `doctor.session_env` before judging a thin app list or missing capture/input as desktop unavailable.
- `AccessibilityTreeActivated`: a Chromium/Electron tree was dormant and sky-cua woke it; the returned elements are the full tree. `AccessibilityTreeDormant` with `remedy=relaunch_with_force_renderer_accessibility`: the app refused activation; use pixels or ask the user to relaunch it with that flag.
- `AccessibilityAppNotRegistered` with `remedy=relaunch_app`: the app (typically Chromium/Electron) started before session accessibility was on; relaunch it, or launch GUI apps via `desktop_launch_app` or with `ACCESSIBILITY_ENABLED=1` from the shell.
