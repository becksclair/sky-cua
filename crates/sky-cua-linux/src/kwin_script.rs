//! KWin scripting channel for active-window readback and verified activation.
//!
//! KWin Wayland exposes no non-interactive DBus method for "current active
//! window" (`org.kde.KWin.queryWindowInfo` is an interactive picker) and no
//! foreign-toplevel Wayland protocol to unprivileged clients. The reliable
//! seam is the KWin scripting API: load a JavaScript snippet through
//! `org.kde.KWin /Scripting`, and have the script push its result back over
//! the session bus with `callDBus` to a callback object served by this
//! process. This is the same pattern kdotool uses.
//!
//! Three flows are built on that channel:
//! - one-shot active-window queries (`active_window`),
//! - activation with in-compositor readback (`activate_window_verified`),
//! - a persistent watcher script that streams `windowActivated` events into a
//!   local cache so repeated `focused_window` calls avoid script churn.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sky_cua_platform::diagnostics::{BackendError, BackendErrorCode};
use tokio::sync::OnceCell;
use tokio::sync::oneshot;

const CALLBACK_PATH: &str = "/com/skycua/KWinScript";
const CALLBACK_INTERFACE: &str = "com.skycua.KWinScript";
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a positive `isScriptLoaded` watcher check is trusted before the
/// liveness probe runs again. Keeps steady-state cache reads free of DBus
/// round-trips while still noticing a KWin restart within a few seconds.
const WATCHER_LIVENESS_TTL: Duration = Duration::from_secs(5);

/// Active-window fields pushed out of KWin scripts as a JSON payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveWindow {
    /// Normalized (braceless, lowercase) KWin internalId UUID.
    pub uuid: String,
    pub caption: Option<String>,
    pub resource_class: Option<String>,
    pub pid: Option<i64>,
}

/// Outcome of a verified activation script run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationVerdict {
    /// The script activated the window and read it back as active.
    Verified,
    /// The script activated the window, but focus had not landed by the time
    /// it read `workspace.activeWindow`; callers should poll `focused_window`.
    Dispatched,
    /// The script found no window with the requested internalId.
    NoMatch,
}

pub fn normalize_uuid(value: &str) -> String {
    value.trim().trim_matches(['{', '}']).to_ascii_lowercase()
}

/// Query the active window, preferring the persistent watcher cache and
/// falling back to a transient one-shot script.
pub async fn active_window() -> Result<Option<ActiveWindow>, BackendError> {
    let channel = channel().await?;
    if let Some(cached) = channel.cached_active_window().await? {
        return Ok(cached);
    }
    channel.query_active_window().await
}

/// Normalized internalId UUID of the active window, if any.
pub async fn active_window_uuid() -> Result<Option<String>, BackendError> {
    Ok(active_window().await?.map(|window| window.uuid))
}

/// Activate `uuid` (normalized internalId) and read back the active window
/// inside the same script run.
pub async fn activate_window_verified(uuid: &str) -> Result<ActivationVerdict, BackendError> {
    let channel = channel().await?;
    let verdict = channel
        .run_transient(&activation_script_body(&normalize_uuid(uuid)))
        .await?;
    match verdict.as_str() {
        "verified" => Ok(ActivationVerdict::Verified),
        "dispatched" => Ok(ActivationVerdict::Dispatched),
        "no-match" => Ok(ActivationVerdict::NoMatch),
        other => Err(BackendError::new(
            BackendErrorCode::Internal,
            format!("KWin activation script returned an unexpected verdict: {other}"),
        )),
    }
}

/// Best-effort graceful-exit cleanup: unload this process's persistent
/// focus watcher from KWin and remove its backing script file. Without
/// this, a daemon exit leaves the watcher firing `callDBus` to a dead bus
/// name on every focus change until the next sky-cua start sweeps it or
/// KWin restarts. Crashes still rely on `sweep_stale_watchers`.
pub async fn shutdown() {
    let Some(channel) = CHANNEL.get() else {
        return;
    };
    let _gate = channel.watcher_gate.lock().await;
    if let Ok(scripting) = channel.scripting_proxy().await {
        let _ = call_with_timeout::<i32>(
            &scripting,
            "unloadScript",
            &(channel.watcher_plugin.as_str(),),
        )
        .await;
    }
    if let Some(path) = channel
        .watcher_script_path
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
    {
        let _ = fs::remove_file(path);
    }
    if let Ok(mut verified) = channel.watcher_verified_at.lock() {
        *verified = None;
    }
}

struct KWinScriptChannel {
    connection: zbus::Connection,
    bus_name: String,
    pending: Arc<StdMutex<HashMap<String, oneshot::Sender<String>>>>,
    /// `None` = no event observed yet; `Some(None)` = no active window.
    active_cache: Arc<StdMutex<Option<Option<ActiveWindow>>>>,
    watcher_plugin: String,
    /// Serializes watcher (re)loads: concurrent callers racing the
    /// `isScriptLoaded` check would otherwise double-load the plugin,
    /// duplicate the signal subscription, and orphan a temp script file.
    watcher_gate: tokio::sync::Mutex<()>,
    /// Script file backing the currently loaded watcher, removed on reload.
    watcher_script_path: StdMutex<Option<PathBuf>>,
    /// Last time the watcher was confirmed loaded; see WATCHER_LIVENESS_TTL.
    watcher_verified_at: StdMutex<Option<std::time::Instant>>,
    token_counter: AtomicU64,
}

static CHANNEL: OnceCell<KWinScriptChannel> = OnceCell::const_new();

async fn channel() -> Result<&'static KWinScriptChannel, BackendError> {
    CHANNEL.get_or_try_init(KWinScriptChannel::connect).await
}

struct CallbackService {
    pending: Arc<StdMutex<HashMap<String, oneshot::Sender<String>>>>,
    active_cache: Arc<StdMutex<Option<Option<ActiveWindow>>>>,
}

#[zbus::interface(name = "com.skycua.KWinScript")]
impl CallbackService {
    /// Receives one-shot script results: `callDBus(bus, path, iface, "Result", token, payload)`.
    #[zbus(name = "Result")]
    fn result(&self, token: String, payload: String) {
        let sender = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&token));
        if let Some(sender) = sender {
            let _ = sender.send(payload);
        }
    }

    /// Receives watcher-script focus events as active-window JSON payloads.
    #[zbus(name = "ActiveWindowChanged")]
    fn active_window_changed(&self, payload: String) {
        if let Ok(mut cache) = self.active_cache.lock() {
            *cache = Some(parse_active_window_payload(&payload));
        }
    }
}

impl KWinScriptChannel {
    async fn connect() -> Result<Self, BackendError> {
        let connection = zbus::Connection::session().await.map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("failed to connect to the session bus for KWin scripting: {error}"),
            )
        })?;
        let pending = Arc::new(StdMutex::new(HashMap::new()));
        let active_cache = Arc::new(StdMutex::new(None));
        connection
            .object_server()
            .at(
                CALLBACK_PATH,
                CallbackService {
                    pending: Arc::clone(&pending),
                    active_cache: Arc::clone(&active_cache),
                },
            )
            .await
            .map_err(|error| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    format!("failed to register the KWin script callback object: {error}"),
                )
            })?;
        let bus_name = connection
            .unique_name()
            .map(|name| name.to_string())
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    "session bus connection has no unique name for KWin script callbacks",
                )
            })?;
        let channel = Self {
            connection,
            bus_name,
            pending,
            active_cache,
            watcher_plugin: format!("sky-cua-focus-watch-{}", std::process::id()),
            watcher_gate: tokio::sync::Mutex::new(()),
            watcher_script_path: StdMutex::new(None),
            watcher_verified_at: StdMutex::new(None),
            token_counter: AtomicU64::new(0),
        };
        channel.sweep_stale_watchers().await;
        Ok(channel)
    }

    /// Unload scripts left behind by dead sky-cua processes. There is no
    /// shutdown hook for the static channel, so a crashed or exited daemon
    /// can leave its pid-keyed watcher (or, after a mid-run crash, a
    /// transient query script) loaded in KWin — the watcher then fires a
    /// failing `callDBus` on every focus change until the compositor
    /// restarts. The orphaned temp script files record which plugin names
    /// were loaded; pid-keyed names stay per-process so concurrent live
    /// daemons never reclaim each other's scripts.
    async fn sweep_stale_watchers(&self) {
        let Ok(scripting) = self.scripting_proxy().await else {
            return;
        };
        let Ok(entries) = fs::read_dir(env::temp_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            let Some((plugin, pid)) = stale_script_plugin(name) else {
                continue;
            };
            if pid == std::process::id() || PathBuf::from(format!("/proc/{pid}")).exists() {
                continue;
            }
            let _ = call_with_timeout::<i32>(&scripting, "unloadScript", &(plugin.as_str(),)).await;
            let _ = fs::remove_file(entry.path());
        }
    }

    async fn scripting_proxy(&self) -> Result<zbus::Proxy<'static>, BackendError> {
        zbus::Proxy::new(
            &self.connection,
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("KWin scripting is unavailable on the session bus: {error}"),
            )
        })
    }

    /// Run a transient script that reports through `send(payload)` and return
    /// the payload. The script is loaded, run, stopped, and unloaded.
    async fn run_transient(&self, body: &str) -> Result<String, BackendError> {
        let token = self
            .token_counter
            .fetch_add(1, Ordering::Relaxed)
            .to_string();
        let plugin = format!("sky-cua-kwin-query-{}-{token}", std::process::id());
        let script = format!("{}\n{body}", send_helper(&self.bus_name, &token),);
        let path = write_script_file(&plugin, &script)?;
        let path_string = path.display().to_string();

        let (sender, receiver) = oneshot::channel();
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(token.clone(), sender);
        }

        let result = self
            .load_and_run(&plugin, &path_string, Some(receiver))
            .await;

        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&token);
        }
        let _ = fs::remove_file(&path);
        result.and_then(|payload| {
            payload.ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::Internal,
                    "KWin transient script ran without reporting a result",
                )
            })
        })
    }

    /// Load + run a script; when `receiver` is given, await its payload with a
    /// timeout and stop/unload the script afterwards. Without a receiver the
    /// script is left loaded (persistent watcher).
    async fn load_and_run(
        &self,
        plugin: &str,
        path: &str,
        receiver: Option<oneshot::Receiver<String>>,
    ) -> Result<Option<String>, BackendError> {
        let scripting = self.scripting_proxy().await?;
        // Drop any stale instance from a previous crashed run.
        let _ = call_with_timeout::<i32>(&scripting, "unloadScript", &(plugin,)).await;
        let script_id: i32 = call_with_timeout(&scripting, "loadScript", &(path, plugin)).await?;
        let script_object = format!("/Scripting/Script{script_id}");
        let script_proxy = zbus::Proxy::new(
            &self.connection,
            "org.kde.KWin",
            script_object,
            "org.kde.kwin.Script",
        )
        .await
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to address the loaded KWin script object: {error}"),
            )
        })?;
        let run_result = call_with_timeout::<()>(&script_proxy, "run", &()).await;

        let Some(receiver) = receiver else {
            return run_result.map(|()| None);
        };

        let payload = match run_result {
            Ok(()) => tokio::time::timeout(SCRIPT_TIMEOUT, receiver)
                .await
                .map_err(|_| {
                    BackendError::new(
                        BackendErrorCode::Internal,
                        "timed out waiting for the KWin script result callback",
                    )
                })
                .and_then(|received| {
                    received.map_err(|_| {
                        BackendError::new(
                            BackendErrorCode::Internal,
                            "the KWin script result channel closed before a result arrived",
                        )
                    })
                }),
            Err(error) => Err(error),
        };
        let _ = call_with_timeout::<()>(&script_proxy, "stop", &()).await;
        let _ = call_with_timeout::<i32>(&scripting, "unloadScript", &(plugin,)).await;
        payload.map(Some)
    }

    /// Best-effort cached active window from the persistent watcher script.
    /// Returns `Ok(None)` when the cache cannot be trusted and the caller
    /// should fall back to a one-shot query.
    async fn cached_active_window(&self) -> Result<Option<Option<ActiveWindow>>, BackendError> {
        if !self.ensure_watcher().await {
            return Ok(None);
        }
        Ok(self
            .active_cache
            .lock()
            .ok()
            .and_then(|cache| cache.clone()))
    }

    /// Ensure the persistent focus watcher script is loaded. Returns whether
    /// the watcher is currently live; failures degrade to one-shot queries.
    async fn ensure_watcher(&self) -> bool {
        let _gate = self.watcher_gate.lock().await;
        // Trust a recent liveness check so steady-state cache reads cost no
        // DBus round-trips; a dropped watcher (KWin restart) is re-detected
        // within WATCHER_LIVENESS_TTL.
        if self
            .watcher_verified_at
            .lock()
            .ok()
            .and_then(|verified| *verified)
            .is_some_and(|verified_at| verified_at.elapsed() < WATCHER_LIVENESS_TTL)
        {
            return true;
        }
        let Ok(scripting) = self.scripting_proxy().await else {
            return false;
        };
        match call_with_timeout::<bool>(
            &scripting,
            "isScriptLoaded",
            &(self.watcher_plugin.as_str(),),
        )
        .await
        {
            Ok(true) => {
                self.mark_watcher_verified();
                return true;
            }
            Ok(false) => {}
            Err(_) => return false,
        }
        // The watcher is not loaded (first use, or KWin restarted and dropped
        // it). Any cached state predates this KWin instance.
        if let Ok(mut cache) = self.active_cache.lock() {
            *cache = None;
        }
        let script = watcher_script(&self.bus_name);
        let Ok(path) = write_script_file(&self.watcher_plugin, &script) else {
            return false;
        };
        let path_string = path.display().to_string();
        let loaded = self
            .load_and_run(&self.watcher_plugin, &path_string, None)
            .await
            .is_ok();
        let previous = self
            .watcher_script_path
            .lock()
            .ok()
            .and_then(|mut slot| std::mem::replace(&mut *slot, loaded.then(|| path.clone())));
        if let Some(previous) = previous {
            let _ = fs::remove_file(previous);
        }
        if loaded {
            self.mark_watcher_verified();
        } else {
            let _ = fs::remove_file(&path);
        }
        loaded
    }

    fn mark_watcher_verified(&self) {
        if let Ok(mut verified) = self.watcher_verified_at.lock() {
            *verified = Some(std::time::Instant::now());
        }
    }

    /// One-shot active-window query through a transient script.
    async fn query_active_window(&self) -> Result<Option<ActiveWindow>, BackendError> {
        let payload = self.run_transient(ACTIVE_WINDOW_QUERY_BODY).await?;
        Ok(parse_active_window_payload(&payload))
    }
}

async fn call_with_timeout<R>(
    proxy: &zbus::Proxy<'_>,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> Result<R, BackendError>
where
    R: for<'de> serde::Deserialize<'de> + zbus::zvariant::Type,
{
    tokio::time::timeout(SCRIPT_TIMEOUT, proxy.call::<_, _, R>(method, body))
        .await
        .map_err(|_| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("KWin scripting call {method} timed out"),
            )
        })?
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::ActionUnsupportedForEnvironment,
                format!("KWin scripting call {method} failed: {error}"),
            )
        })
}

fn write_script_file(plugin: &str, contents: &str) -> Result<PathBuf, BackendError> {
    use std::os::unix::fs::OpenOptionsExt as _;

    // Predictable names in a shared temp dir are a symlink-attack surface on
    // multi-user hosts (CWE-377): an unpredictable suffix plus O_EXCL
    // (`create_new`) refuses to follow or truncate a pre-planted path, and
    // 0600 keeps the script private.
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    let mut path = env::temp_dir();
    path.push(format!("{plugin}-{suffix:08x}.js"));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|error| {
            BackendError::new(
                BackendErrorCode::Internal,
                format!("failed to create KWin script file: {error}"),
            )
        })?;
    file.write_all(contents.as_bytes()).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("failed to write KWin script file: {error}"),
        )
    })?;
    Ok(path)
}

/// Shared JS helpers building the active-window JSON payload consumed by
/// `parse_active_window_payload`; prepended to both transient scripts and
/// the persistent watcher so the payload shape has a single producer.
const ACTIVE_WINDOW_PAYLOAD_JS: &str = r#"function activeWindowPayload(w) {
    if (w === null || w === undefined) {
        return "{}";
    }
    return JSON.stringify({
        uuid: String(w.internalId),
        caption: String(w.caption || ""),
        resourceClass: String(w.resourceClass || ""),
        pid: Number(w.pid || 0)
    });
}
function currentActiveWindow() {
    return workspace.activeWindow !== undefined ? workspace.activeWindow : workspace.activeClient;
}"#;

fn send_helper(bus_name: &str, token: &str) -> String {
    format!(
        r#"function send(payload) {{
    callDBus("{bus_name}", "{CALLBACK_PATH}", "{CALLBACK_INTERFACE}", "Result", "{token}", String(payload));
}}
{ACTIVE_WINDOW_PAYLOAD_JS}"#
    )
}

const ACTIVE_WINDOW_QUERY_BODY: &str = "send(activeWindowPayload(currentActiveWindow()));";

fn activation_script_body(uuid: &str) -> String {
    format!(
        r#"var target = "{uuid}";
function normalize(value) {{
    return String(value || "").replace(/[{{}}]/g, "").toLowerCase();
}}
function candidates() {{
    if (typeof workspace.windowList === "function") {{
        return workspace.windowList();
    }}
    if (workspace.stackingOrder) {{
        return workspace.stackingOrder;
    }}
    return [];
}}
var matched = null;
var windows = candidates();
for (var i = 0; i < windows.length; i++) {{
    if (normalize(windows[i].internalId) === target) {{
        matched = windows[i];
        break;
    }}
}}
if (matched === null) {{
    send("no-match");
}} else {{
    if (typeof workspace.activateWindow === "function") {{
        workspace.activateWindow(matched);
    }} else {{
        workspace.activeWindow = matched;
    }}
    if (typeof workspace.raiseWindow === "function") {{
        workspace.raiseWindow(matched);
    }}
    var active = currentActiveWindow();
    if (active !== null && active !== undefined && normalize(active.internalId) === target) {{
        send("verified");
    }} else {{
        send("dispatched");
    }}
}}"#
    )
}

fn watcher_script(bus_name: &str) -> String {
    format!(
        r#"{ACTIVE_WINDOW_PAYLOAD_JS}
function pushActive(w) {{
    callDBus("{bus_name}", "{CALLBACK_PATH}", "{CALLBACK_INTERFACE}", "ActiveWindowChanged", activeWindowPayload(w));
}}
pushActive(currentActiveWindow());
var activatedSignal = workspace.windowActivated !== undefined ? workspace.windowActivated : workspace.clientActivated;
if (activatedSignal !== undefined) {{
    activatedSignal.connect(function (w) {{
        pushActive(w);
    }});
}}"#
    )
}

/// Map an orphaned sky-cua script file name back to its KWin plugin name and
/// owning pid. File names are `<plugin>-<8-hex-suffix>.js` (the suffixless
/// `<plugin>.js` form also occurs in the wild from earlier builds), where
/// `<plugin>` is `sky-cua-focus-watch-<pid>` or `sky-cua-kwin-query-<pid>-<token>`.
fn stale_script_plugin(file_name: &str) -> Option<(String, u32)> {
    let stem = file_name.strip_suffix(".js")?;
    let pid = stem
        .strip_prefix("sky-cua-focus-watch-")
        .or_else(|| stem.strip_prefix("sky-cua-kwin-query-"))?
        .split('-')
        .next()?
        .parse::<u32>()
        .ok()?;
    // Drop the trailing unpredictable file suffix to recover the plugin name.
    // Linux pids cap at 7 digits, so an 8-hex-char tail is never the pid.
    // This strip assumes `write_script_file` always appends its 8-hex suffix
    // (subsec_nanos formatted {:08x} is always exactly 8 chars); if the
    // suffix ever becomes optional, an 8-hex query token could be misparsed
    // as the suffix (harmless: unloadScript of a nonexistent name).
    let plugin = match stem.rsplit_once('-') {
        Some((head, tail))
            if tail.len() == 8 && tail.chars().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            head
        }
        _ => stem,
    };
    Some((plugin.to_string(), pid))
}

fn parse_active_window_payload(payload: &str) -> Option<ActiveWindow> {
    let value: serde_json::Value = serde_json::from_str(payload.trim()).ok()?;
    let uuid = value.get("uuid")?.as_str()?;
    let uuid = normalize_uuid(uuid);
    if uuid.is_empty() {
        return None;
    }
    let non_empty = |key: &str| {
        value
            .get(key)
            .and_then(|entry| entry.as_str())
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(ToOwned::to_owned)
    };
    Some(ActiveWindow {
        uuid,
        caption: non_empty("caption"),
        resource_class: non_empty("resourceClass"),
        pid: value
            .get("pid")
            .and_then(|pid| pid.as_i64())
            .filter(|pid| *pid > 0),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ActiveWindow, activation_script_body, normalize_uuid, parse_active_window_payload,
        send_helper, watcher_script,
    };

    #[test]
    fn normalizes_uuid_tokens() {
        assert_eq!(
            normalize_uuid("{503E2ED7-832F-4E52-B74A-8EDCFAC72BC8}"),
            "503e2ed7-832f-4e52-b74a-8edcfac72bc8"
        );
        assert_eq!(normalize_uuid(" abc "), "abc");
    }

    #[test]
    fn parses_active_window_payload() {
        let parsed = parse_active_window_payload(
            r#"{"uuid": "{503E2ED7-832F-4E52-B74A-8EDCFAC72BC8}", "caption": "TIDAL Hi-Fi", "resourceClass": "tidal-hifi", "pid": 4242}"#,
        );
        assert_eq!(
            parsed,
            Some(ActiveWindow {
                uuid: "503e2ed7-832f-4e52-b74a-8edcfac72bc8".to_string(),
                caption: Some("TIDAL Hi-Fi".to_string()),
                resource_class: Some("tidal-hifi".to_string()),
                pid: Some(4242),
            })
        );
    }

    #[test]
    fn empty_payload_means_no_active_window() {
        assert_eq!(parse_active_window_payload("{}"), None);
        assert_eq!(parse_active_window_payload(""), None);
        assert_eq!(parse_active_window_payload("not-json"), None);
    }

    #[test]
    fn payload_with_zero_pid_and_blank_fields_normalizes_to_none() {
        let parsed = parse_active_window_payload(
            r#"{"uuid": "503e2ed7-832f-4e52-b74a-8edcfac72bc8", "caption": "", "resourceClass": " ", "pid": 0}"#,
        )
        .expect("expected a parsed active window");
        assert_eq!(parsed.caption, None);
        assert_eq!(parsed.resource_class, None);
        assert_eq!(parsed.pid, None);
    }

    #[test]
    fn send_helper_targets_the_callback_object() {
        let helper = send_helper(":1.42", "7");
        assert!(helper.contains(r#"callDBus(":1.42", "/com/skycua/KWinScript", "com.skycua.KWinScript", "Result", "7", String(payload));"#));
    }

    #[test]
    fn activation_script_reports_a_verdict_for_every_branch() {
        let script = activation_script_body("503e2ed7-832f-4e52-b74a-8edcfac72bc8");
        assert!(script.contains(r#"send("no-match");"#));
        assert!(script.contains(r#"send("verified");"#));
        assert!(script.contains(r#"send("dispatched");"#));
    }

    #[test]
    fn stale_script_plugin_recovers_plugin_names_and_pids() {
        assert_eq!(
            super::stale_script_plugin("sky-cua-focus-watch-4242-0abc1234.js"),
            Some(("sky-cua-focus-watch-4242".to_string(), 4242))
        );
        assert_eq!(
            super::stale_script_plugin("sky-cua-focus-watch-4242.js"),
            Some(("sky-cua-focus-watch-4242".to_string(), 4242))
        );
        assert_eq!(
            super::stale_script_plugin("sky-cua-kwin-query-4242-7-0abc1234.js"),
            Some(("sky-cua-kwin-query-4242-7".to_string(), 4242))
        );
        assert_eq!(super::stale_script_plugin("unrelated.js"), None);
        assert_eq!(super::stale_script_plugin("sky-cua-focus-watch-x.js"), None);
    }

    #[test]
    fn watcher_script_pushes_initial_state_and_subscribes() {
        let script = watcher_script(":1.42");
        assert!(script.contains("pushActive(currentActiveWindow())"));
        assert!(script.contains("function activeWindowPayload"));
        assert!(script.contains("workspace.windowActivated"));
        assert!(script.contains("\"ActiveWindowChanged\""));
    }
}
