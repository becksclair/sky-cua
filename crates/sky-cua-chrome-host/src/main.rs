mod app_server;
mod frame;
#[cfg(unix)]
mod host;
mod manifest;

use anyhow::{Context, Result, bail};
use manifest::{Browser, HostManifest, host_manifest_path};
use std::path::PathBuf;

const CODEX_COMPAT_HOST_NAME: &str = "com.openai.codexextension";
const SKY_CUA_ALIAS_HOST_NAME: &str = "io.github.becksclair.sky_cua_extension";
const SKY_CUA_CHROME_HOST_COMPAT_CODEX_ENV: &str = "SKY_CUA_CHROME_HOST_COMPAT_CODEX";

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("install-manifest") => install_manifest(&args[1..]),
        Some("serve") => host_serve(&args[1..]),
        Some(origin) if origin.starts_with("chrome-extension://") => {
            host_serve_with_origin(&[], Some(origin))
        }
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        None => host_serve(&[]),
        Some(other) => bail!("unsupported sky-cua-chrome-host command: {other}"),
    }
}

fn install_manifest(args: &[String]) -> Result<()> {
    let mut browser = None;
    let mut host_name = default_host_name();
    let mut extension_id = None;
    let mut host_path = std::env::current_exe().context("failed to resolve current executable")?;
    let mut compat_codex = env_flag(SKY_CUA_CHROME_HOST_COMPAT_CODEX_ENV);

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--browser" => {
                browser = Some(parse_value(args, &mut index, "--browser")?.parse()?);
            }
            "--host-name" => host_name = parse_value(args, &mut index, "--host-name")?,
            "--extension-id" => {
                extension_id = Some(parse_value(args, &mut index, "--extension-id")?)
            }
            "--host-path" => {
                host_path = PathBuf::from(parse_value(args, &mut index, "--host-path")?)
            }
            "--compat-codex" => compat_codex = true,
            "--sky-cua-alias" => host_name = SKY_CUA_ALIAS_HOST_NAME.to_string(),
            other => bail!("unsupported install-manifest argument: {other}"),
        }
        index += 1;
    }

    let browser = browser.context("install-manifest requires --browser chrome|brave|chromium")?;
    let extension_id = extension_id.context("install-manifest requires --extension-id")?;
    write_manifest(browser, &host_name, &extension_id, &host_path)?;
    if compat_codex {
        write_manifest(browser, CODEX_COMPAT_HOST_NAME, &extension_id, &host_path)?;
    }
    Ok(())
}

fn default_host_name() -> String {
    std::env::var("SKY_CUA_CHROME_HOST_NAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| CODEX_COMPAT_HOST_NAME.to_string())
}

fn parse_host_name_args(args: &[String]) -> Result<String> {
    let mut host_name = default_host_name();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--host-name" => host_name = parse_value(args, &mut index, "--host-name")?,
            "--sky-cua-alias" => host_name = SKY_CUA_ALIAS_HOST_NAME.to_string(),
            other => bail!("unsupported serve argument: {other}"),
        }
        index += 1;
    }
    Ok(host_name)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| truthy(&value))
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn write_manifest(
    browser: Browser,
    host_name: &str,
    extension_id: &str,
    host_path: &std::path::Path,
) -> Result<()> {
    let manifest = HostManifest::new(host_name, host_path, extension_id)?;
    let path = host_manifest_path(browser, host_name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let rendered = serde_json::to_vec_pretty(&manifest)?;
    std::fs::write(&path, [rendered.as_slice(), b"\n"].concat())
        .with_context(|| format!("failed to write {}", path.display()))?;
    eprintln!("wrote native host manifest {}", path.display());
    Ok(())
}

fn parse_value(args: &[String], index: &mut usize, flag: &str) -> Result<String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .with_context(|| format!("{flag} requires a value"))
}

#[cfg(unix)]
fn host_serve(args: &[String]) -> Result<()> {
    host_serve_with_origin(args, None)
}

#[cfg(unix)]
fn host_serve_with_origin(args: &[String], origin: Option<&str>) -> Result<()> {
    let host_name = parse_host_name_args(args)?;
    host::serve(host_name, extension_id_from_origin(origin))
}

#[cfg(not(unix))]
fn host_serve(_args: &[String]) -> Result<()> {
    bail!("serve mode is only available on Unix platforms")
}

#[cfg(not(unix))]
fn host_serve_with_origin(_args: &[String], _origin: Option<&str>) -> Result<()> {
    bail!("serve mode is only available on Unix platforms")
}

fn extension_id_from_origin(origin: Option<&str>) -> Option<String> {
    let origin = origin?;
    origin
        .strip_prefix("chrome-extension://")
        .and_then(|value| value.strip_suffix('/'))
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .map(str::to_string)
}

fn print_help() {
    println!(
        "sky-cua-chrome-host\n\nUsage:\n  sky-cua-chrome-host install-manifest --browser chrome|brave|chromium --extension-id EXTENSION_ID [--host-name NAME] [--host-path PATH] [--compat-codex] [--sky-cua-alias]\n  sky-cua-chrome-host serve [--host-name NAME] [--sky-cua-alias]\n  sky-cua-chrome-host\n\nDefault host name: com.openai.codexextension\nNo-argument mode is the Chrome native messaging entrypoint."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_serve_host_name_override() {
        assert_eq!(
            parse_host_name_args(&["--host-name".to_string(), "io.github.test".to_string()])
                .unwrap(),
            "io.github.test"
        );
    }

    #[test]
    fn parses_serve_sky_cua_alias() {
        assert_eq!(
            parse_host_name_args(&["--sky-cua-alias".to_string()]).unwrap(),
            SKY_CUA_ALIAS_HOST_NAME
        );
    }

    #[test]
    fn parses_truthy_env_flags() {
        assert!(truthy("1"));
        assert!(truthy("true"));
        assert!(truthy("YES"));
        assert!(truthy("on"));
        assert!(!truthy("false"));
        assert!(!truthy(""));
    }

    #[test]
    fn extracts_extension_id_from_native_messaging_origin() {
        assert_eq!(
            extension_id_from_origin(Some("chrome-extension://hehggadaopoacecdllhhajmbjkdcmajg/")),
            Some("hehggadaopoacecdllhhajmbjkdcmajg".to_string())
        );
        assert_eq!(extension_id_from_origin(None), None);
        assert_eq!(
            extension_id_from_origin(Some("chrome-extension://bad/path/")),
            None
        );
    }
}
