use std::sync::{Arc, Mutex};

use sky_cua_platform::model::{
    CaptureBackendKind, DoctorCheck, DoctorPlatformReport, DoctorReadiness, DoctorReport,
    DoctorSessionPresenceReport, EnvironmentInfo, InputBackendKind, SemanticBackendKind,
    SessionPresenceIntent, SessionPresenceStatus,
};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows_sys::Win32::System::Power::{
    POWER_REQUEST_TYPE, PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired,
    PowerRequestExecutionRequired, PowerRequestSystemRequired, PowerSetRequest,
};
use windows_sys::Win32::System::SystemServices::POWER_REQUEST_CONTEXT_VERSION;
use windows_sys::Win32::System::Threading::{
    POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
};

const BACKEND_NAME: &str = "windows-power-request";
const POWER_REQUEST_REASON: &str = "sky-cua automation session active";
const UNLOCK_UNSUPPORTED_DETAIL: &str = "Windows does not expose a programmatic unlock API; LockWorkStation has no unlock counterpart and the secure desktop is LocalSystem-only";
const LOCK_STATE_UNREADABLE_DETAIL: &str =
    "Windows session lock state is not exposed to ordinary desktop processes";

pub fn windows_doctor_report(
    environment: EnvironmentInfo,
    session_presence: DoctorSessionPresenceReport,
) -> DoctorReport {
    let checks = vec![
        DoctorCheck {
            name: "semantic_backend".to_string(),
            ok: environment.semantic_backend != SemanticBackendKind::None,
            detail: format!("{:?}", environment.semantic_backend),
        },
        DoctorCheck {
            name: "capture_backend".to_string(),
            ok: environment.capture_backend != CaptureBackendKind::None,
            detail: format!("{:?}", environment.capture_backend),
        },
        DoctorCheck {
            name: "input_backend".to_string(),
            ok: environment.input_backend != InputBackendKind::None,
            detail: format!("{:?}", environment.input_backend),
        },
    ];
    let can_build_accessibility_tree = environment.semantic_backend != SemanticBackendKind::None;
    let can_capture_screen = environment.capture_backend != CaptureBackendKind::None;
    let can_send_input = environment.input_backend != InputBackendKind::None;
    let can_inhibit_presence =
        session_presence.inhibit_lock.ok || session_presence.inhibit_suspend.ok;
    let can_unlock_session = session_presence.unlock.ok && session_presence.lock_state_readable.ok;
    let mut blockers = Vec::new();
    if !can_build_accessibility_tree {
        blockers.push("Windows UI Automation is unavailable".to_string());
    }
    if !can_capture_screen {
        blockers.push("Windows GDI screenshot capture is unavailable".to_string());
    }
    if !can_send_input {
        blockers.push("Windows input injection is unavailable".to_string());
    }
    let recommended_next_step = if blockers.is_empty() {
        "Computer Use core backends are ready.".to_string()
    } else {
        format!("{}.", blockers.join(". "))
    };

    DoctorReport {
        environment: environment.clone(),
        checks,
        readiness: DoctorReadiness {
            can_register_mcp_tools: true,
            can_build_accessibility_tree,
            can_capture_screen,
            can_send_input,
            can_list_windows: false,
            can_target_windows: false,
            can_inhibit_presence,
            can_unlock_session,
            recommended_next_step,
            blockers,
        },
        platform: Some(DoctorPlatformReport {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            session_kind: environment.session_kind.clone(),
            xdg_session_type: environment.xdg_session_type.clone(),
            desktop_environment: environment.desktop_environment.clone(),
            compositor: environment.compositor.clone(),
            display: environment.display.clone(),
            wayland_display: environment.wayland_display.clone(),
        }),
        session_env: None,
        portal: None,
        accessibility: None,
        windowing: None,
        input: None,
        browser_integration: None,
        session_presence: Some(session_presence),
    }
}

#[derive(Clone)]
pub struct SessionPresenceManager {
    inner: Arc<Mutex<SessionPresenceState>>,
    api: Arc<dyn PowerRequestApi>,
}

impl SessionPresenceManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionPresenceState::default())),
            api: Arc::new(NativePowerRequestApi),
        }
    }

    #[cfg(test)]
    fn with_api(api: Arc<dyn PowerRequestApi>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionPresenceState::default())),
            api,
        }
    }

    pub async fn ensure(&self, intent: SessionPresenceIntent) -> SessionPresenceStatus {
        let mut state = self.inner.lock().expect("session presence mutex poisoned");
        let mut details = Vec::new();

        if intent.unlock {
            details.push(format!(
                "unlock requested but skipped: {UNLOCK_UNSUPPORTED_DETAIL}"
            ));
        }

        if intent.inhibit_lock || intent.inhibit_suspend {
            if state.request.is_none() {
                match self.api.create_request() {
                    Ok(handle) => {
                        state.request = Some(HeldPowerRequest::new(handle));
                        details.push("created Windows power request handle".to_string());
                    }
                    Err(error) => details.push(error),
                }
            }

            if let Some(request) = state.request.as_mut() {
                details.extend(request.set(PowerRequestKind::Execution));
                if intent.inhibit_suspend {
                    details.extend(request.set(PowerRequestKind::System));
                }
                if intent.inhibit_lock {
                    details.extend(request.set(PowerRequestKind::Display));
                }
            }
        }

        status_from_state(&state, details)
    }

    pub async fn release(&self, relock: bool) -> SessionPresenceStatus {
        let mut state = self.inner.lock().expect("session presence mutex poisoned");
        let mut details = Vec::new();

        if let Some(request) = state.request.take() {
            details.extend(request.release());
        } else {
            details.push("no Windows power request was held".to_string());
        }

        if relock {
            details.push(format!(
                "relock requested but skipped: {UNLOCK_UNSUPPORTED_DETAIL}"
            ));
        }

        status_from_state(&state, details)
    }

    pub async fn status(&self) -> SessionPresenceStatus {
        let state = self.inner.lock().expect("session presence mutex poisoned");
        status_from_state(&state, Vec::new())
    }

    pub fn doctor_report(&self) -> DoctorSessionPresenceReport {
        let (inhibit_lock, inhibit_suspend) = self.probe_power_request();
        DoctorSessionPresenceReport {
            backend: BACKEND_NAME.to_string(),
            unlock: DoctorCheck {
                name: "unlock".to_string(),
                ok: false,
                detail: UNLOCK_UNSUPPORTED_DETAIL.to_string(),
            },
            inhibit_lock,
            inhibit_suspend,
            lock_state_readable: DoctorCheck {
                name: "lock_state_readable".to_string(),
                ok: false,
                detail: LOCK_STATE_UNREADABLE_DETAIL.to_string(),
            },
        }
    }

    fn probe_power_request(&self) -> (DoctorCheck, DoctorCheck) {
        let mut request = match self.api.create_request() {
            Ok(handle) => HeldPowerRequest::new(handle),
            Err(error) => {
                let detail = format!("PowerCreateRequest failed: {error}");
                return (
                    DoctorCheck {
                        name: "inhibit_lock".to_string(),
                        ok: false,
                        detail: detail.clone(),
                    },
                    DoctorCheck {
                        name: "inhibit_suspend".to_string(),
                        ok: false,
                        detail,
                    },
                );
            }
        };

        let execution_result = request.set(PowerRequestKind::Execution);
        let display_result = request.set(PowerRequestKind::Display);
        let system_result = request.set(PowerRequestKind::System);
        let release_details = request.release();

        let execution_ok = !execution_result
            .iter()
            .any(|detail| detail.contains("failed"));
        let display_ok = !display_result
            .iter()
            .any(|detail| detail.contains("failed"));
        let system_ok = !system_result.iter().any(|detail| detail.contains("failed"));
        let release_detail = detail_string(release_details);

        (
            DoctorCheck {
                name: "inhibit_lock".to_string(),
                ok: execution_ok && display_ok,
                detail: format!(
                    "PowerRequestExecutionRequired={} PowerRequestDisplayRequired={}; {release_detail}",
                    ok_label(execution_ok),
                    ok_label(display_ok)
                ),
            },
            DoctorCheck {
                name: "inhibit_suspend".to_string(),
                ok: execution_ok && system_ok,
                detail: format!(
                    "PowerRequestExecutionRequired={} PowerRequestSystemRequired={}; {release_detail}",
                    ok_label(execution_ok),
                    ok_label(system_ok)
                ),
            },
        )
    }
}

impl Default for SessionPresenceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SessionPresenceManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionPresenceManager")
            .field("backend", &BACKEND_NAME)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct SessionPresenceState {
    request: Option<HeldPowerRequest>,
}

struct HeldPowerRequest {
    handle: Box<dyn PowerRequestHandle>,
    execution_required: bool,
    system_required: bool,
    display_required: bool,
}

impl HeldPowerRequest {
    fn new(handle: Box<dyn PowerRequestHandle>) -> Self {
        Self {
            handle,
            execution_required: false,
            system_required: false,
            display_required: false,
        }
    }

    fn set(&mut self, kind: PowerRequestKind) -> Vec<String> {
        if self.is_set(kind) {
            return Vec::new();
        }

        match self.handle.set_request(kind) {
            Ok(()) => {
                self.set_flag(kind, true);
                vec![format!("set {}", kind.label())]
            }
            Err(error) => vec![error],
        }
    }

    fn release(mut self) -> Vec<String> {
        let mut details = Vec::new();
        details.extend(self.clear(PowerRequestKind::Display));
        details.extend(self.clear(PowerRequestKind::System));
        details.extend(self.clear(PowerRequestKind::Execution));
        match self.handle.close() {
            Ok(()) => details.push("closed Windows power request handle".to_string()),
            Err(error) => details.push(error),
        }
        details
    }

    fn clear(&mut self, kind: PowerRequestKind) -> Vec<String> {
        if !self.is_set(kind) {
            return Vec::new();
        }

        match self.handle.clear_request(kind) {
            Ok(()) => {
                self.set_flag(kind, false);
                vec![format!("cleared {}", kind.label())]
            }
            Err(error) => vec![error],
        }
    }

    fn is_set(&self, kind: PowerRequestKind) -> bool {
        match kind {
            PowerRequestKind::Execution => self.execution_required,
            PowerRequestKind::System => self.system_required,
            PowerRequestKind::Display => self.display_required,
        }
    }

    fn set_flag(&mut self, kind: PowerRequestKind, value: bool) {
        match kind {
            PowerRequestKind::Execution => self.execution_required = value,
            PowerRequestKind::System => self.system_required = value,
            PowerRequestKind::Display => self.display_required = value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerRequestKind {
    Execution,
    System,
    Display,
}

impl PowerRequestKind {
    fn raw(self) -> POWER_REQUEST_TYPE {
        match self {
            Self::Execution => PowerRequestExecutionRequired,
            Self::System => PowerRequestSystemRequired,
            Self::Display => PowerRequestDisplayRequired,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Execution => "PowerRequestExecutionRequired",
            Self::System => "PowerRequestSystemRequired",
            Self::Display => "PowerRequestDisplayRequired",
        }
    }
}

trait PowerRequestApi: Send + Sync {
    fn create_request(&self) -> Result<Box<dyn PowerRequestHandle>, String>;
}

trait PowerRequestHandle: Send {
    fn set_request(&mut self, kind: PowerRequestKind) -> Result<(), String>;
    fn clear_request(&mut self, kind: PowerRequestKind) -> Result<(), String>;
    fn close(&mut self) -> Result<(), String>;
}

struct NativePowerRequestApi;

impl PowerRequestApi for NativePowerRequestApi {
    fn create_request(&self) -> Result<Box<dyn PowerRequestHandle>, String> {
        let mut reason = wide_null(POWER_REQUEST_REASON);
        let context = REASON_CONTEXT {
            Version: POWER_REQUEST_CONTEXT_VERSION,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: reason.as_mut_ptr(),
            },
        };
        let handle = unsafe { PowerCreateRequest(&context) };
        if handle.is_null() {
            return Err(last_windows_error("PowerCreateRequest"));
        }
        Ok(Box::new(NativePowerRequestHandle {
            handle: handle as usize,
        }))
    }
}

struct NativePowerRequestHandle {
    handle: usize,
}

impl PowerRequestHandle for NativePowerRequestHandle {
    fn set_request(&mut self, kind: PowerRequestKind) -> Result<(), String> {
        if self.handle == 0 {
            return Err(format!(
                "{} failed: power request handle is closed",
                kind.label()
            ));
        }
        let ok = unsafe { PowerSetRequest(self.raw(), kind.raw()) };
        if ok == 0 {
            Err(last_windows_error(&format!(
                "PowerSetRequest({})",
                kind.label()
            )))
        } else {
            Ok(())
        }
    }

    fn clear_request(&mut self, kind: PowerRequestKind) -> Result<(), String> {
        if self.handle == 0 {
            return Ok(());
        }
        let ok = unsafe { PowerClearRequest(self.raw(), kind.raw()) };
        if ok == 0 {
            Err(last_windows_error(&format!(
                "PowerClearRequest({})",
                kind.label()
            )))
        } else {
            Ok(())
        }
    }

    fn close(&mut self) -> Result<(), String> {
        if self.handle == 0 {
            return Ok(());
        }
        let ok = unsafe { CloseHandle(self.raw()) };
        self.handle = 0;
        if ok == 0 {
            Err(last_windows_error("CloseHandle(power request)"))
        } else {
            Ok(())
        }
    }
}

impl NativePowerRequestHandle {
    fn raw(&self) -> HANDLE {
        self.handle as HANDLE
    }
}

impl Drop for NativePowerRequestHandle {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn status_from_state(
    state: &SessionPresenceState,
    mut details: Vec<String>,
) -> SessionPresenceStatus {
    if details.is_empty() {
        details.push("Windows power request backend is ready".to_string());
    }
    SessionPresenceStatus {
        backend: BACKEND_NAME.to_string(),
        supported: true,
        unlock_supported: false,
        locked: None,
        lock_inhibited: state
            .request
            .as_ref()
            .is_some_and(|request| request.display_required),
        suspend_inhibited: state
            .request
            .as_ref()
            .is_some_and(|request| request.system_required),
        detail: detail_string(details),
    }
}

fn detail_string(details: Vec<String>) -> String {
    if details.is_empty() {
        "session presence is ready".to_string()
    } else {
        details.join("; ")
    }
}

fn ok_label(ok: bool) -> &'static str {
    if ok { "ok" } else { "failed" }
}

fn last_windows_error(operation: &str) -> String {
    let error = unsafe { GetLastError() };
    format!("{operation} failed with Windows error {error}")
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MockPowerRequestApi {
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl MockPowerRequestApi {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("mock mutex poisoned").clone()
        }
    }

    impl PowerRequestApi for MockPowerRequestApi {
        fn create_request(&self) -> Result<Box<dyn PowerRequestHandle>, String> {
            self.calls
                .lock()
                .expect("mock mutex poisoned")
                .push("create".to_string());
            Ok(Box::new(MockPowerRequestHandle {
                calls: Arc::clone(&self.calls),
                closed: false,
            }))
        }
    }

    struct MockPowerRequestHandle {
        calls: Arc<Mutex<Vec<String>>>,
        closed: bool,
    }

    impl PowerRequestHandle for MockPowerRequestHandle {
        fn set_request(&mut self, kind: PowerRequestKind) -> Result<(), String> {
            self.calls
                .lock()
                .expect("mock mutex poisoned")
                .push(format!("set:{}", kind.label()));
            Ok(())
        }

        fn clear_request(&mut self, kind: PowerRequestKind) -> Result<(), String> {
            self.calls
                .lock()
                .expect("mock mutex poisoned")
                .push(format!("clear:{}", kind.label()));
            Ok(())
        }

        fn close(&mut self) -> Result<(), String> {
            if !self.closed {
                self.calls
                    .lock()
                    .expect("mock mutex poisoned")
                    .push("close".to_string());
                self.closed = true;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn ensure_sets_requested_power_requests_and_skips_unlock() {
        let api = Arc::new(MockPowerRequestApi::default());
        let manager = SessionPresenceManager::with_api(api.clone());

        let status = manager
            .ensure(SessionPresenceIntent {
                unlock: true,
                inhibit_lock: true,
                inhibit_suspend: true,
            })
            .await;

        assert!(status.supported);
        assert!(!status.unlock_supported);
        assert!(status.lock_inhibited);
        assert!(status.suspend_inhibited);
        assert!(status.detail.contains("unlock requested but skipped"));
        assert_eq!(
            api.calls(),
            vec![
                "create",
                "set:PowerRequestExecutionRequired",
                "set:PowerRequestSystemRequired",
                "set:PowerRequestDisplayRequired",
            ]
        );
    }

    #[tokio::test]
    async fn ensure_and_release_are_idempotent() {
        let api = Arc::new(MockPowerRequestApi::default());
        let manager = SessionPresenceManager::with_api(api.clone());
        let intent = SessionPresenceIntent {
            unlock: false,
            inhibit_lock: true,
            inhibit_suspend: true,
        };

        let first = manager.ensure(intent).await;
        let second = manager.ensure(intent).await;
        let released_once = manager.release(false).await;
        let released_twice = manager.release(false).await;

        assert!(first.lock_inhibited);
        assert!(first.suspend_inhibited);
        assert!(second.lock_inhibited);
        assert!(second.suspend_inhibited);
        assert!(!released_once.lock_inhibited);
        assert!(!released_once.suspend_inhibited);
        assert!(!released_twice.lock_inhibited);
        assert!(!released_twice.suspend_inhibited);
        assert_eq!(
            api.calls(),
            vec![
                "create",
                "set:PowerRequestExecutionRequired",
                "set:PowerRequestSystemRequired",
                "set:PowerRequestDisplayRequired",
                "clear:PowerRequestDisplayRequired",
                "clear:PowerRequestSystemRequired",
                "clear:PowerRequestExecutionRequired",
                "close",
            ]
        );
    }
}
