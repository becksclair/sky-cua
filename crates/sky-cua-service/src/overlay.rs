use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use sky_cua_overlay_host::{
    OVERLAY_HOST_PROTOCOL_VERSION, OverlayHostMessage, OverlayHostMessageKind, OverlayHostReply,
    cursor_asset,
};
use sky_cua_platform::model::{
    ActionName, ActionOutcome, ActionRequest, AgentCursorBackendKind, AgentCursorCapabilities,
    AgentCursorPoint, AgentCursorState, AgentCursorSystemCursorBackendKind, AppStateSnapshot,
    CaptureBackendKind, CaptureInfo, CoordinateSpace, DiagnosticEntry, ElementNode,
    ModelImageFormat, PixelSize, RectF,
};

const AGENT_CURSOR_ENV: &str = "SKY_CUA_AGENT_CURSOR";
const OVERLAY_BACKEND_ENV: &str = "SKY_CUA_OVERLAY_BACKEND";
const OVERLAY_HIDE_FOR_CAPTURE_ENV: &str = "SKY_CUA_OVERLAY_HIDE_FOR_CAPTURE";
const OVERLAY_HOST_PATH_ENV: &str = "SKY_CUA_OVERLAY_HOST_PATH";
const SCREENSHOT_CURSOR_ENV: &str = "SKY_CUA_SCREENSHOT_CURSOR";
const DEFAULT_JPEG_QUALITY: u8 = 85;
const DEFAULT_WEBP_QUALITY: u8 = 75;
const HOST_START_TIMEOUT: Duration = Duration::from_secs(2);
const HOST_CONNECT_INTERVAL: Duration = Duration::from_millis(25);
const HOST_READ_TIMEOUT: Duration = Duration::from_secs(2);
const HOST_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const HOST_STOP_TIMEOUT: Duration = Duration::from_secs(2);
static AGENT_CURSOR_IMAGE: LazyLock<Result<RgbaImage, String>> = LazyLock::new(|| {
    let image = image::load_from_memory(cursor_asset::AGENT_CURSOR_PNG)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    if image.width() != cursor_asset::AGENT_CURSOR_SOURCE_WIDTH
        || image.height() != cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT
    {
        return Err(format!(
            "expected {}x{} cursor asset, got {}x{}",
            cursor_asset::AGENT_CURSOR_SOURCE_WIDTH,
            cursor_asset::AGENT_CURSOR_SOURCE_HEIGHT,
            image.width(),
            image.height()
        ));
    }
    Ok(image::imageops::resize(
        &image,
        cursor_asset::AGENT_CURSOR_WIDTH,
        cursor_asset::AGENT_CURSOR_HEIGHT,
        FilterType::Lanczos3,
    ))
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug)]
pub struct OverlayController {
    state: Option<AgentCursorState>,
    next_sequence: u64,
    agent_cursor_mode: CursorMode,
    hide_for_capture_mode: CursorMode,
    screenshot_cursor_mode: CursorMode,
    host: OverlayHostConnection,
    host_capabilities: Option<AgentCursorCapabilities>,
}

impl Default for OverlayController {
    fn default() -> Self {
        Self::new(Path::new(""))
    }
}

impl OverlayController {
    #[must_use]
    pub fn new(service_socket_path: &Path) -> Self {
        let agent_cursor_mode = mode_from_env(AGENT_CURSOR_ENV);
        Self {
            state: None,
            next_sequence: 1,
            agent_cursor_mode,
            hide_for_capture_mode: mode_from_env(OVERLAY_HIDE_FOR_CAPTURE_ENV),
            screenshot_cursor_mode: mode_from_env(SCREENSHOT_CURSOR_ENV),
            host: OverlayHostConnection::from_service_socket(service_socket_path),
            host_capabilities: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests() -> Self {
        Self {
            state: None,
            next_sequence: 1,
            agent_cursor_mode: CursorMode::Auto,
            hide_for_capture_mode: CursorMode::Auto,
            screenshot_cursor_mode: CursorMode::Auto,
            host: OverlayHostConnection::disabled_for_tests(),
            host_capabilities: None,
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn new_for_tests_with_host(host_path: PathBuf, socket_path: PathBuf) -> Self {
        Self {
            state: None,
            next_sequence: 1,
            agent_cursor_mode: CursorMode::Auto,
            hide_for_capture_mode: CursorMode::Auto,
            screenshot_cursor_mode: CursorMode::Auto,
            host: OverlayHostConnection::process_for_tests(host_path, socket_path),
            host_capabilities: None,
        }
    }

    #[must_use]
    pub fn capabilities(&self) -> AgentCursorCapabilities {
        if self.agent_cursor_mode == CursorMode::Never {
            return AgentCursorCapabilities {
                backend: AgentCursorBackendKind::None,
                visible_overlay: false,
                screenshot_synthetic_cursor: false,
                click_through: false,
                capture_exclusion: false,
                system_cursor_hide_supported: false,
                system_cursor_hidden: false,
                system_cursor_backend: AgentCursorSystemCursorBackendKind::None,
                needs_user_install: false,
                reason: Some(format!("{AGENT_CURSOR_ENV}=never")),
            };
        }

        self.combined_capabilities()
    }

    #[must_use]
    pub fn state(&self) -> Option<AgentCursorState> {
        self.state.clone()
    }

    pub fn set_state(&mut self, state: AgentCursorState) -> AgentCursorStatus {
        if self.agent_cursor_mode == CursorMode::Never {
            self.state = None;
            return self.status_with_diagnostic(diagnostic(
                "AgentCursorDisabled",
                "Agent cursor state was ignored because agent cursor support is disabled.",
                Some(format!("{AGENT_CURSOR_ENV}=never")),
            ));
        }

        let state = self.normalize_state(state);
        self.state = Some(state);
        self.send_host_message(OverlayHostMessageKind::SetCursor, self.state.clone(), None)
    }

    pub fn hide(&mut self, reason: Option<String>) -> AgentCursorStatus {
        if let Some(mut state) = self.state.clone() {
            state.visible = false;
            state.sequence = self.allocate_sequence();
            state.updated_at_ms = now_ms();
            self.state = Some(state);
        }

        let mut status = self.send_host_message(OverlayHostMessageKind::Hide, None, reason.clone());
        if let Some(reason) = reason.filter(|value| !value.trim().is_empty()) {
            status.diagnostics.push(diagnostic(
                "AgentCursorHidden",
                "Agent cursor was hidden.",
                Some(reason),
            ));
        }
        status
    }

    pub fn show(&mut self) -> AgentCursorStatus {
        if let Some(mut state) = self.state.clone() {
            state.visible = true;
            state.sequence = self.allocate_sequence();
            state.updated_at_ms = now_ms();
            self.state = Some(state);
        }
        self.send_host_message(OverlayHostMessageKind::Show, self.state.clone(), None)
    }

    pub fn status(&mut self) -> AgentCursorStatus {
        self.send_host_message(OverlayHostMessageKind::Capabilities, None, None)
    }

    pub fn update_from_action(
        &mut self,
        request: &ActionRequest,
        outcome: &mut ActionOutcome,
    ) -> Vec<DiagnosticEntry> {
        if !outcome.success || self.agent_cursor_mode == CursorMode::Never {
            return Vec::new();
        }

        if let Some(state) = outcome.agent_cursor.clone() {
            let status = self.set_state(state);
            outcome.agent_cursor = status.state;
            return status.diagnostics;
        }

        let Some(state) = state_from_action_request(request) else {
            if cursor_moving_action(&request.action) {
                self.state = None;
                return self
                    .send_host_message(OverlayHostMessageKind::Hide, None, None)
                    .diagnostics;
            }
            return Vec::new();
        };
        let status = self.set_state(state);
        outcome.agent_cursor = status.state;
        status.diagnostics
    }

    pub fn prepare_for_capture(&mut self) -> OverlayCaptureGuard {
        if !self.should_hide_visible_overlay_for_capture() {
            return OverlayCaptureGuard::default();
        }

        let status = self.send_host_message(OverlayHostMessageKind::Hide, None, None);
        OverlayCaptureGuard {
            restore_visible_overlay: true,
            diagnostics: status.diagnostics,
        }
    }

    pub fn restore_after_capture(&mut self, guard: OverlayCaptureGuard) -> Vec<DiagnosticEntry> {
        if !guard.restore_visible_overlay {
            return Vec::new();
        }
        self.send_host_message(OverlayHostMessageKind::Show, self.state.clone(), None)
            .diagnostics
    }

    pub fn apply_to_snapshot(&mut self, snapshot: &mut AppStateSnapshot) {
        snapshot.agent_cursor = self.state();
        if !self.should_synthesize_cursor() {
            return;
        }

        let Some(state) = self.state.as_ref().filter(|state| state.visible) else {
            return;
        };
        let Some(model_point) = state.model_point.as_ref() else {
            return;
        };
        let Some(capture) = snapshot.capture.as_ref() else {
            return;
        };

        match compose_synthetic_cursor(capture, model_point) {
            Ok(Some(updated_capture)) => snapshot.capture = Some(updated_capture),
            Ok(None) => {}
            Err(diagnostic) => snapshot.diagnostics.push(diagnostic),
        }
    }

    fn should_synthesize_cursor(&self) -> bool {
        self.agent_cursor_mode != CursorMode::Never
            && matches!(
                self.screenshot_cursor_mode,
                CursorMode::Auto | CursorMode::Always
            )
    }

    fn should_hide_visible_overlay_for_capture(&self) -> bool {
        if self.agent_cursor_mode == CursorMode::Never
            || self.hide_for_capture_mode == CursorMode::Never
            || !self.state.as_ref().is_some_and(|state| state.visible)
        {
            return false;
        }

        self.hide_for_capture_mode == CursorMode::Always
            || self
                .host_capabilities
                .as_ref()
                .is_some_and(|capabilities| capabilities.visible_overlay)
    }

    fn send_host_message(
        &mut self,
        kind: OverlayHostMessageKind,
        state: Option<AgentCursorState>,
        reason: Option<String>,
    ) -> AgentCursorStatus {
        let mut diagnostics = Vec::new();
        if self.agent_cursor_mode != CursorMode::Never {
            let message = OverlayHostMessage {
                version: OVERLAY_HOST_PROTOCOL_VERSION,
                kind,
                state,
                reason,
            };
            match self.host.send(message) {
                Ok(reply) => {
                    diagnostics.extend(self.apply_host_reply(reply));
                }
                Err(diagnostic) => diagnostics.push(diagnostic),
            }
        }

        AgentCursorStatus {
            capabilities: self.combined_capabilities(),
            state: self.state(),
            diagnostics,
        }
    }

    fn apply_host_reply(&mut self, reply: OverlayHostReply) -> Vec<DiagnosticEntry> {
        let mut diagnostics = reply.diagnostics;
        if let Some(capabilities) = reply.capabilities {
            self.host_capabilities = Some(capabilities);
        }
        if reply.version != OVERLAY_HOST_PROTOCOL_VERSION {
            diagnostics.push(diagnostic(
                "AgentCursorHostProtocolMismatch",
                "Overlay host replied with an incompatible protocol version.",
                Some(format!(
                    "expected={} got={}",
                    OVERLAY_HOST_PROTOCOL_VERSION, reply.version
                )),
            ));
        }
        if !reply.ok && diagnostics.is_empty() {
            diagnostics.push(diagnostic(
                "AgentCursorHostRejected",
                "Overlay host rejected the cursor request.",
                None,
            ));
        }
        diagnostics
    }

    fn combined_capabilities(&self) -> AgentCursorCapabilities {
        let screenshot_synthetic_cursor = self.screenshot_cursor_mode != CursorMode::Never;
        let Some(host_capabilities) = self.host_capabilities.as_ref() else {
            return AgentCursorCapabilities {
                backend: if screenshot_synthetic_cursor {
                    AgentCursorBackendKind::ScreenshotSynthetic
                } else {
                    AgentCursorBackendKind::None
                },
                visible_overlay: false,
                screenshot_synthetic_cursor,
                click_through: false,
                capture_exclusion: false,
                system_cursor_hide_supported: false,
                system_cursor_hidden: false,
                system_cursor_backend: AgentCursorSystemCursorBackendKind::None,
                needs_user_install: false,
                reason: Some(self.host.default_reason()),
            };
        };

        let mut capabilities = host_capabilities.clone();
        capabilities.screenshot_synthetic_cursor = screenshot_synthetic_cursor;
        if !capabilities.visible_overlay && screenshot_synthetic_cursor {
            capabilities.backend = AgentCursorBackendKind::ScreenshotSynthetic;
        }
        capabilities
    }

    fn normalize_state(&mut self, mut state: AgentCursorState) -> AgentCursorState {
        state.sequence = self.allocate_sequence();
        state.updated_at_ms = now_ms();
        state
    }

    fn allocate_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }

    fn status_with_diagnostic(&self, diagnostic: DiagnosticEntry) -> AgentCursorStatus {
        AgentCursorStatus {
            capabilities: self.capabilities(),
            state: self.state(),
            diagnostics: vec![diagnostic],
        }
    }
}

#[derive(Debug)]
enum OverlayHostConnection {
    Disabled {
        reason: String,
        report_diagnostic: bool,
    },
    #[cfg(unix)]
    Process(ProcessOverlayHostClient),
}

impl OverlayHostConnection {
    fn from_service_socket(service_socket_path: &Path) -> Self {
        if overlay_backend_disabled() {
            return Self::Disabled {
                reason: format!("{OVERLAY_BACKEND_ENV}=none"),
                report_diagnostic: false,
            };
        }

        #[cfg(unix)]
        {
            let Some(socket_path) = overlay_socket_path(service_socket_path) else {
                return Self::Disabled {
                    reason: "service socket path has no parent directory".to_string(),
                    report_diagnostic: true,
                };
            };
            Self::Process(ProcessOverlayHostClient::new(
                overlay_host_path(),
                socket_path,
            ))
        }

        #[cfg(not(unix))]
        {
            let _ = service_socket_path;
            Self::Disabled {
                reason: "overlay host process IPC is not implemented on this platform yet"
                    .to_string(),
                report_diagnostic: true,
            }
        }
    }

    #[cfg(test)]
    fn disabled_for_tests() -> Self {
        Self::Disabled {
            reason: "test overlay host disabled".to_string(),
            report_diagnostic: false,
        }
    }

    #[cfg(all(test, unix))]
    fn process_for_tests(host_path: PathBuf, socket_path: PathBuf) -> Self {
        Self::Process(ProcessOverlayHostClient::new(host_path, socket_path))
    }

    fn send(&mut self, message: OverlayHostMessage) -> Result<OverlayHostReply, DiagnosticEntry> {
        match self {
            Self::Disabled {
                reason,
                report_diagnostic,
            } => {
                if *report_diagnostic {
                    Err(diagnostic(
                        "AgentCursorHostUnavailable",
                        "Overlay host is not available.",
                        Some(reason.clone()),
                    ))
                } else {
                    Ok(OverlayHostReply {
                        version: OVERLAY_HOST_PROTOCOL_VERSION,
                        ok: true,
                        capabilities: None,
                        state: None,
                        diagnostics: Vec::new(),
                    })
                }
            }
            #[cfg(unix)]
            Self::Process(client) => client.send(message),
        }
    }

    fn default_reason(&self) -> String {
        match self {
            Self::Disabled { reason, .. } => reason.clone(),
            #[cfg(unix)]
            Self::Process(client) => client
                .last_error
                .clone()
                .unwrap_or_else(|| "native visible overlay host has not reported yet".to_string()),
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct ProcessOverlayHostClient {
    host_path: PathBuf,
    socket_path: PathBuf,
    child: Option<Child>,
    last_error: Option<String>,
}

#[cfg(unix)]
impl ProcessOverlayHostClient {
    fn new(host_path: PathBuf, socket_path: PathBuf) -> Self {
        Self {
            host_path,
            socket_path,
            child: None,
            last_error: None,
        }
    }

    fn send(&mut self, message: OverlayHostMessage) -> Result<OverlayHostReply, DiagnosticEntry> {
        if let Err(error) = self.ensure_running() {
            self.last_error = Some(error.clone());
            return Err(diagnostic(
                "AgentCursorHostUnavailable",
                "Overlay host process is unavailable.",
                Some(error),
            ));
        }

        match self.send_once(&message) {
            Ok(reply) => {
                self.last_error = None;
                Ok(reply)
            }
            Err(error) => {
                self.last_error = Some(error.clone());
                self.reap_or_reset_child();
                Err(diagnostic(
                    "AgentCursorHostRequestFailed",
                    "Overlay host request failed.",
                    Some(error),
                ))
            }
        }
    }

    fn ensure_running(&mut self) -> Result<(), String> {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(status)) => {
                    self.child = None;
                    let _ = fs::remove_file(&self.socket_path);
                    return Err(format!("overlay host exited early with status {status}"));
                }
                Err(error) => return Err(format!("failed to inspect overlay host: {error}")),
            }
        }

        if !self.host_path.is_file() {
            return Err(format!(
                "overlay host binary not found: {}",
                self.host_path.display()
            ));
        }
        if let Some(parent) = self.socket_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create overlay socket directory: {error}"))?;
        }
        let _ = fs::remove_file(&self.socket_path);
        let child = Command::new(&self.host_path)
            .arg("serve")
            .arg("--socket")
            .arg(&self.socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                format!(
                    "failed to spawn overlay host {}: {error}",
                    self.host_path.display()
                )
            })?;
        self.child = Some(child);
        self.wait_for_socket()
    }

    fn wait_for_socket(&mut self) -> Result<(), String> {
        let started = Instant::now();
        let mut last_error = None;
        while started.elapsed() < HOST_START_TIMEOUT {
            if let Some(child) = self.child.as_mut() {
                match child.try_wait() {
                    Ok(None) => {}
                    Ok(Some(status)) => {
                        self.child = None;
                        return Err(format!("overlay host exited during startup with {status}"));
                    }
                    Err(error) => {
                        return Err(format!(
                            "failed to inspect overlay host during startup: {error}"
                        ));
                    }
                }
            }

            match UnixStream::connect(&self.socket_path) {
                Ok(_) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
            std::thread::sleep(HOST_CONNECT_INTERVAL);
        }

        Err(format!(
            "overlay host socket did not become ready at {}{}",
            self.socket_path.display(),
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        ))
    }

    fn send_once(&self, message: &OverlayHostMessage) -> Result<OverlayHostReply, String> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|error| {
            format!(
                "failed to connect to overlay host socket {}: {error}",
                self.socket_path.display()
            )
        })?;
        stream
            .set_read_timeout(Some(HOST_READ_TIMEOUT))
            .map_err(|error| format!("failed to set overlay host read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(HOST_WRITE_TIMEOUT))
            .map_err(|error| format!("failed to set overlay host write timeout: {error}"))?;
        let payload = serde_json::to_vec(message)
            .map_err(|error| format!("failed to serialize overlay host request: {error}"))?;
        stream
            .write_all(&payload)
            .and_then(|()| stream.write_all(b"\n"))
            .and_then(|()| stream.flush())
            .map_err(|error| format!("failed to write overlay host request: {error}"))?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read overlay host reply: {error}"))?;
        if line.trim().is_empty() {
            return Err("overlay host returned an empty reply".to_string());
        }
        serde_json::from_str(line.trim_end())
            .map_err(|error| format!("invalid overlay host reply JSON: {error}"))
    }

    fn reap_or_reset_child(&mut self) {
        if let Some(child) = self.child.as_mut()
            && child.try_wait().ok().flatten().is_some()
        {
            self.child = None;
            let _ = fs::remove_file(&self.socket_path);
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessOverlayHostClient {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let _ = self.send_once(&OverlayHostMessage {
            version: OVERLAY_HOST_PROTOCOL_VERSION,
            kind: OverlayHostMessageKind::Shutdown,
            state: None,
            reason: None,
        });
        let deadline = Instant::now() + HOST_STOP_TIMEOUT;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                let _ = fs::remove_file(&self.socket_path);
                return;
            }
            std::thread::sleep(HOST_CONNECT_INTERVAL);
        }
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_file(&self.socket_path);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentCursorStatus {
    pub capabilities: AgentCursorCapabilities,
    pub state: Option<AgentCursorState>,
    pub diagnostics: Vec<DiagnosticEntry>,
}

#[derive(Debug, Default)]
pub struct OverlayCaptureGuard {
    restore_visible_overlay: bool,
    pub diagnostics: Vec<DiagnosticEntry>,
}

fn mode_from_env(name: &str) -> CursorMode {
    let value = std::env::var(name)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match value.as_str() {
        "always" => CursorMode::Always,
        "never" | "off" | "false" | "0" => CursorMode::Never,
        _ => CursorMode::Auto,
    }
}

fn overlay_backend_disabled() -> bool {
    matches!(
        std::env::var(OVERLAY_BACKEND_ENV)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "none" | "never" | "off" | "false" | "0"
    )
}

#[cfg(unix)]
fn overlay_socket_path(service_socket_path: &Path) -> Option<PathBuf> {
    service_socket_path.parent().map(|parent| {
        if parent.as_os_str().is_empty() {
            PathBuf::from("agent-cursor.sock")
        } else {
            parent.join("agent-cursor.sock")
        }
    })
}

#[cfg(unix)]
fn overlay_host_path() -> PathBuf {
    if let Some(path) = std::env::var_os(OVERLAY_HOST_PATH_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(sibling) = exe_path
            .parent()
            .map(|parent| parent.join(overlay_host_binary_name()))
        && sibling.is_file()
    {
        return sibling;
    }
    let repo_root = std::env::var_os("SKY_CUA_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    repo_root.join("bin").join(overlay_host_binary_name())
}

#[cfg(unix)]
fn overlay_host_binary_name() -> &'static str {
    "sky-cua-overlay-host"
}

fn state_from_action_request(request: &ActionRequest) -> Option<AgentCursorState> {
    let model_point = model_point_for_action(request);
    let native_point = native_point_for_action(request);
    if model_point.is_none() && native_point.is_none() {
        return None;
    }
    Some(AgentCursorState {
        visible: true,
        sequence: 0,
        model_point,
        native_point,
        snapshot_id: request.snapshot_id.clone(),
        source_action: Some(request.action.clone()),
        updated_at_ms: 0,
    })
}

fn cursor_moving_action(action: &ActionName) -> bool {
    matches!(
        action,
        ActionName::Click | ActionName::PerformSecondaryAction | ActionName::Drag
    )
}

fn model_point_for_action(request: &ActionRequest) -> Option<AgentCursorPoint> {
    match request.action {
        ActionName::Click | ActionName::PerformSecondaryAction => {
            explicit_model_point(request, "x", "y")
                .or_else(|| element_model_point(request.resolved_element.as_ref(), request))
        }
        ActionName::Drag => explicit_model_point(request, "to_x", "to_y")
            .or_else(|| element_model_point(request.resolved_target_element.as_ref(), request)),
        _ => None,
    }
}

fn explicit_model_point(
    request: &ActionRequest,
    x_field: &str,
    y_field: &str,
) -> Option<AgentCursorPoint> {
    let x = request.arguments.get(x_field)?.as_f64()?;
    let y = request.arguments.get(y_field)?.as_f64()?;
    let capture = request.resolved_capture.as_ref()?;
    Some(AgentCursorPoint {
        x,
        y,
        coordinate_space: CoordinateSpace::StreamPixels,
        mapping_id: capture.mapping_id.clone(),
    })
}

fn native_point_for_action(request: &ActionRequest) -> Option<AgentCursorPoint> {
    match request.action {
        ActionName::Click | ActionName::PerformSecondaryAction => {
            explicit_native_point(request, "x", "y")
                .or_else(|| element_native_point(request.resolved_element.as_ref(), request))
        }
        ActionName::Drag => explicit_native_point(request, "to_x", "to_y")
            .or_else(|| element_native_point(request.resolved_target_element.as_ref(), request)),
        _ => None,
    }
}

fn explicit_native_point(
    request: &ActionRequest,
    x_field: &str,
    y_field: &str,
) -> Option<AgentCursorPoint> {
    let x = request.arguments.get(x_field)?.as_f64()?;
    let y = request.arguments.get(y_field)?.as_f64()?;
    request
        .resolved_capture
        .as_ref()
        .and_then(|capture| stream_pixels_to_native_point((x, y), capture))
        .or_else(|| {
            request.snapshot_id.is_none().then_some(AgentCursorPoint {
                x,
                y,
                coordinate_space: CoordinateSpace::DesktopLogical,
                mapping_id: None,
            })
        })
}

fn element_model_point(
    element: Option<&ElementNode>,
    request: &ActionRequest,
) -> Option<AgentCursorPoint> {
    let bounds = element?.bounds.as_ref()?;
    let capture = request.resolved_capture.as_ref()?;
    let (x, y) = rect_center(bounds);
    let (x, y) = point_to_stream_pixels((x, y), bounds.space.clone(), capture)?;
    Some(AgentCursorPoint {
        x,
        y,
        coordinate_space: CoordinateSpace::StreamPixels,
        mapping_id: capture.mapping_id.clone(),
    })
}

fn element_native_point(
    element: Option<&ElementNode>,
    request: &ActionRequest,
) -> Option<AgentCursorPoint> {
    let bounds = element?.bounds.as_ref()?;
    let capture = request.resolved_capture.as_ref();
    let (x, y) = rect_center(bounds);
    if let Some(capture) = capture
        && let Some(stream_pixels) = point_to_stream_pixels((x, y), bounds.space.clone(), capture)
        && let Some(native_point) = stream_pixels_to_native_point(stream_pixels, capture)
    {
        return Some(native_point);
    }
    match bounds.space {
        CoordinateSpace::DesktopLogical | CoordinateSpace::StreamLogical => {
            Some(AgentCursorPoint {
                x,
                y,
                coordinate_space: bounds.space.clone(),
                mapping_id: capture.and_then(|capture| capture.mapping_id.clone()),
            })
        }
        CoordinateSpace::StreamPixels => {
            stream_pixels_to_native_point((x, y), capture?).or_else(|| {
                Some(AgentCursorPoint {
                    x,
                    y,
                    coordinate_space: CoordinateSpace::StreamPixels,
                    mapping_id: capture.and_then(|capture| capture.mapping_id.clone()),
                })
            })
        }
    }
}

fn rect_center(bounds: &RectF) -> (f64, f64) {
    (
        bounds.x + (bounds.width / 2.0),
        bounds.y + (bounds.height / 2.0),
    )
}

fn point_to_stream_pixels(
    point: (f64, f64),
    space: CoordinateSpace,
    capture: &CaptureInfo,
) -> Option<(f64, f64)> {
    match space {
        CoordinateSpace::StreamPixels => Some(point),
        CoordinateSpace::DesktopLogical | CoordinateSpace::StreamLogical => {
            let pixel_size = capture.pixel_size.as_ref()?;
            point_to_pixels_through_rect(point, &space, capture.logical_rect.as_ref(), pixel_size)
                .or_else(|| {
                    (space == CoordinateSpace::StreamLogical)
                        .then_some(capture.logical_to_pixel_scale)
                        .flatten()
                        .map(|scale| (point.0 * scale, point.1 * scale))
                })
        }
    }
}

fn stream_pixels_to_native_point(
    point: (f64, f64),
    capture: &CaptureInfo,
) -> Option<AgentCursorPoint> {
    let pixel_size = capture.pixel_size.as_ref()?;
    if pixel_size.width == 0 || pixel_size.height == 0 {
        return None;
    }
    if let Some(logical_rect) = capture
        .logical_rect
        .as_ref()
        .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
    {
        let x = (point.0 / f64::from(pixel_size.width)) * logical_rect.width;
        let y = (point.1 / f64::from(pixel_size.height)) * logical_rect.height;
        if capture.backend == CaptureBackendKind::PortalPipeWire {
            if logical_rect.space == CoordinateSpace::DesktopLogical {
                return Some(AgentCursorPoint {
                    x: logical_rect.x + x,
                    y: logical_rect.y + y,
                    coordinate_space: CoordinateSpace::DesktopLogical,
                    mapping_id: capture.mapping_id.clone(),
                });
            }
            return Some(AgentCursorPoint {
                x,
                y,
                coordinate_space: CoordinateSpace::StreamLogical,
                mapping_id: capture.mapping_id.clone(),
            });
        }
        return Some(AgentCursorPoint {
            x: logical_rect.x + x,
            y: logical_rect.y + y,
            coordinate_space: logical_rect.space.clone(),
            mapping_id: capture.mapping_id.clone(),
        });
    }
    if let Some(scale) = capture
        .logical_to_pixel_scale
        .filter(|scale| scale.is_finite() && *scale > 0.0)
    {
        return Some(AgentCursorPoint {
            x: point.0 / scale,
            y: point.1 / scale,
            coordinate_space: CoordinateSpace::StreamLogical,
            mapping_id: capture.mapping_id.clone(),
        });
    }
    if capture.backend == CaptureBackendKind::X11
        && let Some(original_pixel_size) = capture.original_pixel_size.as_ref()
        && original_pixel_size.width > 0
        && original_pixel_size.height > 0
    {
        return Some(AgentCursorPoint {
            x: (point.0 / f64::from(pixel_size.width)) * f64::from(original_pixel_size.width),
            y: (point.1 / f64::from(pixel_size.height)) * f64::from(original_pixel_size.height),
            coordinate_space: CoordinateSpace::DesktopLogical,
            mapping_id: capture.mapping_id.clone(),
        });
    }
    Some(AgentCursorPoint {
        x: point.0,
        y: point.1,
        coordinate_space: CoordinateSpace::StreamPixels,
        mapping_id: capture.mapping_id.clone(),
    })
}

fn point_to_pixels_through_rect(
    point: (f64, f64),
    point_space: &CoordinateSpace,
    logical_rect: Option<&RectF>,
    pixel_size: &PixelSize,
) -> Option<(f64, f64)> {
    let logical_rect = logical_rect?;
    if &logical_rect.space != point_space || logical_rect.width <= 0.0 || logical_rect.height <= 0.0
    {
        return None;
    }
    let rel_x = (point.0 - logical_rect.x) / logical_rect.width;
    let rel_y = (point.1 - logical_rect.y) / logical_rect.height;
    Some((
        rel_x * f64::from(pixel_size.width),
        rel_y * f64::from(pixel_size.height),
    ))
}

fn compose_synthetic_cursor(
    capture: &CaptureInfo,
    point: &AgentCursorPoint,
) -> Result<Option<CaptureInfo>, DiagnosticEntry> {
    if point.coordinate_space != CoordinateSpace::StreamPixels {
        return Ok(None);
    }

    let Some(screenshot_path) = capture.screenshot_path.as_ref() else {
        return Ok(None);
    };
    let screenshot_path = Path::new(screenshot_path);
    let started = Instant::now();
    let image = image::open(screenshot_path).map_err(|error| {
        diagnostic(
            "AgentCursorSyntheticFailed",
            "Failed to open screenshot for agent cursor compositing.",
            Some(format!("path={} error={error}", screenshot_path.display())),
        )
    })?;
    let (width, height) = image.dimensions();
    let mut rgba = image.to_rgba8();
    let cursor = agent_cursor_image().map_err(|error| {
        diagnostic(
            "AgentCursorSyntheticFailed",
            "Failed to decode bundled agent cursor image.",
            Some(error),
        )
    })?;
    if !draw_cursor_image(&mut rgba, cursor, point.x, point.y) {
        return Err(diagnostic(
            "AgentCursorSyntheticOutOfBounds",
            "Agent cursor point did not overlap the screenshot.",
            Some(format!(
                "point=({}, {}) image={}x{} path={}",
                point.x,
                point.y,
                width,
                height,
                screenshot_path.display()
            )),
        ));
    }

    let format = output_format(capture, screenshot_path);
    let output_path = cursor_output_path(screenshot_path, format.extension());
    encode_cursor_image(&rgba, &output_path, format, capture).map_err(|error| {
        diagnostic(
            "AgentCursorSyntheticFailed",
            "Failed to write agent cursor screenshot.",
            Some(format!("path={} error={error}", output_path.display())),
        )
    })?;

    let mut updated = capture.clone();
    updated.screenshot_path = Some(output_path.display().to_string());
    updated.model_image_bytes = fs::metadata(&output_path)
        .ok()
        .map(|metadata| metadata.len());
    updated.model_image_encode_ms =
        Some(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX));
    if format == CursorImageFormat::Jpeg {
        updated.model_image_format = Some(ModelImageFormat::Jpeg);
    } else if format == CursorImageFormat::Webp {
        updated.model_image_format = Some(ModelImageFormat::Webp);
    }
    Ok(Some(updated))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CursorImageFormat {
    Jpeg,
    Png,
    Webp,
}

impl CursorImageFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
        }
    }
}

fn output_format(capture: &CaptureInfo, path: &Path) -> CursorImageFormat {
    match capture.model_image_format {
        Some(ModelImageFormat::Jpeg) => CursorImageFormat::Jpeg,
        Some(ModelImageFormat::Webp) => CursorImageFormat::Webp,
        None => match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => CursorImageFormat::Png,
            Some("webp") => CursorImageFormat::Webp,
            _ => CursorImageFormat::Jpeg,
        },
    }
}

fn cursor_output_path(path: &Path, extension: &str) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("screenshot");
    parent.join(format!("{stem}.agent-cursor.{extension}"))
}

fn encode_cursor_image(
    rgba: &RgbaImage,
    path: &Path,
    format: CursorImageFormat,
    capture: &CaptureInfo,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);
    match format {
        CursorImageFormat::Jpeg => {
            let rgb = DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
            let quality = capture.model_image_quality.unwrap_or(DEFAULT_JPEG_QUALITY);
            JpegEncoder::new_with_quality(&mut writer, quality).encode_image(&rgb)?;
        }
        CursorImageFormat::Png => {
            DynamicImage::ImageRgba8(rgba.clone()).write_to(&mut writer, ImageFormat::Png)?;
        }
        CursorImageFormat::Webp => {
            let rgb = DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
            let quality = f32::from(capture.model_image_quality.unwrap_or(DEFAULT_WEBP_QUALITY));
            let encoded =
                webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height()).encode(quality);
            writer.write_all(&encoded)?;
        }
    }
    writer.flush()?;
    Ok(())
}

fn agent_cursor_image() -> Result<&'static RgbaImage, String> {
    match AGENT_CURSOR_IMAGE.as_ref() {
        Ok(image) => Ok(image),
        Err(error) => Err(error.clone()),
    }
}

fn draw_cursor_image(destination: &mut RgbaImage, cursor: &RgbaImage, x: f64, y: f64) -> bool {
    if !x.is_finite() || !y.is_finite() {
        return false;
    }

    let left = x.round() as i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_X;
    let top = y.round() as i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_Y;
    let width = i32::try_from(destination.width()).unwrap_or(i32::MAX);
    let height = i32::try_from(destination.height()).unwrap_or(i32::MAX);
    let mut changed = false;

    for source_y in 0..cursor.height() {
        for source_x in 0..cursor.width() {
            let source = *cursor.get_pixel(source_x, source_y);
            if source[3] == 0 {
                continue;
            }
            let px = left + source_x as i32;
            let py = top + source_y as i32;
            if px < 0 || py < 0 || px >= width || py >= height {
                continue;
            }

            blend_pixel(destination.get_pixel_mut(px as u32, py as u32), source);
            changed = true;
        }
    }

    changed
}

fn blend_pixel(destination: &mut Rgba<u8>, source: Rgba<u8>) {
    let alpha = f32::from(source[3]) / 255.0;
    for channel in 0..3 {
        destination[channel] = ((f32::from(source[channel]) * alpha)
            + (f32::from(destination[channel]) * (1.0 - alpha)))
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    destination[3] = 255;
}

fn diagnostic(code: &str, message: &str, details: Option<String>) -> DiagnosticEntry {
    DiagnosticEntry {
        code: code.to_string(),
        message: message.to_string(),
        details,
    }
}

fn now_ms() -> u64 {
    chrono::Utc::now()
        .timestamp_millis()
        .try_into()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{OverlayController, compose_synthetic_cursor, state_from_action_request};
    use image::{ImageBuffer, Rgba};
    use sky_cua_overlay_host::cursor_asset;
    use sky_cua_platform::model::{
        ActionName, ActionOutcome, ActionRequest, CaptureBackendKind, CaptureInfo, CoordinateSpace,
        ElementNode, ModelImageFormat, PixelSize, RectF,
    };
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::process::Command;

    #[test]
    fn set_state_normalizes_sequence_and_timestamps() {
        let mut controller = OverlayController::new_for_tests();
        let status = controller.set_state(synthetic_state(99, 0));

        let state = status.state.expect("cursor state should be stored");
        assert_eq!(state.sequence, 1);
        assert!(state.updated_at_ms > 0);
        assert!(status.capabilities.screenshot_synthetic_cursor);
    }

    #[test]
    fn hide_and_show_toggle_current_state_without_losing_position() {
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(10, 20));

        let hidden = controller.hide(Some("capture".to_string()));
        let hidden_state = hidden.state.expect("hidden state should remain present");
        assert!(!hidden_state.visible);
        assert_eq!(hidden_state.model_point.as_ref().expect("point").x, 10.0);
        assert!(
            hidden
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHidden")
        );

        let shown = controller.show();
        assert!(shown.state.expect("shown state").visible);
    }

    #[cfg(unix)]
    #[test]
    fn host_process_round_trips_cursor_state_over_private_socket() {
        if Command::new("python3").arg("--version").status().is_err() {
            return;
        }

        let dir = unique_temp_dir("host-process");
        let host_path = dir.join("fake-overlay-host.py");
        let socket_path = dir.join("agent-cursor.sock");
        write_fake_overlay_host(&host_path);

        let mut controller =
            OverlayController::new_for_tests_with_host(host_path, socket_path.clone());

        let status = controller.status();
        assert!(status.diagnostics.is_empty());
        assert!(status.capabilities.visible_overlay);
        assert!(status.capabilities.screenshot_synthetic_cursor);

        let set = controller.set_state(synthetic_state(44, 55));
        assert!(set.diagnostics.is_empty());
        assert!(socket_path.exists());
        assert_eq!(set.state.as_ref().expect("state").sequence, 1);
        assert!(set.state.as_ref().expect("state").visible);

        let hidden = controller.hide(Some("capture".to_string()));
        assert!(!hidden.state.as_ref().expect("state").visible);
        assert!(
            hidden
                .diagnostics
                .iter()
                .any(|entry| entry.code == "OverlayCursorHidden")
        );

        let shown = controller.show();
        assert!(shown.state.as_ref().expect("state").visible);

        let guard = controller.prepare_for_capture();
        assert!(guard.restore_visible_overlay);
        assert!(guard.diagnostics.is_empty());
        assert!(
            controller
                .state()
                .expect("service state stays visible")
                .visible
        );
        let restore_diagnostics = controller.restore_after_capture(guard);
        assert!(restore_diagnostics.is_empty());

        drop(controller);
        assert!(!socket_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn host_process_failure_is_diagnostic_not_action_failure() {
        let dir = unique_temp_dir("host-missing");
        let host_path = dir.join("missing-overlay-host");
        let socket_path = dir.join("agent-cursor.sock");
        let mut controller = OverlayController::new_for_tests_with_host(host_path, socket_path);

        let status = controller.set_state(synthetic_state(1, 2));

        assert!(status.state.is_some());
        assert!(
            status
                .diagnostics
                .iter()
                .any(|entry| entry.code == "AgentCursorHostUnavailable")
        );
        assert!(status.capabilities.screenshot_synthetic_cursor);
    }

    #[test]
    fn derives_cursor_state_from_explicit_click_coordinates() {
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        let state = state_from_action_request(&request).expect("cursor state");
        let point = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(point.x, 12.0);
        assert_eq!(point.y, 34.0);
        assert_eq!(point.coordinate_space, CoordinateSpace::StreamPixels);
        assert_eq!(native.x, 12.0);
        assert_eq!(native.y, 34.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(state.source_action, Some(ActionName::Click));
    }

    #[test]
    fn derives_native_cursor_from_bounded_capture_for_visible_overlay() {
        let mut request =
            action_request(ActionName::Click, serde_json::json!({"x": 40.0, "y": 50.0}));
        request.resolved_capture = Some(capture_with_rect(RectF {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        }));

        let state = state_from_action_request(&request).expect("cursor state");
        let model = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(model.x, 40.0);
        assert_eq!(model.y, 50.0);
        assert_eq!(native.x, 120.0);
        assert_eq!(native.y, 75.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(native.mapping_id.as_deref(), Some("mapping"));
    }

    #[test]
    fn derives_x11_native_cursor_from_original_capture_pixels() {
        let mut request = action_request(
            ActionName::Click,
            serde_json::json!({"x": 960.0, "y": 540.0}),
        );
        request.resolved_capture = Some(x11_capture_with_original_size());

        let state = state_from_action_request(&request).expect("cursor state");
        let model = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(model.x, 960.0);
        assert_eq!(model.y, 540.0);
        assert_eq!(native.x, 1280.0);
        assert_eq!(native.y, 720.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
    }

    #[test]
    fn snapshotless_explicit_click_sets_native_only_cursor() {
        let mut request =
            action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        request.snapshot_id = None;
        request.resolved_capture = None;

        let state = state_from_action_request(&request).expect("cursor state");

        assert!(state.model_point.is_none());
        let native = state.native_point.expect("native point");
        assert_eq!(native.x, 12.0);
        assert_eq!(native.y, 34.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(native.mapping_id, None);
    }

    #[test]
    fn derives_element_click_center_in_stream_pixels() {
        let mut request = action_request(ActionName::Click, serde_json::json!({}));
        request.resolved_element = Some(element_with_bounds(RectF {
            x: 110.0,
            y: 70.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::DesktopLogical,
        }));
        request.resolved_capture = Some(capture_with_rect(RectF {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        }));

        let state = state_from_action_request(&request).expect("cursor state");
        let point = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(point.x, 40.0);
        assert_eq!(point.y, 50.0);
        assert_eq!(native.x, 120.0);
        assert_eq!(native.y, 75.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
    }

    #[test]
    fn derives_element_native_cursor_through_stream_logical_capture_scale() {
        let mut request = action_request(ActionName::Click, serde_json::json!({}));
        request.resolved_element = Some(element_with_bounds(RectF {
            x: 10.0,
            y: 15.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::StreamLogical,
        }));
        request.resolved_capture = Some(capture_with_rect_and_scale(
            RectF {
                x: 100.0,
                y: 50.0,
                width: 200.0,
                height: 100.0,
                space: CoordinateSpace::DesktopLogical,
            },
            Some(2.0),
        ));

        let state = state_from_action_request(&request).expect("cursor state");
        let model = state.model_point.expect("model point");
        let native = state.native_point.expect("native point");

        assert_eq!(model.x, 40.0);
        assert_eq!(model.y, 40.0);
        assert_eq!(native.x, 120.0);
        assert_eq!(native.y, 70.0);
        assert_eq!(native.coordinate_space, CoordinateSpace::DesktopLogical);
        assert_eq!(native.mapping_id.as_deref(), Some("mapping"));
    }

    #[test]
    fn derives_drag_cursor_from_target_element() {
        let mut request = action_request(ActionName::Drag, serde_json::json!({}));
        request.resolved_target_element = Some(element_with_bounds(RectF {
            x: 150.0,
            y: 70.0,
            width: 20.0,
            height: 10.0,
            space: CoordinateSpace::DesktopLogical,
        }));
        request.resolved_capture = Some(capture_with_rect(RectF {
            x: 100.0,
            y: 50.0,
            width: 200.0,
            height: 100.0,
            space: CoordinateSpace::DesktopLogical,
        }));

        let point = state_from_action_request(&request)
            .expect("cursor state")
            .model_point
            .expect("model point");

        assert_eq!(point.x, 120.0);
        assert_eq!(point.y, 50.0);
    }

    #[test]
    fn non_pointer_action_does_not_move_cursor() {
        let request = action_request(ActionName::TypeText, serde_json::json!({"text": "hello"}));
        assert!(state_from_action_request(&request).is_none());
    }

    #[test]
    fn update_from_action_attaches_derived_cursor_to_successful_outcome() {
        let mut controller = OverlayController::new_for_tests();
        let request = action_request(ActionName::Click, serde_json::json!({"x": 12.0, "y": 34.0}));
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        controller.update_from_action(&request, &mut outcome);

        let state = outcome.agent_cursor.expect("outcome should carry cursor");
        assert_eq!(state.sequence, 1);
        assert_eq!(controller.state().expect("controller state").sequence, 1);
    }

    #[test]
    fn update_from_unmapped_pointer_action_clears_stale_cursor() {
        let mut controller = OverlayController::new_for_tests();
        controller.set_state(synthetic_state(10, 20));
        let mut request = action_request(ActionName::Click, serde_json::json!({}));
        request.resolved_capture = None;
        let mut outcome = ActionOutcome {
            success: true,
            message: "ok".to_string(),
            code: "Ok".to_string(),
            diagnostics: Vec::new(),
            agent_cursor: None,
        };

        let diagnostics = controller.update_from_action(&request, &mut outcome);

        assert!(diagnostics.is_empty());
        assert!(outcome.agent_cursor.is_none());
        assert!(controller.state().is_none());
    }

    #[test]
    fn composites_chrome_cursor_asset_into_png_screenshot_near_requested_point() {
        let dir = unique_temp_dir("compose-center");
        let path = dir.join("capture.png");
        let image = ImageBuffer::from_pixel(96, 96, Rgba([240u8, 240, 240, 255]));
        image.save(&path).expect("write source image");
        let capture = capture_with_path(&path, None);
        let point = synthetic_point(48.0, 48.0);

        let updated = compose_synthetic_cursor(&capture, &point)
            .expect("composite should succeed")
            .expect("capture should update");

        let output_path = updated.screenshot_path.expect("updated path");
        assert!(output_path.ends_with("capture.agent-cursor.png"));
        let rendered = image::open(&output_path).expect("open output").to_rgba8();
        let black_source_x = 8_i32;
        let black_source_y = 8_i32;
        let black_dest_x = 48_i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_X + black_source_x;
        let black_dest_y = 48_i32 - cursor_asset::AGENT_CURSOR_HOTSPOT_Y + black_source_y;
        assert_eq!(
            rendered.get_pixel(black_dest_x as u32, black_dest_y as u32),
            &Rgba([0u8, 0, 0, 255])
        );
        assert_eq!(rendered.get_pixel(95, 95), &Rgba([240u8, 240, 240, 255]));
        assert!(updated.model_image_bytes.unwrap_or_default() > 0);
    }

    #[test]
    fn composites_chrome_cursor_asset_when_hotspot_is_on_image_edge() {
        let dir = unique_temp_dir("compose-edge");
        let path = dir.join("capture.png");
        ImageBuffer::from_pixel(16, 16, Rgba([240u8, 240, 240, 255]))
            .save(&path)
            .expect("write source image");
        let capture = capture_with_path(&path, None);
        let point = synthetic_point(0.0, 0.0);

        let updated = compose_synthetic_cursor(&capture, &point)
            .expect("edge composite should not fail")
            .expect("capture should update");

        let rendered = image::open(updated.screenshot_path.expect("path"))
            .expect("open output")
            .to_rgba8();
        assert!(
            rendered
                .pixels()
                .any(|pixel| pixel != &Rgba([240u8, 240, 240, 255]))
        );
    }

    #[test]
    fn out_of_bounds_synthetic_point_returns_diagnostic() {
        let dir = unique_temp_dir("compose-oob");
        let path = dir.join("capture.png");
        ImageBuffer::from_pixel(8, 8, Rgba([0u8, 0, 0, 255]))
            .save(&path)
            .expect("write source image");
        let capture = capture_with_path(&path, None);
        let point = synthetic_point(100.0, 100.0);

        let diagnostic = compose_synthetic_cursor(&capture, &point)
            .expect_err("out-of-bounds cursor should produce diagnostic");

        assert_eq!(diagnostic.code, "AgentCursorSyntheticOutOfBounds");
    }

    #[test]
    fn webp_capture_keeps_webp_format_for_cursor_output() {
        let dir = unique_temp_dir("compose-webp");
        let path = dir.join("capture.webp");
        let rgba = ImageBuffer::from_pixel(16, 16, Rgba([0u8, 0, 0, 255]));
        let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
        let encoded = webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height()).encode(75.0);
        std::fs::write(&path, &*encoded).expect("write source webp");
        let capture = capture_with_path(&path, Some(ModelImageFormat::Webp));
        let point = synthetic_point(8.0, 8.0);

        let updated = compose_synthetic_cursor(&capture, &point)
            .expect("webp composite should succeed")
            .expect("capture should update");

        assert!(
            updated
                .screenshot_path
                .expect("path")
                .ends_with("capture.agent-cursor.webp")
        );
        assert_eq!(updated.model_image_format, Some(ModelImageFormat::Webp));
    }

    fn synthetic_state(x: u64, y: u64) -> sky_cua_platform::model::AgentCursorState {
        sky_cua_platform::model::AgentCursorState {
            visible: true,
            sequence: 0,
            model_point: Some(synthetic_point(x as f64, y as f64)),
            native_point: None,
            snapshot_id: Some("snap".to_string()),
            source_action: Some(ActionName::Click),
            updated_at_ms: 0,
        }
    }

    fn synthetic_point(x: f64, y: f64) -> sky_cua_platform::model::AgentCursorPoint {
        sky_cua_platform::model::AgentCursorPoint {
            x,
            y,
            coordinate_space: CoordinateSpace::StreamPixels,
            mapping_id: Some("stream".to_string()),
        }
    }

    fn action_request(action: ActionName, arguments: serde_json::Value) -> ActionRequest {
        ActionRequest {
            action,
            snapshot_id: Some("snap".to_string()),
            element_index: None,
            arguments,
            resolved_element: None,
            resolved_target_element: None,
            resolved_capture: Some(capture_with_rect(RectF {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
                space: CoordinateSpace::DesktopLogical,
            })),
            resolved_focused_app: None,
            environment: None,
        }
    }

    fn element_with_bounds(bounds: RectF) -> ElementNode {
        ElementNode {
            element_index: 0,
            parent_index: None,
            role: "button".to_string(),
            name: Some("OK".to_string()),
            description: None,
            value: None,
            state_flags: vec!["showing".to_string()],
            semantic_actions: vec!["activate".to_string()],
            bounds: Some(bounds),
            backend_ref: None,
        }
    }

    fn capture_with_rect(logical_rect: RectF) -> CaptureInfo {
        capture_with_rect_and_scale(logical_rect, None)
    }

    fn capture_with_rect_and_scale(
        logical_rect: RectF,
        logical_to_pixel_scale: Option<f64>,
    ) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: Some("stream".to_string()),
            source_type: Some(1),
            mapping_id: Some("mapping".to_string()),
            logical_rect: Some(logical_rect),
            pixel_size: Some(PixelSize {
                width: 400,
                height: 200,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale,
            screenshot_path: None,
            original_screenshot_path: None,
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }

    fn capture_with_path(path: &std::path::Path, format: Option<ModelImageFormat>) -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::PortalPipeWire,
            image_backend: Some(CaptureBackendKind::PortalPipeWire),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: Some("mapping".to_string()),
            logical_rect: None,
            pixel_size: Some(PixelSize {
                width: 31,
                height: 31,
            }),
            original_pixel_size: None,
            logical_to_pixel_scale: None,
            screenshot_path: Some(path.display().to_string()),
            original_screenshot_path: None,
            model_image_format: format,
            model_image_quality: Some(85),
            model_image_bytes: None,
            model_image_encode_ms: None,
        }
    }

    fn x11_capture_with_original_size() -> CaptureInfo {
        CaptureInfo {
            backend: CaptureBackendKind::X11,
            image_backend: Some(CaptureBackendKind::X11),
            coordinate_space: Some(CoordinateSpace::StreamPixels),
            stream_id: None,
            source_type: None,
            mapping_id: Some("x11-root".to_string()),
            logical_rect: None,
            pixel_size: Some(PixelSize {
                width: 1920,
                height: 1080,
            }),
            original_pixel_size: Some(PixelSize {
                width: 2560,
                height: 1440,
            }),
            logical_to_pixel_scale: None,
            screenshot_path: Some("/tmp/capture.jpg".to_string()),
            original_screenshot_path: Some("/tmp/capture.png".to_string()),
            model_image_format: Some(ModelImageFormat::Jpeg),
            model_image_quality: Some(85),
            model_image_bytes: Some(1234),
            model_image_encode_ms: Some(7),
        }
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sky-cua-agent-cursor-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[cfg(unix)]
    fn write_fake_overlay_host(path: &std::path::Path) {
        std::fs::write(
            path,
            r#"#!/usr/bin/env python3
import json
import os
import socket
import sys

if len(sys.argv) != 4 or sys.argv[1:3] != ["serve", "--socket"]:
    raise SystemExit(f"unexpected argv: {sys.argv!r}")

socket_path = sys.argv[3]
try:
    os.unlink(socket_path)
except FileNotFoundError:
    pass

server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(socket_path)
server.listen(8)
state = None
capabilities = {
    "backend": "wayland_layer_shell",
    "visible_overlay": True,
    "screenshot_synthetic_cursor": False,
    "click_through": True,
    "capture_exclusion": False,
    "needs_user_install": False,
    "reason": "fake host",
}

while True:
    conn, _ = server.accept()
    with conn:
        data = b""
        while not data.endswith(b"\n"):
            chunk = conn.recv(4096)
            if not chunk:
                break
            data += chunk
        if not data.strip():
            continue
        message = json.loads(data.decode("utf-8"))
        kind = message["kind"]
        diagnostics = []
        if kind == "set_cursor":
            state = message.get("state")
        elif kind == "hide":
            if state is not None:
                state["visible"] = False
            if message.get("reason"):
                diagnostics.append({
                    "code": "OverlayCursorHidden",
                    "message": "Overlay host hid the cursor.",
                    "details": message["reason"],
                })
        elif kind == "show":
            if state is not None:
                state["visible"] = True
        reply = {
            "version": 1,
            "ok": True,
            "capabilities": capabilities,
            "state": state,
            "diagnostics": diagnostics,
        }
        conn.sendall(json.dumps(reply).encode("utf-8") + b"\n")
        if kind == "shutdown":
            break

server.close()
try:
    os.unlink(socket_path)
except FileNotFoundError:
    pass
"#,
        )
        .expect("write fake overlay host");
        let mut permissions = std::fs::metadata(path)
            .expect("fake host metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).expect("chmod fake overlay host");
    }
}
