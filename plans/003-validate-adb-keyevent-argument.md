# Plan 003: Validate the `adb shell input keyevent` argument so model-supplied key names cannot reach the device shell

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-service/src/phone/adb.rs crates/sky-cua-service/src/phone/adb/tests.rs crates/sky-cua-service/src/phone/command.rs crates/sky-cua-service/src/phone/manager/routing.rs crates/sky-cua-client/src/mcp_tools/phone/response.rs`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts are abridged where marked; formatting-only differences
> are not drift.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: security
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

`adb shell <args...>` joins its arguments into one string and runs it through
the device's `sh -c`. Every other ADB wrapper in
`crates/sky-cua-service/src/phone/adb.rs` single-quotes untrusted text with
`single_quote_for_shell` for exactly this reason (the comment at lines 734-736
documents it). `input_keyevent` is the one wrapper that does not: it passes
the model-supplied `key_code` through `normalize_keyevent`, which only trims,
upper-cases and prefixes `KEYCODE_`, and sends the result as a bare shell
word. The MCP schemas for `phone_key_event.key_code` (`minLength: 1`) and
`phone_keyboard.key` with `operation=press_key` (non-blank string,
`definitions/phone.rs:403-407`) enforce nothing further. So a key name containing `;`, `|`, `&`, `$(...)` or backticks,
whether produced by the model or planted through prompt injection in page or
notification text, runs as a command on the phone as the adb shell user.
Upper-casing makes a payload awkward, not impossible. The fix is a strict
allow-list on the normalized token, which also gives the model a clear
structured error instead of an opaque adb failure.

## Current state

- `crates/sky-cua-service/src/phone/adb.rs`
  - Lines 517-530 (doc comment 517-518, fn 519-530):

    ```rust
    pub(super) async fn input_keyevent(
        runner: &dyn CommandRunner,
        configured_adb_path: Option<&str>,
        serial: &str,
        key: &str,
    ) -> Result<InputOutcome, CommandError> {
        let adb = resolve_adb_path(configured_adb_path);
        let normalized = normalize_keyevent(key);
        let argv = serial_args(serial, &["shell", "input", "keyevent", &normalized]);
        let output = runner.run(&adb, &argv).await?;
        Ok(input_outcome(&adb, &argv, &output))
    }
    ```

  - Lines 555-566 (verbatim):

    ```rust
    pub(super) fn normalize_keyevent(key: &str) -> String {
        let trimmed = key.trim();
        if trimmed.chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty() {
            return trimmed.to_string();
        }
        let upper = trimmed.to_ascii_uppercase();
        if upper.starts_with("KEYCODE_") {
            upper
        } else {
            format!("KEYCODE_{upper}")
        }
    }
    ```

  - For contrast, `input_text` at line 511:
    `let command = format!("input text {}", single_quote_for_shell(text));`

  - Three call sites pass model input straight in:
    `crates/sky-cua-service/src/phone/manager/routing.rs:232-237` (`&request.key`,
    reached from `phone_keyboard` `press_key`), `:457-462` and `:493-498`
    (`&request.key_code`, from `phone_key_event`). Each maps the error with
    `adb::command_error_diagnostic("adb shell input keyevent", &error)`. The
    companion-direct lane also forwards the raw key (`routing.rs:209-216`,
    `:422-433`) but the companion resolves it with `KeyEvent.keyCodeFromString`
    (`SkyAccessibilityService.kt:705-707`), no shell involved, so only the ADB
    path needs the check. `resolve_adb_path` returns `String`
    (`command.rs:159`).

- `crates/sky-cua-service/src/phone/command.rs`: the `CommandError` enum
  (lines 70-86) has variants `NotImplemented { program }`,
  `Spawn { program, message }`, `Timeout { program, timeout_ms }`; `code()`
  (lines 88-97) maps them to `"PhoneCommandNotImplemented"`,
  `"PhoneCommandSpawnFailed"`, `"PhoneCommandTimedOut"`; and
  `impl std::fmt::Display for CommandError` (lines 99-117) is hand-written
  with an exhaustive `match` (no `thiserror`). These are the only two
  exhaustive matches on `CommandError` in the workspace.
  `command_error_diagnostic` (adb.rs lines 74-83) turns `error.code()` into a
  `DiagnosticEntry.code`, so a new variant gets a structured code for free.

- `crates/sky-cua-client/src/mcp_tools/phone/response.rs`: the client
  classifies phone diagnostic codes. `phone_diagnostic_is_error_code`
  (lines 45-108) lists `| "PhoneCommandTimedOut"` at line 54, and the test
  `every_documented_code_is_classified_deliberately` (doc comment at 736-746,
  `error_codes` array from ~748) contains `"PhoneCommandTimedOut",` at line
  755. Its doc comment says every new `Phone*` code introduced in the service
  MUST be added to both. Behaviour does not depend on it today (failed actions
  are reported with `backend: PhoneBackendKind::None`, which the client
  already treats as an error), but the repo's own rule requires the entry.

- `crates/sky-cua-service/src/phone/adb/tests.rs`
  - Imports: `use super::*;` and
    `use crate::phone::command::{CommandError, CommandOutput, FakeCommandRunner};`
  - `normalize_keyevent_forms` (lines 400-405) pins the current normalization:
    `"4"` → `"4"`, `"back"` → `"KEYCODE_BACK"`, `"KEYCODE_HOME"` unchanged,
    `"keycode_enter"` → `"KEYCODE_ENTER"`. Keep these passing.
  - `input_text_passes_single_quoted_command_argument` (lines 378-397) is the
    exemplar for driving a wrapper through `FakeCommandRunner::new()`,
    `runner.set_stdout(program, &argv, stdout)` and
    `runner.recorded_calls()` (a `Vec<String>` of joined command lines).

Conventions: structured error codes, never prose matching
(`crates/sky-cua-service/AGENTS.md`). Commit style is conventional, for example
`fix(phone): allow collapsible_if for node_action fence`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-service --all-targets -- -D warnings` | exit 0 |
| ADB tests | `cargo nextest run -p sky-cua-service phone::adb` | all pass |
| Manager tests | `cargo nextest run -p sky-cua-service phone::manager` | all pass |
| Crate tests | `cargo nextest run -p sky-cua-service` | all pass |

## Scope

**In scope** (the only files you should modify):
- `crates/sky-cua-service/src/phone/adb.rs`
- `crates/sky-cua-service/src/phone/adb/tests.rs`
- `crates/sky-cua-service/src/phone/command.rs` (one new `CommandError` variant, its `code()` arm, its `Display` arm)
- `crates/sky-cua-client/src/mcp_tools/phone/response.rs` (register the new diagnostic code only)

**Out of scope** (do NOT touch):
- `crates/sky-cua-client/src/mcp_tools/definitions/phone.rs` and the MCP
  fixtures under `crates/sky-cua-client/tests/fixtures/`. Tightening the JSON
  schema is a public-contract change that regenerates two large snapshot
  files; the service-side check is sufficient and must exist regardless.
- `routing.rs` call sites. They already map `CommandError` to a diagnostic.
- Other ADB wrappers; they already quote.
- The Android companion.

## Git workflow

- Branch: `bex/adb-keyevent-allowlist`
- Commit only if the operator authorised it; style:
  `fix(phone): reject keyevent names outside the KEYCODE allow-list`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Add an `InvalidArgument` error variant

In `command.rs`, add to `CommandError`:

```rust
/// The caller supplied an argument that must not be forwarded to the
/// program (for example a keycode token containing shell metacharacters).
InvalidArgument { program: String, message: String },
```

and to `code()`:

```rust
CommandError::InvalidArgument { .. } => "PhoneCommandInvalidArgument",
```

`CommandError` implements `Display` by hand (lines 99-117, exhaustive
`match`), so without a new arm the crate fails to compile (E0004). Add after
the `Timeout` arm:

```rust
CommandError::InvalidArgument { program, message } => {
    write!(f, "refused to run `{program}`: {message}")
}
```

(Not "rejected argument": the final diagnostic reads
"`adb shell input keyevent` could not run: refused to run `adb`: ...", and
adb never ran.)

**Verify**: `cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0.

### Step 1b: Register the code with the client's diagnostic classifier

In `crates/sky-cua-client/src/mcp_tools/phone/response.rs`:
- in `phone_diagnostic_is_error_code`, add `| "PhoneCommandInvalidArgument"`
  directly after `| "PhoneCommandTimedOut"` (line 54);
- in the `error_codes` array inside
  `every_documented_code_is_classified_deliberately`, add
  `"PhoneCommandInvalidArgument",` directly after `"PhoneCommandTimedOut",`
  (line 755).

**Verify**: `cargo nextest run -p sky-cua-client every_documented_code_is_classified_deliberately` → passes.

### Step 2: Validate the normalized token in `input_keyevent`

In `adb.rs`, add next to `normalize_keyevent`:

```rust
/// `adb shell input keyevent` accepts either a decimal keycode or a
/// `KEYCODE_*` name. Anything else is refused before it can reach the device
/// shell, which joins and re-parses its arguments.
pub(super) fn is_valid_keyevent_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if token.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    token
        .strip_prefix("KEYCODE_")
        .is_some_and(|rest| {
            !rest.is_empty()
                && rest.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        })
}
```

Then in `input_keyevent`, after `let normalized = normalize_keyevent(key);`:

```rust
if !is_valid_keyevent_token(&normalized) {
    return Err(CommandError::InvalidArgument {
        program: adb,
        message: format!("key {key:?} is not a KEYCODE_* name or a decimal keycode"),
    });
}
```

(`adb` is a `String` from `resolve_adb_path`; the function returns right
after, so no clone is needed. The message says "key" because one caller's
field is `key` and the other's is `key_code`.)

Do not change the argv shape (`["shell", "input", "keyevent", &normalized]`);
existing recorded-call assertions depend on it, and with the allow-list no
quoting is needed.

**Verify**: `cargo nextest run -p sky-cua-service phone::adb` → all existing
tests pass (`normalize_keyevent_forms` unchanged).

### Step 3: Add tests

In `adb/tests.rs`:

1. `keyevent_token_allowlist` (plain `#[test]`): assert `true` for
   `"4"`, `"KEYCODE_BACK"`, `"KEYCODE_DPAD_UP"`, `"KEYCODE_F12"`; assert
   `false` for `""`, `"KEYCODE_"`, `"KEYCODE_BACK; ID"`, `"KEYCODE_$(ID)"`,
   `"KEYCODE_A|B"`, `"KEYCODE_a"` (lowercase after the prefix never survives
   normalization, but the validator must still refuse it), `"4;5"`.

2. `input_keyevent_rejects_shell_metacharacters` (`#[tokio::test]`):

   ```rust
   let runner = FakeCommandRunner::new();
   for input in ["back; id", "$(id)", "back|id", "`id`"] {
       let result = input_keyevent(&runner, None, "S", input).await;
       assert!(matches!(result, Err(CommandError::InvalidArgument { .. })), "{input:?}");
   }
   assert!(runner.recorded_calls().is_empty());
   ```

   (`recorded_calls()` is populated inside `run`, and the wrapper returns
   before calling it, so the list stays empty.)

3. `input_keyevent_passes_valid_tokens_unquoted` (`#[tokio::test]`): set
   `runner.set_stdout("adb", &["-s", "S", "shell", "input", "keyevent", "KEYCODE_BACK"], "")`,
   call with `"back"`, assert `Ok` and that `recorded_calls()` contains
   `"adb -s S shell input keyevent KEYCODE_BACK"` (mirror the string-matching
   style of `input_text_passes_single_quoted_command_argument`).

Run `cargo fmt` after editing; the Step 2 snippet is not guaranteed to be
rustfmt-shaped (the `strip_prefix`/`is_some_and` chain and the long `bytes().all(...)` line).

**Verify**: `cargo nextest run -p sky-cua-service phone::adb` → all pass
including the 3 new tests.

### Step 4: Confirm the diagnostic code surfaces through the manager

Run `cargo nextest run -p sky-cua-service phone::manager` to confirm no
manager test depended on an invalid key reaching the runner. Then grep the
manager tests for key-event cases:
`grep -n 'KeyEvent\|key_code' crates/sky-cua-service/src/phone/manager/tests.rs | head`.
If none exist (true at `e928a40`), that is expected; adding manager-level key
event coverage is a separate audit finding (`TESTS-09`) and out of scope.

**Verify**: `cargo nextest run -p sky-cua-service` → all pass.

## Test plan

- Unit: `keyevent_token_allowlist`, `input_keyevent_rejects_shell_metacharacters`,
  `input_keyevent_passes_valid_tokens_unquoted` in `adb/tests.rs`.
- Pattern: `input_text_passes_single_quoted_command_argument` in the same file.
- Verification: `cargo nextest run -p sky-cua-service` → all pass, 3 new tests.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-service --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-service` exits 0 and includes the 3 new tests
- [ ] `cargo nextest run -p sky-cua-client every_documented_code_is_classified_deliberately` passes
- [ ] `grep -n 'PhoneCommandInvalidArgument' crates/sky-cua-service/src/phone/command.rs crates/sky-cua-client/src/mcp_tools/phone/response.rs` matches in both files (two matches in `response.rs`)
- [ ] `grep -n 'is_valid_keyevent_token' crates/sky-cua-service/src/phone/adb.rs` shows the definition and its use inside `input_keyevent`
- [ ] `git status --porcelain -- crates/` lists exactly the four in-scope files (untracked `plans/*.md` files are expected and excluded by the path filter)
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- `CommandError` is matched exhaustively somewhere outside the in-scope files
  and the new variant breaks compilation there. Report the file; do not add
  wildcard arms in files outside scope.
- A manager test fails because it sent an invalid key name on purpose
  (report the test; the maintainer decides whether the test or the allow-list
  is right).
- Any `recorded_calls()` assertion elsewhere relies on keyevent quoting.

## Maintenance notes

- The allow-list intentionally mirrors what Android's `input keyevent`
  accepts (`KeyEvent.keyCodeFromString` names and integers). If a future
  companion-direct key path accepts other tokens, validate it separately; do
  not loosen this one.
- A reviewer should confirm the new `DiagnosticEntry.code`
  (`PhoneCommandInvalidArgument`) is acceptable as a stable public code; it
  will appear in `phone_key_event` and `phone_keyboard` tool responses.
- Behaviour change: multi-token input such as `"back home"` previously worked
  by accident (normalized to `KEYCODE_BACK HOME`, which the device shell
  split into two words that `input keyevent` accepted) and is now refused.
  Flags such as `--longpress` never worked through `normalize_keyevent`
  (they became `KEYCODE_--LONGPRESS`), so nothing is lost there.
- On the companion-direct lane `phone_key_event` makes one direct round trip
  before the ADB fallback rejects a bad key, and the merged diagnostic carries
  the new code. Acceptable; the direct lane never touches a shell.
- Optional follow-up outside this plan: add a `pattern` to the
  `phone_key_event.key_code` JSON schema so hosts reject bad input before the
  round trip. That touches the MCP contract fixtures and belongs in its own
  change.
