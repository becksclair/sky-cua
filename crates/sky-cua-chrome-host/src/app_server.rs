mod proxy;

use self::proxy::ProxyServer;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
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
    proxy: ProxyServer,
}

#[derive(Debug)]
pub(crate) struct AppServerControlError {
    message: String,
    error_type: Option<&'static str>,
}

pub(crate) type AppServerControlResult = std::result::Result<Value, AppServerControlError>;

#[derive(Clone, Copy)]
enum ResponseUrlScheme {
    Http,
    WebSocket,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeConstraints {
    extension_build_channel: String,
    extension_id: String,
    extension_version: String,
    native_host_name: String,
    required_app_server_protocol_version: u64,
    required_native_host_protocol_version: u64,
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

    pub fn hello(&self, params: Option<&Value>) -> AppServerControlResult {
        let constraints = runtime_constraints(params)?;
        let entry = self
            .select_entry(Some(&constraints))
            .map_err(AppServerControlError::no_matching_install)?;
        Ok(json!({
            "manifestSchemaVersion": entry.schema_version,
            "nativeHostProtocolVersion": entry.native_host_protocol_version,
            "supportedProtocolVersions": [REQUIRED_NATIVE_HOST_PROTOCOL_VERSION],
            "supportedMethods": []
        }))
    }

    pub fn ensure(&self, params: Option<&Value>) -> AppServerControlResult {
        let constraints = runtime_constraints(params)?;
        self.start_or_reuse(Some(&constraints), true, ResponseUrlScheme::WebSocket)
    }

    pub fn ensure_legacy(&self) -> AppServerControlResult {
        self.start_or_reuse(None, true, ResponseUrlScheme::Http)
    }

    pub fn restart(&self, params: Option<&Value>) -> AppServerControlResult {
        let constraints = runtime_constraints(params)?;
        self.start_or_reuse(Some(&constraints), false, ResponseUrlScheme::WebSocket)
    }

    fn start_or_reuse(
        &self,
        constraints: Option<&RuntimeConstraints>,
        reuse_live_process: bool,
        response_url_scheme: ResponseUrlScheme,
    ) -> AppServerControlResult {
        let entry = self
            .select_entry(constraints)
            .map_err(AppServerControlError::no_matching_install)?;
        let mut process = self
            .process
            .lock()
            .expect("app server manager mutex poisoned");
        if reuse_live_process
            && let Some(current) = process.as_mut()
            && current
                .child
                .try_wait()
                .map_err(anyhow::Error::from)
                .map_err(AppServerControlError::untyped)?
                .is_none()
            && current.entry.entry_id == entry.entry_id
            && current.proxy.is_running()
        {
            return Ok(status_value(current, response_url_scheme));
        }
        *process = None;

        let started = start_app_server(entry, self.extension_id.as_deref())
            .map_err(AppServerControlError::untyped)?;
        let status = status_value(&started, response_url_scheme);
        *process = Some(started);
        Ok(status)
    }

    fn select_entry(&self, constraints: Option<&RuntimeConstraints>) -> Result<AppServerEntry> {
        select_entry(
            &resources_config_paths(),
            self.host_name.as_deref(),
            self.extension_id.as_deref(),
            constraints,
        )
    }
}

fn runtime_constraints(
    params: Option<&Value>,
) -> std::result::Result<RuntimeConstraints, AppServerControlError> {
    let params = params
        .context("Codex runtime request is missing params")
        .map_err(AppServerControlError::untyped)?;
    let constraints = params
        .get("constraints")
        .context("Codex runtime request is missing constraints")
        .map_err(AppServerControlError::untyped)?;
    let constraints: RuntimeConstraints = serde_json::from_value(constraints.clone())
        .context("Codex runtime constraints are invalid")
        .map_err(AppServerControlError::untyped)?;
    if constraints.extension_version.trim().is_empty() {
        return Err(AppServerControlError::untyped(anyhow::anyhow!(
            "Codex extension version is empty"
        )));
    }
    if constraints.required_app_server_protocol_version != REQUIRED_APP_SERVER_PROTOCOL_VERSION
        || constraints.required_native_host_protocol_version
            != REQUIRED_NATIVE_HOST_PROTOCOL_VERSION
    {
        return Err(AppServerControlError::typed(
            "version_mismatch",
            anyhow::anyhow!("Codex runtime protocol version is incompatible with this native host"),
        ));
    }
    Ok(constraints)
}

impl AppServerControlError {
    fn untyped(error: anyhow::Error) -> Self {
        Self {
            message: error.to_string(),
            error_type: None,
        }
    }

    fn no_matching_install(error: anyhow::Error) -> Self {
        Self::typed("no_matching_codex_install", error)
    }

    pub(crate) fn typed(error_type: &'static str, error: anyhow::Error) -> Self {
        Self {
            message: error.to_string(),
            error_type: Some(error_type),
        }
    }

    pub(crate) fn error_type(&self) -> Option<&'static str> {
        self.error_type
    }
}

impl fmt::Display for AppServerControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppServerControlError {}

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
    constraints: Option<&RuntimeConstraints>,
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
            entry_is_compatible(entry, host_name, extension_id, constraints)
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
    constraints: Option<&RuntimeConstraints>,
) -> bool {
    entry.schema_version == REQUIRED_SCHEMA_VERSION
        && entry.app_server_protocol_version == REQUIRED_APP_SERVER_PROTOCOL_VERSION
        && entry.native_host_protocol_version == REQUIRED_NATIVE_HOST_PROTOCOL_VERSION
        && host_name.is_none_or(|name| entry.native_host_names.iter().any(|item| item == name))
        && extension_id.is_none_or(|id| entry.extension_ids.iter().any(|item| item == id))
        && constraints.map_or_else(
            || {
                entry
                    .extension_build_channels
                    .iter()
                    .any(|channel| channel == &entry.channel)
            },
            |constraints| {
                constraints.required_app_server_protocol_version
                    == REQUIRED_APP_SERVER_PROTOCOL_VERSION
                    && constraints.required_native_host_protocol_version
                        == REQUIRED_NATIVE_HOST_PROTOCOL_VERSION
                    && entry
                        .extension_build_channels
                        .iter()
                        .any(|channel| channel == &constraints.extension_build_channel)
                    && entry
                        .extension_ids
                        .iter()
                        .any(|id| id == &constraints.extension_id)
                    && entry
                        .native_host_names
                        .iter()
                        .any(|name| name == &constraints.native_host_name)
                    && extension_id.is_none_or(|id| id == constraints.extension_id)
                    && host_name.is_none_or(|name| name == constraints.native_host_name)
            },
        )
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
    let proxy_listener = TcpListener::bind((entry.proxy_host.as_str(), entry.proxy_port))
        .with_context(|| {
            format!(
                "failed to bind Codex app-server proxy on {}:{}",
                entry.proxy_host, entry.proxy_port
            )
        })?;
    let proxy_address = proxy_listener.local_addr()?;
    if !proxy_address.ip().is_loopback() {
        bail!("Codex app-server proxy must bind a loopback address");
    }
    let listen_url = format!("ws://{}:0", entry.proxy_host);
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
        .env(
            "CODEX_APP_SERVER_PROXY_PORT",
            proxy_address.port().to_string(),
        )
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
    let backend_url = loop {
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

    let proxy = match ProxyServer::start(proxy_listener, &backend_url, extension_id) {
        Ok(proxy) => proxy,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let local_app_server_url = format!("ws://{proxy_address}");

    Ok(AppServerProcess {
        child,
        entry,
        local_app_server_url,
        proxy,
    })
}

fn parse_listening_url(line: &str) -> Option<String> {
    let value = line.trim().strip_prefix("listening on:")?.trim();
    let url = Url::parse(value).ok()?;
    if url.scheme() != "ws" || url.host_str().is_none() || url.port().is_none() {
        return None;
    }
    Some(url.to_string().trim_end_matches('/').to_string())
}

fn status_value(process: &AppServerProcess, response_url_scheme: ResponseUrlScheme) -> Value {
    let entry = &process.entry;
    let local_app_server_url = match response_url_scheme {
        ResponseUrlScheme::Http => process
            .local_app_server_url
            .strip_prefix("ws://")
            .map_or_else(
                || process.local_app_server_url.clone(),
                |address| format!("http://{address}"),
            ),
        ResponseUrlScheme::WebSocket => process.local_app_server_url.clone(),
    };
    json!({
        "entryId": entry.entry_id,
        "localAppServerUrl": local_app_server_url,
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

    fn fixture_constraints() -> RuntimeConstraints {
        RuntimeConstraints {
            extension_build_channel: "prod".to_string(),
            extension_id: "extension-1".to_string(),
            extension_version: "1.2.27203.26575".to_string(),
            native_host_name: "com.openai.codexextension".to_string(),
            required_app_server_protocol_version: 2,
            required_native_host_protocol_version: 2,
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

        let constraints = fixture_constraints();
        let selected = select_entry(
            &[manifest],
            Some("com.openai.codexextension"),
            Some("extension-1"),
            Some(&constraints),
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
            Some("extension-1"),
            None,
        ));
        assert!(!entry_is_compatible(
            &entry,
            Some("com.openai.codexextension"),
            Some("extension-2"),
            None,
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_incompatible_runtime_constraints() {
        let root = temp_dir("app-server-runtime-constraints");
        let entry = fixture_entry(&root);
        let mut constraints = fixture_constraints();
        constraints.required_native_host_protocol_version = 3;
        assert!(!entry_is_compatible(
            &entry,
            Some("com.openai.codexextension"),
            Some("extension-1"),
            Some(&constraints),
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_selection_preserves_entry_channel_compatibility() {
        let root = temp_dir("app-server-legacy-channel");
        let mut entry = fixture_entry(&root);
        entry.channel = "internal".to_string();

        assert!(!entry_is_compatible(
            &entry,
            Some("com.openai.codexextension"),
            Some("extension-1"),
            None,
        ));
        assert!(entry_is_compatible(
            &entry,
            Some("com.openai.codexextension"),
            Some("extension-1"),
            Some(&fixture_constraints()),
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn parses_installed_extension_runtime_constraints() {
        let params = json!({
            "constraints": {
                "extensionBuildChannel": "prod",
                "extensionId": "extension-1",
                "extensionVersion": "1.2.27203.26575",
                "nativeHostName": "com.openai.codexextension",
                "requiredAppServerProtocolVersion": 2,
                "requiredNativeHostProtocolVersion": 2
            }
        });
        let constraints = runtime_constraints(Some(&params)).unwrap();
        assert_eq!(constraints.extension_build_channel, "prod");
        assert_eq!(constraints.required_native_host_protocol_version, 2);
    }

    #[test]
    fn incompatible_runtime_protocol_has_a_typed_error() {
        let params = json!({
            "constraints": {
                "extensionBuildChannel": "prod",
                "extensionId": "extension-1",
                "extensionVersion": "1.2.27203.26575",
                "nativeHostName": "com.openai.codexextension",
                "requiredAppServerProtocolVersion": 3,
                "requiredNativeHostProtocolVersion": 2
            }
        });

        let error = match runtime_constraints(Some(&params)) {
            Ok(_) => panic!("incompatible runtime protocol should fail"),
            Err(error) => error,
        };
        assert_eq!(error.error_type(), Some("version_mismatch"));
    }

    #[test]
    fn parses_codex_websocket_listening_address() {
        assert_eq!(
            parse_listening_url("  listening on: ws://127.0.0.1:46287"),
            Some("ws://127.0.0.1:46287".to_string())
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
        let local_url = status_value(&process, ResponseUrlScheme::WebSocket)["localAppServerUrl"]
            .as_str()
            .unwrap()
            .to_string();
        let local_url = Url::parse(&local_url).unwrap();
        assert_eq!(local_url.scheme(), "ws");
        assert_eq!(local_url.host_str(), Some("127.0.0.1"));
        assert!(local_url.port().is_some());
        assert!(
            status_value(&process, ResponseUrlScheme::Http)["localAppServerUrl"]
                .as_str()
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );
        drop(process);
        fs::remove_dir_all(root).unwrap();
    }
}
