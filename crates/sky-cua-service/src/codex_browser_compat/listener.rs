use std::{
    future::pending,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use sky_cua_platform::config::resolved_browser_control_config;
use tokio::net::{UnixListener, UnixStream};

use super::CODEX_BROWSER_SOCKET_PATH_ENV;

pub(crate) struct CodexBrowserCompatListener {
    path: PathBuf,
    listener: UnixListener,
    _singleton_lock: std::fs::File,
}

impl CodexBrowserCompatListener {
    pub(crate) fn bind_configured(service_socket_path: &Path) -> Result<Option<Self>> {
        let Some(path) = configured_socket_path()? else {
            return Ok(None);
        };
        if path == service_socket_path {
            anyhow::bail!(
                "{CODEX_BROWSER_SOCKET_PATH_ENV} must differ from the ordinary service socket"
            );
        }
        Self::bind(path).map(Some)
    }

    pub(super) fn bind(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("create Codex browser socket directory {}", parent.display())
            })?;
        }
        let singleton_lock =
            crate::ipc_server::acquire_singleton_lock(&path)?.ok_or_else(|| {
                anyhow::anyhow!(
                    "another live daemon owns Codex browser socket {}",
                    path.display()
                )
            })?;
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("remove stale Codex browser socket {}", path.display()))?;
        }
        let listener = UnixListener::bind(&path)
            .with_context(|| format!("bind Codex browser socket {}", path.display()))?;
        set_socket_owner_only(&path)?;
        Ok(Self {
            path,
            listener,
            _singleton_lock: singleton_lock,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) async fn accept(&self) -> std::io::Result<UnixStream> {
        self.listener.accept().await.map(|(stream, _)| stream)
    }

    pub(crate) fn rebind_if_unlinked(&mut self) -> Result<bool> {
        if self.path.exists() {
            return Ok(false);
        }
        let replacement = UnixListener::bind(&self.path)
            .with_context(|| format!("re-bind Codex browser socket {}", self.path.display()))?;
        set_socket_owner_only(&self.path)?;
        self.listener = replacement;
        Ok(true)
    }

    pub(crate) async fn remove_socket(&self) {
        let _ = tokio::fs::remove_file(&self.path).await;
    }
}

pub(crate) async fn accept_configured(
    listener: Option<&CodexBrowserCompatListener>,
) -> std::io::Result<UnixStream> {
    match listener {
        Some(listener) => listener.accept().await,
        None => pending().await,
    }
}

pub(super) fn configured_socket_path() -> Result<Option<PathBuf>> {
    resolved_browser_control_config()
        .map(|config| config.codex_socket_path.map(PathBuf::from))
        .map_err(anyhow::Error::msg)
}

fn set_socket_owner_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restrict Codex browser socket {}", path.display()))
}

#[cfg(test)]
mod tests {
    use sky_cua_platform::config::{CODEX_BROWSER_SOCKET_PATH_ENV, MACHINE_CONFIG_PATH_ENV};

    use super::{CodexBrowserCompatListener, configured_socket_path};

    #[tokio::test]
    async fn live_codex_socket_owner_cannot_be_unlinked_or_replaced() {
        let root = std::env::temp_dir().join(format!(
            "sky-cua-codex-listener-owner-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("codex.sock");
        let first = CodexBrowserCompatListener::bind(path.clone()).unwrap();
        let inode = std::fs::metadata(&path).unwrap();

        let error = CodexBrowserCompatListener::bind(path.clone())
            .err()
            .expect("second live owner must be rejected");
        assert!(error.to_string().contains("another live daemon owns"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode.ino());
        }

        drop(first);
        let replacement = CodexBrowserCompatListener::bind(path.clone()).unwrap();
        drop(replacement);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn configured_socket_path_resolves_env_then_machine_then_unset() {
        let root = std::env::temp_dir().join(format!(
            "sky-cua-codex-listener-config-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let config = root.join("sky-cua.toml");
        std::fs::write(
            &config,
            "[browser_control]\ncodex_socket_path = \"/machine/codex.sock\"\n",
        )
        .unwrap();
        unsafe {
            std::env::set_var(MACHINE_CONFIG_PATH_ENV, &config);
            std::env::remove_var(CODEX_BROWSER_SOCKET_PATH_ENV);
        }
        assert_eq!(
            configured_socket_path().unwrap().as_deref(),
            Some(std::path::Path::new("/machine/codex.sock"))
        );

        unsafe { std::env::set_var(CODEX_BROWSER_SOCKET_PATH_ENV, "/env/codex.sock") };
        assert_eq!(
            configured_socket_path().unwrap().as_deref(),
            Some(std::path::Path::new("/env/codex.sock"))
        );

        unsafe { std::env::remove_var(CODEX_BROWSER_SOCKET_PATH_ENV) };
        std::fs::remove_file(&config).unwrap();
        assert_eq!(configured_socket_path().unwrap(), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn invalid_machine_config_fails_closed() {
        let config = std::env::temp_dir().join(format!(
            "sky-cua-invalid-codex-listener-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(&config, "[browser_control]\nmode = \"automatic\"\n").unwrap();
        unsafe {
            std::env::set_var(MACHINE_CONFIG_PATH_ENV, &config);
            std::env::remove_var(CODEX_BROWSER_SOCKET_PATH_ENV);
        }
        assert!(configured_socket_path().is_err());
        let _ = std::fs::remove_file(config);
    }
}
