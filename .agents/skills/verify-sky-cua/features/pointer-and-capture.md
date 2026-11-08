# Pointer and capture

Pointer and capture let the agent see exact pixels of a window or display and
click, secondary-click, or drag at coordinates in that image. The snapshot that
produced the image enforces the geometry.

## Sub-features

- `ptr-click` sends `desktop_pointer {operation: "click", appshot_id, x, y, snapshot_id}`
  at crop-local pixels.
- `ptr-secondary` sends `operation: "secondary_click"`.
- `ptr-drag` sends `operation: "drag"` with `from_x`, `from_y`, `to_x`, `to_y`,
  and an optional `duration_ms`.
- `cap-window` runs `capture_desktop` by window selector and returns a
  `snapshot_id` with crop geometry.
- `cap-display` runs `capture_desktop` with no selector, which captures the main
  display only.

## How to get to it (user POV)

- Call `observe {surface: "desktop", window_title}` and use its image plus
  `action_snapshot.snapshot_id`.
- Call `capture_desktop {...}`, then
  `desktop_pointer {operation, appshot_id, x, y, snapshot_id}`.

## Driving it with verify.sh

Preconditions:

- Baseline from `README.md`. `verify.sh doctor` prints `verdict=drivable`.
- No scripted journey exists yet. Every sub-feature here is unverified until one
  is added to `scripts/drive_isolated.py` and run.

- **Open the target.** Reuse the zenity entry dialog and set its value with
  `desktop_set_value` (see `semantic-actions.md`) so OK has an observable effect.
- **Capture.** Call `observe {surface: "desktop", window_title: <marker>}`. Keep
  `appshot_id` and `action_snapshot.snapshot_id`. The window crop is
  `images/*-observe-observe-*.webp`. The full-display source is
  `images/*-<snapshot_id>.png` (1280x800).
- **Click OK by pixels.** Locate the OK button in the crop image the observe
  returned, not in a guessed desktop position. Call
  `desktop_pointer {operation: "click", appshot_id, snapshot_id, x: <crop x>, y: <crop y>}`.
  Never translate crop coordinates into desktop coordinates; the
  `snapshot_id` carries that geometry.
- **Side effect.** The zenity output file equals the value, the exit file is `0`,
  and the window is gone from `list_resources` windows.
- **Proof.** Save the crop image with the click point, the pointer response, and
  the after-state window list.

## Gotchas

- GTK4 zenity buttons report `bounds: null` in the accessibility projection, so
  click coordinates must come from the image. A script needs a deterministic
  locator (for example a known dialog layout at a fixed width) or a vision
  step. Record which one in this file when adding the journey.
- X11 pointer input uses `xdotool mousemove --sync` and `click` without
  `--window` (real XTest), so it is not affected by the synthetic-event defect in
  `keyboard-input.md`. That is unverified until a journey runs.
- Pixel coordinates expire after any visible transition. Re-observe before every
  click.
