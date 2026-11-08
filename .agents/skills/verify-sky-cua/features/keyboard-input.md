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
  optional `snapshot_id` and window-target fields (`window_id`, `pid`, `app_id`,
  `wm_class`, `title`, and the `tty`/`terminal_*` fields).
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
  `Typed text through the X11 input fallback.` The check
  `desktop_keyboard type_text returned success` only asserts a non-error
  response and records the message as its detail; read the detail to confirm
  the XTest route.
- **Read back.** A fresh `observe`. The check
  `fresh structured readback: entry value == token and != initial` must show
  `final == "keyboard-<hex>"`.
- **Accept.** `desktop_keyboard {operation: "press_key", key: "Return", appshot_id: <fresh>}`.
- **Side effect.** `keyboard-zenity-output.txt` equals the token,
  `keyboard-zenity-exit.txt` is `0`, and the window is gone.
- **Proof.** `images/*-keyboard-observe-*.png` before and after typing, plus
  `steps.jsonl`.

## Gotchas

- On the X11 lane, sky-cua resolves the target window, focuses it
  (`xdotool windowactivate --sync`, falling back to `windowfocus --sync`), and
  then sends real XTest input with `xdotool type`/`key --clearmodifiers` and no
  `--window` (`crates/sky-cua-linux/src/x11/input_xtest.rs`, `focus_target`).
  When a target window id resolves, a failed focus returns an error instead of
  a false success. When no window id resolves, `focus_target` is a no-op and
  input goes to whichever window already has focus, so the readback and
  side-effect checks stay mandatory.
- Before 76af95b the fallback used `xdotool --window`, which sends synthetic
  `XSendEvent` events that GTK ignores: both calls returned `success: true`
  while readback stayed `''` and zenity never exited. A binary built before
  that commit fails this journey's readback and side-effect checks. Keep the
  journey strict; `success: true` alone is never proof. Always require the
  readback and the app-side side effect.
- On the Wayland portal lane (the user's real session) keyboard input uses the
  RemoteDesktop portal or a KDE clipboard paste instead. This skill cannot
  verify that lane safely; use `vm-tests` (`text-readback`, `wayland-pointer`).
