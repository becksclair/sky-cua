use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sky_cua_platform::portal_tokens_path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedPortalToken {
    pub restore_token: String,
    pub updated_at: DateTime<Utc>,
    pub xdg_session_type: Option<String>,
    pub compositor: Option<String>,
    pub remote_desktop_version: Option<u32>,
    pub screencast_version: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct PortalTokenStore {
    path: PathBuf,
}

impl PortalTokenStore {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            path: portal_tokens_path()?,
        })
    }

    #[cfg(test)]
    pub fn for_path(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<Option<PersistedPortalToken>> {
        match fs::read_to_string(&self.path) {
            Ok(raw) => {
                let parsed =
                    serde_json::from_str::<PersistedPortalToken>(&raw).map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "failed to parse persisted portal token file {}: {error}",
                                self.path.display()
                            ),
                        )
                    })?;
                Ok(Some(parsed))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self, record: &PersistedPortalToken) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            let parent_existed = parent.exists();
            fs::create_dir_all(parent)?;
            if !parent_existed {
                set_owner_only_directory_permissions(parent)?;
            }
        }

        let payload = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
        let tmp_path = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&tmp_path, payload)?;
        set_owner_only_file_permissions(&tmp_path)?;
        fs::rename(&tmp_path, &self.path)?;
        set_owner_only_file_permissions(&self.path)?;
        Ok(())
    }

    pub fn clear(&self) -> io::Result<bool> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }
}

fn set_owner_only_directory_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }

    Ok(())
}

fn set_owner_only_file_permissions(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PersistedPortalToken, PortalTokenStore};
    use chrono::Utc;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sky-cua-portal-token-test-{name}-{}-{}.json",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    #[test]
    fn saves_loads_and_clears_token_records() {
        let path = temp_path("roundtrip");
        let store = PortalTokenStore::for_path(path.clone());
        let record = PersistedPortalToken {
            restore_token: "token-1".to_string(),
            updated_at: Utc::now(),
            xdg_session_type: Some("wayland".to_string()),
            compositor: Some("kde-kwin-wayland".to_string()),
            remote_desktop_version: Some(2),
            screencast_version: Some(5),
        };

        store.save(&record).expect("record should save");
        let loaded = store.load().expect("record should load");
        assert_eq!(
            loaded.as_ref().map(|item| &item.restore_token),
            Some(&"token-1".to_string())
        );
        assert!(store.clear().expect("clear should succeed"));
        assert_eq!(store.load().expect("missing should load"), None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn clear_returns_false_when_missing() {
        let path = temp_path("missing");
        let store = PortalTokenStore::for_path(path.clone());
        assert!(!store.clear().expect("missing clear should succeed"));
    }
}
