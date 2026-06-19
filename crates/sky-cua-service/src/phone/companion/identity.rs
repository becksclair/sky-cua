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
    /// Installed cert does not match expected cert -> refuse to silently replace.
    /// Recovery requires an explicit uninstall/reinstall by the operator.
    RefuseSignatureMismatch {
        installed_cert: String,
        expected_cert: String,
    },
    /// Installed package exists but the host cannot prove its signing cert matches
    /// the packaged companion. Do not provision an RPC token to a same-name APK
    /// whose identity is unknown.
    RefuseSignatureUnverified {
        installed_cert: Option<String>,
        expected_cert: Option<String>,
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
            CompanionInstallDecision::RefuseSignatureUnverified { .. } => {
                "CompanionSignatureUnverified"
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
        (Some(installed_cert), Some(expected_cert)) if cert_eq(installed_cert, expected_cert) => {}
        (Some(installed_cert), Some(expected_cert)) => {
            return CompanionInstallDecision::RefuseSignatureMismatch {
                installed_cert: installed_cert.clone(),
                expected_cert: expected_cert.clone(),
            };
        }
        (installed_cert, expected_cert) => {
            return CompanionInstallDecision::RefuseSignatureUnverified {
                installed_cert: installed_cert.cloned(),
                expected_cert: expected_cert.cloned(),
            };
        }
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

/// Argument key for a device-local token file path. The host pushes the token to
/// the companion app's external cache, then launches setup with this non-secret
/// path instead of placing the bearer token in process argv.
pub(crate) const SETUP_TOKEN_FILE_EXTRA: &str = "sky_cua_rpc_token_file";

/// Device path where the host stages the setup token for the companion to read.
pub(crate) fn setup_token_device_path(package_name: &str) -> String {
    format!("/sdcard/Android/data/{package_name}/cache/sky_cua_rpc_token")
}

/// Build the ADB argv that pushes the token file onto the device. The returned
/// argv carries only paths, never the token itself.
pub(crate) fn setup_token_push_argv(
    serial: &str,
    local_token_path: &str,
    package_name: &str,
) -> Vec<String> {
    vec![
        "-s".to_string(),
        serial.to_string(),
        "push".to_string(),
        local_token_path.to_string(),
        setup_token_device_path(package_name),
    ]
}

/// Build the ADB argv that launches the companion `SetupActivity` to consume the
/// already-pushed token file. Does NOT run adb; returns the argv for the
/// integrator/`CommandRunner`:
///
/// ```text
/// adb -s <serial> shell am start -n <pkg>/.SetupActivity \
///   --es sky_cua_rpc_token_file /sdcard/Android/data/<pkg>/cache/sky_cua_rpc_token \
///   --el sky_cua_rpc_token_expires_at_ms <epoch_ms>
/// ```
///
/// The returned vector starts at `-s` (the program `adb` is supplied by the
/// runner). `--es` is an Android string extra; `--el` is a long extra.
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
        SETUP_TOKEN_FILE_EXTRA.to_string(),
        setup_token_device_path(package_name),
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
