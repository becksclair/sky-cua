#![cfg_attr(not(test), expect(dead_code))]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InstalledCompanion {
    pub(crate) version_name: Option<String>,
    pub(crate) version_code: Option<u64>,
    pub(crate) cert_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ExpectedCompanion {
    pub(crate) package_name: String,
    pub(crate) version_name: Option<String>,
    pub(crate) version_code: Option<u64>,
    pub(crate) cert_sha256: Option<String>,
    pub(crate) apk_sha256: Option<String>,
    pub(crate) apk_path: String,
    pub(crate) allow_downgrade: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CompanionBootstrapOptions {
    pub(crate) allow_install: bool,
    pub(crate) force_reinstall: bool,
    pub(crate) allow_downgrade: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompanionInstallDecision {
    Install,
    Update {
        reason: String,
    },
    UpToDate,
    RefuseSignatureMismatch {
        installed_cert: String,
        expected_cert: String,
    },
    RefuseDowngrade {
        installed_version_code: u64,
        expected_version_code: u64,
    },
}

impl CompanionInstallDecision {
    pub(crate) fn requires_install(&self) -> bool {
        matches!(
            self,
            CompanionInstallDecision::Install | CompanionInstallDecision::Update { .. }
        )
    }
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

pub(crate) fn certs_match(left: &str, right: &str) -> bool {
    cert_eq(left, right)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CompanionMetadata {
    pub(crate) version_name: Option<String>,
    pub(crate) version_code: Option<u64>,
    pub(crate) cert_sha256: Option<String>,
    pub(crate) apk_sha256: Option<String>,
}

pub(crate) fn metadata_path_for_apk(apk_path: &str) -> PathBuf {
    Path::new(apk_path).with_extension("json")
}

pub(crate) fn load_companion_metadata(apk_path: &str) -> CompanionMetadata {
    match std::fs::read_to_string(metadata_path_for_apk(apk_path)) {
        Ok(text) => parse_companion_metadata(&text),
        Err(_) => CompanionMetadata::default(),
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompanionToken {
    pub(crate) token: String,
    pub(crate) expires_at_ms: u64,
}

impl CompanionToken {
    pub(crate) fn is_expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }
}

pub(crate) fn generate_token(now_ms: u64, ttl_ms: u64) -> CompanionToken {
    let bytes = urandom_bytes().unwrap_or_else(|| fallback_token_bytes(now_ms, ttl_ms));
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    CompanionToken {
        token,
        expires_at_ms: now_ms.saturating_add(ttl_ms),
    }
}

fn urandom_bytes() -> Option<[u8; 32]> {
    use std::io::Read as _;
    let mut file = std::fs::File::open("/dev/urandom").ok()?;
    let mut bytes = [0u8; 32];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

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
        (&lane as *const usize as u64).hash(&mut hasher);
        let value = hasher.finish().to_le_bytes();
        chunk.copy_from_slice(&value[..chunk.len()]);
    }
    bytes
}

pub(crate) const SETUP_TOKEN_EXPIRES_EXTRA: &str = "sky_cua_rpc_token_expires_at_ms";
pub(crate) const SETUP_TOKEN_EXTRA: &str = "sky_cua_rpc_token";

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

    fn expected(
        version_code: Option<u64>,
        cert: Option<&str>,
        allow_downgrade: bool,
    ) -> ExpectedCompanion {
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
            decide_install(
                Some(&newer),
                &expected(Some(3), Some(CERT_A), false),
                Some(true)
            ),
            CompanionInstallDecision::UpToDate
        );
        // The override can also turn an allowed downgrade back into a refusal.
        assert!(matches!(
            decide_install(
                Some(&newer),
                &expected(Some(3), Some(CERT_A), true),
                Some(false)
            ),
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
            decide_install(
                Some(&installed(None, Some(CERT_A))),
                &expected(Some(3), Some(CERT_A), false),
                None
            ),
            CompanionInstallDecision::UpToDate
        );
        assert_eq!(
            decide_install(
                Some(&installed(Some(3), Some(CERT_A))),
                &expected(None, Some(CERT_A), false),
                None
            ),
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
        assert_eq!(
            parse_companion_metadata("not json"),
            CompanionMetadata::default()
        );
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
