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
