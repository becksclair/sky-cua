//! Model adapters for the optional browser ranker. The HTTP client is reusable
//! and provider-neutral; this module owns the Jev and Terra wire contracts.

use serde_json::{Value, json};
use sky_cua_model_client::ModelClient;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

mod terra;

static HTTP_CLIENT: OnceLock<ModelClient> = OnceLock::new();
const MAX_AUTH_STORE_BYTES: u64 = 1024 * 1024;

fn opencode_key(
    api_key: Option<String>,
    zen_key: Option<String>,
    data_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<String> {
    api_key
        .filter(|key| !key.is_empty())
        .or_else(|| zen_key.filter(|key| !key.is_empty()))
        .or_else(|| {
            let data_home = data_home
                .filter(|path| !path.as_os_str().is_empty())
                .or_else(|| {
                    home.filter(|path| !path.as_os_str().is_empty())
                        .map(|path| path.join(".local/share"))
                })?;
            stored_opencode_key(&data_home.join("opencode/auth.json"))
        })
}

fn stored_opencode_key(path: &Path) -> Option<String> {
    // Cap the read itself so a growing or oversized store cannot allocate without bound.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_AUTH_STORE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_AUTH_STORE_BYTES {
        return None;
    }
    let store: Value = serde_json::from_slice(&bytes).ok()?;
    let entry = store.get("opencode")?;
    if entry.get("type")?.as_str()? != "api" {
        return None;
    }
    entry
        .get("key")?
        .as_str()
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
}

pub(crate) struct ModelDecision {
    pub choice: String,
    pub probability: Option<f64>,
    pub elapsed_ms: u64,
}

#[derive(Debug)]
pub(crate) struct ProviderFailure(pub &'static str);
impl fmt::Display for ProviderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for ProviderFailure {}

pub(crate) struct RankModels {
    http: ModelClient,
    key: Option<String>,
}

impl RankModels {
    pub fn from_env() -> Self {
        let key = opencode_key(
            std::env::var("OPENCODE_API_KEY").ok(),
            std::env::var("OPENCODE_ZEN_API_KEY").ok(),
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
            std::env::var_os("HOME").map(PathBuf::from),
        );
        let http = HTTP_CLIENT
            .get_or_init(|| {
                ModelClient::new(Duration::from_secs(30), 4 * 1024 * 1024)
                    .expect("fixed model HTTP configuration is valid")
            })
            .clone();
        Self { http, key }
    }

    pub fn jev(&self, payload: &Value) -> Result<ModelDecision, ProviderFailure> {
        let key = self
            .key
            .as_ref()
            .ok_or(ProviderFailure("missing_credentials"))?;
        let mut request = payload.clone();
        request["model"] = json!("jev-1.13");
        let authorization = format!("Bearer {key}");
        let response = self
            .http
            .post_json(
                "https://opencode.ai/inference/systemone/v1/systemone",
                &[("Authorization", &authorization)],
                &request,
            )
            .map_err(|_| ProviderFailure("request_failed"))?;
        let answer = &response.body["answers"]["target"];
        let choice = answer["choice"]
            .as_str()
            .ok_or(ProviderFailure("invalid_response"))?
            .to_owned();
        Ok(ModelDecision {
            probability: answer["probabilities"][&choice].as_f64(),
            choice,
            elapsed_ms: response.elapsed.as_millis().try_into().unwrap_or(u64::MAX),
        })
    }

    pub fn terra(&self, payload: &Value) -> Result<ModelDecision, ProviderFailure> {
        terra::select(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AuthStore(PathBuf);

    impl AuthStore {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("sky-cua-auth-{}", uuid::Uuid::new_v4())))
        }

        fn write(&self, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for AuthStore {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn opencode_environment_keys_take_precedence_over_store() {
        let store = AuthStore::new();
        store.write(
            "opencode/auth.json",
            r#"{"opencode":{"type":"api","key":"stored"}}"#,
        );
        for (api, zen, expected) in [
            (Some("api"), Some("zen"), "api"),
            (None, Some("zen"), "zen"),
            (Some(""), Some("zen"), "zen"),
            (Some(""), Some(""), "stored"),
            (None, None, "stored"),
        ] {
            assert_eq!(
                opencode_key(
                    api.map(str::to_owned),
                    zen.map(str::to_owned),
                    Some(store.0.clone()),
                    None
                ),
                Some(expected.to_owned())
            );
        }
    }

    #[test]
    fn opencode_store_uses_data_home_or_standard_home_location() {
        let store = AuthStore::new();
        store.write(
            "opencode/auth.json",
            r#"{"opencode":{"type":"api","key":"xdg"}}"#,
        );
        store.write(
            ".local/share/opencode/auth.json",
            r#"{"opencode":{"type":"api","key":"home"}}"#,
        );
        assert_eq!(
            opencode_key(None, None, Some(store.0.clone()), Some(store.0.clone())),
            Some("xdg".into())
        );
        assert_eq!(
            opencode_key(None, None, None, Some(store.0.clone())),
            Some("home".into())
        );
        assert_eq!(
            opencode_key(None, None, Some(PathBuf::new()), Some(store.0.clone())),
            Some("home".into())
        );
        // An explicit data home selects one store; missing credentials do not search others.
        assert_eq!(
            opencode_key(
                None,
                None,
                Some(store.0.join("missing")),
                Some(store.0.clone())
            ),
            None
        );
        assert_eq!(opencode_key(None, None, None, None), None);
    }

    #[test]
    fn opencode_store_ignores_other_providers_and_invalid_entries() {
        let store = AuthStore::new();
        for contents in [
            r#"{"other":{"type":"api","key":"other"}}"#,
            r#"{"opencode":{"type":"oauth","key":"oauth"}}"#,
            r#"{"opencode":{"key":"missing-type"}}"#,
            r#"{"opencode":{"type":"api","key":""}}"#,
            r#"{"opencode":{"type":"api","key":123}}"#,
            r#"{"opencode":{"type":"api"}}"#,
            r#"{"opencode":null}"#,
            "[]",
            "null",
            "malformed",
        ] {
            let path = store.write("opencode/auth.json", contents);
            assert_eq!(stored_opencode_key(&path), None);
        }
        assert_eq!(stored_opencode_key(&store.0.join("missing.json")), None);
        assert_eq!(stored_opencode_key(&store.0), None);
    }

    #[test]
    fn opencode_store_read_is_bounded() {
        let store = AuthStore::new();
        let mut contents = br#"{"opencode":{"type":"api","key":"stored"}}"#.to_vec();
        contents.resize(MAX_AUTH_STORE_BYTES as usize, b' ');
        let path = store.write("opencode/auth.json", &contents);
        assert_eq!(stored_opencode_key(&path), Some("stored".into()));
        contents.push(b' ');
        store.write("opencode/auth.json", contents);
        assert_eq!(stored_opencode_key(&path), None);
    }
}
