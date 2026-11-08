# sky-cua verification map

This directory is the maintained source for verifying the user-facing behavior
of sky-cua's desktop MCP surface on the isolated-desktop lane. Read the index
before driving the instance, then use the matching feature file as the recipe.
`../SKILL.md` covers launch, doctor, evidence, and cleanup.

## Baseline preconditions

- `verify.sh sync`, `verify.sh build`, and `verify.sh launch` completed for the
  checkout under test.
- `verify.sh doctor` prints `verdict=drivable`, including
  `user_daemons_unchanged=true`.
- The private display is `:147` (or `VERIFY_DISPLAY`) at `1280x800`, with no
  viewer attached.
- `zenity` is installed on the host. Every journey's app is a zenity entry
  dialog launched by the journey itself.
- Never drive an instance that was not started by this verification run. Never
  drive the user's daemon on `service.sock`.

## Driving conventions

- Scripted journeys run with `verify.sh drive --journey <id>`. Ad-hoc operator
  calls run with `verify.sh cli <command>`.
- MCP calls follow the computer-use contract: observe, use the current
  `appshot_id` on every state change, and pair `element_index` with the
  observation's `snapshot_id`.
- Target elements by role, name, and advertised `semantic_actions` from the
  observation, never by index constants or coordinates guessed ahead of time.
- Give every launched app a unique title marker and match windows by it.
- Kill only processes the journey launched, by PID. `drive` reaps them after
  each journey.

## Proof and skip reporting

- Capture the action and the resulting state: the `observe` before, the action
  response, and a fresh `observe` after, all in `steps.jsonl` with images.
- A state change needs structured readback (`final == requested`,
  `final != initial`) plus an app-side side effect (output file, exit code,
  window gone).
- Non-interference proof: the launched app's environ shows the private
  `DISPLAY`, no `WAYLAND_DISPLAY`, and a non-user session bus. Its window is
  absent from every queryable host X display, and the user's daemon set is
  unchanged.
- Report each feature by its sub-feature ID and the entry point used.
- Report an unscripted or unreachable entry point as not verified, with the
  reason. Never count it as verified through a different path.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the
user-visible behavior. It then has exactly four H2 sections in this order:
`Sub-features`, `How to get to it (user POV)`, `Driving it with verify.sh`, and
`Gotchas`.

## Features

- [Isolated desktop and app launch](./isolated-desktop.md): private desktop
  lifecycle, `desktop_launch_app`, and non-interference with the user's session.
  Scripted (every app journey).
- [Semantic actions](./semantic-actions.md): `observe`, `desktop_set_value`,
  and `desktop_action` activate, with structured readback. Scripted
  (`--journey semantic`), passing.
- [Keyboard input](./keyboard-input.md): `desktop_keyboard` type_text and
  press_key. Scripted (`--journey keyboard`), currently failing on a known
  product defect.
- [Doctor and discovery](./doctor-and-discovery.md): `doctor`, `tools/list`,
  `list_resources`, and the operator CLI. Partly scripted (`--journey smoke`).
- [Pointer and capture](./pointer-and-capture.md): `desktop_pointer` and
  `capture_desktop`. Mapped, not yet scripted.
