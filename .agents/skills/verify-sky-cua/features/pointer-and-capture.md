# Pointer and capture

Pointer and capture let the agent see exact pixels of a window or display and
click, secondary-click, or drag at coordinates in that image. The snapshot that
produced the image enforces the geometry.

## Sub-features

- `ptr-click` sends `desktop_pointer {operation: "click", appshot_id, x, y, snapshot_id}`
  at crop-local pixels.
- `ptr-secondary` sends `operation: "secondary_click"`.
- `ptr-drag` sends `operation: "drag"` with `from_x`, `from_y`, `to_x`, `to_y`
  (or `x`/`y` as the start, or `element_index`/`to_element_index`), and an
  optional `duration_ms`.
- `desktop_pointer` has only these three operations. There is no move,
  double-click, or scroll (`desktop_scroll` is a separate tool), and the strict
  schema rejects `click_count` and `mouse_button`.
- `cap-window` runs `capture_desktop` by window selector and returns a
  `snapshot_id` with crop geometry.
- `cap-display` runs `capture_desktop` with no selector, which captures the main
  display only.

## How to get to it (user POV)

- Call `observe {surface: "desktop", window_title}`, then
  `desktop_pointer {operation, appshot_id, snapshot_id, x, y}` with that
  observation's `appshot_id` and `action_snapshot.snapshot_id`, and `x`/`y` read
  off its window-crop image.
- `desktop_pointer` also accepts a semantic target (`snapshot_id` with
  `element_index`, `name`, or `text`) or bare screen `x`/`y` without a
  `snapshot_id`.
- Call `capture_desktop {}` (main display), `{title|window_id|pid|app_id|wm_class}`
  (window crop), or `{display_id|display_name|display_index}`. It returns a
  `snapshot_id` and `capture.{pixel_size, logical_rect, images[].role}`, but no
  `appshot_id`, so it cannot be paired with `desktop_pointer` on its own.

## Driving it with verify.sh

Preconditions:

- Baseline from `README.md`. `verify.sh doctor` prints `verdict=drivable`.

- **Open the target.** Run `verify.sh drive --journey pointer`. It launches the
  zenity entry dialog (see `isolated-desktop.md`).
- **Capture the display.** `capture_desktop {}`. The check
  `main-display capture is the full private screen` requires
  `role == "primary_display_capture"` and `pixel_size == 1280x800`.
- **Capture the window.** `capture_desktop {title: <marker>}`. The check
  `window capture is cropped to the dialog's bounds` requires
  `role == "target_window_crop"` and `capture.logical_rect` equal to the observed
  window `bounds` (`430,269 420x262`). Both come from the same window
  geometry, so this proves the two crops agree; the crop image itself
  (`*-window.png`, 420x262) is the visual check.
- **Set a placeholder.** A fresh `observe`, then `desktop_set_value` with
  `placeholder-<hex>` and a readback `observe` (as in `semantic-actions.md`).
- **Drag-select it.** `desktop_pointer {operation: "drag", appshot_id, snapshot_id, from_x, from_y, to_x, to_y, duration_ms: 600}`
  across the entry, right to left at crop-local `(88%, 53%)` to `(11%, 53%)`
  (`(370, 139)` to `(46, 139)`). Check
  `desktop_pointer drag across the entry text succeeded`.
- **Prove the selection.** Observe, `desktop_keyboard type_text` the token, and
  observe again. Typing replaces the selection, so the check
  `drag selected the whole entry: typing replaced it with exactly the token`
  passes only if the drag selected the whole placeholder.
- **Click OK by pixels.** `desktop_pointer {operation: "click", appshot_id, snapshot_id, x, y}`
  from the last observation, at crop-local `(72%, 80%)` of the window bounds
  (`(302, 210)` in the 420x262 crop). The check
  `desktop_pointer click on OK by crop pixels succeeded` asserts a non-error
  response and records the message (`Clicked the target through the X11 input
  fallback.`) as its detail.
- **Side effect.** `pointer-zenity-output.txt` equals the token,
  `pointer-zenity-exit.txt` is `0`, and the window is gone.
- **Proof.** Per capture id, `images/*-pointer-capture_desktop-<id>.png` (the
  full-display grab), `<id>-window.png` (the crop), and `<id>.webp` (what the
  agent sees); the readback crop `images/*-pointer-observe-observe-*.webp`; and
  the `desktop_pointer` entries in `steps.jsonl`.
- **Secondary click (manual).** Not scripted, because its only proof is
  visual. Recipe: observe the dialog, then
  `desktop_pointer {operation: "secondary_click", appshot_id, snapshot_id, x: 210, y: 139}`
  on the entry. Expect `Performed the secondary click through the X11 input
  fallback.`, then take `capture_desktop {}` and look at the full-display
  `<id>.png`. GTK's context menu (Cut, Copy, Paste, Delete, Select All) opens
  at the click point, outside the dialog's bounds, and it appears neither in
  the AT-SPI projection nor in the window list. Close it with `press_key Escape`.
  Verified this way on 2026-11-08.
- **Not scripted.** Display-selector captures
  (`display_id|display_name|display_index`) have no journey. Report them as not
  verified.

## Gotchas

- GTK4 zenity elements (buttons and the entry) report `bounds: null` in the
  accessibility projection, so pointer coordinates must come from the image.
  The journey uses fixed-layout locators measured from the 420x262 crop at
  `--width=420`: the entry spans x 10% to 90% at y 53%, and OK is centred at
  about 72% x 80%. If zenity's layout changes, re-read a `*-window.png` crop
  and update the fractions in `drive_isolated.py` (`pointer`).
- The `x`/`y` pair is crop-local. Never translate it into desktop coordinates;
  the `snapshot_id` carries the crop-to-screen geometry. The same point read as
  desktop coordinates lands outside the dialog, so a passing side effect proves
  the mapping.
- X11 pointer input uses `xdotool mousemove --sync` and `click` without
  `--window` (real XTest).
- Any newer snapshot, including a `capture_desktop`, invalidates earlier
  AppShots. Clicks are also refused unless the AppShot's window is focused.
  Observe again right before every click.
