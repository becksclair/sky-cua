#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]

use std::{
    env,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
#[cfg(target_os = "linux")]
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixStream;

use sky_cua_platform::model::AgentCursorSystemCursorBackendKind;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemPointerPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug)]
pub enum SystemCursorAdapter {
    Unsupported(UnsupportedSystemCursorAdapter),
    #[cfg(target_os = "linux")]
    KwinEffect(KwinEffectSystemCursorAdapter),
    #[cfg(target_os = "linux")]
    CosmicTransparentXcursor(CosmicTransparentXcursorAdapter),
    #[cfg(target_os = "linux")]
    Cosmic(CosmicCompBridgeAdapter),
    #[cfg(target_os = "linux")]
    Hyprland(HyprlandSystemCursorAdapter),
}

impl SystemCursorAdapter {
    #[must_use]
    pub fn wayland_client_unsupported(reason: impl Into<String>) -> Self {
        Self::unsupported_with_backend(
            AgentCursorSystemCursorBackendKind::WaylandClientUnsupported,
            reason,
        )
    }

    #[cfg(target_os = "linux")]
    #[must_use]
    pub fn for_wayland_session() -> Self {
        if is_kde_session()
            && let Some(adapter) = KwinEffectSystemCursorAdapter::probe()
        {
            return Self::KwinEffect(adapter);
        }
        if let Some(adapter) = HyprlandSystemCursorAdapter::probe() {
            return Self::Hyprland(adapter);
        }
        if is_cosmic_session() {
            if let Some(adapter) = CosmicTransparentXcursorAdapter::probe() {
                return Self::CosmicTransparentXcursor(adapter);
            }
            if let Some(adapter) = CosmicCompBridgeAdapter::probe() {
                return Self::Cosmic(adapter);
            }
            return Self::unsupported_with_backend(
                AgentCursorSystemCursorBackendKind::CosmicCompBridge,
                "COSMIC cursor bridge is not installed or not reachable; generic Wayland clients cannot hide the compositor cursor globally",
            );
        }
        Self::wayland_client_unsupported(
            "Wayland clients can only hide the pointer for their own pointer focus; the click-through layer-shell overlay has an empty input region, so compositor cursor hiding requires a compositor-specific adapter",
        )
    }

    #[must_use]
    pub fn unsupported_with_backend(
        backend: AgentCursorSystemCursorBackendKind,
        reason: impl Into<String>,
    ) -> Self {
        Self::Unsupported(UnsupportedSystemCursorAdapter::new(backend, reason))
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        match self {
            Self::Unsupported(adapter) => adapter.backend(),
            #[cfg(target_os = "linux")]
            Self::KwinEffect(adapter) => adapter.backend(),
            #[cfg(target_os = "linux")]
            Self::CosmicTransparentXcursor(adapter) => adapter.backend(),
            #[cfg(target_os = "linux")]
            Self::Cosmic(adapter) => adapter.backend(),
            #[cfg(target_os = "linux")]
            Self::Hyprland(adapter) => adapter.backend(),
        }
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        match self {
            Self::Unsupported(adapter) => adapter.supported(),
            #[cfg(target_os = "linux")]
            Self::KwinEffect(adapter) => adapter.supported(),
            #[cfg(target_os = "linux")]
            Self::CosmicTransparentXcursor(adapter) => adapter.supported(),
            #[cfg(target_os = "linux")]
            Self::Cosmic(adapter) => adapter.supported(),
            #[cfg(target_os = "linux")]
            Self::Hyprland(adapter) => adapter.supported(),
        }
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        match self {
            Self::Unsupported(adapter) => adapter.hidden(),
            #[cfg(target_os = "linux")]
            Self::KwinEffect(adapter) => adapter.hidden(),
            #[cfg(target_os = "linux")]
            Self::CosmicTransparentXcursor(adapter) => adapter.hidden(),
            #[cfg(target_os = "linux")]
            Self::Cosmic(adapter) => adapter.hidden(),
            #[cfg(target_os = "linux")]
            Self::Hyprland(adapter) => adapter.hidden(),
        }
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Unsupported(adapter) => adapter.reason(),
            #[cfg(target_os = "linux")]
            Self::KwinEffect(adapter) => adapter.reason(),
            #[cfg(target_os = "linux")]
            Self::CosmicTransparentXcursor(adapter) => adapter.reason(),
            #[cfg(target_os = "linux")]
            Self::Cosmic(adapter) => adapter.reason(),
            #[cfg(target_os = "linux")]
            Self::Hyprland(adapter) => adapter.reason(),
        }
    }

    pub fn set_hidden(&mut self, hidden: bool) -> Result<()> {
        match self {
            Self::Unsupported(adapter) => adapter.set_hidden(hidden),
            #[cfg(target_os = "linux")]
            Self::KwinEffect(adapter) => adapter.set_hidden(hidden),
            #[cfg(target_os = "linux")]
            Self::CosmicTransparentXcursor(adapter) => adapter.set_hidden(hidden),
            #[cfg(target_os = "linux")]
            Self::Cosmic(adapter) => adapter.set_hidden(hidden),
            #[cfg(target_os = "linux")]
            Self::Hyprland(adapter) => adapter.set_hidden(hidden),
        }
    }
}

#[cfg(test)]
impl SystemCursorAdapter {
    #[must_use]
    pub(crate) fn test_kwin_effect(hidden: bool) -> Self {
        Self::KwinEffect(KwinEffectSystemCursorAdapter {
            qdbus: "qdbus6".to_string(),
            hidden,
            reason: "KWin effect cursor shim is loaded for tests".to_string(),
            last_show: None,
        })
    }
}

#[cfg(target_os = "linux")]
fn is_cosmic_session() -> bool {
    [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ]
    .into_iter()
    .filter_map(|name| env::var(name).ok())
    .flat_map(|value| {
        value
            .split([':', ';'])
            .map(|part| part.trim().to_ascii_lowercase())
            .collect::<Vec<_>>()
    })
    .any(|part| part == "cosmic")
}

#[cfg(target_os = "linux")]
fn is_kde_session() -> bool {
    [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ]
    .into_iter()
    .filter_map(|name| env::var(name).ok())
    .flat_map(|value| {
        value
            .split([':', ';'])
            .map(|part| part.trim().to_ascii_lowercase())
            .collect::<Vec<_>>()
    })
    .any(|part| matches!(part.as_str(), "kde" | "plasma"))
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub struct KwinEffectSystemCursorAdapter {
    qdbus: String,
    hidden: bool,
    reason: String,
    last_show: Option<Instant>,
}

#[cfg(target_os = "linux")]
impl KwinEffectSystemCursorAdapter {
    const KWIN_SERVICE: &str = "org.kde.KWin";
    const KWIN_AGENT_CURSOR_PATH: &str = "/com/skycua/AgentCursor";
    const KWIN_AGENT_CURSOR_INTERFACE: &str = "com.skycua.AgentCursor";
    // Re-affirm `Show` at least this often while hidden. The effect's idle-hide
    // failsafe restores the cursor after 8s without shim activity (so a dead
    // host can't strand a hidden cursor); re-affirming on a shorter cadence keeps
    // the effect — and its pointer-position polling that drives the agent-cursor
    // follow — alive for the whole session the overlay is up.
    const REAFFIRM_INTERVAL: Duration = Duration::from_secs(4);

    #[must_use]
    pub fn probe() -> Option<Self> {
        let qdbus = find_qdbus()?;
        let adapter = Self {
            qdbus,
            hidden: false,
            reason: "KWin effect cursor shim is loaded through com.skycua.AgentCursor".to_string(),
            last_show: None,
        };
        adapter
            .call_agent_cursor_method("BuildId", std::iter::empty::<&str>())
            .ok()?;
        Some(adapter)
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        AgentCursorSystemCursorBackendKind::KwinEffect
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        true
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        Some(self.reason.as_str())
    }

    pub fn set_hidden(&mut self, hidden: bool) -> Result<()> {
        if self.hidden == hidden {
            // Already in the requested state. While hidden, the host calls this
            // every render tick; turn those no-op calls into a heartbeat that
            // re-affirms `Show` before the effect's idle-hide failsafe fires, so
            // the effect keeps polling the pointer and the agent-cursor follow
            // does not stall a few seconds into a session.
            if hidden
                && self
                    .last_show
                    .is_none_or(|at| at.elapsed() >= Self::REAFFIRM_INTERVAL)
            {
                self.call_agent_cursor_method("Show", std::iter::empty::<&str>())?;
                self.last_show = Some(Instant::now());
            }
            return Ok(());
        }
        let method = if hidden { "Show" } else { "Hide" };
        self.call_agent_cursor_method(method, std::iter::empty::<&str>())?;
        self.hidden = hidden;
        self.last_show = hidden.then(Instant::now);
        Ok(())
    }

    fn call_agent_cursor_method<I, S>(&self, method: &str, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.call_qdbus(
            Self::KWIN_SERVICE,
            Self::KWIN_AGENT_CURSOR_PATH,
            format!("{}.{method}", Self::KWIN_AGENT_CURSOR_INTERFACE),
            args,
        )
    }

    fn call_qdbus<I, S>(
        &self,
        service: &str,
        object_path: &str,
        method: String,
        args: I,
    ) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        // This runs on the overlay render/tick heartbeat, so a stuck qdbus
        // subprocess must not hang the caller indefinitely.
        const QDBUS_TIMEOUT: Duration = Duration::from_secs(5);

        let mut command = Command::new(&self.qdbus);
        command
            .arg(service)
            .arg(object_path)
            .arg(method)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = spawn_with_timeout(&mut command, QDBUS_TIMEOUT)
            .with_context(|| format!("failed to run {}", self.qdbus))?;
        if !output.status.success() {
            bail!(
                "{} exited with status {}: {}",
                self.qdbus,
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Spawn `command`, waiting up to `timeout` for it to finish. On timeout the
/// child is killed and reaped before returning an error, so a wedged helper
/// never leaks a zombie process or hangs the caller.
fn spawn_with_timeout(command: &mut Command, timeout: Duration) -> Result<std::process::Output> {
    let mut child = command.spawn().context("failed to spawn subprocess")?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return child.wait_with_output().context("collect output"),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                bail!("subprocess timed out after {timeout:?}");
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("failed to wait for subprocess");
            }
        }
    }
}

fn find_qdbus() -> Option<String> {
    ["qdbus6", "qdbus"]
        .into_iter()
        .find(|candidate| command_exists(candidate))
        .map(str::to_string)
}

fn command_exists(command: &str) -> bool {
    env::var_os("PATH").is_some_and(|paths| {
        env::split_paths(&paths).any(|dir| {
            let path = dir.join(command);
            path.is_file()
        })
    })
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct CosmicTransparentXcursorAdapter {
    hidden: bool,
    reason: String,
}

#[cfg(target_os = "linux")]
impl CosmicTransparentXcursorAdapter {
    const THEME_NAME: &str = "sky-cua-blank";

    #[must_use]
    pub fn probe() -> Option<Self> {
        let theme = env::var("XCURSOR_THEME").ok()?;
        if theme != Self::THEME_NAME {
            return None;
        }
        if !blank_xcursor_theme_exists(&theme)
            || !cosmic_comp_uses_blank_xcursor_theme(Self::THEME_NAME)
        {
            return None;
        }
        let reason = format!(
            "COSMIC compositor is running with transparent XCURSOR theme {theme}; native cursor remains transparent for the full session"
        );
        Some(Self {
            hidden: false,
            reason,
        })
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        AgentCursorSystemCursorBackendKind::CosmicTransparentXcursor
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        true
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        Some(self.reason.as_str())
    }

    pub fn set_hidden(&mut self, hidden: bool) -> Result<()> {
        self.hidden = hidden;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn blank_xcursor_theme_exists(theme: &str) -> bool {
    let Some(home) = env::var_os("HOME").map(PathBuf::from) else {
        return false;
    };
    let cursor_dir = home.join(".local/share/icons").join(theme).join("cursors");
    ["left_ptr", "default", "xterm", "hand2"]
        .into_iter()
        .all(|name| cursor_dir.join(name).is_file())
}

#[cfg(target_os = "linux")]
fn cosmic_comp_uses_blank_xcursor_theme(theme: &str) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            fs::read_to_string(path.join("comm"))
                .map(|name| name.trim() == "cosmic-comp")
                .unwrap_or(false)
        })
        .any(|path| {
            fs::read(path.join("environ"))
                .map(|environ| environ_has_xcursor_theme(&environ, theme))
                .unwrap_or(false)
        })
}

#[cfg(target_os = "linux")]
fn environ_has_xcursor_theme(environ: &[u8], theme: &str) -> bool {
    environ
        .split(|byte| *byte == b'\0')
        .any(|entry| entry == format!("XCURSOR_THEME={theme}").as_bytes())
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct CosmicCompBridgeAdapter {
    socket_path: PathBuf,
    bridge_process: Option<Child>,
    supported: bool,
    hidden: bool,
    reason: Option<String>,
}

#[cfg(target_os = "linux")]
impl CosmicCompBridgeAdapter {
    pub fn probe() -> Option<Self> {
        let socket_path = cosmic_bridge_socket_path()?;
        let mut bridge_process = None;
        let status = match cosmic_bridge_request(&socket_path, "status") {
            Ok(status) => status,
            Err(_) => {
                bridge_process = start_cosmic_cursor_bridge(&socket_path).ok();
                let Some(status) = wait_for_cosmic_bridge_status(&socket_path) else {
                    stop_child_process(&mut bridge_process);
                    return None;
                };
                status
            }
        };
        if !status.supported {
            stop_child_process(&mut bridge_process);
            return None;
        }
        Some(Self {
            socket_path,
            bridge_process,
            supported: true,
            hidden: status.hidden,
            reason: Some(status.detail),
        })
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        AgentCursorSystemCursorBackendKind::CosmicCompBridge
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        self.supported
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub fn set_hidden(&mut self, hidden: bool) -> Result<()> {
        if self.hidden == hidden {
            return Ok(());
        }
        let command = if hidden { "hide" } else { "show" };
        let response = cosmic_bridge_request(&self.socket_path, command)?;
        if !response.ok {
            bail!("COSMIC cursor bridge {command} failed: {}", response.detail);
        }
        self.supported = response.supported;
        self.hidden = response.hidden;
        self.reason = Some(response.detail);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for CosmicCompBridgeAdapter {
    fn drop(&mut self) {
        if self.hidden {
            let _ = self.set_hidden(false);
        }
        stop_child_process(&mut self.bridge_process);
    }
}

#[cfg(target_os = "linux")]
fn stop_child_process(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(target_os = "linux")]
fn start_cosmic_cursor_bridge(socket_path: &PathBuf) -> Result<Child> {
    let helper = cosmic_helper_path().context("sky-cua-cosmic-helper binary not found")?;
    Command::new(&helper)
        .arg("cursor-bridge")
        .arg("--socket")
        .arg(socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start {}", helper.display()))
}

#[cfg(target_os = "linux")]
fn wait_for_cosmic_bridge_status(socket_path: &PathBuf) -> Option<CosmicBridgeResponse> {
    for _ in 0..20 {
        if let Ok(status) = cosmic_bridge_request(socket_path, "status") {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(50));
    }
    None
}

#[cfg(target_os = "linux")]
fn cosmic_helper_path() -> Option<PathBuf> {
    for env_name in ["SKY_CUA_COSMIC_HELPER", "CODEX_COMPUTER_USE_COSMIC_HELPER"] {
        if let Some(path) = env::var_os(env_name) {
            let path = PathBuf::from(path);
            if !path.as_os_str().is_empty() {
                return Some(path);
            }
        }
    }
    if let Ok(current_exe) = env::current_exe() {
        let sibling = current_exe.with_file_name("sky-cua-cosmic-helper");
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    command_path("sky-cua-cosmic-helper")
}

#[cfg(target_os = "linux")]
#[derive(Debug, Serialize)]
struct CosmicBridgeRequest<'a> {
    command: &'a str,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Deserialize)]
struct CosmicBridgeResponse {
    ok: bool,
    supported: bool,
    hidden: bool,
    detail: String,
}

#[cfg(target_os = "linux")]
fn cosmic_bridge_request(socket_path: &PathBuf, command: &str) -> Result<CosmicBridgeResponse> {
    const BRIDGE_IO_TIMEOUT: Duration = Duration::from_secs(2);

    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect to {}", socket_path.display()))?;
    stream
        .set_read_timeout(Some(BRIDGE_IO_TIMEOUT))
        .context("configure COSMIC cursor bridge read timeout")?;
    stream
        .set_write_timeout(Some(BRIDGE_IO_TIMEOUT))
        .context("configure COSMIC cursor bridge write timeout")?;
    let request = serde_json::to_vec(&CosmicBridgeRequest { command })
        .context("serialize COSMIC cursor bridge request")?;
    stream
        .write_all(&request)
        .context("failed to write COSMIC cursor bridge request")?;
    stream
        .write_all(b"\n")
        .context("failed to terminate COSMIC cursor bridge request")?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("failed to read COSMIC cursor bridge response")?;
    serde_json::from_str(response.trim()).context("COSMIC cursor bridge returned invalid JSON")
}

#[cfg(target_os = "linux")]
fn cosmic_bridge_socket_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("SKY_CUA_COSMIC_CURSOR_BRIDGE") {
        let path = PathBuf::from(path);
        if path.as_os_str().is_empty() {
            return None;
        }
        return Some(path);
    }
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(runtime_dir).join("sky-cua-cosmic-cursor.sock"))
}

#[derive(Debug)]
pub struct UnsupportedSystemCursorAdapter {
    backend: AgentCursorSystemCursorBackendKind,
    reason: String,
}

impl UnsupportedSystemCursorAdapter {
    #[must_use]
    pub fn new(backend: AgentCursorSystemCursorBackendKind, reason: impl Into<String>) -> Self {
        Self {
            backend,
            reason: reason.into(),
        }
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        self.backend
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        false
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        false
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        Some(self.reason.as_str())
    }

    pub fn set_hidden(&mut self, _hidden: bool) -> Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct HyprlandSystemCursorAdapter {
    previous: Option<bool>,
    hidden: bool,
    reason: Option<String>,
}

#[cfg(target_os = "linux")]
impl HyprlandSystemCursorAdapter {
    #[must_use]
    pub fn probe() -> Option<Self> {
        env::var("HYPRLAND_INSTANCE_SIGNATURE")
            .ok()
            .filter(|value| !value.trim().is_empty())?;
        command_path("hyprctl")?;
        Some(Self {
            previous: None,
            hidden: false,
            reason: None,
        })
    }

    #[must_use]
    pub fn backend(&self) -> AgentCursorSystemCursorBackendKind {
        AgentCursorSystemCursorBackendKind::HyprlandConfig
    }

    #[must_use]
    pub fn supported(&self) -> bool {
        true
    }

    #[must_use]
    pub fn hidden(&self) -> bool {
        self.hidden
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    pub fn set_hidden(&mut self, hidden: bool) -> Result<()> {
        if self.hidden == hidden {
            return Ok(());
        }
        if hidden {
            if self.previous.is_none() {
                self.previous = Some(query_hyprland_cursor_invisible()?);
            }
            set_hyprland_cursor_invisible(true)?;
            self.hidden = true;
            self.reason = Some("Hyprland cursor:invisible is enabled for sky-cua".to_string());
            return Ok(());
        }
        let restore_to = self.previous.take().unwrap_or(false);
        set_hyprland_cursor_invisible(restore_to)?;
        self.hidden = false;
        self.reason = Some(format!(
            "Hyprland cursor:invisible restored to {restore_to}"
        ));
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for HyprlandSystemCursorAdapter {
    fn drop(&mut self) {
        if self.hidden {
            let _ = self.set_hidden(false);
        }
    }
}

// hyprctl runs on the same overlay tick heartbeat as the qdbus calls above,
// so a stuck subprocess must not hang the caller indefinitely.
#[cfg(target_os = "linux")]
const HYPRCTL_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(target_os = "linux")]
fn run_hyprctl(args: &[&str]) -> Result<std::process::Output> {
    let mut command = Command::new("hyprctl");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    spawn_with_timeout(&mut command, HYPRCTL_TIMEOUT)
        .with_context(|| format!("failed to run hyprctl {}", args.join(" ")))
}

#[cfg(target_os = "linux")]
fn query_hyprland_cursor_invisible() -> Result<bool> {
    let output = run_hyprctl(&["getoption", "cursor:invisible", "-j"])?;
    if output.status.success()
        && let Some(value) =
            parse_hyprland_cursor_invisible(&String::from_utf8_lossy(&output.stdout))
    {
        return Ok(value);
    }

    let output = run_hyprctl(&["getoption", "cursor:invisible"])?;
    if !output.status.success() {
        bail!(
            "hyprctl getoption cursor:invisible failed: {}",
            command_detail(&output.stdout, &output.stderr)
        );
    }
    parse_hyprland_cursor_invisible(&String::from_utf8_lossy(&output.stdout))
        .context("hyprctl getoption cursor:invisible output did not contain a boolean value")
}

#[cfg(target_os = "linux")]
fn set_hyprland_cursor_invisible(hidden: bool) -> Result<()> {
    let value = if hidden { "true" } else { "false" };
    let output = run_hyprctl(&["keyword", "cursor:invisible", value])?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "hyprctl keyword cursor:invisible {value} failed: {}",
        command_detail(&output.stdout, &output.stderr)
    )
}

#[cfg(target_os = "linux")]
fn parse_hyprland_cursor_invisible(output: &str) -> Option<bool> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        for key in ["value", "int", "float", "str", "set"] {
            if let Some(parsed) = value.get(key).and_then(json_boolish) {
                return Some(parsed);
            }
        }
        if let Some(parsed) = json_boolish(&value) {
            return Some(parsed);
        }
    }
    for line in output.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if lower.contains("true") || lower.ends_with(": 1") || lower.ends_with(" = 1") {
            return Some(true);
        }
        if lower.contains("false") || lower.ends_with(": 0") || lower.ends_with(" = 0") {
            return Some(false);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn json_boolish(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(value) => Some(*value),
        serde_json::Value::Number(value) => value.as_i64().map(|value| value != 0),
        serde_json::Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn command_path(binary: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|entry| entry.join(binary))
        .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "linux")]
fn command_detail(stdout: &[u8], stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    if stdout.is_empty() {
        "no output".to_string()
    } else {
        stdout
    }
}

#[cfg(test)]
mod tests {
    use super::SystemCursorAdapter;
    #[cfg(target_os = "linux")]
    use super::{environ_has_xcursor_theme, parse_hyprland_cursor_invisible};
    use sky_cua_platform::model::AgentCursorSystemCursorBackendKind;

    #[test]
    fn unsupported_adapter_reports_no_effective_hide() {
        let mut adapter = SystemCursorAdapter::unsupported_with_backend(
            AgentCursorSystemCursorBackendKind::Unsupported,
            "wayland clients cannot hide globally",
        );

        assert_eq!(
            adapter.backend(),
            AgentCursorSystemCursorBackendKind::Unsupported
        );
        assert!(!adapter.supported());
        adapter
            .set_hidden(true)
            .expect("unsupported hide is a no-op");
        assert!(!adapter.hidden());
        assert_eq!(
            adapter.reason(),
            Some("wayland clients cannot hide globally")
        );
    }

    #[test]
    fn wayland_adapter_reports_client_level_limitation() {
        let adapter =
            SystemCursorAdapter::wayland_client_unsupported("layer-shell cannot hide globally");

        assert_eq!(
            adapter.backend(),
            AgentCursorSystemCursorBackendKind::WaylandClientUnsupported
        );
        assert!(!adapter.supported());
        assert!(!adapter.hidden());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_hyprland_cursor_invisible_json_and_text() {
        assert_eq!(
            parse_hyprland_cursor_invisible(r#"{"option":"cursor:invisible","set":true}"#),
            Some(true)
        );
        assert_eq!(
            parse_hyprland_cursor_invisible(r#"{"option":"cursor:invisible","int":0}"#),
            Some(false)
        );
        assert_eq!(
            parse_hyprland_cursor_invisible(r#"{"option":"cursor:invisible","set":true,"int":0}"#),
            Some(false)
        );
        assert_eq!(
            parse_hyprland_cursor_invisible("option cursor:invisible\nint: 1"),
            Some(true)
        );
        assert_eq!(
            parse_hyprland_cursor_invisible("option cursor:invisible\nint: 0"),
            Some(false)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn detects_exact_xcursor_theme_in_proc_environ() {
        assert!(environ_has_xcursor_theme(
            b"USER=skycua\0XCURSOR_THEME=sky-cua-blank\0XCURSOR_SIZE=24\0",
            "sky-cua-blank"
        ));
        assert!(!environ_has_xcursor_theme(
            b"USER=skycua\0XCURSOR_THEME=sky-cua-blankish\0",
            "sky-cua-blank"
        ));
    }
}
