use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Brave,
    Chromium,
}

impl std::str::FromStr for Browser {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "chrome" | "google-chrome" => Ok(Self::Chrome),
            "brave" | "brave-browser" => Ok(Self::Brave),
            "chromium" | "chromium-browser" => Ok(Self::Chromium),
            other => bail!("unsupported browser {other:?}; expected chrome, brave, or chromium"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostManifest {
    pub name: String,
    pub description: String,
    pub path: String,
    #[serde(rename = "type")]
    pub manifest_type: String,
    pub allowed_origins: Vec<String>,
}

impl HostManifest {
    pub fn new(host_name: &str, host_path: &Path, extension_id: &str) -> Result<Self> {
        validate_host_name(host_name)?;
        validate_extension_id(extension_id)?;
        if !host_path.is_absolute() {
            bail!(
                "Chrome native messaging host path must be absolute: {}",
                host_path.display()
            );
        }
        let host_path = host_path
            .canonicalize()
            .unwrap_or_else(|_| host_path.to_path_buf());
        Ok(Self {
            name: host_name.to_string(),
            description: "sky-cua browser automation native host".to_string(),
            path: host_path.display().to_string(),
            manifest_type: "stdio".to_string(),
            allowed_origins: vec![format!("chrome-extension://{extension_id}/")],
        })
    }
}

pub fn host_manifest_path(browser: Browser, host_name: &str) -> Result<PathBuf> {
    validate_host_name(host_name)?;
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let root = match browser {
        Browser::Chrome => PathBuf::from(home).join(".config/google-chrome/NativeMessagingHosts"),
        Browser::Brave => {
            PathBuf::from(home).join(".config/BraveSoftware/Brave-Browser/NativeMessagingHosts")
        }
        Browser::Chromium => PathBuf::from(home).join(".config/chromium/NativeMessagingHosts"),
    };
    Ok(root.join(format!("{host_name}.json")))
}

fn validate_host_name(value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '.'))
        && value.contains('.');
    if valid {
        Ok(())
    } else {
        bail!("invalid Chrome native host name {value:?}")
    }
}

fn validate_extension_id(value: &str) -> Result<()> {
    let valid = value.len() == 32 && value.chars().all(|ch| matches!(ch, 'a'..='p'));
    if valid {
        Ok(())
    } else {
        bail!("invalid Chrome extension id {value:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_uses_exact_extension_origin() {
        let manifest = HostManifest::new(
            "com.openai.codexextension",
            Path::new("/tmp/sky-cua-chrome-host"),
            "abcdefghijklmnopabcdefghijklmnop",
        )
        .unwrap();

        assert_eq!(manifest.manifest_type, "stdio");
        assert_eq!(
            manifest.allowed_origins,
            vec!["chrome-extension://abcdefghijklmnopabcdefghijklmnop/"]
        );
    }

    #[test]
    fn rejects_broad_or_invalid_extension_ids() {
        assert!(HostManifest::new("io.github.test", Path::new("/tmp/host"), "*").is_err());
        assert!(HostManifest::new("io.github.test", Path::new("/tmp/host"), "abcd").is_err());
    }

    #[test]
    fn rejects_relative_host_paths() {
        let error = HostManifest::new(
            "io.github.test",
            Path::new("bin/host"),
            "abcdefghijklmnopabcdefghijklmnop",
        )
        .expect_err("relative host path should be rejected");
        assert!(error.to_string().contains("must be absolute"));
    }

    #[test]
    fn parses_supported_browsers() {
        assert_eq!("chrome".parse::<Browser>().unwrap(), Browser::Chrome);
        assert_eq!("brave-browser".parse::<Browser>().unwrap(), Browser::Brave);
        assert_eq!("chromium".parse::<Browser>().unwrap(), Browser::Chromium);
    }
}
