use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

const RESOURCES_FILE_NAME: &str = "chrome-native-hosts-v2.json";
const REQUIRED_SCHEMA_VERSION: u64 = 2;
const REQUIRED_APP_SERVER_PROTOCOL_VERSION: u64 = 2;
const REQUIRED_NATIVE_HOST_PROTOCOL_VERSION: u64 = 2;
const APP_SERVER_START_TIMEOUT: Duration = Duration::from_secs(15);

pub struct AppServerManager {
    host_name: Option<String>,
    extension_id: Option<String>,
    process: Mutex<Option<AppServerProcess>>,
}

struct AppServerProcess {
    child: Child,
    entry: AppServerEntry,
    local_app_server_url: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourcesConfig {
    schema_version: u64,
    entries: Vec<AppServerEntry>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppServerEntry {
    schema_version: u64,
    app_server_protocol_version: u64,
    app_version: String,
    channel: String,
    cli_version: String,
    entry_id: String,
    extension_build_channels: Vec<String>,
    extension_ids: Vec<String>,
    native_host_names: Vec<String>,
    native_host_protocol_version: u64,
    native_host_version: String,
    paths: AppServerPaths,
    proxy_host: String,
    proxy_port: u16,
    updated_at: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppServerPaths {
    browser_client_path: String,
    codex_cli_path: String,
    codex_home: String,
    node_module_dirs: Option<Vec<String>>,
    node_path: String,
    node_repl_path: String,
    resources_path: String,
}

impl AppServerManager {
    pub fn new(host_name: Option<String>, extension_id: Option<String>) -> Self {
        Self {
            host_name,
            extension_id,
            process: Mutex::new(None),
        }
    }

    pub fn ensure(&self) -> Result<Value> {
        let mut process = self
            .process
            .lock()
            .expect("app server manager mutex poisoned");
        if let Some(current) = process.as_mut()
            && current.child.try_wait()?.is_none()
        {
            return Ok(status_value(current));
        }
        *process = None;

        let entry = select_entry(
            &resources_config_paths(),
            self.host_name.as_deref(),
            self.extension_id.as_deref(),
        )?;
        let started = start_app_server(entry, self.extension_id.as_deref())?;
        let status = status_value(&started);
        *process = Some(started);
        Ok(status)
    }
}

impl Drop for AppServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn resources_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(state_home) = env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
        paths.push(state_home.join("openai-codex").join(RESOURCES_FILE_NAME));
    } else if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        paths.push(
            home.join(".local")
                .join("state")
                .join("openai-codex")
                .join(RESOURCES_FILE_NAME),
        );
    }
    if let Some(codex_home) = codex_home() {
        paths.push(codex_home.join(RESOURCES_FILE_NAME));
    }
    paths.sort();
    paths.dedup();
    paths
}

fn codex_home() -> Option<PathBuf> {
    env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
}

fn select_entry(
    paths: &[PathBuf],
    host_name: Option<&str>,
    extension_id: Option<&str>,
) -> Result<AppServerEntry> {
    let mut entries = Vec::new();
    let mut found_config = false;
    for path in paths {
        let contents = match fs::read(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        found_config = true;
        let config: ResourcesConfig = serde_json::from_slice(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        if config.schema_version != REQUIRED_SCHEMA_VERSION {
            continue;
        }
        entries.extend(config.entries.into_iter().filter(|entry| {
            entry_is_compatible(entry, host_name, extension_id)
                && required_paths_exist(&entry.paths)
        }));
    }

    if !found_config {
        bail!("Codex Chrome native host v2 manifest is missing");
    }
    entries.sort_by(|left, right| left.updated_at.cmp(&right.updated_at));
    entries
        .pop()
        .context("No compatible Codex app-server entry was found")
}

fn entry_is_compatible(
    entry: &AppServerEntry,
    host_name: Option<&str>,
    extension_id: Option<&str>,
) -> bool {
    entry.schema_version == REQUIRED_SCHEMA_VERSION
        && entry.app_server_protocol_version == REQUIRED_APP_SERVER_PROTOCOL_VERSION
        && entry.native_host_protocol_version == REQUIRED_NATIVE_HOST_PROTOCOL_VERSION
        && host_name.is_none_or(|name| entry.native_host_names.iter().any(|item| item == name))
        && extension_id.is_none_or(|id| entry.extension_ids.iter().any(|item| item == id))
        && entry
            .extension_build_channels
            .iter()
            .any(|channel| channel == &entry.channel)
        && !entry.entry_id.trim().is_empty()
}

fn required_paths_exist(paths: &AppServerPaths) -> bool {
    [
        &paths.browser_client_path,
        &paths.codex_cli_path,
        &paths.node_path,
        &paths.node_repl_path,
        &paths.resources_path,
    ]
    .into_iter()
    .all(|path| Path::new(path).exists())
}

fn start_app_server(entry: AppServerEntry, extension_id: Option<&str>) -> Result<AppServerProcess> {
    let listen_url = format!("ws://{}:{}", entry.proxy_host, entry.proxy_port);
    let mut command = Command::new(&entry.paths.codex_cli_path);
    command
        .arg("app-server")
        .arg("--listen")
        .arg(&listen_url)
        .arg("--analytics-default-enabled")
        .env("CODEX_CLI_PATH", &entry.paths.codex_cli_path)
        .env("CODEX_HOME", &entry.paths.codex_home)
        .env("CODEX_BROWSER_USE_NODE_PATH", &entry.paths.node_path)
        .env(
            "CODEX_BROWSER_CLIENT_PATH",
            &entry.paths.browser_client_path,
        )
        .env("CODEX_NODE_REPL_PATH", &entry.paths.node_repl_path)
        .env("CODEX_APP_SERVER_PROXY_HOST", &entry.proxy_host)
        .env("CODEX_APP_SERVER_PROXY_PORT", entry.proxy_port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(extension_id) = extension_id {
        command.env("CODEX_EXTENSION_ID", extension_id);
    }
    if let Some(module_dirs) = entry.paths.node_module_dirs.as_ref() {
        command.env("NODE_PATH", env::join_paths(module_dirs)?);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start Codex app-server with {}", listen_url))?;
    let stderr = child
        .stderr
        .take()
        .context("Codex app-server stderr is unavailable")?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else {
                break;
            };
            let _ = sender.send(line.clone());
            eprintln!("[sky-cua-chrome-host] app-server stderr: {line}");
        }
    });

    let deadline = Instant::now() + APP_SERVER_START_TIMEOUT;
    let local_app_server_url = loop {
        if let Some(status) = child.try_wait()? {
            bail!("Codex app-server exited during startup with {status}");
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Timed out waiting for Codex app-server to report its listening address");
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(line) => {
                if let Some(url) = parse_listening_url(&line) {
                    break url;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                bail!("Codex app-server stderr closed before reporting its listening address");
            }
        }
    };

    Ok(AppServerProcess {
        child,
        entry,
        local_app_server_url,
    })
}

fn parse_listening_url(line: &str) -> Option<String> {
    let value = line.trim().strip_prefix("listening on:")?.trim();
    let url = Url::parse(value).ok()?;
    if url.scheme() != "ws" || url.host_str().is_none() || url.port().is_none() {
        return None;
    }
    let mut http_url = url;
    http_url.set_scheme("http").ok()?;
    Some(http_url.to_string().trim_end_matches('/').to_string())
}

fn status_value(process: &AppServerProcess) -> Value {
    let entry = &process.entry;
    json!({
        "entryId": entry.entry_id,
        "localAppServerUrl": process.local_app_server_url,
        "appServerProtocolVersion": entry.app_server_protocol_version,
        "appVersion": entry.app_version,
        "channel": entry.channel,
        "cliVersion": entry.cli_version,
        "nativeHostProtocolVersion": entry.native_host_protocol_version,
        "nativeHostVersion": entry.native_host_version,
        "runtimeConfig": {
            "desktopAgentModeDefaults": {},
            "trustedBrowserClientSha256s": []
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture_paths(root: &Path) -> AppServerPaths {
        let files = [
            "browser-client.mjs",
            "codex",
            "node",
            "node_repl",
            "resources",
        ];
        for file in files {
            let path = root.join(file);
            if file == "resources" {
                fs::create_dir_all(path).unwrap();
            } else {
                fs::write(path, "fixture").unwrap();
            }
        }
        AppServerPaths {
            browser_client_path: root.join("browser-client.mjs").display().to_string(),
            codex_cli_path: root.join("codex").display().to_string(),
            codex_home: root.display().to_string(),
            node_module_dirs: None,
            node_path: root.join("node").display().to_string(),
            node_repl_path: root.join("node_repl").display().to_string(),
            resources_path: root.join("resources").display().to_string(),
        }
    }

    fn fixture_entry(root: &Path) -> AppServerEntry {
        AppServerEntry {
            schema_version: 2,
            app_server_protocol_version: 2,
            app_version: "0.1.7".to_string(),
            channel: "prod".to_string(),
            cli_version: "0.1.7".to_string(),
            entry_id: "entry-1".to_string(),
            extension_build_channels: vec!["prod".to_string()],
            extension_ids: vec!["extension-1".to_string()],
            native_host_names: vec!["com.openai.codexextension".to_string()],
            native_host_protocol_version: 2,
            native_host_version: "0.1.7".to_string(),
            paths: fixture_paths(root),
            proxy_host: "127.0.0.1".to_string(),
            proxy_port: 0,
            updated_at: "2026-07-16T00:00:00Z".to_string(),
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("sky-cua-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn selects_latest_compatible_v2_entry() {
        let root = temp_dir("app-server-entry");
        let mut older = fixture_entry(&root);
        older.entry_id = "older".to_string();
        older.updated_at = "2026-07-15T00:00:00Z".to_string();
        let newer = fixture_entry(&root);
        let manifest = root.join(RESOURCES_FILE_NAME);
        fs::write(
            &manifest,
            serde_json::to_vec(&json!({
                "schemaVersion": 2,
                "entries": [older, newer]
            }))
            .unwrap(),
        )
        .unwrap();

        let selected = select_entry(
            &[manifest],
            Some("com.openai.codexextension"),
            Some("extension-1"),
        )
        .unwrap();
        assert_eq!(selected.entry_id, "entry-1");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_wrong_host_or_extension() {
        let root = temp_dir("app-server-constraints");
        let entry = fixture_entry(&root);
        assert!(!entry_is_compatible(
            &entry,
            Some("com.openai.codexextension.dev"),
            Some("extension-1")
        ));
        assert!(!entry_is_compatible(
            &entry,
            Some("com.openai.codexextension"),
            Some("extension-2")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_codex_websocket_listening_address_as_http_url() {
        assert_eq!(
            parse_listening_url("  listening on: ws://127.0.0.1:46287"),
            Some("http://127.0.0.1:46287".to_string())
        );
        assert_eq!(
            parse_listening_url("readyz: http://127.0.0.1:46287/readyz"),
            None
        );
    }

    #[test]
    #[cfg(unix)]
    fn starts_configured_app_server_and_reuses_its_status() {
        let root = temp_dir("app-server-start");
        let entry = fixture_entry(&root);
        fs::write(
            &entry.paths.codex_cli_path,
            "#!/bin/sh\necho '  listening on: ws://127.0.0.1:47531' >&2\nexec sleep 60\n",
        )
        .unwrap();
        fs::set_permissions(
            &entry.paths.codex_cli_path,
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let process = start_app_server(entry.clone(), Some("extension-1")).unwrap();
        assert_eq!(
            status_value(&process)["localAppServerUrl"],
            json!("http://127.0.0.1:47531")
        );
        drop(process);
        fs::remove_dir_all(root).unwrap();
    }
}
