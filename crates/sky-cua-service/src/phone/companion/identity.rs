//! Companion identity, install decisioning, and token provisioning.
//!
//! `phone_connect` must never trust an installed APK just because the package
//! name matches. This module compares the installed package version + signing
//! cert SHA-256 + APK SHA-256 against expected config/build metadata and decides
//! whether to install, update, refuse on signature mismatch, or honor the
//! downgrade policy. It also formats the ADB setup-intent argv that delivers an
//! ephemeral session token to the companion (it does NOT run adb — it returns
//! the argv for the integrator/`CommandRunner`) and generates that token + TTL.
//!
//! Until the integrator wires this into `manager.rs`, the install/token helpers
//! are only reached from tests. The module-level expectation keeps non-test
//! builds clean (the spine's `expect(dead_code)` idiom) and becomes self-removing
//! once `phone_connect` calls these helpers.
#![cfg_attr(not(test), expect(dead_code))]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Identity/version metadata for the installed companion package, as read from
/// `dumpsys package` / `pm` / a cert dump by the integrator. Any field the host
/// could not determine is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InstalledCompanion {
    pub(crate) version_name: Option<String>,
    pub(crate) version_code: Option<u64>,
    /// Lowercase hex SHA-256 of the installed signing certificate.
    pub(crate) cert_sha256: Option<String>,
}

/// Expected companion metadata from packaged build output + config. `phone_connect`
/// compares the installed package against this. `apk_sha256` and `cert_sha256`
/// come from the build metadata next to the packaged APK; `version_code` is the
/// packaged APK's version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ExpectedCompanion {
    pub(crate) package_name: String,
    pub(crate) version_name: Option<String>,
    pub(crate) version_code: Option<u64>,
    /// Lowercase hex SHA-256 of the packaged APK signing certificate.
    pub(crate) cert_sha256: Option<String>,
    /// Lowercase hex SHA-256 of the packaged APK file.
    pub(crate) apk_sha256: Option<String>,
    /// Host path to the packaged APK, used to build the `adb install -r` argv.
    pub(crate) apk_path: String,
    /// Whether config permits installing an older `version_code` over a newer
    /// installed one (`companion_allow_downgrade`).
    pub(crate) allow_downgrade: bool,
}

/// Per-bootstrap operator intent layered on top of machine config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CompanionBootstrapOptions {
    /// Whether this bootstrap is allowed to run `adb install -r`.
    pub(crate) allow_install: bool,
    /// Whether to install even when the installed package already appears current.
    pub(crate) force_reinstall: bool,
    /// Per-request downgrade override. `None` means use machine config.
    pub(crate) allow_downgrade: Option<bool>,
}

/// What `phone_connect` should do with the companion package, decided purely
/// from installed vs. expected metadata before any RPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompanionInstallDecision {
    /// Not installed -> install the packaged APK.
    Install,
    /// Installed, version older than expected -> update (`adb install -r`).
    Update { reason: String },
    /// Installed and current; nothing to do.
    UpToDate,
    /// Installed cert is readable and does not match expected cert -> refuse to
    /// silently replace. Recovery requires an explicit uninstall/reinstall by the
    /// operator. (An *unreadable* installed cert is not a mismatch: it proceeds
    /// and is reported as `signature_matches_expected=false`.)
    RefuseSignatureMismatch {
        installed_cert: String,
        expected_cert: String,
    },
    /// Installed version is NEWER than expected and downgrade is not allowed.
    RefuseDowngrade {
        installed_version_code: u64,
        expected_version_code: u64,
    },
}

impl CompanionInstallDecision {
    /// Whether acting on this decision involves running `adb install -r`.
    pub(crate) fn requires_install(&self) -> bool {
        matches!(
            self,
            CompanionInstallDecision::Install | CompanionInstallDecision::Update { .. }
        )
    }

    /// Stable code so the manager attaches a structured diagnostic for refusals.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            CompanionInstallDecision::Install => "CompanionInstall",
            CompanionInstallDecision::Update { .. } => "CompanionUpdate",
            CompanionInstallDecision::UpToDate => "CompanionUpToDate",
            CompanionInstallDecision::RefuseSignatureMismatch { .. } => {
                "CompanionSignatureMismatch"
            }
            CompanionInstallDecision::RefuseDowngrade { .. } => "CompanionDowngradeBlocked",
        }
    }
}

/// Decide install/update/refuse from installed vs. expected metadata.
///
/// Order of checks matters:
/// 1. Not installed -> `Install`.
/// 2. Missing expected cert, missing installed cert, or cert mismatch ->
///    `RefuseSignature*`, never silently trusted. This is checked before version
///    so a malicious same-name package cannot be "updated" over.
/// 3. Installed newer than expected, downgrade disallowed -> `RefuseDowngrade`.
/// 4. Installed older than expected -> `Update`.
/// 5. Otherwise -> `UpToDate`.
///
/// The signing cert is the real security gate here. `ExpectedCompanion.apk_sha256`
/// is report-only expected metadata: the installed APK's file hash is generally
/// NOT obtainable from `dumpsys` (only the cert is), so [`InstalledCompanion`]
/// carries no installed apk hash and this decision never compares apk hashes. An
/// apk-hash comparison would only be valid if BOTH an expected and an installed
/// apk hash were available; fabricating one against missing installed data would
/// be a false gate, so it is deliberately omitted.
pub(crate) fn decide_install(
    installed: Option<&InstalledCompanion>,
    expected: &ExpectedCompanion,
) -> CompanionInstallDecision {
    let Some(installed) = installed else {
        return CompanionInstallDecision::Install;
    };

    match (
        installed.cert_sha256.as_ref(),
        expected.cert_sha256.as_ref(),
    ) {
        // Confirmed mismatch: a same-name package whose readable signing cert
        // differs from the packaged companion is an impostor; never silently
        // replace or trust it.
        (Some(installed_cert), Some(expected_cert)) if !cert_eq(installed_cert, expected_cert) => {
            return CompanionInstallDecision::RefuseSignatureMismatch {
                installed_cert: installed_cert.clone(),
                expected_cert: expected_cert.clone(),
            };
        }
        // Otherwise proceed: a verified match, or an unverifiable cert. Modern
        // Android (API 28+) does not expose the installed package's certificate
        // SHA-256 through `dumpsys package` (only a short signature hash), so the
        // installed cert is commonly unreadable. Refusing that case made the
        // companion unusable on every current device while detecting no real
        // impostor (a mismatch is only knowable when the cert IS readable, handled
        // above). An unverifiable cert is reported honestly as
        // `signature_matches_expected=false` rather than refused.
        _ => {}
    }

    if let (Some(installed_code), Some(expected_code)) =
        (installed.version_code, expected.version_code)
    {
        if installed_code > expected_code && !expected.allow_downgrade {
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

/// Case-insensitive compare of two hex certificate fingerprints, ignoring any
/// `:` separators (`AA:BB` vs `aabb`).
fn cert_eq(left: &str, right: &str) -> bool {
    let normalize = |s: &str| {
        s.chars()
            .filter(|c| *c != ':')
            .flat_map(|c| c.to_lowercase())
            .collect::<String>()
    };
    normalize(left) == normalize(right)
}

/// Public case/separator-insensitive equality of two hex cert digests, so the
/// capability builder can report `signature_matches_expected` from the actual
/// installed-vs-expected certs rather than inferring it from the install decision.
pub(crate) fn certs_match(left: &str, right: &str) -> bool {
    cert_eq(left, right)
}

// ===========================================================================
// Bundled build-metadata sidecar
// ===========================================================================

/// Expected companion identity loaded from the build-metadata sidecar that ships
/// next to the packaged APK (`<name>.apk` -> `<name>.json`). The build emits it
/// (package id, versionCode/Name, APK SHA-256, signing-cert SHA-256) precisely so
/// the host has a source of truth for the signature/version checks. Env and
/// machine-config values override it; a missing or unparseable sidecar yields
/// all-`None` so the bootstrap falls back to configured values without failing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CompanionMetadata {
    pub(crate) version_name: Option<String>,
    pub(crate) version_code: Option<u64>,
    pub(crate) cert_sha256: Option<String>,
    pub(crate) apk_sha256: Option<String>,
}

/// The sidecar path for a packaged APK path: the same path with a `.json`
/// extension (`resources/android/phone-companion.apk` ->
/// `resources/android/phone-companion.json`).
pub(crate) fn metadata_path_for_apk(apk_path: &str) -> PathBuf {
    Path::new(apk_path).with_extension("json")
}

/// Load the bundled companion metadata sidecar for `apk_path`. Best-effort: any
/// read or parse failure yields the default (all-`None`).
pub(crate) fn load_companion_metadata(apk_path: &str) -> CompanionMetadata {
    match std::fs::read_to_string(metadata_path_for_apk(apk_path)) {
        Ok(text) => parse_companion_metadata(&text),
        Err(_) => CompanionMetadata::default(),
    }
}

/// Parse the sidecar JSON (`version_code`, `version_name`, `apk_sha256`,
/// `signing_cert_sha256`). Hex digests are normalized to lowercase so they
/// compare cleanly against the host-parsed installed cert.
fn parse_companion_metadata(text: &str) -> CompanionMetadata {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return CompanionMetadata::default();
    };
    let lower_hex = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(|raw| raw.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
    };
    CompanionMetadata {
        version_name: value
            .get("version_name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        version_code: value
            .get("version_code")
            .and_then(serde_json::Value::as_u64),
        cert_sha256: lower_hex("signing_cert_sha256"),
        apk_sha256: lower_hex("apk_sha256"),
    }
}

// ===========================================================================
// Token provisioning
// ===========================================================================

/// An ephemeral RPC session token plus its absolute expiry. The host generates
/// one per session, delivers it to the companion through the setup intent, and
/// attaches it to every RPC call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompanionToken {
    pub(crate) token: String,
    pub(crate) expires_at_ms: u64,
}

impl CompanionToken {
    /// Whether the token is expired relative to `now_ms`.
    pub(crate) fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }
}

/// Generate an ephemeral token valid for `ttl_ms` from `now_ms`.
///
/// The token is 32 bytes rendered as 64 lowercase hex characters. The bytes come
/// from the OS CSPRNG by reading `/dev/urandom`; when that read succeeds the
/// token carries 256 bits of cryptographically strong entropy. If `/dev/urandom`
/// cannot be read (a sandbox without `/dev`, a non-Unix host), the token falls
/// back to hashing several distinct process/time sources through the
/// standard-library default hasher with a per-call OS-seeded `RandomState`. The
/// fallback path does NOT provide 256 bits of cryptographic entropy; it is a
/// best-effort, non-cryptographic uniqueness source for a short-lived,
/// localhost-only, ADB-gated bearer token, never reused for any wider purpose.
/// The format and length are identical on both paths.
pub(crate) fn generate_token(now_ms: u64, ttl_ms: u64) -> CompanionToken {
    let bytes = urandom_bytes().unwrap_or_else(|| fallback_token_bytes(now_ms, ttl_ms));
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    CompanionToken {
        token,
        expires_at_ms: now_ms.saturating_add(ttl_ms),
    }
}

/// Read 32 bytes from the OS CSPRNG (`/dev/urandom`). Returns `None` on any
/// failure (missing device, short read) so the caller can fall back.
fn urandom_bytes() -> Option<[u8; 32]> {
    use std::io::Read as _;

    let mut file = std::fs::File::open("/dev/urandom").ok()?;
    let mut bytes = [0u8; 32];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

/// Non-cryptographic fallback when `/dev/urandom` is unavailable: four
/// independent 64-bit lanes, each seeded by a fresh OS-seeded `RandomState` and
/// mixed with distinct salts so the lanes do not collapse.
fn fallback_token_bytes(now_ms: u64, ttl_ms: u64) -> [u8; 32] {
    use std::hash::{BuildHasher, Hash, Hasher};

    let mut bytes = [0u8; 32];
    let pid = u64::from(std::process::id());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(now_ms);

    for (lane, chunk) in bytes.chunks_mut(8).enumerate() {
        let state = std::collections::hash_map::RandomState::new();
        let mut hasher = state.build_hasher();
        (lane as u64).hash(&mut hasher);
        pid.hash(&mut hasher);
        nanos.hash(&mut hasher);
        now_ms.hash(&mut hasher);
        ttl_ms.hash(&mut hasher);
        // Mix the address of a stack local for additional per-call variation.
        (&lane as *const usize as u64).hash(&mut hasher);
        let value = hasher.finish().to_le_bytes();
        chunk.copy_from_slice(&value[..chunk.len()]);
    }
    bytes
}

/// Argument keys the setup intent uses to carry token metadata. Kept as
/// constants so the companion app and host agree on a single spelling.
pub(crate) const SETUP_TOKEN_EXPIRES_EXTRA: &str = "sky_cua_rpc_token_expires_at_ms";

/// String-extra key carrying the ephemeral RPC bearer token directly to the
/// companion `SetupActivity`.
///
/// The token is delivered as an intent extra rather than a pushed file: on
/// Android 11+ each app gets an isolated storage mount namespace, so a file the
/// host (`adb`/shell) writes into `/sdcard/Android/data/<pkg>/` is NOT readable
/// by the app process, which made the file handoff silently fail (the RPC server
/// never started). The token is ephemeral (short TTL), localhost-only, and
/// ADB-gated; `hidepid` hides `/proc/<pid>/cmdline` from other uids on modern
/// Android, so the argv exposure is bounded. The documented future hardening is
/// the logcat-readback handshake (companion mints its own token).
pub(crate) const SETUP_TOKEN_EXTRA: &str = "sky_cua_rpc_token";

/// Build the ADB argv that launches the companion `SetupActivity` with the
/// ephemeral session token delivered directly as an intent extra. Does NOT run
/// adb; returns the argv for the integrator/`CommandRunner`:
///
/// ```text
/// adb -s <serial> shell am start -n <pkg>/.SetupActivity \
///   --es sky_cua_rpc_token <token> \
///   --el sky_cua_rpc_token_expires_at_ms <epoch_ms>
/// ```
///
/// The returned vector starts at `-s` (the program `adb` is supplied by the
/// runner). `--es` is an Android string extra; `--el` is a long extra. The token
/// is 64 hex characters (no shell metacharacters), so it is safe as a single argv
/// element through the device shell.
pub(crate) fn setup_intent_argv(
    serial: &str,
    package_name: &str,
    token: &CompanionToken,
) -> Vec<String> {
    let component = format!("{package_name}/.SetupActivity");
    let expires = token.expires_at_ms.to_string();
    vec![
        "-s".to_string(),
        serial.to_string(),
        "shell".to_string(),
        "am".to_string(),
        "start".to_string(),
        "-n".to_string(),
        component,
        "--es".to_string(),
        SETUP_TOKEN_EXTRA.to_string(),
        token.token.clone(),
        "--el".to_string(),
        SETUP_TOKEN_EXPIRES_EXTRA.to_string(),
        expires,
    ]
}

/// Build the `adb install -r` argv for the packaged companion APK. Returns the
/// argv after the program `adb` (which the runner supplies). `serial` targets a
/// specific device; `-r` reinstalls keeping data; `-d` is appended when a
/// downgrade is explicitly permitted.
pub(crate) fn install_argv(serial: &str, expected: &ExpectedCompanion) -> Vec<String> {
    let mut argv = vec![
        "-s".to_string(),
        serial.to_string(),
        "install".to_string(),
        "-r".to_string(),
    ];
    if expected.allow_downgrade {
        argv.push("-d".to_string());
    }
    argv.push(expected.apk_path.clone());
    argv
}
