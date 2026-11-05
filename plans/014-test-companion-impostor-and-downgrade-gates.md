# Plan 014: Test the phone companion impostor and downgrade gates, which decide whether a same-name APK is trusted

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report. Do not improvise. When done, update the status row for this plan
> in `plans/README.md` unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**:
> `git diff --stat e928a40 -- crates/sky-cua-service/src/phone/protocol/identity.rs crates/sky-cua-service/src/phone/manager/companion_lane.rs crates/sky-cua-service/src/phone/manager/companion_probe.rs crates/sky-cua-service/src/phone/manager/tests.rs`
> (no `..HEAD`, so uncommitted edits count). If any in-scope file changed
> since this plan was written, compare the "Current state" excerpts against
> the live code before proceeding; on a semantic mismatch, treat it as a STOP
> condition. Excerpts are verbatim unless marked abridged.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests
- **Planned at**: commit `e928a40`, 2026-11-03

## Why this matters

When the phone lane bootstraps the companion app over ADB (the default lane:
`direct_enabled` resolves to `false` unless configured), the host decides
whether to trust, update, or refuse an already-installed package with the
same name. `decide_install` in `phone/protocol/identity.rs` returns
`RefuseSignatureMismatch` when the installed signing certificate differs
from the packaged one (an impostor) and `RefuseDowngrade` when the installed
version is newer and downgrades are not allowed. `companion_lane.rs` then
refuses to trust or replace an impostor. No test references
`RefuseSignatureMismatch`, `RefuseDowngrade`, `decide_install`, `cert_eq`,
or `parse_companion_metadata`; the manager tests only cover the matching-cert
and not-installed cases. If the match-arm order, the colon/case
normalisation, or the metadata key name regresses, the daemon would trust a
repackaged app that then receives full device-control capabilities, and no
test would fail. The functions are pure and the manager test harness already
scripts `dumpsys package` output, so this is cheap to close.

## Current state

`crates/sky-cua-service/src/phone/protocol/identity.rs` (257 lines; line 1
is `#![cfg_attr(not(test), expect(dead_code))]`, no `#[cfg(test)]` module):

- Types (lines 6-47): `InstalledCompanion { version_name: Option<String>, version_code: Option<u64>, cert_sha256: Option<String> }`
  (derives `Default`); `ExpectedCompanion { package_name, version_name, version_code, cert_sha256, apk_sha256, apk_path: String, allow_downgrade: bool }`
  (derives `Default`); `CompanionInstallDecision { Install, Update { reason }, UpToDate, RefuseSignatureMismatch { installed_cert, expected_cert }, RefuseDowngrade { installed_version_code, expected_version_code } }`
  with `code()` → `"CompanionInstall" | "CompanionUpdate" | "CompanionUpToDate" | "CompanionSignatureMismatch" | "CompanionDowngradeBlocked"`.
- Lines 68-107 (verbatim; `cert_eq` follows at 109-117, `certs_match` at 119-121):

  ```rust
  pub(crate) fn decide_install(
      installed: Option<&InstalledCompanion>,
      expected: &ExpectedCompanion,
      _allow_downgrade_override: Option<bool>,
  ) -> CompanionInstallDecision {
      let Some(installed) = installed else {
          return CompanionInstallDecision::Install;
      };
      match (
          installed.cert_sha256.as_ref(),
          expected.cert_sha256.as_ref(),
      ) {
          (Some(installed_cert), Some(expected_cert)) if !cert_eq(installed_cert, expected_cert) => {
              return CompanionInstallDecision::RefuseSignatureMismatch {
                  installed_cert: installed_cert.clone(),
                  expected_cert: expected_cert.clone(),
              };
          }
          _ => {}
      }
      let allow_downgrade = _allow_downgrade_override.unwrap_or(expected.allow_downgrade);
      if let (Some(installed_code), Some(expected_code)) =
          (installed.version_code, expected.version_code)
      {
          if installed_code > expected_code && !allow_downgrade {
              return CompanionInstallDecision::RefuseDowngrade {
                  installed_version_code: installed_code,
                  expected_version_code: expected_code,
              };
          }
          if installed_code < expected_code {
              return CompanionInstallDecision::Update {
                  reason: format!(
                      "installed version_code {installed_code} < expected {expected_code}"
                  ),
              };
          }
      }
      CompanionInstallDecision::UpToDate
  }

  fn cert_eq(left: &str, right: &str) -> bool {
      let normalize = |s: &str| {
          s.chars()
              .filter(|c| *c != ':')
              .flat_map(|c| c.to_lowercase())
              .collect::<String>()
      };
      normalize(left) == normalize(right)
  }
  ```

  Note the semantics worth pinning: a downgrade with `allow_downgrade` true
  falls through to `UpToDate` (not `Update`), because the later
  `installed_code < expected_code` check is false; `force_reinstall` is
  handled by the caller, not here.

- Lines 131-164: `metadata_path_for_apk(apk_path)` → same path with `.json`
  extension; `load_companion_metadata` reads it; `parse_companion_metadata(text)`
  (private) reads JSON keys `version_name` (string), `version_code` (u64),
  `signing_cert_sha256` and `apk_sha256` (strings, trimmed, lower-cased,
  empty → `None`); invalid JSON → `CompanionMetadata::default()`.

`crates/sky-cua-service/src/phone/manager/companion_lane.rs`:

- Lines 206-224: when `self.selection.direct_enabled` the whole legacy
  bootstrap is skipped (`PhoneCompanionRpcRemoved`). The gate below runs only
  on the ADB lane, which is the default (`config.rs:743-745`:
  `direct_enabled: env_bool(...).or(phone.direct_enabled).unwrap_or(false)`).
- Lines 236-252: `expected.cert_sha256` is `selection.companion_expected_cert_sha256`
  or the sidecar metadata's cert; lines 258-260: `installed = self.read_installed_companion(serial, &package).await`
  then `decide_install(installed.as_ref(), &expected, options.allow_downgrade)`.
  Line 261: `force_reinstall` turns `UpToDate` into `Update`; it leaves
  `RefuseDowngrade` alone. The install block (269-335) runs only when
  `decision.requires_install()`, so neither refusal ever installs.
- There is NO diagnostic for `RefuseDowngrade` on the connect path: decision
  codes surface only through `install_outcome_diagnostic` (when an install
  runs) or the signature-mismatch branch below. A blocked downgrade is
  observable only indirectly: `force_reinstall` does not turn it into an
  install.
- `companion_operator_mode` defaults to `true` (`config.rs:718-720`), so on
  a non-impostor connect the permission-grant path runs (`settings get secure
  enabled_accessibility_services`, `cmd notification allow_listener ...`),
  followed by `adb forward` and the `.SetupActivity` token intent. The
  impostor early return (line 359) is the only thing preventing those.
- Lines 337-360 (abridged): if the decision is `RefuseSignatureMismatch`, push
  `DiagnosticEntry { code: decision.code(), message: "refusing to trust a same-name companion package whose signing certificate does not match the packaged companion; explicit operator reinstall required", .. }`,
  set `profile.companion = protocol::capabilities_unreachable(&package, installed, expected_cert, expected_apk_sha256, false)`
  (the trailing `false` is `signature_matches_expected`; the generic
  unreachable path at line 532 passes `true`) and `return (None, diagnostics)`
  before any permission grant. Bootstrap diagnostics are stored on the session
  entry (`lifecycle.rs:354`, `companion_diagnostics`) and exposed to tests
  via `PhoneManager::companion_bootstrap_diagnostics(&self, session_id)`
  (`companion_probe.rs:359`, `pub(crate)`); the `PhoneSession` returned by
  `connect` has no `diagnostics` field.

`crates/sky-cua-service/src/phone/manager/companion_probe.rs`:

- `read_installed_companion` (lines 163-185): `pm path <pkg>` must succeed
  and contain `package:`; then `dumpsys package <pkg>` is parsed by
  `parse_installed_companion` (line 445). `parse_signing_cert_sha256` (486)
  takes any line mentioning sha-256/cert/signature/signing and
  `extract_hex64` (508) strips `:` separators, lower-cases, and keeps a run
  only if it is exactly 64 nibbles, so a colon-separated uppercase digest is
  accepted. Those parsers already have unit tests (lines 545-564).

`crates/sky-cua-service/src/phone/manager/tests.rs`:

- Line 31 `const SERIAL: &str = "emulator-5554";`; line 33
  `const COMPANION_CERT_SHA256: &str = ...` (a hex digest used as the
  expected cert).
- Helpers: `script_device_probes(&runner)` (lines 529-616) scripts the
  device probes and reports both secure settings lists as already populated;
  `script_verified_installed_companion(&runner, package, version_code, version_name)`
  (lines 618-636) scripts `pm path` → `package:/data/app/companion.apk` and
  `dumpsys package` → `versionCode=..\nversionName=..\nSHA-256 cert digest: {COMPANION_CERT_SHA256}\n`;
  `script_installed_companion_without_cert(...)` (line 638) omits the
  digest line; `companion_manager()` (lines 662-674) builds a
  companion-enabled, auto-install manager over a `FakeCommandRunner` with the
  package reported as not installed; `connect(&mut manager).await` (line
  1427) opens a session and returns a `PhoneSession` with
  `session.capability_profile`. `manager.selection` is readable from tests
  (used at line 1530). `FakeCommandRunner::run` records every call as
  `program + " " + args.join(" ")` (`command.rs:294-301`) BEFORE looking up a
  scripted result (`command.rs:357-361`), so unscripted commands still show
  up in `recorded_calls()`.
- Manager tests resolve no companion sidecar by default: the default
  `companion_apk_path` is relative (`resources/android/phone-companion.apk`)
  and nextest runs in the crate directory, so `expected.version_code` is
  `None` unless `companion_apk_path` is pointed at a sidecar. The only
  `_for_tests` setter is `set_companion_apk_sha256_for_tests`
  (`test_support.rs:99`). `SKY_CUA_REPO_ROOT` is assumed unset.
- Exemplar that sets the expected cert and connects:
  lines 2240-2248 (`script_verified_installed_companion(&runner, &package, 2, "0.1.1")`,
  `selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string())`,
  `PhoneManager::with_runner(runner.clone(), selection)`, `connect`).
  `explicit_install_companion_honors_force_reinstall_and_allow_downgrade`
  (4161-4201) is the positive control for the downgrade test below: it shows
  `force_reinstall: true` producing a `CompanionUpdate` diagnostic and an
  `adb -s emulator-5554 install -r -d <apk>` call when the decision is
  `UpToDate`.
- No test contains `CompanionSignatureMismatch` or `CompanionDowngradeBlocked`.

Conventions: `cargo nextest run`; conventional commits, e.g.
`fix(phone): allow collapsible_if for node_action fence`.

## Commands you will need

| Purpose | Command | Expected on success |
|---------|---------|---------------------|
| Format | `cargo fmt --check` | exit 0 |
| Lint | `cargo clippy -p sky-cua-service --all-targets -- -D warnings` | exit 0 |
| Identity tests | `cargo nextest run -p sky-cua-service phone::protocol::identity` | new tests pass |
| Manager tests | `cargo nextest run -p sky-cua-service phone::manager::tests` | all pass |
| Crate tests | `cargo nextest run -p sky-cua-service` | all pass; count is the pre-change `cargo nextest list -p sky-cua-service` total plus 12 |

## Scope

**In scope** (the only files you should modify):
- `crates/sky-cua-service/src/phone/protocol/identity.rs` (add a `#[cfg(test)] mod tests` only)
- `crates/sky-cua-service/src/phone/manager/tests.rs` (add tests)
- `plans/README.md` (status row only)

**Out of scope** (do NOT touch):
- Any production logic in `identity.rs`, `companion_lane.rs`,
  `companion_probe.rs`. If a test exposes a behaviour you think is wrong,
  pin the current behaviour and report it; do not fix it here.
- The direct-lane bootstrap (`direct_enabled == true` path).
- The module-level `#![cfg_attr(not(test), expect(dead_code))]` on
  `identity.rs:1`.

## Git workflow

- Branch: `bex/companion-identity-gate-tests`
- Commit only if the operator authorised it; style:
  `test(phone): cover the companion impostor and downgrade install gates`
- Do NOT push or open a PR unless the operator instructed it.

## Steps

### Step 1: Table-driven unit tests for `decide_install` and `cert_eq`

Append to `identity.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const CERT_A: &str = "aabbccdd";
    const CERT_A_COLON_UPPER: &str = "AA:BB:CC:DD";
    const CERT_B: &str = "11223344";

    fn installed(version_code: Option<u64>, cert: Option<&str>) -> InstalledCompanion {
        InstalledCompanion {
            version_name: None,
            version_code,
            cert_sha256: cert.map(str::to_string),
        }
    }

    fn expected(version_code: Option<u64>, cert: Option<&str>, allow_downgrade: bool) -> ExpectedCompanion {
        ExpectedCompanion {
            package_name: "com.skycua.phonecompanion".into(),
            version_code,
            cert_sha256: cert.map(str::to_string),
            allow_downgrade,
            ..ExpectedCompanion::default()
        }
    }

    #[test]
    fn not_installed_is_install() {
        assert_eq!(
            decide_install(None, &expected(Some(3), Some(CERT_A), false), None),
            CompanionInstallDecision::Install
        );
    }

    #[test]
    fn mismatched_cert_is_refused_before_any_version_logic() {
        // Even an older installed version is refused when its cert differs.
        let decision = decide_install(
            Some(&installed(Some(1), Some(CERT_B))),
            &expected(Some(3), Some(CERT_A), false),
            None,
        );
        assert_eq!(
            decision,
            CompanionInstallDecision::RefuseSignatureMismatch {
                installed_cert: CERT_B.into(),
                expected_cert: CERT_A.into(),
            }
        );
        assert_eq!(decision.code(), "CompanionSignatureMismatch");
        assert!(!decision.requires_install());
    }

    #[test]
    fn cert_comparison_ignores_colons_and_case() {
        assert!(certs_match(CERT_A, CERT_A_COLON_UPPER));
        let decision = decide_install(
            Some(&installed(Some(3), Some(CERT_A_COLON_UPPER))),
            &expected(Some(3), Some(CERT_A), false),
            None,
        );
        assert_eq!(decision, CompanionInstallDecision::UpToDate);
    }

    #[test]
    fn unreadable_installed_cert_is_not_a_mismatch() {
        let decision = decide_install(
            Some(&installed(Some(3), None)),
            &expected(Some(3), Some(CERT_A), false),
            None,
        );
        assert_eq!(decision, CompanionInstallDecision::UpToDate);
    }

    #[test]
    fn no_expected_cert_skips_the_signature_gate() {
        let decision = decide_install(
            Some(&installed(Some(3), Some(CERT_B))),
            &expected(Some(3), None, false),
            None,
        );
        assert_eq!(decision, CompanionInstallDecision::UpToDate);
    }

    #[test]
    fn newer_installed_version_is_a_blocked_downgrade_by_default() {
        let decision = decide_install(
            Some(&installed(Some(5), Some(CERT_A))),
            &expected(Some(3), Some(CERT_A), false),
            None,
        );
        assert_eq!(
            decision,
            CompanionInstallDecision::RefuseDowngrade {
                installed_version_code: 5,
                expected_version_code: 3,
            }
        );
        assert_eq!(decision.code(), "CompanionDowngradeBlocked");
    }

    #[test]
    fn downgrade_allowed_by_expected_or_override_falls_through_to_up_to_date() {
        // Current behaviour: an allowed downgrade is not an Update; the caller
        // decides whether to reinstall (force_reinstall). Pinned on purpose.
        let newer = installed(Some(5), Some(CERT_A));
        assert_eq!(
            decide_install(Some(&newer), &expected(Some(3), Some(CERT_A), true), None),
            CompanionInstallDecision::UpToDate
        );
        assert_eq!(
            decide_install(Some(&newer), &expected(Some(3), Some(CERT_A), false), Some(true)),
            CompanionInstallDecision::UpToDate
        );
        // The override can also turn an allowed downgrade back into a refusal.
        assert!(matches!(
            decide_install(Some(&newer), &expected(Some(3), Some(CERT_A), true), Some(false)),
            CompanionInstallDecision::RefuseDowngrade { .. }
        ));
    }

    #[test]
    fn older_installed_version_is_an_update() {
        let decision = decide_install(
            Some(&installed(Some(2), Some(CERT_A))),
            &expected(Some(3), Some(CERT_A), false),
            None,
        );
        assert!(matches!(decision, CompanionInstallDecision::Update { .. }));
        assert!(decision.requires_install());
    }

    #[test]
    fn missing_version_codes_are_up_to_date() {
        assert_eq!(
            decide_install(Some(&installed(None, Some(CERT_A))), &expected(Some(3), Some(CERT_A), false), None),
            CompanionInstallDecision::UpToDate
        );
        assert_eq!(
            decide_install(Some(&installed(Some(3), Some(CERT_A))), &expected(None, Some(CERT_A), false), None),
            CompanionInstallDecision::UpToDate
        );
    }

    #[test]
    fn companion_metadata_parses_sidecar_keys_and_normalises_hex() {
        let parsed = parse_companion_metadata(
            r#"{"version_name":"0.1.2","version_code":3,"signing_cert_sha256":" AABB ","apk_sha256":"CCDD"}"#,
        );
        assert_eq!(parsed.version_name.as_deref(), Some("0.1.2"));
        assert_eq!(parsed.version_code, Some(3));
        assert_eq!(parsed.cert_sha256.as_deref(), Some("aabb"));
        assert_eq!(parsed.apk_sha256.as_deref(), Some("ccdd"));
        assert_eq!(parse_companion_metadata("not json"), CompanionMetadata::default());
        assert_eq!(
            parse_companion_metadata(r#"{"signing_cert_sha256":""}"#).cert_sha256,
            None
        );
        assert_eq!(
            metadata_path_for_apk("resources/android/phone-companion.apk"),
            PathBuf::from("resources/android/phone-companion.json")
        );
    }
}
```

`ExpectedCompanion` derives `Default`, so the struct-update syntax compiles.
Run `cargo fmt` after adding (long lines above will be reflowed).

**Verify**: `cargo nextest run -p sky-cua-service phone::protocol::identity`
→ 10 pass. `cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0.
(The `#![cfg_attr(not(test), expect(dead_code))]` attribute is absent in the
test build, and the non-test build is unchanged by a `#[cfg(test)]` module,
so no unfulfilled-expectation warning is possible. If clippy nonetheless
reports one, STOP.)

### Step 2: Manager-level tests: impostor refused and never granted; blocked downgrade not bypassed

In `manager/tests.rs`, add next to the other companion bootstrap tests
(after `unreachable_companion_reports_expected_apk_sha256`, line 4420):

```rust
/// A same-name companion package whose signing cert differs from the
/// configured expected cert is an impostor: the bootstrap must surface
/// `CompanionSignatureMismatch`, report the companion unreachable with
/// `signature_matches_expected == false`, and never install, uninstall,
/// grant permissions to, forward to, or deliver a token to it.
#[tokio::test]
async fn mismatched_installed_cert_refuses_companion_and_grants_nothing() {
    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "pm", "path", &package],
        "package:/data/app/companion.apk",
    );
    runner.set_stdout(
        "adb",
        &["-s", SERIAL, "shell", "dumpsys", "package", &package],
        "versionCode=2\nversionName=0.1.1\nSHA-256 cert digest: 00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF\n",
    );
    selection.companion_enabled = true;
    selection.companion_auto_install = true;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);

    let session = connect(&mut manager).await;

    let companion = &session.capability_profile.companion;
    assert!(!companion.rpc_reachable);
    assert!(!companion.signature_matches_expected, "impostor path must report a mismatch");
    assert_eq!(
        companion.installed_cert_sha256.as_deref(),
        Some("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff")
    );
    let diagnostics = manager.companion_bootstrap_diagnostics(&session.session_id);
    assert!(
        diagnostics.iter().any(|d| d.code == "CompanionSignatureMismatch"),
        "diagnostics: {diagnostics:?}"
    );
    let calls = runner.recorded_calls();
    let install = format!("adb -s {SERIAL} install");
    let uninstall = format!("adb -s {SERIAL} uninstall");
    assert!(
        !calls.iter().any(|c| c.starts_with(&install) || c.starts_with(&uninstall)),
        "impostor must not be installed over or removed: {calls:?}"
    );
    assert!(
        !calls.iter().any(|c| c.contains("allow_listener")
            || c.contains("enabled_accessibility_services")
            || c.contains(" forward ")
            || c.contains(".SetupActivity")),
        "no permission grant, forward, or token delivery may run for an impostor: {calls:?}"
    );
}
```

The scripted digest is accepted by `parse_signing_cert_sha256` (colons
stripped, lower-cased) and parses to
`00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff`, which
differs from `COMPANION_CERT_SHA256`. The negative substrings are the real
recorded commands on the grant path (`cmd notification allow_listener`,
`settings get secure enabled_accessibility_services`, `adb -s S forward`,
`am start -n <pkg>/.SetupActivity`); `appops` and `settings put` never
appear in these tests, so do not use them.

One-time sanity check (do not commit it): temporarily change the scripted
digest to `{COMPANION_CERT_SHA256}` so the cert matches, run the test, and
confirm it FAILS on the `CompanionSignatureMismatch` assertion while
`recorded_calls()` now contains `allow_listener`. Revert the digest.

Then add the downgrade counterpart. There is no `CompanionDowngradeBlocked`
diagnostic on the connect path, so the observable behaviour is that
`force_reinstall` (which converts `UpToDate` into `Update`) must NOT convert a
blocked downgrade into an install:

```rust
/// A same-cert companion newer than the packaged one is a blocked downgrade:
/// even an explicit force_reinstall without allow_downgrade must not install
/// over it (force_reinstall only upgrades an UpToDate decision). The expected
/// version_code comes from the sidecar JSON next to the configured APK path.
#[tokio::test]
async fn force_reinstall_does_not_bypass_a_blocked_downgrade() {
    let dir = std::env::temp_dir().join(format!("sky-cua-plan014-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("sidecar dir");
    std::fs::write(dir.join("phone-companion.json"), r#"{"version_code": 1}"#)
        .expect("write sidecar");
    let apk = dir.join("phone-companion.apk").to_string_lossy().into_owned();

    let runner = Arc::new(FakeCommandRunner::new());
    script_device_probes(&runner);
    let mut selection = resolve_phone_selection(&PhoneConfig::default());
    let package = selection.companion_package.clone();
    script_verified_installed_companion(&runner, &package, 999, "9.9.9");
    selection.companion_enabled = true;
    selection.companion_auto_install = false;
    selection.companion_allow_downgrade = false;
    selection.companion_expected_cert_sha256 = Some(COMPANION_CERT_SHA256.to_string());
    selection.companion_apk_path = apk.clone();
    let mut manager = PhoneManager::with_runner(runner.clone(), selection);
    let session = connect(&mut manager).await;

    match manager
        .handle(PhoneRequest::InstallCompanion(
            sky_cua_platform::model::PhoneInstallCompanionRequest {
                session: selector(&session.session_id),
                force_reinstall: true,
                allow_downgrade: false,
            },
        ))
        .await
    {
        PhoneResponse::Action(action) => assert!(
            !action
                .diagnostics
                .iter()
                .any(|d| d.code == "CompanionUpdate" || d.code == "CompanionInstall"),
            "a blocked downgrade must not be turned into an install: {:?}",
            action.diagnostics
        ),
        other => panic!("unexpected: {other:?}"),
    }
    let install = format!("adb -s {SERIAL} install");
    let calls = runner.recorded_calls();
    assert!(
        !calls.iter().any(|c| c.starts_with(&install)),
        "no adb install may run over a newer same-cert companion: {calls:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
```

Why it is a real regression test: if the downgrade gate regressed to
`UpToDate`, `force_reinstall` would turn it into `Update` and an
`adb -s emulator-5554 install -r <apk>` call would be recorded (the fake
runner records before lookup, so the install needs no scripting). Check the
exact `PhoneInstallCompanionRequest` field names against the exemplar at
lines 4161-4201 before running.

**Verify**: `cargo nextest run -p sky-cua-service phone::manager::tests` →
all pass including the 2 new tests.

### Step 3: Full crate run

**Verify**: `cargo fmt --check` → exit 0;
`cargo clippy -p sky-cua-service --all-targets -- -D warnings` → exit 0;
`cargo nextest run -p sky-cua-service` → all pass.

## Test plan

- `identity.rs`: 10 unit tests (Step 1).
- `manager/tests.rs`: `mismatched_installed_cert_refuses_companion_and_grants_nothing`
  and `force_reinstall_does_not_bypass_a_blocked_downgrade` (Step 2).
- Pattern: existing companion bootstrap tests around lines 2240 and 4170.

## Done criteria

Machine-checkable. ALL must hold:

- [ ] `cargo fmt --check` exits 0
- [ ] `cargo clippy -p sky-cua-service --all-targets -- -D warnings` exits 0
- [ ] `cargo nextest run -p sky-cua-service` exits 0 with 12 new tests
- [ ] `grep -c '^#\[cfg(test)\]' crates/sky-cua-service/src/phone/protocol/identity.rs` prints `1`
- [ ] `grep -cE 'fn (mismatched_installed_cert_refuses_companion_and_grants_nothing|force_reinstall_does_not_bypass_a_blocked_downgrade)' crates/sky-cua-service/src/phone/manager/tests.rs` prints `2`
- [ ] `git diff e928a40 -- crates/sky-cua-service/src/phone/protocol/identity.rs | grep -c '^-[^-]'` prints `0` (no production line removed) and `git diff -U0 e928a40 -- crates/sky-cua-service/src/phone/protocol/identity.rs | grep -c '^@@'` prints `1` with that hunk header starting `@@ -257,0 +258,`
- [ ] `git diff --name-only e928a40 -- crates/` prints exactly `crates/sky-cua-service/src/phone/manager/tests.rs` and `crates/sky-cua-service/src/phone/protocol/identity.rs`
- [ ] `plans/README.md` status row updated

## STOP conditions

Stop and report back (do not improvise) if:

- The excerpts in "Current state" do not match the live code.
- A Step 1 test fails: that means `decide_install` does not do what this
  plan says it does at `e928a40`. Report the failing assertion; do not change
  production code and do not flip the assertion without understanding why.
- The impostor test fails on the `CompanionSignatureMismatch` assertion.
  Check that `selection.companion_expected_cert_sha256` is `Some`, that the
  scripted digest parses to 64 hex nibbles, and that
  `SKY_CUA_PHONE_DIRECT_ENABLED` and `SKY_CUA_REPO_ROOT` are unset in the
  environment.
- The one-time sanity check (matching cert) does NOT make the impostor test
  fail: then the assertions are vacuous; report rather than proceed.
- Clippy complains about the module-level `expect(dead_code)` after adding
  tests.
- Any previously passing manager test fails.

## Maintenance notes

- `decide_install`'s "allowed downgrade falls through to `UpToDate`" is now
  pinned. If that is ever changed to `Update`, update
  `downgrade_allowed_by_expected_or_override_falls_through_to_up_to_date`
  deliberately.
- The impostor gate runs only on the ADB bootstrap lane. The direct lane has
  its own identity model (HMAC device secret per host); there is no
  signature check there because there is no APK install. If the ADB lane is
  ever retired, these tests go with it.
- A reviewer should check that the impostor test's negative assertions use
  substrings that really appear in recorded commands on the non-impostor
  path (the sanity check in Step 2 proves it once; keep it in the PR
  description).
- `RefuseDowngrade` is silent on the connect path. If a diagnostic for it is
  ever added (`CompanionDowngradeBlocked` already exists as a code), the
  downgrade test can assert it directly instead of the force-reinstall
  indirection.
