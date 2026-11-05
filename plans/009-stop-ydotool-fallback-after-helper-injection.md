# Plan 009: Stop re-sending a key chord through ydotool after the privileged helper may already have injected it

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-linux/src/virtual_input.rs crates/sky-cua-input-helper/src/protocol.rs`
> (no `..HEAD`, so uncommitted edits count). If either file changed since
> this plan was written, compare the "Current state" excerpts against the
> live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts are verbatim unless marked abridged.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: 001 (so `sky-cua-linux` tests run in the gate; not a code dependency)
- **Category**: bug
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

`LinuxVirtualInput::press_key_sequence` tries the privileged uinput helper
first and, on any error, falls back to ydotool and re-sends the whole chord.
But the helper injects a batch before it writes its response, so a failure
reported after the send (a read timeout at the 3 s `HELPER_COMMAND_TIMEOUT`,
a closed socket, an unparsable reply, or an `ok: false` reply for a batch that
partially applied) can mean the keys already landed. The fallback then presses
them again: Enter submits twice, Ctrl+W closes two tabs, Ctrl+Z undoes two
steps. The sibling `type_text` path already solved exactly this with a
two-variant error (`HelperTypeError::BeforeInjection` / `AfterInjection`) and
only falls back on pre-send failures. `press_key_sequence` must do the same.

## Current state

`crates/sky-cua-linux/src/virtual_input.rs` (1,951 lines; inline tests from
about line 1710):

- Lines 64-67:

  ```rust
  enum HelperTypeError {
      BeforeInjection(BackendError),
      AfterInjection(BackendError),
  }
  ```

- The exemplar, `type_text` (lines 391-416), verbatim:

  ```rust
  pub fn type_text(&self, text: &str) -> Result<(), BackendError> {
      if let Some(socket_path) = self.probe.helper_socket_path.as_deref()
          && socket_is_connectable(socket_path)
      {
          match self.type_text_via_helper(socket_path, text) {
              Ok(()) => return Ok(()),
              // A partial prefix is already in the field; a ydotool retype of
              // the whole string would duplicate it, so surface the error.
              Err(HelperTypeError::AfterInjection(error)) => return Err(error),
              Err(HelperTypeError::BeforeInjection(error))
                  if self.probe.adapter != VirtualInputAdapterKind::Ydotool
                      || !self.probe.supports_keyboard() =>
              {
                  return Err(error);
              }
              Err(HelperTypeError::BeforeInjection(_)) => {}
          }
      }
      match self.probe.adapter {
          VirtualInputAdapterKind::PrivilegedHelper => Err(self.missing_helper_error()),
          VirtualInputAdapterKind::Ydotool => {
              self.require_keyboard_adapter()?;
              self.run_ydotool(type_text_args(text))
          }
      }
  }
  ```

  `type_text_via_helper` (lines 430-480) maps resolver failures to
  `BeforeInjection` and every `run_helper_command` failure to
  `AfterInjection`, with the comment at 456-460: "The helper injects a batch
  before it writes its response, so a failed send may already have applied
  that keystroke."

- The defect, `press_key_sequence` (lines 482-516), verbatim:

  ```rust
  pub fn press_key_sequence(&self, keys: &[String]) -> Result<(), BackendError> {
      if let Some(socket_path) = self.probe.helper_socket_path.as_deref()
          && socket_is_connectable(socket_path)
      {
          match LinuxKeyResolver::from_environment()
              .and_then(|resolver| resolver.key_sequence_events(keys))
              .and_then(|mut events| {
                  self.push_cosmic_input_flush(
                      &mut events,
                      desktop_is_cosmic(),
                      COSMIC_KEY_FLUSH_KEYCODE,
                  );
                  run_helper_command(socket_path, HelperCommand::KeyEvents { events })
              }) {
              Ok(()) => return Ok(()),
              Err(error)
                  if self.probe.adapter != VirtualInputAdapterKind::Ydotool
                      || !self.probe.supports_keyboard() =>
              {
                  return Err(error);
              }
              Err(_) => {}
          }
      }
      match self.probe.adapter {
          VirtualInputAdapterKind::PrivilegedHelper => Err(self.missing_helper_error()),
          VirtualInputAdapterKind::Ydotool => {
              self.require_keyboard_adapter()?;
              let events = key_sequence_events(keys)?;
              let mut args = vec!["key".to_string()];
              args.extend(events);
              self.run_ydotool(args)
          }
      }
  }
  ```

  The `and_then` chain collapses resolver errors and helper errors into one
  `BackendError`, and the final `Err(_) => {}` falls through to ydotool for
  both.

- `run_helper_command` (lines 851-865) returns `Err` for connect failure,
  write failure, read failure/timeout, parse failure, and `ok: false`
  replies (message `"privileged input helper failed: {code}: {message}"`).
  Connect failure happens before any send; everything else is after. The
  existing `type_text_via_helper` treats all of them as `AfterInjection` for
  simplicity, and this plan does the same.

- Helper wire types come from `sky_cua_input_helper::protocol` (imported at
  line 11): `HelperCommand::KeyEvents { events: Vec<KeyEventCommand> }`,
  `KeyEventCommand { code: u16, pressed: bool }`,
  `HelperResponse { ok: bool, capabilities: Option<..>, error: Option<HelperErrorInfo> }`,
  `HelperErrorInfo { code: String, message: String }`. Responses are one JSON
  line.

- `LinuxKeyResolver::from_environment()` (line 931) compiles an XKB keymap
  from `SKY_CUA_XKB_*`, then `XKB_DEFAULT_*`, then `setxkbmap -query` /
  `localectl status` (each with a 3 s timeout), then `evdev/pc105` defaults
  (`XkbNames::from_environment`, line 1067). No test calls it today, but the
  crate suite already requires XKB data: `portal/eis_keymap.rs:428-446`
  (`resolves_shift_modifier_from_xkb_level_masks`) compiles a US keymap with
  `.expect(...)`. Any runner that passes the existing `sky-cua-linux` suite
  has the data; the dev host does (`/usr/share/X11/xkb/rules/evdev`).

- `run_ydotool` (lines 560-565) is a thin wrapper over `run_ydotool_command`
  (starts line 1373), which spawns `probe.ydotool_path`; with a nonexistent
  path it returns
  `BackendError { code: ActionUnsupportedForEnvironment, message: "failed to start ydotool: ..." }`.
  This is how the regression test tells "fell back to ydotool" from "returned
  the helper error".

- Tests: 14 `#[test]`s in the inline module `mod tests` starting at line
  1707 (its `use` block at 1709-1721 imports `UnixListener`, `PathBuf` and a
  fixed `use super::{...}` list; it does NOT import `Path`,
  `LinuxKeyResolver`, `HelperTypeError`, `BackendError` or
  `BackendErrorCode`). Exemplar that builds a `LinuxVirtualInput` by hand
  with a `VirtualInputProbe` literal: `helper_without_ydotool_is_keyboard_only`
  (line 1819). The probe fields are
  `adapter, coordinate_plane, ydotool_path, socket_path, helper_socket_path, pointer_via_helper, desktop_bounds`.
  `ydotool_socket_selection_uses_connectable_later_candidate` (fn at 1797)
  shows the pattern for a temp dir plus `UnixListener::bind`.
  `supports_keyboard()` (lines 785-795) returns `true` for the Ydotool
  adapter when `ydotool_path` is `Some` and `socket_path` is `None`, so the
  fallback guard in `press_key_sequence` does not short-circuit the test
  probe below; before the fix the ydotool branch really runs.
  The nextest filter `virtual_input::tests` selects exactly these 14; the
  bare word `virtual_input` also matches 5 tests in other modules whose names
  contain it.

Conventions: `cargo nextest run`; conventional commits, e.g.
`fix(linux): make X11 display-liveness tests hermetic`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-linux --all-targets -- -D warnings` | exit 0 |
| Module tests | `cargo nextest run -p sky-cua-linux virtual_input::tests` | all pass (14 at `e928a40`, plus yours) |
| Crate tests | `cargo nextest run -p sky-cua-linux` | all pass (385 at `e928a40`) |

## Scope

**In scope** (the only file you should modify):
- `crates/sky-cua-linux/src/virtual_input.rs`

**Out of scope** (do NOT touch):
- `key_state` (line 518 onwards): it has no ydotool fallback, so it has no
  double-injection problem.
- `crates/sky-cua-input-helper/**`: making the helper release held keys on a
  partial batch is `CORRECTNESS-07`, a separate finding.
- The X11/XTest and portal keyboard paths (`x11/input_xtest.rs`,
  `portal/eis_input.rs`).
- Renaming `HelperTypeError`. It now covers key chords too; a rename is
  cosmetic churn across many lines.

## Git workflow

- Branch: `bex/helper-key-sequence-no-double-inject`
- Commit only if the operator authorised it; style:
  `fix(linux): never retype a key chord through ydotool after the helper may have injected it`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

Order matters so the regression test is seen failing before the fix:
Step 1 → Step 3 (add the test; confirm it FAILS) → Step 2 (fix; confirm it
PASSES). The `dead_code` warning after Step 1 is expected until Step 2.

### Step 1: Split the helper path into a classified helper

Add, directly above `press_key_sequence`:

```rust
/// Press `keys` through the privileged helper. Resolver failures happen
/// before any keystroke is sent and are eligible for the ydotool fallback;
/// anything after the request is written may already have been injected, so
/// it is `AfterInjection` and must not be retried through another device.
fn press_key_sequence_via_helper(
    &self,
    socket_path: &Path,
    keys: &[String],
) -> Result<(), HelperTypeError> {
    let resolver =
        LinuxKeyResolver::from_environment().map_err(HelperTypeError::BeforeInjection)?;
    let mut events = resolver
        .key_sequence_events(keys)
        .map_err(HelperTypeError::BeforeInjection)?;
    self.push_cosmic_input_flush(&mut events, desktop_is_cosmic(), COSMIC_KEY_FLUSH_KEYCODE);
    run_helper_command(socket_path, HelperCommand::KeyEvents { events })
        .map_err(HelperTypeError::AfterInjection)
}
```

**Verify**: `cargo check -p sky-cua-linux` compiles (a `dead_code` warning for
the new fn is expected until Step 2).

### Step 2: Make `press_key_sequence` mirror `type_text`

Replace the `if let Some(socket_path) ... { match ... }` block in
`press_key_sequence` (lines 483-505) with:

```rust
if let Some(socket_path) = self.probe.helper_socket_path.as_deref()
    && socket_is_connectable(socket_path)
{
    match self.press_key_sequence_via_helper(socket_path, keys) {
        Ok(()) => return Ok(()),
        // The chord may already have been injected; a ydotool resend would
        // press it twice (double Enter, two closed tabs), so surface the error.
        Err(HelperTypeError::AfterInjection(error)) => return Err(error),
        Err(HelperTypeError::BeforeInjection(error))
            if self.probe.adapter != VirtualInputAdapterKind::Ydotool
                || !self.probe.supports_keyboard() =>
        {
            return Err(error);
        }
        Err(HelperTypeError::BeforeInjection(_)) => {}
    }
}
```

Leave the trailing `match self.probe.adapter { ... }` (the ydotool path)
unchanged. Run `cargo fmt`.

**Verify**: `cargo clippy -p sky-cua-linux --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-linux virtual_input::tests` → 15 pass, including
`helper_failure_after_send_does_not_fall_back_to_ydotool` from Step 3.

### Step 3: Add a regression test with a fake helper (do this before Step 2)

In the inline test module, add a helper that serves one connection and
replies with a failure, then the test:

```rust
/// Bind a Unix socket, accept one connection, read the request line and
/// answer with an `ok: false` helper response. Mimics a helper that received
/// (and may have injected) the batch before reporting a failure.
fn spawn_failing_helper(socket_path: &Path) -> std::thread::JoinHandle<String> {
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path).unwrap();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut request = String::new();
        std::io::BufRead::read_line(&mut reader, &mut request).unwrap();
        let mut writer = stream;
        std::io::Write::write_all(
            &mut writer,
            b"{\"ok\":false,\"error\":{\"code\":\"uinput_error\",\"message\":\"simulated\"}}\n",
        )
        .unwrap();
        request
    })
}

#[test]
fn helper_failure_after_send_does_not_fall_back_to_ydotool() {
    // Without XKB keymap data the classification below cannot be reached;
    // fail loudly (with the real error) rather than pass vacuously.
    LinuxKeyResolver::from_environment()
        .expect("XKB keymap must compile on this host; see plan 009 STOP conditions");
    let socket_dir = std::env::temp_dir().join(format!(
        "sky-cua-helper-fallback-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&socket_dir).unwrap();
    let socket_path = socket_dir.join("helper.sock");
    let server = spawn_failing_helper(&socket_path);

    let input = LinuxVirtualInput {
        probe: VirtualInputProbe {
            adapter: VirtualInputAdapterKind::Ydotool,
            coordinate_plane: VirtualInputCoordinatePlane::DesktopLogical,
            ydotool_path: Some(PathBuf::from("/nonexistent/sky-cua-test-ydotool")),
            socket_path: None,
            helper_socket_path: Some(socket_path.clone()),
            pointer_via_helper: false,
            desktop_bounds: None,
        },
    };

    let error = input
        .press_key_sequence(&["Return".to_string()])
        .expect_err("helper failure must surface");
    let request = server.join().unwrap();
    assert!(
        request.contains("\"op\":\"key_events\""),
        "helper must have received the batch: {request}"
    );
    assert!(
        error.message.contains("privileged input helper failed"),
        "expected the helper error to be returned, not a ydotool fallback: {}",
        error.message
    );
    assert!(
        !error.message.contains("failed to start ydotool"),
        "chord was re-sent through ydotool after the helper may have injected it"
    );
    std::fs::remove_dir_all(socket_dir).unwrap();
}
```

`HelperCommand` is `#[serde(tag = "op", rename_all = "snake_case")]`, so the
request line contains `"op":"key_events"`; `uinput_error` is the code the
real helper uses for injection failures (`server.rs`,
`HelperResponse::error("uinput_error", ...)`).

In the test module's imports: change `use std::path::PathBuf;` to
`use std::path::{Path, PathBuf};` and add `LinuxKeyResolver` to the
`use super::{...}` list (it is a private item of the parent module, reachable
as `super::LinuxKeyResolver`). Nothing else is needed; `LinuxVirtualInput`,
`VirtualInputProbe`, `VirtualInputAdapterKind` and
`VirtualInputCoordinatePlane` are already imported (they are used by
`helper_without_ydotool_is_keyboard_only`).

**Verify** (before Step 2):
`cargo nextest run -p sky-cua-linux virtual_input::tests::helper_failure_after_send_does_not_fall_back_to_ydotool`
→ FAILS, and the failure message contains `failed to start ydotool` (the
ydotool fallback ran). If it passes here, STOP. If it hangs for more than
30 s, STOP (the client never connected to the fake socket).

## Test plan

- `helper_failure_after_send_does_not_fall_back_to_ydotool` (Step 3),
  observed to fail before Step 2 and pass after.
- Existing 14 `virtual_input::tests` tests unchanged.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-linux --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-linux` exits 0 with the 1 new test (386 at `e928a40` + 1)
- [ ] `grep -n 'fn press_key_sequence_via_helper' crates/sky-cua-linux/src/virtual_input.rs` matches
- [ ] `sed -n '/pub fn press_key_sequence/,/pub fn key_state/p' crates/sky-cua-linux/src/virtual_input.rs | grep -c 'Err(_) => {}'` prints `0` (grep's exit status 1 is expected when the count is 0; do not chain it with `&&`)
- [ ] `git status --porcelain -- crates/` lists only `crates/sky-cua-linux/src/virtual_input.rs` (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- `LinuxKeyResolver::from_environment()` fails on the development host (the
  Step 3 test panics with the plan-009 message and the real error). Report
  the error text; the maintainer decides between installing
  xkeyboard-config on the runner or gating the test.
- The Step 3 test passes before Step 2 is applied (then it is not exercising
  the fallback; report rather than weaken).
- The Step 3 test hangs for more than 30 s (the client never connected to
  the fake socket, so `server.join()` blocks; `.config/nextest.toml` sets no
  per-test timeout).
- Any existing `virtual_input::tests` test fails.

## Maintenance notes

- Connect failures (`UnixStream::connect` in `exchange_helper_command`) are
  classified `AfterInjection` here, as in `type_text_via_helper`, even though
  nothing was sent. That is conservative: with the helper configured but
  unreachable, the chord is reported as failed instead of silently going
  through ydotool. If that proves too strict in the field, split
  `run_helper_command` into connect and exchange phases and classify connect
  failures as `BeforeInjection`; do it for both call sites at once.
- `key_state` (hold/release a single key) uses `?` without fallback and
  needs no change. If a fallback is ever added there, it must follow the same
  before/after split.
- Helper-side `ok: false` replies that arrive before any injection
  (`invalid_json`, `unsupported_version`) are also classified
  `AfterInjection`. Conservative and acceptable: a protocol mismatch with the
  installed helper should surface, not silently route through ydotool.
- The Ydotool-adapter probe always sets `helper_socket_path` (default
  `/run/sky-cua/input-helper.sock`) even when the helper is not running
  (lines 751-759), and `socket_is_connectable` only checks the path is a
  socket file (1493-1496). After this change a stale socket file turns
  `press_key_sequence` from "silently uses ydotool" into a hard error, the
  same behaviour `type_text` already has. `RuntimeDirectory=sky-cua` and the
  helper's startup `remove_file` make this rare.
- The fake helper's reply shape (`ok`, `error.code`, `error.message`)
  deserializes into `HelperResponse` (no `deny_unknown_fields`; `capabilities`
  is optional). The test does not clean its socket directory on a failed
  assertion; each nextest process has its own pid, so leftovers are harmless
  but a guard struct would be tidier if the suite starts leaving temp dirs.
