# macOS per-window targeting: background input and a window-scoped cursor

## Context

The macOS backend observes and acts on the whole foreground desktop. Physical
input is posted at the HID tap, so it moves the user's real pointer and
requires the target window to be frontmost
(`crates/sky-cua-macos/src/input.rs`, `backend/focus.rs`), and the agent
cursor overlay is one full-screen panel per display at screen-saver level
(`crates/sky-cua-overlay-host/src/macos_panel.rs`). An agent therefore cannot
work in one window while the user keeps working in another.

OpenAI's Codex Computer Use service for macOS does allow that. The research
question: **which mechanisms let a macOS computer-use agent drive a single
background window without taking the user's focus or pointer, and do they
work when reproduced outside that product?** This is the investigation behind
the `ROADMAP.md` item "Pointer-preserving input on macOS".

## Investigation

### Reference implementation

Source: the exported Swift symbol names of the locally installed service
(version 26.1105), plus the constants of four event constructors. Mechanisms
are stated at the level those names and constants support; anything beyond
that is marked as inference.

- **Observation is one-shot and per-window.** Stills come from a
  ScreenCaptureKit window filter (`desktopIndependentWindow`) and from a
  window-server capture of a list of window IDs. Accessibility elements are
  mapped to window IDs exactly rather than by geometry. This matches what
  `capture/sck.rs` already does, apart from the exact ID mapping.
- **The per-window stream is a viewer feature.** A continuous `SCStream` of
  the target window, plus a second stream of the cursor window, feeds a
  picture-in-picture view hosted by another process, with a video encoder as
  an alternate frame path. The model does not consume this stream.
- **Input is delivered to the process, not the desktop.** Semantic
  accessibility actions come first. Synthesized events are addressed to a
  window (window ID, window bounds, window-local location) and posted to the
  target process ID. A per-process "focus enforcer" tracks whether the
  application *believes* it is active separately from whether it *is* active,
  and sends it synthesized activation and deactivation events. A second
  component watches for the target genuinely taking focus and for
  menu-dismissal side effects (inference: it reverts or suppresses them).
- **The cursor is a small window in the normal window stack.** It is ordered
  directly above a specific target window ID and positioned relative to that
  window, with observers for the target's bounds and ordering. It is raised
  to an overlay level only for stated reasons such as open menus. Anything
  that covers the target window therefore covers the cursor.
- **The user is a first-class conflict source.** A monitor detects user
  interaction with the same application and returns an error asking the
  agent to re-query state.

### Event recipe

Read from the constructors and reproduced in the probe:

| Event | Construction |
|---|---|
| Application activated | `NSEvent.otherEvent`, type `appKitDefined` (13), subtype 1, `windowNumber` = target window ID, modifier flags `0xC0000`; converted with `.cgEvent` |
| Application deactivated | Same, subtype 2, window number 0, flags 0 |
| Mouse down / up | `NSEvent.mouseEvent` with the window number, converted with `.cgEvent`; then flags cleared, global location set, button number 0, mouse subtype 3, event fields 91 and 92 set to the window ID, and the **window-local location** set to `global - window origin` (top-left origin, not flipped) |
| Key down / up | Ordinary keyboard events with a Unicode string |
| Delivery | Timestamp set to now, then posted to the target process ID |

Setting the window-local location needs `CGEventSetWindowLocation`, which is
not public API. Everything else in the table is public. The reference
implementation's activation helper also sends a mouse down/up pair at an
"activation point"; the probe explains why.

### Probe

A throwaway Swift program (not kept in the repository) built the events
above and posted them with `CGEventPostToPid`. Host: macOS 27.2.1 (26B131),
arm64. Targets were launched in the background and never made frontmost by
the probe: TextEdit 1.21 (AppKit) with a five-line document, and Brave
157.1.99.55 (Chromium) on an isolated profile showing a page that reports
clicks, typed text, and `document.hasFocus()` through its title. The
frontmost process was read from the Process Manager and the real pointer
location from Core Graphics before and after every step.

TextEdit:

| Step | Result |
|---|---|
| Keys, no activation | Landed in the document |
| Click, no activation | Dropped (caret did not move) |
| Activation event | `AXFrontmost` became true; the system's frontmost process did not change |
| First click after activation | Swallowed; the window consumed it to become key |
| Later clicks | 3 of 3 placed the caret on the exact target character |
| Keys after a click | Inserted at the clicked caret |
| Deactivation event | `AXFrontmost` returned to false |

Variants on the click, all after activation:

- Window-local location flipped vertically: clicks landed on the wrong line.
- Window-local location omitted: no click landed, and the test window was
  displaced toward the screen origin during this variant. The cause was not
  isolated.
- Mouse subtype 0 instead of 3: no observable difference.
- An extra "key focus returned" event (type 21, subtype `0x8000`): no
  observable difference; the first click was still swallowed.
- Making the window key through accessibility (`AXMain`, `AXFocused`): the
  window moved up the z-order, and the first click was still swallowed.

Brave:

| Step | Result |
|---|---|
| Click, no activation | Landed |
| Click into the text area and keys, no activation | Landed; `document.hasFocus()` stayed false |
| Activation event, then a click | First click swallowed; `document.hasFocus()` became true |
| Click and keys after activation | Landed |
| Deactivation event | `document.hasFocus()` returned to false |

In every controlled run the frontmost process was unchanged and the real
pointer did not move. One earlier Brave run ended with Brave genuinely
frontmost during the activated phase, while the machine was also in
interactive use; a controlled repeat did not reproduce it. Whether the
target activated itself or was clicked is unresolved.

A follow-up run covered keyboard routing in TextEdit with three documents
open and the application in the background:

- Posted keys always arrived in the application's `AXFocusedWindow`, which
  was also its main window, whichever window was intended.
- Return, Tab, Backspace, the arrow keys, and chords a text view binds
  itself (Shift+Left, Option+Left, Control+A, Command+Left) all took effect.
- Menu shortcuts did not: Command+A and Command+Z did nothing, with or
  without a synthetic activation event first. Whether a key window (after a
  priming click) changes that was not tested.

A third run looked for a way to make a background window key without the
click costing anything, on TextEdit (a text view, which does not act on the
click that makes its window key) and Calculator (buttons, which do):

- The key state does not persist. After a deactivation event and a new
  activation, the first click on the text view was swallowed again.
- Calculator's buttons acted on clicks with no activation at all, and acted
  on the first click after one. Priming by clicking the target itself would
  therefore press a button twice.
- A window's `AXActivationPoint` is a point in its title bar: 30 points in
  and 16 down for TextEdit, 10 and 16 for Calculator. A hit-test there
  returned the window itself or a hosting view with no actions. A click
  there after activation made the window key and did nothing else: no
  minimize, no move, no change in window order, no change in Calculator's
  display. The centre of the title bar is not safe in general; in Calculator
  it is the display, which has a press action.
- Windows of Chromium, Electron, and the Finder desktop reported an
  activation point outside their own frame.
- With activation, a priming click at the activation point, and then the
  real click: a single click placed the TextEdit caret on the exact
  character, a double click selected the word, and one click on a Calculator
  key entered one digit.
- Scroll-wheel events carrying the same window fields and window-local
  location had no effect on TextEdit, with or without activation.

Not exercised: drags, secondary clicks, menus, windows that are minimized or
on another Space, Electron, Mac Catalyst, and secure text fields.

### Sharing indicator

When the reference implementation drives a window, macOS marks that window
and the menu bar with its lavender sharing indicator. A second throwaway
probe tested whether that comes from the live stream: it opened a
`SCStream` on a background TextEdit window through a single-window filter,
at 64x64 pixels and one frame every two seconds, and compared the window's
accessibility tree and screenshots before, during, and after.

While the stream was live, the window's close, minimize, and zoom buttons
were replaced by one lavender menu button that accessibility reports as
"Window sharing", and a lavender sharing item appeared in the menu bar. Both
disappeared when the stream stopped. Three frames were delivered in four
seconds, so the smallest practical stream is enough.

The reference implementation keeps the stream and the model's observations
apart. Its only holder of a live stream is the picture-in-picture renderer,
which publishes a remote layer and encoded video frames to the hosting
application for one conversation turn. The observation requests return
accessibility text and a screenshot file per request.

## Conclusion

Driving a background window without taking the user's focus or pointer is
feasible on macOS 27 with one private symbol.

1. Keyboard events posted to a process reach its key window without any
   activation, except menu shortcuts, which an inactive application ignores.
2. Mouse events need a window-addressed event (window number, fields 91 and
   92, and the unflipped window-local location). An AppKit application also
   has to believe it is active; Chromium does not.
3. Synthetic activation is a single AppKit-defined event and is reversible
   with the matching deactivation event. It does not change the system's
   frontmost process.
4. The first click after a synthetic activation is consumed to make the
   window key. A click delivered to a freshly activated application needs a
   priming click first, which is the purpose of the reference
   implementation's activation point.
5. Accessibility attributes are the wrong tool for making the window key:
   they raise it.

Open risks: a target that reacts to synthetic activation by activating
itself for real; views that accept the first mouse click, where a priming
click would act twice; and reliance on `CGEventSetWindowLocation` and on
undocumented event field values across macOS releases.

## Implications

- **Observation.** `MacOsBackend::screenshot` should stop activating an
  unfocused target; the capture path does not need it.
- **Input lane.** Add process-targeted delivery beside the HID path, keeping
  the accessibility-first order: keys directly, clicks with the
  window-addressed recipe, synthetic activation only for applications that
  need it, and deactivation when the action sequence ends. Resolve the
  private symbol at runtime in one module and report the delivery mode and
  any fallback as structured fields.
- **Guards.** Compare the frontmost process before and after each delivery
  and report a real activation as a diagnostic rather than hiding it. Detect
  user input in the target application between observation and action.
- **Priming click.** Click the window's `AXActivationPoint`, and only when
  it lies inside the window and a hit-test there finds no action a click
  would trigger. Skip activation and priming for Chromium engines, which act
  on the click directly. A window with no such point takes the foreground
  path.
- **Overlay.** A window-scoped cursor needs a target window ID and
  window-relative coordinates in the overlay protocol, a panel at normal
  level ordered above the target's window number, and tracking of the
  target's bounds and ordering. This part is design only; ordering a panel
  above another process's window was not probed.
- **Streaming.** A per-window `SCStream` is not needed for observation or
  input. It has two uses: a live view of the agent's window when something
  consumes one, and the system sharing indicator, which a minimal stream
  raises on exactly the target window. While it is live the indicator
  replaces the window's close, minimize, and zoom buttons, which changes
  what the user and the accessibility tree see in the title bar.
