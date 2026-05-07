use std::path::PathBuf;

use sky_cua_platform::{approvals_path, portal_tokens_path};

#[derive(Debug, Clone)]
pub struct ApprovalStore {
    pub approvals_path: PathBuf,
    pub portal_tokens_path: PathBuf,
}

impl ApprovalStore {
    pub fn initialize() -> std::io::Result<()> {
        Self::new().map(|_| ())
    }

    fn new() -> std::io::Result<Self> {
        let store = Self {
            approvals_path: approvals_path()?,
            portal_tokens_path: portal_tokens_path()?,
        };
        store.ensure_parent_dirs()?;
        Ok(store)
    }

    fn ensure_parent_dirs(&self) -> std::io::Result<()> {
        if let Some(parent) = self.approvals_path.parent() {
            std::fs::create_dir_all(parent)?;
            set_owner_only_permissions(parent)?;
        }
        if let Some(parent) = self.portal_tokens_path.parent() {
            std::fs::create_dir_all(parent)?;
            set_owner_only_permissions(parent)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}
