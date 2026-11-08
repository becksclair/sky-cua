# Keyboard input

Keyboard input lets the agent type text and press keys or chords into the
observed application through `desktop_keyboard`, for example filling a text
field and pressing Enter to accept a dialog.

## Sub-features

- `kbd-type` types a text string into the focused element of the targeted
  window (`operation: "type_text"`).
- `kbd-key` presses a key or chord such as `Return` or `ctrl+s`
  (`operation: "press_key"`).
- `kbd-newline` turns newlines in `type_text` into `Return` presses.

## How to get to it (user POV)

- Call MCP `desktop_keyboard {operation: "type_text", text, appshot_id}`, with
  optional `window_id`, `pid`, `app_id`, or `title`.
- Call MCP `desktop_keyboard {operation: "press_key", key, appshot_id}`.

## Driving it with verify.sh

Preconditions:

- Baseline from `README.md`. `verify.sh doctor` prints `verdict=drivable`.

- **Open the target.** Run `verify.sh drive --journey keyboard`. It launches a
  zenity entry dialog titled `sky-cua-verify-keyboard-<hex>`, and the entry has
  focus.
- **Observe.** `observe {surface: "desktop", window_title: <marker>}`. Record the
  entry element's initial `value` (`''`).
- **Type.** `desktop_keyboard {operation: "type_text", text: "keyboard-<hex>", appshot_id}`.
  The response should be `success: true` with
  `Typed text through the X11 input fallback.`
- **Read back.** A fresh `observe`. The check
  `fresh structured readback: entry value == token and != initial` must show
  `final == "keyboard-<hex>"`.
- **Accept.** `desktop_keyboard {operation: "press_key", key: "Return", appshot_id: <fresh>}`.
- **Side effect.** `keyboard-zenity-output.txt` equals the token,
  `keyboard-zenity-exit.txt` is `0`, and the window is gone.
- **Proof.** `images/*-keyboard-observe-*.png` before and after typing, plus
  `steps.jsonl`.

## Gotchas

- **Known failing (2026-11-08, rev 5b79228).** On the X11 lane, sky-cua resolves
  the target window and runs `xdotool type --window <id>` and
  `xdotool key --window <id>` (`crates/sky-cua-linux/src/x11/input_xtest.rs`).
  `--window` sends synthetic `XSendEvent` events, which GTK ignores. Both calls
  return `success: true`, readback stays `''`, zenity never exits, and the
  `*-exit.txt` file is never written. A manual `xdotool type`/`key` without
  `--window` (real XTest to the focused window) on `:147` types and accepts the
  same dialog. This is a product defect. Keep the journey strict so it starts
  passing when the defect is fixed.
- Because of that, a keyboard `success: true` on X11 proves nothing. Always
  require the readback and the app-side side effect.
- On the Wayland portal lane (the user's real session) keyboard input uses the
  RemoteDesktop portal or a KDE clipboard paste instead. This skill cannot
  verify that lane safely; use `vm-tests` (`text-readback`, `wayland-pointer`).
